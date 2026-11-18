// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! "Export bookmarks…" and "Import bookmarks from file…": Netscape bookmark files
//! (`bookmarks.html`), through `mercury-import`'s writer and parser.
//!
//! Reading, parsing and writing the file happen on a thread of their own, never on the UI
//! thread. An import goes into a new folder "Imported <date>" under Other Bookmarks, created
//! first, and then through the service's stepwise `import_bookmarks`, chunked by that folder's
//! top-level entries. Each chunk is its own transaction, so a failure part-way leaves what the
//! chunks before it imported, and the summary says how much that was.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_store::{BookmarkId, BookmarkImport, BookmarkRoot};
use slint::ComponentHandle;

use super::import_plan::plan_import;
use super::manager::{self, Manager};
use super::tree;
use crate::controls::file_panel;
use crate::profile_data::bookmarks::FolderRef;
use crate::profile_data::import::{IMPORT_CHUNK, ImportOutcome, ImportProgress, ImportSummary};
use crate::profile_data::{self, ProfileData, ProfileDataError, new_guid, now_ms};

/// Bookmark files larger than this are refused: about eight times the largest real export
/// seen (10,000 bookmarks take about 2 MB).
pub(crate) const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Writes an export: to a temporary file next to `path`, then renamed over it, so a failed
/// write never leaves half a file where the user asked for one. The file gets the usual
/// permissions of a file the user creates (0666 less the umask), never wider.
pub(crate) fn write_export(path: &Path, html: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the export has no file name"))?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".mercury-{}.tmp", std::process::id()));
    let temporary = path.with_file_name(temporary_name);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(html.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// The folder an import from a file goes into.
pub(crate) fn import_folder_name((year, month, day): (i32, u32, u32)) -> String {
    format!("Imported {year:04}-{month:02}-{day:02}")
}

/// The date of `days` since 1970-01-01 (Howard Hinnant's `civil_from_days`).
pub(crate) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

/// Today's date where the user is: the local time zone on Unix, otherwise UTC.
fn today() -> (i32, u32, u32) {
    let now_ms = now_ms();
    #[cfg(unix)]
    {
        let seconds = (now_ms / 1000) as libc::time_t;
        // SAFETY: `tm` is plain data that `localtime_r` fills; both pointers are valid for
        // the call, and `localtime_r` is the thread-safe form.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
            return (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32);
        }
    }
    civil_from_days(now_ms.div_euclid(86_400_000))
}

/// "1,204"
pub(crate) fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn items(count: u64) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{} items", grouped(count))
    }
}

/// What an import from a file did. Items are bookmarks, folders and separators; the store
/// leaves out what it refuses (an address too long, folders nested too deeply).
pub(crate) fn import_summary_text(folder: &str, summary: &ImportSummary) -> String {
    let main = match &summary.outcome {
        ImportOutcome::Completed => {
            format!("Imported {} into “{folder}”.", items(summary.imported))
        }
        ImportOutcome::Cancelled => format!(
            "The import stopped: {} of {} were imported into “{folder}”.",
            grouped(summary.imported),
            items(summary.total)
        ),
        ImportOutcome::Failed(error) => format!(
            "Imported {} of {} into “{folder}”; the rest could not be imported: {error}",
            grouped(summary.imported),
            items(summary.total)
        ),
    };
    match summary.skipped {
        0 => main,
        1 => format!(
            "{main} 1 item was left out: its address is too long or it is nested too deeply."
        ),
        n => format!(
            "{main} {} items were left out: their addresses are too long or they are nested \
             too deeply.",
            grouped(n)
        ),
    }
}

/// Reads and parses a bookmarks file into the store's import form, with fresh GUIDs.
pub(crate) fn read_bookmarks_file(path: &Path) -> Result<(Vec<BookmarkImport>, u64), String> {
    use std::io::Read as _;

    let name = path.display();
    let unreadable = |error: std::io::Error| format!("{name} could not be read: {error}");
    // One read, bounded: no size check that the file could outgrow before it is read.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(unreadable)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!(
            "{name} is larger than {} MB, too large for a bookmarks file.",
            MAX_FILE_BYTES / 1024 / 1024
        ));
    }
    let text = String::from_utf8_lossy(&bytes);
    let nodes = mercury_import::parse_netscape_bookmarks(&text)
        .map_err(|error| format!("{name} is not a bookmarks file: {error}"))?;
    let count = tree::count_imported(&nodes);
    let imports = tree::bookmark_imports(&nodes, &mut new_guid)
        .ok_or("the system's random source is unavailable")?;
    Ok((imports, count))
}

/// What an import into a new folder did. `summary` counts the folders created up front with
/// what the stepwise import stored; `folder` is the new folder, or `None` if nothing was
/// imported and it was removed again.
#[derive(Debug)]
pub(crate) struct FileImportResult {
    pub(crate) summary: ImportSummary,
    pub(crate) folder: Option<BookmarkId>,
}

/// Imports `nodes` into a new folder `name` at the end of Other Bookmarks, in chunks of at
/// most [`IMPORT_CHUNK`] nodes whatever the file's shape (see `import_plan`), keeping the
/// file's order. The folder is removed again if nothing was imported.
pub(crate) fn import_into_new_folder(
    data: &ProfileData,
    name: String,
    nodes: Vec<BookmarkImport>,
    on_progress: impl Fn(ImportProgress) + 'static,
    on_done: impl FnOnce(Result<FileImportResult, ProfileDataError>) + 'static,
) {
    // Each reply carries the handle for the next request (dropped once it has run, so no
    // cycle stays behind): the flow also runs on a service that is not this run's.
    let service = data.clone();
    data.bookmark_root(BookmarkRoot::Other, move |root| {
        let root = match root {
            Ok(root) => root,
            Err(error) => return on_done(Err(error)),
        };
        let next = service.clone();
        service.add_bookmark_folder(root, None, name, move |target| {
            let target = match target {
                Ok(target) => target,
                Err(error) => return on_done(Err(error)),
            };
            let plan = plan_import(target, nodes, IMPORT_CHUNK);
            let (folders, places): (Vec<_>, Vec<_>) = plan.folders.into_iter().unzip();
            let parents = folders
                .iter()
                .map(|folder| folder.parent)
                .collect::<Vec<_>>();
            let sets = plan.sets;
            let service = next.clone();
            next.create_folders(folders, move |created| {
                let ids = match created {
                    Ok(ids) => ids,
                    Err(error) => return on_done(Err(error)),
                };
                let resolve = |parent: FolderRef| match parent {
                    FolderRef::Existing(id) => Some(id),
                    FolderRef::Created(index) => ids.get(index).copied(),
                };
                let sets = sets
                    .into_iter()
                    .filter_map(|(parent, nodes)| Some((resolve(parent)?, nodes)))
                    .collect::<Vec<_>>();
                let places = ids
                    .iter()
                    .zip(&parents)
                    .zip(&places)
                    .filter_map(|((id, parent), place)| Some((*id, resolve(*parent)?, *place)))
                    .collect::<Vec<_>>();
                let created = ids.len() as u64;
                let after = service.clone();
                let _job = service.import_bookmarks(sets, on_progress, move |mut summary| {
                    summary.imported += created;
                    summary.total += created;
                    let finish = after.clone();
                    after.place_folders(places, move |placed| {
                        if let Err(error) = placed {
                            log::warn!("imported folders could not be put in order: {error}");
                        }
                        if summary.imported > 0 {
                            return on_done(Ok(FileImportResult {
                                summary,
                                folder: Some(target),
                            }));
                        }
                        finish.remove_bookmark(target, move |removed| {
                            let folder = match removed {
                                Ok(()) => None,
                                Err(error) => {
                                    log::warn!("an empty import folder stays: {error}");
                                    Some(target)
                                }
                            };
                            on_done(Ok(FileImportResult { summary, folder }));
                        });
                    });
                });
            });
        });
    });
}

pub(crate) fn import_from_file(manager: &Rc<Manager>) {
    if !file_panel::BOOKMARK_PANELS_AVAILABLE {
        manager.set_status("Importing a bookmarks file is not available on this platform yet.");
        return;
    }
    if manager.busy() {
        return;
    }
    file_panel::choose_bookmarks_file(
        manager.window().window(),
        Box::new(|path| {
            if let Some(path) = path {
                read_in_background(path);
            }
        }),
    );
}

fn read_in_background(path: PathBuf) {
    if let Some(manager) = manager::current() {
        manager.set_busy(true);
        manager.set_status(&format!("Reading {}…", path.display()));
    }
    let spawned = std::thread::Builder::new()
        .name("bookmarks file".to_owned())
        .spawn(move || {
            let read = read_bookmarks_file(&path);
            let _queued = slint::invoke_from_event_loop(move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                import_read(read);
            });
        });
    if let Err(error) = spawned {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        import_read(Err(format!("the file could not be read: {error}")));
    }
}

fn finish_busy(manager: &Rc<Manager>, status: &str) {
    manager.set_status(status);
    manager.set_busy(false);
}

fn import_read(read: Result<(Vec<BookmarkImport>, u64), String>) {
    let Some(manager) = manager::current() else {
        return;
    };
    let (imports, count) = match read {
        Ok(read) => read,
        Err(problem) => return finish_busy(&manager, &problem),
    };
    if count == 0 {
        return finish_busy(&manager, "The file has no bookmarks to import.");
    }
    let Some(data) = profile_data::handle() else {
        return finish_busy(&manager, "Bookmarks are not available in this run.");
    };
    let folder = import_folder_name(today());
    manager.set_status(&format!("Importing {} into “{folder}”…", items(count)));
    let progress_folder = folder.clone();
    import_into_new_folder(
        &data,
        folder.clone(),
        imports,
        move |ImportProgress { done, total, .. }| {
            if let Some(manager) = manager::current() {
                manager.set_status(&format!(
                    "Importing into “{progress_folder}”: {} of {}…",
                    grouped(done),
                    items(total)
                ));
            }
        },
        move |result| {
            if let Some(manager) = manager::current() {
                let status = match result {
                    Ok(result) if result.folder.is_none() => format!(
                        "Nothing was imported. {}",
                        import_summary_text(&folder, &result.summary)
                    ),
                    Ok(result) => import_summary_text(&folder, &result.summary),
                    Err(error) => format!("The bookmarks could not be imported: {error}"),
                };
                finish_busy(&manager, &status);
            }
        },
    );
}

pub(crate) fn export_to_file(manager: &Rc<Manager>) {
    if !file_panel::BOOKMARK_PANELS_AVAILABLE {
        manager.set_status("Exporting a bookmarks file is not available on this platform yet.");
        return;
    }
    file_panel::choose_bookmarks_export(
        manager.window().window(),
        "bookmarks.html",
        Box::new(|path| {
            let (Some(path), Some(data)) = (path, profile_data::handle()) else {
                return;
            };
            data.bookmark_trees(move |trees| {
                let Some(manager) = manager::current() else {
                    return;
                };
                let (toolbar, other) = match trees {
                    Ok(trees) => trees,
                    Err(error) => return manager.set_status(&error.to_string()),
                };
                let nodes = tree::export_nodes(&toolbar, &other);
                // The bar's folder itself is not counted.
                let count = tree::count_imported(&nodes).saturating_sub(1);
                manager.set_status(&format!("Writing {}…", path.display()));
                let spawned = std::thread::Builder::new()
                    .name("bookmarks file".to_owned())
                    .spawn(move || {
                        let html = mercury_import::write_netscape_bookmarks(&nodes);
                        let status = match write_export(&path, &html) {
                            Ok(()) => format!("Exported {} to {}.", items(count), path.display()),
                            Err(error) => {
                                format!("{} could not be written: {error}", path.display())
                            }
                        };
                        let _queued = slint::invoke_from_event_loop(move || {
                            let _slint = crate::callback_scope::InSlintCallback::enter();
                            if let Some(manager) = manager::current() {
                                manager.set_status(&status);
                            }
                        });
                    });
                if let Err(error) = spawned {
                    manager.set_status(&format!("The file could not be written: {error}"));
                }
            });
        }),
    );
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::profile_data::ProfileDataError;

    #[test]
    fn the_import_folder_is_named_by_date() {
        assert_eq!(import_folder_name((2026, 11, 6)), "Imported 2026-11-06");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_408), (2025, 11, 16));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn an_export_replaces_the_file_and_leaves_no_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bookmarks.html");
        std::fs::write(&path, "old").unwrap();
        write_export(&path, "<new>").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<new>");
        let names = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["bookmarks.html"]);
        let missing = directory.path().join("no such directory").join("b.html");
        assert!(write_export(&missing, "x").is_err());
    }

    #[test]
    fn a_file_larger_than_the_cap_is_refused_without_reading_it_all() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("big.html");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(
            read_bookmarks_file(&path)
                .unwrap_err()
                .contains("too large")
        );
        assert_eq!(MAX_FILE_BYTES, 16 * 1024 * 1024);
    }

    #[test]
    fn counts_are_grouped() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_204), "1,204");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    fn summary(imported: u64, total: u64, outcome: ImportOutcome) -> ImportSummary {
        ImportSummary {
            imported,
            skipped: 0,
            total,
            resume_from: 0,
            outcome,
            longest_chunk: Duration::ZERO,
        }
    }

    #[test]
    fn the_summary_says_what_was_imported() {
        let folder = "Imported 2026-11-16";
        assert_eq!(
            import_summary_text(folder, &summary(1_204, 1_204, ImportOutcome::Completed)),
            "Imported 1,204 items into “Imported 2026-11-16”."
        );
        assert_eq!(
            import_summary_text(folder, &summary(1, 1, ImportOutcome::Completed)),
            "Imported 1 item into “Imported 2026-11-16”."
        );
        let failed = import_summary_text(
            folder,
            &summary(
                500,
                900,
                ImportOutcome::Failed(ProfileDataError::Unavailable("gone".into())),
            ),
        );
        assert!(
            failed.starts_with("Imported 500 of 900 items into “Imported 2026-11-16”; the rest")
        );
        assert!(failed.contains("gone"));
        let mut skipped = summary(8, 10, ImportOutcome::Completed);
        skipped.skipped = 2;
        assert_eq!(
            import_summary_text(folder, &skipped),
            "Imported 8 items into “Imported 2026-11-16”. 2 items were left out: their \
             addresses are too long or they are nested too deeply."
        );
        skipped.skipped = 1;
        assert!(
            import_summary_text(folder, &skipped).ends_with(
                "1 item was left out: its address is too long or it is nested too deeply."
            )
        );
        assert_eq!(
            import_summary_text(folder, &summary(0, 3, ImportOutcome::Cancelled)),
            "The import stopped: 0 of 3 items were imported into “Imported 2026-11-16”."
        );
    }

    #[test]
    fn a_bookmarks_file_is_read_with_fresh_guids() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bookmarks.html");
        std::fs::write(
            &path,
            "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL><p>\n\
             <DT><H3>Folder</H3>\n<DL><p>\n<DT><A HREF=\"https://a.test/\">A</A>\n</DL><p>\n\
             <DT><A HREF=\"https://b.test/\">B</A>\n</DL><p>\n",
        )
        .unwrap();
        let (imports, count) = read_bookmarks_file(&path).unwrap();
        assert_eq!(count, 3);
        let BookmarkImport::Folder { guid, children, .. } = &imports[0] else {
            panic!("the folder first");
        };
        assert_eq!(guid.len(), 12);
        assert_eq!(children.len(), 1);

        std::fs::write(&path, "just some text").unwrap();
        assert!(
            read_bookmarks_file(&path)
                .unwrap_err()
                .contains("not a bookmarks file")
        );
        assert!(
            read_bookmarks_file(&directory.path().join("missing.html"))
                .unwrap_err()
                .contains("could not be read")
        );
    }
}
