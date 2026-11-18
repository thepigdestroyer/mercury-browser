// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Keeping the tab strip's model in step with the browser's tabs without recreating the tab
//! items, so a drag in progress survives tabs opening and closing elsewhere.

use slint::{Model, VecModel};

use crate::BrowserTab;

/// Makes `model` hold `rows`, matching rows by tab id. Rows of closed tabs are removed and rows
/// of new tabs inserted where they belong; a row whose tab is still there only has its data
/// updated, so its tab item (and a drag it holds) lives on. Never resets the whole model.
pub fn update_tab_rows(model: &VecModel<BrowserTab>, rows: Vec<BrowserTab>) {
    let id_at = |index: usize| model.row_data(index).map(|row| row.id);
    for index in (0..model.row_count()).rev() {
        if id_at(index).is_some_and(|id| !rows.iter().any(|row| row.id == id)) {
            model.remove(index);
        }
    }
    let count = rows.len();
    for (index, row) in rows.into_iter().enumerate() {
        match model.row_data(index) {
            Some(current) if current.id == row.id => {
                if current != row {
                    model.set_row_data(index, row);
                }
            }
            _ => {
                // A tab that moved: its row goes from its old place to this one.
                if let Some(old) =
                    (index + 1..model.row_count()).find(|old| id_at(*old) == Some(row.id))
                {
                    model.remove(old);
                }
                model.insert(index, row);
            }
        }
    }
    while model.row_count() > count {
        model.remove(model.row_count() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
