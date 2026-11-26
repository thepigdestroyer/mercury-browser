// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The persistent Slint models of the dock: created once, set on the dock once and refilled in
//! place by [`DockModels::update`], so the `ModelRc`s never change identity. While the number of
//! rows is unchanged the rows are patched with `set_row_data` (only the rows that differ), so the
//! dock's `for` elements survive and their `animate` blocks run; when the number changes the model is
//! replaced with `set_vec` (brief risk R2).

use std::rc::Rc;

use slint::{Model, ModelRc, VecModel};

use crate::{DockPill, DockSliver};

use super::capsule::CapsuleFrame;
use super::data::DockFrame;

/// One `VecModel` per array of the dock and the capsule.
#[derive(Default)]
pub struct DockModels {
    pills: Rc<VecModel<DockPill>>,
    slivers: Rc<VecModel<DockSliver>>,
}

/// Makes `model` hold `rows`: in place while the count is the same, `set_vec` otherwise.
fn patch<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: &[T]) {
    if model.row_count() != rows.len() {
        model.set_vec(rows.to_vec());
        return;
    }
    for (index, row) in rows.iter().enumerate() {
        if model.row_data(index).as_ref() != Some(row) {
            model.set_row_data(index, row.clone());
        }
    }
}

impl DockModels {
    /// Two empty models.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fills the models with the frame's pills and the capsule's slivers: rows are patched in
    /// place while their count is unchanged and replaced when it changes.
    pub fn update(&self, frame: &DockFrame, capsule: &CapsuleFrame) {
        patch(&self.pills, &frame.pills);
        patch(&self.slivers, &capsule.slivers);
    }

    /// The model for `GroupDock.pills` (the same `ModelRc` identity for the life of `self`).
    pub fn pills(&self) -> ModelRc<DockPill> {
        ModelRc::from(Rc::clone(&self.pills))
    }

    /// The model for `DockCapsule.slivers`.
    pub fn slivers(&self) -> ModelRc<DockSliver> {
        ModelRc::from(Rc::clone(&self.slivers))
    }
}
