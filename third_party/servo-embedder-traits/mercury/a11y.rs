/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the a11y lane. Add variants (and the payload types they need) here
// only, never in another lane's file or in a shared enum.

use std::fmt;

use serde::{Deserialize, Serialize};
use servo_base::Epoch;
use strum::IntoStaticStr;

/// Messages from the engine to Mercury for the `a11y` lane (accessibility).
///
/// Empty until that lane adds its variants.
///
/// `Debug` prints only the variant name, never a payload.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum A11yMsg {}

impl fmt::Debug for A11yMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}

// Mercury: a11y-1 item 5 (EE7, iframes). The epoch of an iframe's accessibility tree. No message is
// needed: the constellation chooses the epoch it activates a child document with, layout sends it
// back with every update (`EmbedderMsg::AccessibilityTreeUpdate`), and `WebView` reads it.

/// The bit of an [`Epoch`] that says "this update comes from a child (iframe) document".
///
/// A top-level document's epoch counts navigations (`Epoch::next()` once per document the
/// `WebView` shows), so it never gets near 2^31.
const CHILD_DOCUMENT_BIT: u32 = 1 << 31;

/// The epoch the constellation activates the accessibility of a child (iframe) document with,
/// given the epoch of the top-level document of its `WebView`.
///
/// `WebView::process_accessibility_tree_update()` treats an update with a newer epoch than the
/// grafted one as "a new top-level document: graft its tree as the page". A child document's
/// update must never do that, and may arrive before the top-level document's own first update, so
/// the child's epoch carries a flag that [`top_level_epoch_of_child_document`] reads: the
/// `WebView` passes such an update to the embedder untouched and drops it only when the top-level
/// document it belonged to is already gone.
pub fn epoch_for_child_document(top_level_epoch: Epoch) -> Epoch {
    Epoch(top_level_epoch.0 | CHILD_DOCUMENT_BIT)
}

/// The epoch of the top-level document that a child document's update belongs to, when `epoch`
/// is the epoch of a child document (see [`epoch_for_child_document`]); `None` for the epoch of a
/// top-level document.
pub fn top_level_epoch_of_child_document(epoch: Epoch) -> Option<Epoch> {
    (epoch.0 & CHILD_DOCUMENT_BIT != 0).then_some(Epoch(epoch.0 & !CHILD_DOCUMENT_BIT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_epoch_names_its_top_level_epoch() {
        for value in [0, 1, 2, 37, 1 << 20, (1 << 31) - 1] {
            let top_level = Epoch(value);
            let child = epoch_for_child_document(top_level);
            assert_ne!(child, top_level);
            assert_eq!(top_level_epoch_of_child_document(child), Some(top_level));
        }
    }

    #[test]
    fn a_top_level_epoch_is_not_a_child_epoch() {
        for value in [0, 1, 2, 37, 1 << 20, (1 << 31) - 1] {
            assert_eq!(top_level_epoch_of_child_document(Epoch(value)), None);
        }
    }

    #[test]
    fn the_flag_does_not_change_which_top_level_a_child_belongs_to() {
        // The comparisons `WebView` makes are on the decoded top-level epoch, so an older
        // document's child is stale and a newer one is not, whatever the flag does to the order
        // of the raw numbers.
        let older = top_level_epoch_of_child_document(epoch_for_child_document(Epoch(3))).unwrap();
        let newer = top_level_epoch_of_child_document(epoch_for_child_document(Epoch(4))).unwrap();
        assert!(older < newer);
        assert!(older < Epoch(4));
    }

    #[test]
    fn the_epoch_is_a_plain_number_that_crosses_a_process_boundary() {
        // `Epoch` is `Serialize`: the flag is part of the number, nothing else travels with it.
        let child = epoch_for_child_document(Epoch(5));
        assert_eq!(child.0, 5 | (1 << 31));
    }
}
