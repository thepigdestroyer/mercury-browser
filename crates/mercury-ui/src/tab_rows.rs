// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Keeping the tab strip's model in step with the browser's tabs without recreating the tab
//! items, so a drag in progress survives tabs opening and closing elsewhere, and so does a tab
//! that starts or stops playing sound.

use slint::{Model, VecModel};

use crate::BrowserTab;

/// The list of rows that `sync_rows` changes: a `VecModel` in the application, a recording list
/// in the tests. Only these edits exist, so a row can be told apart as changed in place (`set_row`)
/// from one removed and inserted again, which would recreate its tab item.
trait RowStore {
    fn count(&self) -> usize;
    fn row(&self, index: usize) -> Option<BrowserTab>;
    fn remove_row(&self, index: usize);
    fn insert_row(&self, index: usize, row: BrowserTab);
    fn set_row(&self, index: usize, row: BrowserTab);
}

impl RowStore for VecModel<BrowserTab> {
    fn count(&self) -> usize {
        self.row_count()
    }

    fn row(&self, index: usize) -> Option<BrowserTab> {
        self.row_data(index)
    }

    fn remove_row(&self, index: usize) {
        self.remove(index);
    }

    fn insert_row(&self, index: usize, row: BrowserTab) {
        self.insert(index, row);
    }

    fn set_row(&self, index: usize, row: BrowserTab) {
        self.set_row_data(index, row);
    }
}

/// Makes `model` hold `rows`, matching rows by tab id. Rows of closed tabs are removed and rows
/// of new tabs inserted where they belong; a row whose tab is still there only has its data
/// updated (a title, the sound state, the active mark), so its tab item (and a drag it holds)
/// lives on. Never resets the whole model.
pub fn update_tab_rows(model: &VecModel<BrowserTab>, rows: Vec<BrowserTab>) {
    sync_rows(model, rows);
}

fn sync_rows(model: &impl RowStore, rows: Vec<BrowserTab>) {
    let id_at = |index: usize| model.row(index).map(|row| row.id);
    for index in (0..model.count()).rev() {
        if id_at(index).is_some_and(|id| !rows.iter().any(|row| row.id == id)) {
            model.remove_row(index);
        }
    }
    let count = rows.len();
    for (index, row) in rows.into_iter().enumerate() {
        match model.row(index) {
            Some(current) if current.id == row.id => {
                if !same_row(&current, &row) {
                    model.set_row(index, row);
                }
            }
            _ => {
                // A tab that moved: its row goes from its old place to this one.
                if let Some(old) =
                    (index + 1..model.count()).find(|old| id_at(*old) == Some(row.id))
                {
                    model.remove_row(old);
                }
                model.insert_row(index, row);
            }
        }
    }
    while model.count() > count {
        model.remove_row(model.count() - 1);
    }
}

/// The offset of each tab's slot from the strip's first slot: a group label chip of `chip_width`
/// sits before the first tab of every run of one group. `groups[i]` is the group of tab `i`
/// (`None` for a loose tab), so the offset of tab `i` is the number of chips before it.
pub fn chip_offsets(groups: &[Option<i32>], chip_width: f32) -> Vec<f32> {
    let mut chips = 0;
    let mut previous = None;
    groups
        .iter()
        .map(|&group| {
            if group.is_some() && group != previous {
                chips += 1;
            }
            previous = group;
            chips as f32 * chip_width
        })
        .collect()
}

/// The index of the slot that contains `x`, given the slot starts `offsets` (ascending): the last
/// slot starting at or before `x`. A pointer left of the first slot is slot 0.
pub fn drop_index(offsets: &[f32], x: f32) -> usize {
    offsets
        .partition_point(|&start| start <= x)
        .saturating_sub(1)
}

/// Whether two rows of the same tab show the same thing. Slint never counts one empty `Image` as
/// equal to another, so `==` would call every row without a page icon changed on every sync (and
/// have the strip re-read it); such a row's `icon` is not shown, so it is left out. Every field
/// is named, so a field added to `BrowserTab` has to be compared here too.
fn same_row(a: &BrowserTab, b: &BrowserTab) -> bool {
    let BrowserTab {
        title,
        icon,
        has_icon,
        active,
        loading,
        id,
        pinned,
        url,
        audio,
        chip_before,
    } = a;
    let icons_match = (!*has_icon && !b.has_icon) || *icon == b.icon;
    icons_match
        && *title == b.title
        && *has_icon == b.has_icon
        && *active == b.active
        && *loading == b.loading
        && *id == b.id
        && *pinned == b.pinned
        && *url == b.url
        && *audio == b.audio
        && *chip_before == b.chip_before
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::TabAudioState;

    fn row(id: i32, title: &str) -> BrowserTab {
        BrowserTab {
            id,
            title: title.into(),
            ..Default::default()
        }
    }

    fn ids(model: &VecModel<BrowserTab>) -> Vec<i32> {
        model.iter().map(|row| row.id).collect()
    }

    #[test]
    fn rows_follow_tabs_that_open_close_move_and_change() {
        let model = VecModel::from(vec![row(1, "a"), row(2, "b"), row(3, "c")]);

        update_tab_rows(&model, vec![row(1, "a"), row(3, "c"), row(4, "d")]);
        assert_eq!(ids(&model), [1, 3, 4]);

        update_tab_rows(&model, vec![row(4, "d"), row(1, "a"), row(3, "C")]);
        assert_eq!(ids(&model), [4, 1, 3]);
        assert_eq!(model.row_data(2).map(|row| row.title), Some("C".into()));

        update_tab_rows(&model, vec![row(5, "e")]);
        assert_eq!(ids(&model), [5]);

        update_tab_rows(&model, Vec::new());
        assert_eq!(model.row_count(), 0);
    }

    /// What was done to a list of rows, by tab id.
    #[derive(Debug, PartialEq)]
    enum Edit {
        Remove(usize),
        Insert(usize, i32),
        Set(usize, i32),
    }

    /// A list of rows that remembers how it was changed.
    #[derive(Default)]
    struct Recorder {
        rows: RefCell<Vec<BrowserTab>>,
        edits: RefCell<Vec<Edit>>,
    }

    impl RowStore for Recorder {
        fn count(&self) -> usize {
            self.rows.borrow().len()
        }

        fn row(&self, index: usize) -> Option<BrowserTab> {
            self.rows.borrow().get(index).cloned()
        }

        fn remove_row(&self, index: usize) {
            self.rows.borrow_mut().remove(index);
            self.edits.borrow_mut().push(Edit::Remove(index));
        }

        fn insert_row(&self, index: usize, row: BrowserTab) {
            self.edits.borrow_mut().push(Edit::Insert(index, row.id));
            self.rows.borrow_mut().insert(index, row);
        }

        fn set_row(&self, index: usize, row: BrowserTab) {
            self.edits.borrow_mut().push(Edit::Set(index, row.id));
            self.rows.borrow_mut()[index] = row;
        }
    }

    fn with_sound(mut row: BrowserTab, audio: TabAudioState) -> BrowserTab {
        row.audio = audio;
        row
    }

    // A tab that starts or stops playing, or is muted, keeps its row (and so its tab item and a
    // drag that holds it): the sync changes its data and nothing else.
    #[test]
    fn a_row_whose_only_change_is_its_sound_is_updated_in_place() {
        let store = Recorder::default();
        let rows =
            |audio: TabAudioState| vec![row(1, "a"), with_sound(row(2, "b"), audio), row(3, "c")];
        sync_rows(&store, rows(TabAudioState::Silent));
        store.edits.borrow_mut().clear();

        for (audio, expected) in [
            (TabAudioState::Playing, [Edit::Set(1, 2)]),
            (TabAudioState::Muted, [Edit::Set(1, 2)]),
            (TabAudioState::Silent, [Edit::Set(1, 2)]),
        ] {
            sync_rows(&store, rows(audio));
            assert_eq!(*store.edits.borrow(), expected, "{audio:?}");
            assert_eq!(store.row(1).map(|row| row.audio), Some(audio));
            store.edits.borrow_mut().clear();
        }

        // Nothing changed: nothing is touched.
        sync_rows(&store, rows(TabAudioState::Silent));
        assert!(store.edits.borrow().is_empty(), "{:?}", store.edits);
    }

    // A page icon that arrives or changes is a change; the icon of a row that shows none is not.
    #[test]
    fn a_new_page_icon_updates_the_row_and_an_unshown_one_does_not() {
        let icon =
            || slint::Image::from_rgba8(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(1, 1));
        let with_icon = |icon: &slint::Image| BrowserTab {
            has_icon: true,
            icon: icon.clone(),
            ..row(1, "a")
        };
        // Slint compares the pixel buffers of two images by identity: the strip's rows hold
        // clones of the same decoded icon.
        let (first, second) = (icon(), icon());
        let store = Recorder::default();
        sync_rows(&store, vec![row(1, "a")]);
        store.edits.borrow_mut().clear();

        sync_rows(&store, vec![with_icon(&first)]);
        assert_eq!(*store.edits.borrow(), [Edit::Set(0, 1)]);
        sync_rows(&store, vec![with_icon(&first)]);
        assert_eq!(
            store.edits.borrow().len(),
            1,
            "the same icon changes nothing"
        );
        sync_rows(&store, vec![with_icon(&second)]);
        assert_eq!(store.edits.borrow().len(), 2, "another icon is a change");

        // Not shown: the empty image in a row without an icon is not a difference.
        sync_rows(&store, vec![row(1, "a")]);
        assert_eq!(store.edits.borrow().len(), 3);
        sync_rows(&store, vec![row(1, "a")]);
        assert_eq!(store.edits.borrow().len(), 3, "nothing changed");
    }

    /// `tabs` tabs with exactly `runs` runs of one group each, every run three tabs long, and
    /// loose tabs between them (none when `runs` is 0). The runs are spread over the strip.
    fn groups_with_runs(tabs: usize, runs: usize) -> Vec<Option<i32>> {
        let mut groups = vec![None; tabs];
        for run in 0..runs {
            // Run `run` starts at its share of the strip, then holds three tabs of group `run`.
            let start = (run + 1) * tabs / (runs + 1);
            for group in groups.iter_mut().skip(start).take(3) {
                *group = Some(run as i32);
            }
        }
        groups
    }

    fn slot_starts(tabs: usize, runs: usize, tab_width: f32, chip_width: f32) -> Vec<f32> {
        let groups = groups_with_runs(tabs, runs);
        chip_offsets(&groups, chip_width)
            .into_iter()
            .enumerate()
            .map(|(index, chips)| index as f32 * tab_width + chips)
            .collect()
    }

    #[test]
    fn chips_count_the_runs_before_each_tab() {
        // Three runs: [a a] loose [b] loose loose [c c c]. Each run's chip sits before its first
        // tab, so that tab's offset already counts its own chip.
        let groups = [
            Some(1),
            Some(1),
            None,
            Some(2),
            None,
            None,
            Some(3),
            Some(3),
            Some(3),
        ];
        assert_eq!(
            chip_offsets(&groups, 96.0),
            [96.0, 96.0, 96.0, 192.0, 192.0, 192.0, 288.0, 288.0, 288.0]
        );
        assert_eq!(chip_offsets(&[None, None], 96.0), [0.0, 0.0]);
        assert_eq!(chip_offsets(&[], 96.0), Vec::<f32>::new());
        // Two adjacent groups: each starts with its own chip.
        assert_eq!(chip_offsets(&[Some(1), Some(2)], 96.0), [96.0, 192.0]);
    }

    #[test]
    fn chips_are_counted_for_200_tabs_with_0_1_and_3_runs() {
        for runs in [0usize, 1, 3] {
            let groups = groups_with_runs(200, runs);
            let offsets = chip_offsets(&groups, 96.0);
            assert_eq!(offsets.len(), 200);
            // Exactly one chip per run, so the last tab sits after all of them.
            assert_eq!(offsets.last(), Some(&(runs as f32 * 96.0)), "runs {runs}");
            assert!(offsets.windows(2).all(|pair| pair[0] <= pair[1]));
        }
    }

    #[test]
    fn drop_index_inverts_slot_x_at_every_slot_centre_and_boundary() {
        const TAB: f32 = 190.0;
        const CHIP: f32 = 96.0;
        for runs in [0usize, 1, 3] {
            let starts = slot_starts(200, runs, TAB, CHIP);
            for (index, &start) in starts.iter().enumerate() {
                assert_eq!(
                    drop_index(&starts, start),
                    index,
                    "runs {runs}, left edge of {index}"
                );
                assert_eq!(
                    drop_index(&starts, start + TAB / 2.0),
                    index,
                    "runs {runs}, centre of {index}"
                );
                assert_eq!(
                    drop_index(&starts, start + TAB - 0.01),
                    index,
                    "runs {runs}, right edge of {index}"
                );
            }
            // Left of the first slot is slot 0.
            assert_eq!(drop_index(&starts, -50.0), 0);
            // Past the last slot is the last slot.
            assert_eq!(drop_index(&starts, f32::MAX), starts.len() - 1);
        }
    }

    #[test]
    fn drop_index_of_an_empty_strip_is_zero() {
        assert_eq!(drop_index(&[], 10.0), 0);
    }

    #[test]
    fn the_model_keeps_its_rows_when_only_the_sound_changes() {
        let model = VecModel::from(vec![row(1, "a"), row(2, "b")]);
        update_tab_rows(
            &model,
            vec![row(1, "a"), with_sound(row(2, "b"), TabAudioState::Muted)],
        );
        assert_eq!(ids(&model), [1, 2]);
        assert_eq!(
            model.row_data(1).map(|row| row.audio),
            Some(TabAudioState::Muted)
        );
        assert_eq!(
            model.row_data(0).map(|row| row.audio),
            Some(TabAudioState::Silent)
        );
    }
}
