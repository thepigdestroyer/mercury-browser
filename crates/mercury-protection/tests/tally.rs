// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `WeeklyTally`: blocked counts per UTC day, with a fake clock (MH14).

use mercury_protection::{Category, TALLY_DAYS_KEPT, WeeklyTally};

const DAY: i64 = 86_400_000;
const T0: i64 = 40 * DAY;

fn ads(tally: &mut WeeklyTally, now: i64, n: u64) {
    tally.record(now, Category::AdsAndTrackers, n);
}

#[test]
fn day_of_floors_and_handles_negative_milliseconds() {
    assert_eq!(WeeklyTally::day_of(0), 0);
    assert_eq!(WeeklyTally::day_of(86_399_999), 0);
    assert_eq!(WeeklyTally::day_of(86_400_000), 1);
    assert_eq!(WeeklyTally::day_of(-1), -1);
    assert_eq!(WeeklyTally::day_of(-86_400_000), -1);
    assert_eq!(WeeklyTally::day_of(-86_400_001), -2);
}

#[test]
fn events_at_the_day_edges_land_in_adjacent_days() {
    let mut tally = WeeklyTally::default();
    ads(&mut tally, 40 * DAY - 1, 1);
    ads(&mut tally, 40 * DAY, 10);

    let before = tally.week(40 * DAY - 1);
    assert_eq!(before.per_day, [0, 0, 0, 0, 0, 0, 1]);
    let after = tally.week(40 * DAY);
    assert_eq!(after.per_day, [0, 0, 0, 0, 0, 1, 10]);
}

#[test]
fn week_sums_today_and_the_six_days_before_and_excludes_day_minus_seven() {
    let mut tally = WeeklyTally::default();
    let today = T0;
    ads(&mut tally, today, 1);
    ads(&mut tally, today - 6 * DAY, 2);
    ads(&mut tally, today - 7 * DAY, 100);
    tally.record(today, Category::Cryptominers, 4);

    let week = tally.week(today);
    assert_eq!(week.total, 1 + 2 + 4);
    assert_eq!(week.by_category[Category::AdsAndTrackers.index()], 3);
    assert_eq!(week.by_category[Category::Cryptominers.index()], 4);
    assert_eq!(week.by_category.iter().sum::<u64>(), week.total);
}

#[test]
fn per_day_is_oldest_first_and_last_is_today() {
    let mut tally = WeeklyTally::default();
    ads(&mut tally, T0 - 6 * DAY, 1);
    ads(&mut tally, T0, 7);
    ads(&mut tally, T0 - 3 * DAY, 3);

    let week = tally.week(T0);
    assert_eq!(week.per_day, [1, 0, 0, 3, 0, 0, 7]);
}

#[test]
fn future_days_are_not_counted_when_the_clock_goes_backwards() {
    let mut tally = WeeklyTally::default();
    ads(&mut tally, T0 + 2 * DAY, 9);
    ads(&mut tally, T0, 1);

    let week = tally.week(T0);
    assert_eq!(week.total, 1);
    assert_eq!(week.per_day, [0, 0, 0, 0, 0, 0, 1]);
}

#[test]
fn prune_drops_days_older_than_35() {
    let mut tally = WeeklyTally::default();
    let today = T0;
    ads(&mut tally, today - 35 * DAY, 5);
    ads(&mut tally, today - 34 * DAY, 2);
    ads(&mut tally, today, 4);
    assert_eq!(TALLY_DAYS_KEPT, 35);
    assert_eq!(tally.week(today - 35 * DAY).total, 5);

    tally.prune(today);

    assert_eq!(tally.week(today - 35 * DAY).total, 0);
    assert_eq!(tally.week(today - 34 * DAY).total, 2);
    assert_eq!(tally.week(today).total, 4);
}

#[test]
fn take_dirty_returns_each_changed_day_and_category_once() {
    let mut tally = WeeklyTally::default();
    ads(&mut tally, T0, 2);
    ads(&mut tally, T0, 3);
    tally.record(T0, Category::Cryptominers, 1);

    let mut first = tally.take_dirty();
    first.sort_by_key(|d| (d.day, d.category));
    assert_eq!(first.len(), 2);
    let ads_delta = first
        .iter()
        .find(|d| d.category == Category::AdsAndTrackers);
    assert_eq!(ads_delta.map(|d| d.delta), Some(5));
    assert_eq!(ads_delta.map(|d| d.day), Some(WeeklyTally::day_of(T0)));
    let miner = first.iter().find(|d| d.category == Category::Cryptominers);
    assert_eq!(miner.map(|d| d.delta), Some(1));

    assert!(tally.take_dirty().is_empty());
    ads(&mut tally, T0, 1);
    let again = tally.take_dirty();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].delta, 1);
}

#[test]
fn from_rows_then_week_equals_the_original() {
    let mut tally = WeeklyTally::default();
    ads(&mut tally, T0, 3);
    ads(&mut tally, T0 - 2 * DAY, 5);
    tally.record(T0 - DAY, Category::Fingerprinting, 7);

    let rows: Vec<_> = tally
        .take_dirty()
        .into_iter()
        .map(|d| (d.day, d.category, d.delta))
        .collect();
    let rebuilt = WeeklyTally::from_rows(rows);
    assert_eq!(rebuilt.week(T0), tally.week(T0));
}

#[test]
fn clear_empties_the_tally() {
    let mut tally = WeeklyTally::default();
    ads(&mut tally, T0, 3);
    tally.clear();

    assert_eq!(tally.week(T0).total, 0);
    assert!(tally.take_dirty().is_empty());
}

#[test]
fn counts_saturate_at_u64_max() {
    let mut tally = WeeklyTally::default();
    ads(&mut tally, T0, u64::MAX);
    ads(&mut tally, T0, 10);

    let week = tally.week(T0);
    assert_eq!(week.total, u64::MAX);
    assert_eq!(week.by_category[Category::AdsAndTrackers.index()], u64::MAX);
}

#[test]
fn negative_clock_values_work_too() {
    let mut tally = WeeklyTally::default();
    tally.record(-1, Category::AdsAndTrackers, 4);
    tally.record(-DAY - 1, Category::AdsAndTrackers, 1);

    let week = tally.week(-1);
    assert_eq!(week.total, 5);
    assert_eq!(week.per_day, [0, 0, 0, 0, 0, 1, 4]);
}
