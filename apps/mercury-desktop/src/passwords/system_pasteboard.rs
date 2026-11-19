// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The general pasteboard, for copying a user name or password from the settings window. Only
//! normal interactive runs use it; tests and automated runs use an in-memory fake.

use mercury_password_manager::Pasteboard;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardContentsOptions, NSPasteboardItem, NSPasteboardTypeString,
    NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSString};

/// Markers that clipboard managers following nspasteboard.org honour: a concealed item is kept
/// out of their history, a transient one is not kept at all.
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";

pub(crate) struct SystemPasteboard;

impl Pasteboard for SystemPasteboard {
    fn write_text(&self, text: &str, concealed: bool) -> Option<i64> {
        write_to(&NSPasteboard::generalPasteboard(), text, concealed)
    }

    fn change_count(&self) -> i64 {
        i64::try_from(NSPasteboard::generalPasteboard().changeCount()).unwrap_or_default()
    }

    fn clear(&self) {
        NSPasteboard::generalPasteboard().clearContents();
    }
}

/// Replaces the contents of `pasteboard` with `text` and returns its change count, or `None` if
/// AppKit refused the write. Split out of `write_text` so that a check can aim it at a private
/// pasteboard instead of the user's general one.
fn write_to(pasteboard: &NSPasteboard, text: &str, concealed: bool) -> Option<i64> {
    // SAFETY: an AppKit constant, valid for the life of the process.
    let string_type = unsafe { NSPasteboardTypeString };
    if concealed {
        // Asks for this Mac only (no Universal Clipboard). This also empties the pasteboard.
        // Nothing after it should reset the option, since the data goes in with one
        // `writeObjects:` rather than `declareTypes:owner:`; AppKit cannot read the option back,
        // so the tests below cannot check that.
        pasteboard.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
        // One item carries the string and both markers, written at once: a clipboard
        // manager never sees the string without them.
        let item = NSPasteboardItem::new();
        let empty = NSString::from_str("");
        if !item.setString_forType(&NSString::from_str(text), string_type)
            || !item.setString_forType(&empty, &NSString::from_str(CONCEALED_TYPE))
            || !item.setString_forType(&empty, &NSString::from_str(TRANSIENT_TYPE))
        {
            return None;
        }
        let items = NSArray::from_retained_slice(&[
            ProtocolObject::<dyn NSPasteboardWriting>::from_retained(item),
        ]);
        if !pasteboard.writeObjects(&items) {
            return None;
        }
    } else {
        pasteboard.clearContents();
        if !pasteboard.setString_forType(&NSString::from_str(text), string_type) {
            return None;
        }
    }
    Some(i64::try_from(pasteboard.changeCount()).unwrap_or_default())
}

// passwords-ui (review round 3): the concealed copy had no test in the repository. These aim
// `write_to` at a pasteboard of the test's own (`pasteboardWithUniqueName`), never at the
// user's general one, and read back what AppKit holds.
//
// Not covered, because AppKit offers no way to read it back: that the pasteboard is still
// "this Mac only" (`NSPasteboardContentsCurrentHostOnly`) after the `writeObjects:` that
// follows `prepareForNewContentsWithOptions:`. Only Apple's documentation, and the order of the
// calls, vouch for that; no device was paired to see Universal Clipboard refuse the item.
#[cfg(test)]
mod tests {
    use objc2::rc::Retained;
    use objc2::runtime::NSObjectProtocol;
    use objc2::{msg_send, sel};

    use super::*;

    /// A pasteboard no other program knows about; gone from the pasteboard server when dropped.
    struct Private(Retained<NSPasteboard>);

    impl Private {
        fn new() -> Self {
            Self(NSPasteboard::pasteboardWithUniqueName())
        }

        fn write(&self, text: &str, concealed: bool) -> Option<i64> {
            write_to(&self.0, text, concealed)
        }

        /// What it holds: for each item, its types (sorted) and its string.
        fn items(&self) -> Vec<(Vec<String>, Option<String>)> {
            // SAFETY: an AppKit constant, valid for the life of the process.
            let string_type = unsafe { NSPasteboardTypeString };
            let Some(items) = self.0.pasteboardItems() else {
                return Vec::new();
            };
            items
                .to_vec()
                .iter()
                .map(|item| {
                    let mut types: Vec<String> = item
                        .types()
                        .to_vec()
                        .iter()
                        .map(|name| name.to_string())
                        .collect();
                    types.sort();
                    let string = item.stringForType(string_type).map(|s| s.to_string());
                    (types, string)
                })
                .collect()
        }

        fn change_count(&self) -> i64 {
            i64::try_from(self.0.changeCount()).expect("a change count")
        }
    }

    impl Drop for Private {
        fn drop(&mut self) {
            // `releaseGlobally` frees the pasteboard in the pasteboard server. The bindings do
            // not have it (AppKit marks it as no longer needed), so ask only if it is there.
            let release = sel!(releaseGlobally);
            if self.0.respondsToSelector(release) {
                // SAFETY: `releaseGlobally` takes no arguments and returns nothing; the
                // pasteboard is not used again.
                let () = unsafe { msg_send![&*self.0, releaseGlobally] };
            }
        }
    }

    fn has(types: &[String], name: &str) -> bool {
        types.iter().any(|candidate| candidate == name)
    }

    fn string_type() -> String {
        // SAFETY: an AppKit constant, valid for the life of the process.
        unsafe { NSPasteboardTypeString }.to_string()
    }

    #[test]
    fn a_concealed_copy_is_one_item_with_the_string_and_both_markers() {
        let pasteboard = Private::new();
        let secret = "fake-pässwörd 🔑 aB3#".repeat(8);

        let count = pasteboard.write(&secret, true);

        assert_eq!(
            count,
            Some(pasteboard.change_count()),
            "the new change count"
        );
        let items = pasteboard.items();
        assert_eq!(items.len(), 1, "one item, not one per type: {items:?}");
        let (types, string) = &items[0];
        assert_eq!(string.as_deref(), Some(secret.as_str()));
        assert!(has(types, &string_type()), "the string type: {types:?}");
        assert!(
            has(types, CONCEALED_TYPE),
            "the concealed marker: {types:?}"
        );
        assert!(
            has(types, TRANSIENT_TYPE),
            "the transient marker: {types:?}"
        );
    }

    #[test]
    fn a_plain_copy_is_one_item_with_the_string_and_no_marker() {
        let pasteboard = Private::new();

        let count = pasteboard.write("alice@example.com", false);

        assert_eq!(count, Some(pasteboard.change_count()));
        let items = pasteboard.items();
        assert_eq!(items.len(), 1, "{items:?}");
        let (types, string) = &items[0];
        assert_eq!(string.as_deref(), Some("alice@example.com"));
        assert!(has(types, &string_type()), "{types:?}");
        assert!(!has(types, CONCEALED_TYPE), "{types:?}");
        assert!(!has(types, TRANSIENT_TYPE), "{types:?}");
    }

    #[test]
    fn each_copy_replaces_the_last_and_leaves_no_marker_behind() {
        let pasteboard = Private::new();

        let first = pasteboard.write("fake-secret", true).expect("written");
        let second = pasteboard
            .write("alice@example.com", false)
            .expect("written");
        let after_plain = pasteboard.items();
        let third = pasteboard.write("fake-secret-2", true).expect("written");
        let after_concealed = pasteboard.items();

        assert!(first < second && second < third, "{first} {second} {third}");
        assert_eq!(after_plain.len(), 1, "{after_plain:?}");
        assert_eq!(after_plain[0].1.as_deref(), Some("alice@example.com"));
        assert!(
            !has(&after_plain[0].0, CONCEALED_TYPE) && !has(&after_plain[0].0, TRANSIENT_TYPE),
            "a user name copied after a password is not concealed: {after_plain:?}"
        );
        assert_eq!(after_concealed.len(), 1, "{after_concealed:?}");
        assert_eq!(after_concealed[0].1.as_deref(), Some("fake-secret-2"));
        assert!(has(&after_concealed[0].0, CONCEALED_TYPE));
        assert!(has(&after_concealed[0].0, TRANSIENT_TYPE));
    }
}
