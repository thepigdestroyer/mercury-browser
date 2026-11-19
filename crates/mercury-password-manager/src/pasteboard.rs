// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::time::Duration;

/// A copied password is removed from the pasteboard after this long, unless something else has
/// been copied since.
pub const CLIPBOARD_CLEAR_DELAY: Duration = Duration::from_secs(60);

/// The system pasteboard, as far as copying secrets needs it.
pub trait Pasteboard {
    /// Replaces the pasteboard's contents with `text`. With `concealed`, marks it as a secret
    /// for clipboard managers that honour the marker (`org.nspasteboard.ConcealedType` on
    /// macOS). Returns the pasteboard's change count after writing, or `None` if it failed.
    fn write_text(&self, text: &str, concealed: bool) -> Option<i64>;
    /// A number that changes whenever anyone writes to the pasteboard.
    fn change_count(&self) -> i64;
    /// Empties the pasteboard.
    fn clear(&self);
}

/// A secret Mercury put on the pasteboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopiedSecret {
    change_count: i64,
}

impl CopiedSecret {
    /// Copies `text` as a concealed item; `None` if the pasteboard refused it.
    pub fn copy(pasteboard: &dyn Pasteboard, text: &str) -> Option<Self> {
        pasteboard
            .write_text(text, true)
            .map(|change_count| Self { change_count })
    }

    /// Empties the pasteboard if it still holds this secret; returns whether it did.
    pub fn clear_if_unchanged(self, pasteboard: &dyn Pasteboard) -> bool {
        let unchanged = pasteboard.change_count() == self.change_count;
        if unchanged {
            pasteboard.clear();
        }
        unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakePasteboard;

    #[test]
    fn a_copied_secret_is_concealed_and_cleared_later() {
        let pasteboard = FakePasteboard::default();

        let copied = CopiedSecret::copy(&pasteboard, "fake-pw").expect("copied");
        assert_eq!(pasteboard.contents(), Some(("fake-pw".to_owned(), true)));

        assert!(copied.clear_if_unchanged(&pasteboard));
        assert_eq!(pasteboard.contents(), None);
    }

    #[test]
    fn something_copied_since_is_left_alone() {
        let pasteboard = FakePasteboard::default();
        let copied = CopiedSecret::copy(&pasteboard, "fake-pw").expect("copied");
        pasteboard.write_text("the user's own text", false);

        assert!(!copied.clear_if_unchanged(&pasteboard));
        assert_eq!(
            pasteboard.contents(),
            Some(("the user's own text".to_owned(), false))
        );
    }
}
