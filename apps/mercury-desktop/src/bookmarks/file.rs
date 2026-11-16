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

use mercury_store::{BookmarkImport, BookmarkRoot};

use super::manager::{self, Manager};
use super::panel;
use super::tree;
use crate::profile_data::import::{ImportOutcome, ImportProgress, ImportSummary};
use crate::profile_data::{self, new_guid, now_ms};

/// Bookmark files larger than this are refused before they are read: far larger than any
/// real export (10,000 bookmarks take about 2 MB).
pub(crate) const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

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
    let name = path.display();
    let size = std::fs::metadata(path)
        .map_err(|error| format!("{name} could not be read: {error}"))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "{name} is larger than {} MB, too large for a bookmarks file.",
            MAX_FILE_BYTES / 1024 / 1024
        ));
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("{name} could not be read: {error}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let nodes = mercury_import::parse_netscape_bookmarks(&text)
        .map_err(|error| format!("{name} is not a bookmarks file: {error}"))?;
    let count = tree::count_imported(&nodes);
    let imports = tree::bookmark_imports(&nodes, &mut new_guid)
        .ok_or("the system's random source is unavailable")?;
    Ok((imports, count))
}

pub(crate) fn import_from_file(manager: &Rc<Manager>) {
    if !panel::AVAILABLE {
        manager.set_status("Importing a bookmarks file is not available on this platform yet.");
        return;
    }
    if manager.busy.get() {
        return;
    }
    panel::choose_file_to_open(
        manager.window(),
        Box::new(|path| {
            if let Some(path) = path {
                read_in_background(path);
            }
        }),
    );
}

fn read_in_background(path: PathBuf) {
    if let Some(manager) = manager::current() {
        manager.busy.set(true);
        manager.window().set_busy(true);
        manager.set_status(&format!("Reading {}…", path.display()));
    }
    let spawned = std::thread::Builder::new()
        .name("bookmarks file".to_owned())
        .spawn(move || {
            let read = read_bookmarks_file(&path);
            let _queued = slint::invoke_from_event_loop(move || import_read(read));
        });
    if let Err(error) = spawned {
        import_read(Err(format!("the file could not be read: {error}")));
    }
}

fn finish_busy(manager: &Manager, status: &str) {
    manager.busy.set(false);
    manager.window().set_busy(false);
    manager.set_status(status);
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
    data.bookmark_root(BookmarkRoot::Other, move |root| {
        let (Some(manager), Some(data)) = (manager::current(), profile_data::handle()) else {
            return;
        };
        let root = match root {
            Ok(root) => root,
            Err(error) => return finish_busy(&manager, &error.to_string()),
        };
        let name = folder.clone();
        data.add_bookmark_folder(root, None, name, move |created| {
            let (Some(manager), Some(data)) = (manager::current(), profile_data::handle()) else {
                return;
            };
            let target = match created {
                Ok(target) => target,
                Err(error) => return finish_busy(&manager, &error.to_string()),
            };
            let progress_folder = folder.clone();
            let _job = data.import_bookmarks(
                vec![(target, imports)],
                move |ImportProgress { done, total, .. }| {
                    if let Some(manager) = manager::current() {
                        manager.set_status(&format!(
                            "Importing into “{progress_folder}”: {} of {}…",
                            grouped(done),
                            items(total)
                        ));
                    }
                },
                move |summary| {
                    if let Some(manager) = manager::current() {
                        finish_busy(&manager, &import_summary_text(&folder, &summary));
                    }
                },
            );
        });
    });
}

pub(crate) fn export_to_file(manager: &Rc<Manager>) {
    if !panel::AVAILABLE {
        manager.set_status("Exporting a bookmarks file is not available on this platform yet.");
        return;
    }
    panel::choose_file_to_save(
        manager.window(),
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
                        let status = match std::fs::write(&path, html) {
                            Ok(()) => format!("Exported {} to {}.", items(count), path.display()),
                            Err(error) => {
                                format!("{} could not be written: {error}", path.display())
                            }
                        };
                        let _queued = slint::invoke_from_event_loop(move || {
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
