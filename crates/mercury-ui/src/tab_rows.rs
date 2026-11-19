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
