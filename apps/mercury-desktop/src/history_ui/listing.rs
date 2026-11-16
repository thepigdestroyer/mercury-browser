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

/// After a history change: the newest `page` visits as listed now (`fresh`), followed by the
/// visits shown before that are older than all of them, so pages fetched with "Show more" stay
/// shown without being read again. Returns the list and whether more can be fetched.
pub(crate) fn merge_refresh(
    shown: &[HistoryEntry],
    fresh: Vec<HistoryEntry>,
    page: usize,
    had_more: bool,
) -> (Vec<HistoryEntry>, bool) {
    let Some(last) = fresh
        .last()
        .filter(|_| fresh.len() >= page)
        .map(HistoryEntry::cursor)
    else {
        return (fresh, false);
    };
    let older = shown
        .iter()
        .filter(|entry| (entry.visited_at_ms, entry.visit_id) < (last.visited_at_ms, last.visit_id))
        .cloned()
        .collect::<Vec<_>>();
    let has_more = if older.is_empty() { true } else { had_more };
    ([fresh, older].concat(), has_more)
}

/// What a deletion removed, to take out of the list at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Removed {
    Visit(mercury_store::VisitId),
    /// Every visit of this host (and of `www.` before it, as the store deletes them).
    Site(String),
}

pub(crate) fn remove_visits(entries: &mut Vec<HistoryEntry>, removed: &Removed) {
    let site = |url: &str, host: &str| {
        Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_lowercase))
            .is_some_and(|candidate| {
                let host = host.to_lowercase();
                let host = host.strip_prefix("www.").unwrap_or(&host);
                candidate.strip_prefix("www.").unwrap_or(&candidate) == host
            })
    };
    entries.retain(|entry| match removed {
        Removed::Visit(id) => entry.visit_id != *id,
        Removed::Site(host) => !site(&entry.url, host),
    });
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
            1 => Self::Today,
            2 => Self::LastSevenDays,
            3 => Self::AllTime,
            // An unknown choice is the least destructive one.
            _ => Self::LastHour,
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
        assert_eq!(Period::from_index(3), Period::AllTime);
        // An unknown choice is the least destructive one.
        assert_eq!(Period::from_index(7), Period::LastHour);
        assert_eq!(Period::from_index(-1), Period::LastHour);
    }

    #[test]
    fn a_refresh_replaces_the_newest_page_and_keeps_older_pages_shown() {
        // Shown: two pages of 3 (ids 6..1), and more after them.
        let shown = (1..=6)
            .rev()
            .map(|id| {
                entry(
                    id,
                    &format!("https://{id}.test/"),
                    None,
                    NOW - (10 - id) * 1000,
                )
            })
            .collect::<Vec<_>>();
        // A new visit (7) and visit 5 deleted: the first page is now 7, 6, 4.
        let fresh = vec![shown[0].clone(), shown[2].clone()];
        let fresh = [vec![entry(7, "https://7.test/", None, NOW)], fresh].concat();

        let (merged, has_more) = merge_refresh(&shown, fresh, 3, true);

        let ids = merged.iter().map(|e| e.visit_id.0).collect::<Vec<_>>();
        assert_eq!(ids, vec![7, 6, 4, 3, 2, 1]);
        assert!(has_more);
    }

    #[test]
    fn a_short_refresh_is_the_whole_list() {
        let shown = vec![
            entry(2, "https://2.test/", None, NOW),
            entry(1, "https://1.test/", None, NOW - 1),
        ];

        let (merged, has_more) = merge_refresh(&shown, vec![shown[1].clone()], 3, true);

        assert_eq!(merged, vec![shown[1].clone()]);
        assert!(!has_more);
    }

    #[test]
    fn deleted_visits_leave_the_list_at_once() {
        let mut shown = vec![
            entry(3, "https://a.test/x", None, NOW),
            entry(2, "https://www.a.test/y", None, NOW - 1),
            entry(1, "https://b.test/", None, NOW - 2),
        ];

        remove_visits(&mut shown, &Removed::Visit(mercury_store::VisitId(1)));
        assert_eq!(shown.len(), 2);
        remove_visits(&mut shown, &Removed::Site("a.test".into()));
        assert!(shown.iter().all(|e| e.url != "https://a.test/x"));
        // `www.` belongs to the site too, as the store deletes it.
        assert!(shown.is_empty());
    }
}
