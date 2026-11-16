//! The history window's list: visits newest first, grouped under a heading per local day, and
//! the time ranges of the clear dialog. Pure: the wiring is in `mod.rs`.

use mercury_store::{ClearRange, HistoryEntry};
use url::Url;

use super::days::{clock_label, day_label, local_time, start_of_day};
use super::suggestions::display_url;

/// Visits fetched per page; "Show more" fetches the next.
pub(crate) const PAGE: u32 = 100;

/// One line of the list, before it becomes a Slint `HistoryRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    Day(String),
    Visit {
        /// Index into the entries listed.
        entry: usize,
        time: String,
        title: String,
        url: String,
        /// The host whose visits "Forget this site" deletes; empty when there is none (`file:`).
        host: String,
    },
}

/// The lines for `entries` (newest first) as of `now_ms`.
pub(crate) fn lines(entries: &[HistoryEntry], now_ms: i64) -> Vec<Line> {
    let today = local_time(now_ms).day;
    let yesterday = local_time(start_of_day(now_ms) - 1).day;
    let mut lines = Vec::with_capacity(entries.len() + 8);
    let mut current_day = None;
    for (index, entry) in entries.iter().enumerate() {
        let time = local_time(entry.visited_at_ms);
        if current_day != Some(time.day) {
            current_day = Some(time.day);
            lines.push(Line::Day(day_label(time.day, today, yesterday)));
        }
        let url = display_url(&entry.url);
        lines.push(Line::Visit {
            entry: index,
            time: clock_label(time),
            title: entry
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| url.clone()),
            url,
            host: Url::parse(&entry.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_default(),
        });
    }
    lines
}

/// The clear dialog's time ranges, in the order it lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Period {
    LastHour,
    Today,
    LastSevenDays,
    AllTime,
}

impl Period {
    /// The dialog's index of each choice.
    pub(crate) fn from_index(index: i32) -> Self {
        match index {
            0 => Self::LastHour,
            1 => Self::Today,
            2 => Self::LastSevenDays,
            _ => Self::AllTime,
        }
    }

    /// The visits this period covers, as of `now_ms`. "Today" starts at local midnight;
    /// "last 7 days" is today and the six days before it.
    pub(crate) fn range(self, now_ms: i64) -> ClearRange {
        const HOUR_MS: i64 = 3_600_000;
        match self {
            Self::LastHour => ClearRange::Since {
                from_ms: now_ms - HOUR_MS,
            },
            Self::Today => ClearRange::Since {
                from_ms: start_of_day(now_ms),
            },
            Self::LastSevenDays => {
                let mut start = start_of_day(now_ms);
                for _ in 0..6 {
                    // The start of the day before, whatever its length (daylight saving).
                    start = start_of_day(start - HOUR_MS);
                }
                ClearRange::Since { from_ms: start }
            }
            Self::AllTime => ClearRange::All,
        }
    }
}

#[cfg(test)]
mod tests {
    use mercury_store::{VisitId, VisitTransition};

    use super::*;
    use crate::history_ui::days::LocalTime;

    const NOW: i64 = 1_794_831_334_759;
    const DAY_MS: i64 = 86_400_000;

    fn entry(id: i64, url: &str, title: Option<&str>, at: i64) -> HistoryEntry {
        HistoryEntry {
            visit_id: VisitId(id),
            url: url.to_owned(),
            title: title.map(str::to_owned),
            visited_at_ms: at,
            transition: VisitTransition::Link,
        }
    }

    fn time_of(ms: i64) -> String {
        clock_label(local_time(ms))
    }

    #[test]
    fn visits_are_grouped_under_their_day() {
        let today = start_of_day(NOW);
        let entries = [
            entry(3, "https://a.test/", Some("A"), NOW),
            entry(2, "https://b.test/x", None, today + 1),
            entry(1, "file:///tmp/c.html", Some("C"), today - 1),
            entry(0, "https://d.test/", Some("D"), today - 5 * DAY_MS),
        ];

        let lines = lines(&entries, NOW);

        let older = local_time(today - 5 * DAY_MS);
        let LocalTime { day, .. } = older;
        let older_label = day_label(day, local_time(NOW).day, local_time(today - 1).day);
        assert_eq!(
            lines,
            vec![
                Line::Day("Today".into()),
                Line::Visit {
                    entry: 0,
                    time: time_of(NOW),
                    title: "A".into(),
                    url: "a.test".into(),
                    host: "a.test".into(),
                },
                Line::Visit {
                    entry: 1,
                    time: time_of(today + 1),
                    title: "b.test/x".into(),
                    url: "b.test/x".into(),
                    host: "b.test".into(),
                },
                Line::Day("Yesterday".into()),
                Line::Visit {
                    entry: 2,
                    time: time_of(today - 1),
                    title: "C".into(),
                    url: "file:///tmp/c.html".into(),
                    host: String::new(),
                },
                Line::Day(older_label),
                Line::Visit {
                    entry: 3,
                    time: time_of(today - 5 * DAY_MS),
                    title: "D".into(),
                    url: "d.test".into(),
                    host: "d.test".into(),
                },
            ]
        );
    }

    #[test]
    fn periods_cover_the_hour_the_day_the_week_or_everything() {
        let today = start_of_day(NOW);

        assert_eq!(
            Period::LastHour.range(NOW),
            ClearRange::Since {
                from_ms: NOW - 3_600_000
            }
        );
        assert_eq!(
            Period::Today.range(NOW),
            ClearRange::Since { from_ms: today }
        );
        let ClearRange::Since { from_ms: week } = Period::LastSevenDays.range(NOW) else {
            panic!("a week is a range");
        };
        // Six whole days before today, give or take an hour of daylight saving.
        assert!(
            (today - week - 6 * DAY_MS).abs() <= 3_600_000,
            "{}",
            today - week
        );
        assert_eq!(local_time(week).hour, 0);
        assert_eq!(Period::AllTime.range(NOW), ClearRange::All);
        assert_eq!(Period::from_index(7), Period::AllTime);
    }
}
