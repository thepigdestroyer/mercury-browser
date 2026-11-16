//! What an import did, in the words the window shows.

use mercury_import::ImportError;

/// How the import of one kind (bookmarks or history) ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KindEnd {
    Completed,
    /// The user cancelled, or the browser quit, between chunks.
    Cancelled,
    /// The other browser's files could not be read; nothing was imported.
    Unreadable(String),
    /// Writing failed part-way; what was written before stays.
    Failed(String),
}

/// One kind's counts: bookmarks (not folders or separators) or visits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KindResult {
    /// Entries added to the profile.
    pub(crate) imported: u64,
    /// Entries of the source the profile already held, left out.
    pub(crate) already_present: u64,
    /// Entries the profile cannot store (an address that is too long, say), left out.
    pub(crate) skipped: u64,
    /// Entries the source holds.
    pub(crate) total: u64,
    pub(crate) end: KindEnd,
}

impl KindResult {
    pub(crate) fn unreadable(reason: String) -> Self {
        Self {
            imported: 0,
            already_present: 0,
            skipped: 0,
            total: 0,
            end: KindEnd::Unreadable(reason),
        }
    }

    /// Whether some of it may still be missing because the import stopped part-way.
    pub(crate) fn interrupted(&self) -> bool {
        matches!(self.end, KindEnd::Cancelled | KindEnd::Failed(_))
    }
}

/// The import of one profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileResult {
    /// "Firefox — Default".
    pub(crate) label: String,
    pub(crate) bookmarks: Option<KindResult>,
    pub(crate) history: Option<KindResult>,
}

impl ProfileResult {
    pub(crate) fn interrupted(&self) -> bool {
        self.kinds().any(|(_, result)| result.interrupted())
    }

    /// Whether nothing could be imported because every kind failed.
    pub(crate) fn failed(&self) -> bool {
        self.kinds().all(|(_, result)| {
            result.imported == 0
                && matches!(result.end, KindEnd::Unreadable(_) | KindEnd::Failed(_))
        })
    }

    fn kinds(&self) -> impl Iterator<Item = (Kind, &KindResult)> {
        [
            self.bookmarks
                .as_ref()
                .map(|result| (Kind::Bookmarks, result)),
            self.history.as_ref().map(|result| (Kind::History, result)),
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Bookmarks,
    History,
}

impl Kind {
    fn count(self, count: u64) -> String {
        let (one, many) = match self {
            Kind::Bookmarks => ("bookmark", "bookmarks"),
            Kind::History => ("visit", "visits"),
        };
        format!(
            "{} {}",
            thousands(count),
            if count == 1 { one } else { many }
        )
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Bookmarks => "Bookmarks",
            Kind::History => "History",
        }
    }
}

/// One line of the summary; `problem` lines are shown as warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummaryLine {
    pub(crate) text: String,
    pub(crate) problem: bool,
}

impl SummaryLine {
    fn plain(text: String) -> Self {
        Self {
            text,
            problem: false,
        }
    }

    fn problem(text: String) -> Self {
        Self {
            text,
            problem: true,
        }
    }
}

/// `1234567` as "1,234,567".
pub(crate) fn thousands(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The summary of one profile's import, first line first.
pub(crate) fn summary_lines(result: &ProfileResult) -> Vec<SummaryLine> {
    let mut lines = Vec::new();
    let imported = result
        .kinds()
        .filter(|(_, counts)| counts.imported > 0)
        .map(|(kind, counts)| kind.count(counts.imported))
        .collect::<Vec<_>>();
    let anything_present = result.kinds().any(|(_, counts)| counts.already_present > 0);
    lines.push(match (imported.is_empty(), anything_present) {
        (false, _) => SummaryLine::plain(format!(
            "Imported {} from {}.",
            imported.join(" and "),
            result.label
        )),
        (true, true) => {
            SummaryLine::plain(format!("Nothing new was imported from {}.", result.label))
        }
        (true, false) => {
            SummaryLine::problem(format!("Nothing was imported from {}.", result.label))
        }
    });
    let present = result
        .kinds()
        .filter(|(_, counts)| counts.already_present > 0)
        .map(|(kind, counts)| kind.count(counts.already_present))
        .collect::<Vec<_>>();
    if !present.is_empty() {
        let verb = were(&present, result);
        lines.push(SummaryLine::plain(format!(
            "{} {verb} already in Mercury Browser and {verb} left out.",
            capitalised(&present.join(" and ")),
        )));
    }
    for (kind, counts) in result.kinds() {
        if counts.skipped > 0 {
            lines.push(SummaryLine::problem(format!(
                "{} could not be kept because {} too long.",
                capitalised(&kind.count(counts.skipped)),
                if counts.skipped == 1 {
                    "its address is"
                } else {
                    "their addresses are"
                }
            )));
        }
        match &counts.end {
            KindEnd::Completed => {}
            KindEnd::Cancelled => lines.push(SummaryLine::problem(format!(
                "The import of {} stopped before it finished: {} of {} had been imported. \
                 Import again to add the rest; nothing will be added twice.",
                kind.name().to_lowercase(),
                thousands(counts.imported + counts.already_present),
                thousands(counts.total),
            ))),
            KindEnd::Unreadable(reason) => lines.push(SummaryLine::problem(format!(
                "{} could not be imported: {reason}",
                kind.name()
            ))),
            KindEnd::Failed(reason) => lines.push(SummaryLine::problem(format!(
                "{} stopped part-way: {reason} {} had been imported.",
                kind.name(),
                capitalised(&kind.count(counts.imported)),
            ))),
        }
    }
    lines
}

/// Whether the one "already present" count is singular.
fn single(result: &ProfileResult) -> bool {
    result
        .kinds()
        .filter(|(_, counts)| counts.already_present > 0)
        .all(|(_, counts)| counts.already_present == 1)
}

fn were(present: &[String], result: &ProfileResult) -> &'static str {
    if present.len() == 1 && single(result) {
        "was"
    } else {
        "were"
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Why another browser's files could not be read, for the summary.
pub(crate) fn describe_error(browser: &str, error: &ImportError) -> String {
    match error {
        ImportError::NotFound(_) => "the profile's files are no longer there.".to_owned(),
        ImportError::Locked(_) => format!(
            "{browser} kept writing to its files while they were being read. Quit {browser} \
             and try again."
        ),
        ImportError::TooLarge { .. } => {
            "the file is larger than any real one of its kind, so it was not read.".to_owned()
        }
        ImportError::Io { source, .. } => format!("the files could not be read ({source})."),
        ImportError::Database { .. } => "the database is damaged.".to_owned(),
        ImportError::Malformed(_) => {
            format!("the files are not in a form this version of {browser} is known to write.")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn done(imported: u64) -> KindResult {
        KindResult {
            imported,
            already_present: 0,
            skipped: 0,
            total: imported,
            end: KindEnd::Completed,
        }
    }

    fn texts(result: &ProfileResult) -> Vec<(String, bool)> {
        summary_lines(result)
            .into_iter()
            .map(|line| (line.text, line.problem))
            .collect()
    }

    #[test]
    fn numbers_have_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_204), "1,204");
        assert_eq!(thousands(38_912), "38,912");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn a_completed_import_says_what_it_brought() {
        let result = ProfileResult {
            label: "Firefox — Default".into(),
            bookmarks: Some(done(1_204)),
            history: Some(done(38_912)),
        };
        assert_eq!(
            texts(&result),
            vec![(
                "Imported 1,204 bookmarks and 38,912 visits from Firefox — Default.".into(),
                false
            )]
        );
        assert!(!result.interrupted());

        let one = ProfileResult {
            label: "Chrome — Work".into(),
            bookmarks: Some(done(1)),
            history: None,
        };
        assert_eq!(
            texts(&one),
            vec![("Imported 1 bookmark from Chrome — Work.".into(), false)]
        );
    }

    #[test]
    fn a_second_import_says_what_was_already_there() {
        let result = ProfileResult {
            label: "Firefox — Default".into(),
            bookmarks: Some(KindResult {
                already_present: 12,
                total: 12,
                ..done(0)
            }),
            history: Some(KindResult {
                imported: 3,
                already_present: 1_000,
                total: 1_003,
                ..done(0)
            }),
        };
        assert_eq!(
            texts(&result),
            vec![
                ("Imported 3 visits from Firefox — Default.".into(), false),
                (
                    "12 bookmarks and 1,000 visits were already in Mercury Browser and were \
                     left out."
                        .into(),
                    false
                ),
            ]
        );
    }

    #[test]
    fn skipped_entries_cancellation_and_errors_are_problems_per_kind() {
        let result = ProfileResult {
            label: "Chrome — Default".into(),
            bookmarks: Some(KindResult::unreadable(describe_error(
                "Chrome",
                &ImportError::Malformed("no roots".into()),
            ))),
            history: Some(KindResult {
                imported: 500,
                already_present: 0,
                skipped: 2,
                total: 38_912,
                end: KindEnd::Cancelled,
            }),
        };
        assert!(result.interrupted());
        assert_eq!(
            texts(&result),
            vec![
                ("Imported 500 visits from Chrome — Default.".into(), false),
                (
                    "Bookmarks could not be imported: the files are not in a form this \
                     version of Chrome is known to write."
                        .into(),
                    true
                ),
                (
                    "2 visits could not be kept because their addresses are too long.".into(),
                    true
                ),
                (
                    "The import of history stopped before it finished: 500 of 38,912 had \
                     been imported. Import again to add the rest; nothing will be added twice."
                        .into(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn an_import_that_added_nothing_says_so_first() {
        let again = ProfileResult {
            label: "Chrome — Work".into(),
            bookmarks: Some(KindResult {
                already_present: 3,
                total: 3,
                ..done(0)
            }),
            history: None,
        };
        assert_eq!(
            texts(&again)[0],
            ("Nothing new was imported from Chrome — Work.".into(), false)
        );
        let broken = ProfileResult {
            label: "Chrome — Broken".into(),
            bookmarks: Some(KindResult::unreadable("the database is damaged.".into())),
            history: None,
        };
        assert_eq!(
            texts(&broken),
            vec![
                ("Nothing was imported from Chrome — Broken.".into(), true),
                (
                    "Bookmarks could not be imported: the database is damaged.".into(),
                    true
                ),
            ]
        );
        assert!(broken.failed());
        assert!(!again.failed());
    }

    #[test]
    fn a_locked_profile_names_the_browser_to_quit() {
        let text = describe_error("Firefox", &ImportError::Locked(PathBuf::from("/x")));
        assert_eq!(
            text,
            "Firefox kept writing to its files while they were being read. Quit Firefox and \
             try again."
        );
        let result = ProfileResult {
            label: "Firefox — Default".into(),
            bookmarks: Some(KindResult::unreadable(text.clone())),
            history: Some(KindResult::unreadable(text)),
        };
        assert!(!result.interrupted(), "nothing was written");
        let lines = summary_lines(&result);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|line| line.problem));
    }
}
