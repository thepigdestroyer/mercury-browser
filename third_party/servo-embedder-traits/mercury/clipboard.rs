/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the text lane (TXT-2). The clipboard messages and their payloads
// live here, not in a shared enum; the `TextMsg` variants that carry them are in `text.rs`.

//! Clipboard messages from the engine to Mercury: copy and paste with rich content, and the
//! asynchronous clipboard API of a page.
//!
//! Every type that holds what a user copied or a page wrote prints only its sizes and flags in
//! `Debug`: clipboard bytes must not reach a log.

use std::fmt;

use serde::{Deserialize, Serialize};
use servo_base::generic_channel::GenericCallback;

/// The most bytes one clipboard item may carry.
pub const MAX_CLIPBOARD_ITEM_BYTES: usize = 8 * 1024 * 1024;
/// The most bytes all items of one clipboard write may carry together.
pub const MAX_CLIPBOARD_TOTAL_BYTES: usize = 32 * 1024 * 1024;
/// The most items one clipboard write may carry.
pub const MAX_CLIPBOARD_ITEMS: usize = 8;
/// The most pixels a raw image may have.
pub const MAX_RAW_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;
/// The most bytes of HTML that a copy puts on the clipboard; a larger selection is copied as text.
pub const MAX_COPIED_HTML_BYTES: usize = 1024 * 1024;

/// What started a clipboard request. The user's own actions are told apart from a page's calls by
/// construction: the embedder sends the first two, a page's script the last two.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum ClipboardSource {
    /// A Cut, Copy or Paste from the browser's own menu or keyboard shortcut.
    UserAction,
    /// A Copy, Copy Image or Paste from the page's context menu.
    ContextMenu,
    /// `navigator.clipboard.read()`, `read()`, `write()` or `writeText()` in a page.
    AsyncApi,
    /// `document.execCommand("copy")` and the like, called by a page.
    ExecCommand,
}

/// Where a clipboard request comes from, as the engine knows it. The browser checks these, and
/// its own activation tracker, before it gives a page's request the clipboard.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ClipboardOrigin {
    /// The serialised origin of the requesting document (`"null"` when it is opaque).
    pub origin: String,
    /// The serialised origin of the top-level document, `None` when it is not known here (a
    /// top-level document in another process).
    pub top_level_origin: Option<String>,
    /// Whether the requesting document is the top-level document.
    pub is_top_level: bool,
    /// Whether the requesting document is same-origin with its top-level document. The frame gate
    /// of the asynchronous clipboard API. `false` when unknown.
    pub same_origin_with_top: bool,
    /// Whether the engine saw transient activation in the requesting window. Advisory: the
    /// browser still checks its own tracker, because a content process may be compromised.
    pub had_transient_activation: bool,
}

impl ClipboardOrigin {
    /// Whether a page's request from this origin may use the asynchronous clipboard API at all,
    /// as far as the engine can tell: same-origin with the top-level document and a user
    /// activation in the window.
    pub fn allows_async_access(&self) -> bool {
        self.same_origin_with_top && self.had_transient_activation
    }
}

/// One item of clipboard data: a MIME type and its bytes. `Debug` prints the size only.
#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct ClipboardItem {
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for ClipboardItem {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("ClipboardItem")
            .field("mime", &self.mime)
            .field("len", &self.bytes.len())
            .finish()
    }
}

/// An image as straight RGBA8 pixels, row by row from the top left. `Debug` prints the size only.
#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct RawImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl fmt::Debug for RawImage {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("RawImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_len", &self.rgba.len())
            .finish()
    }
}

/// A clipboard write: the items (text, HTML, PNG, …) and an optional raw image.
#[derive(Clone, Deserialize, Serialize)]
pub struct ClipboardWrite {
    pub origin: ClipboardOrigin,
    pub source: ClipboardSource,
    /// Whether the clipboard is emptied before the items are written.
    pub clear_first: bool,
    pub items: Vec<ClipboardItem>,
    pub image: Option<RawImage>,
}

impl fmt::Debug for ClipboardWrite {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("ClipboardWrite")
            .field("source", &self.source)
            .field("clear_first", &self.clear_first)
            .field("items", &self.items)
            .field("has_image", &self.image.is_some())
            .finish()
    }
}

/// A clipboard read: the MIME types wanted, in order (empty means every supported type).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClipboardRead {
    pub origin: ClipboardOrigin,
    pub source: ClipboardSource,
    pub mimes: Vec<String>,
}

/// Why the browser refused a clipboard request.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum ClipboardRefusal {
    NotAllowed,
    NoActivation,
    CrossOrigin,
    Unavailable,
    TooLarge,
}

/// The reply to a clipboard write.
#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum ClipboardWriteReply {
    Written,
    Refused(ClipboardRefusal),
}

/// The reply to a clipboard read. `Debug` prints the counts only.
#[derive(Deserialize, Serialize)]
pub enum ClipboardReadReply {
    Items(Vec<ClipboardItem>),
    Refused(ClipboardRefusal),
}

impl fmt::Debug for ClipboardReadReply {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Items(items) => formatter
                .debug_struct("Items")
                .field("count", &items.len())
                .finish(),
            Self::Refused(refusal) => formatter.debug_tuple("Refused").field(refusal).finish(),
        }
    }
}

/// A clipboard write with the reply channel the requester waits on, if it waits at all (a user's
/// Copy does not). `Debug` prints the write only.
#[derive(Deserialize, Serialize)]
pub struct ClipboardWriteMsg {
    pub write: ClipboardWrite,
    pub reply: Option<GenericCallback<ClipboardWriteReply>>,
}

impl fmt::Debug for ClipboardWriteMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("ClipboardWriteMsg")
            .field("write", &self.write)
            .field("has_reply", &self.reply.is_some())
            .finish()
    }
}

/// A clipboard read with the reply channel the requester waits on.
#[derive(Deserialize, Serialize)]
pub struct ClipboardReadMsg {
    pub read: ClipboardRead,
    pub reply: GenericCallback<ClipboardReadReply>,
}

impl fmt::Debug for ClipboardReadMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("ClipboardReadMsg")
            .field("read", &self.read)
            .finish()
    }
}

/// Checks a clipboard write's items against the caps: at most [`MAX_CLIPBOARD_ITEMS`] items, each
/// at most [`MAX_CLIPBOARD_ITEM_BYTES`] and all together at most [`MAX_CLIPBOARD_TOTAL_BYTES`].
/// Items without a MIME type are dropped first. A write over a cap is refused as `TooLarge`.
pub fn clamp_items(items: Vec<ClipboardItem>) -> Result<Vec<ClipboardItem>, ClipboardRefusal> {
    let items: Vec<ClipboardItem> = items
        .into_iter()
        .filter(|item| !item.mime.is_empty())
        .collect();
    if items.len() > MAX_CLIPBOARD_ITEMS {
        return Err(ClipboardRefusal::TooLarge);
    }
    let mut total = 0usize;
    for item in &items {
        if item.bytes.len() > MAX_CLIPBOARD_ITEM_BYTES {
            return Err(ClipboardRefusal::TooLarge);
        }
        total = total.saturating_add(item.bytes.len());
    }
    if total > MAX_CLIPBOARD_TOTAL_BYTES {
        return Err(ClipboardRefusal::TooLarge);
    }
    Ok(items)
}

/// The longest prefix of `text` of at most `max` characters, cut on a character boundary.
pub fn clamp_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(same_origin_with_top: bool, had_transient_activation: bool) -> ClipboardOrigin {
        ClipboardOrigin {
            origin: "https://a.test".to_owned(),
            top_level_origin: Some("https://a.test".to_owned()),
            is_top_level: true,
            same_origin_with_top,
            had_transient_activation,
        }
    }

    fn item(mime: &str, len: usize) -> ClipboardItem {
        ClipboardItem {
            mime: mime.to_owned(),
            bytes: vec![0xAB; len],
        }
    }

    #[test]
    fn debug_never_prints_clipboard_bytes() {
        let printed = format!(
            "{:?}",
            ClipboardItem {
                mime: "text/plain".to_owned(),
                bytes: b"SECRET".to_vec(),
            }
        );
        assert!(printed.contains("text/plain"), "{printed}");
        assert!(printed.contains('6'), "{printed}");
        assert!(!printed.contains("SECRET"), "{printed}");
    }

    #[test]
    fn debug_never_prints_image_pixels() {
        let image = RawImage {
            width: 2,
            height: 2,
            rgba: b"SECRETPIXELS".to_vec(),
        };
        let printed = format!("{image:?}");
        assert!(printed.contains("rgba_len"), "{printed}");
        assert!(!printed.contains("SECRET"), "{printed}");
    }

    #[test]
    fn debug_of_a_write_and_its_reply_never_prints_the_content() {
        let write = ClipboardWrite {
            origin: origin(true, true),
            source: ClipboardSource::AsyncApi,
            clear_first: true,
            items: vec![ClipboardItem {
                mime: "text/plain".to_owned(),
                bytes: b"SECRET".to_vec(),
            }],
            image: None,
        };
        let printed = format!("{write:?}");
        assert!(!printed.contains("SECRET"), "{printed}");
        assert!(printed.contains("has_image"), "{printed}");

        let reply = ClipboardReadReply::Items(vec![ClipboardItem {
            mime: "text/plain".to_owned(),
            bytes: b"SECRET".to_vec(),
        }]);
        let printed = format!("{reply:?}");
        assert!(printed.contains("count"), "{printed}");
        assert!(!printed.contains("SECRET"), "{printed}");
    }

    #[test]
    fn clamp_items_refuses_nine_items() {
        let items: Vec<ClipboardItem> = (0..9).map(|_| item("text/plain", 1)).collect();
        assert_eq!(clamp_items(items), Err(ClipboardRefusal::TooLarge));
    }

    #[test]
    fn clamp_items_refuses_a_nine_mebibyte_item() {
        let items = vec![item("image/png", 9 * 1024 * 1024)];
        assert_eq!(clamp_items(items), Err(ClipboardRefusal::TooLarge));
    }

    #[test]
    fn clamp_items_drops_empty_mime_types() {
        let items = vec![item("", 4), item("text/plain", 4)];
        let kept = clamp_items(items).expect("within the caps");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].mime, "text/plain");
    }

    #[test]
    fn clamp_items_refuses_total_over_32_mib() {
        let items: Vec<ClipboardItem> = (0..5)
            .map(|_| item("image/png", 7 * 1024 * 1024))
            .collect();
        assert_eq!(clamp_items(items), Err(ClipboardRefusal::TooLarge));
    }

    #[test]
    fn clamp_items_keeps_a_write_within_the_caps() {
        let items = vec![item("text/plain", 3), item("text/html", 5)];
        assert_eq!(clamp_items(items.clone()), Ok(items));
    }

    #[test]
    fn allows_async_access_needs_same_origin_and_activation() {
        assert!(origin(true, true).allows_async_access());
        assert!(!origin(false, true).allows_async_access());
        assert!(!origin(true, false).allows_async_access());
        assert!(!origin(false, false).allows_async_access());
    }

    #[test]
    fn clamp_chars_cuts_on_a_char_boundary() {
        assert_eq!(clamp_chars("aé😀b", 3), "aé😀");
        assert_eq!(clamp_chars("abc", 10), "abc");
        assert_eq!(clamp_chars("", 0), "");
    }

    #[test]
    fn the_clipboard_types_cross_the_process_boundary() {
        fn assert_serde<T: Serialize + for<'de> Deserialize<'de>>() {}
        assert_serde::<ClipboardWrite>();
        assert_serde::<ClipboardRead>();
        assert_serde::<ClipboardWriteReply>();
        assert_serde::<ClipboardReadReply>();
        assert_serde::<ClipboardWriteMsg>();
        assert_serde::<ClipboardReadMsg>();
    }
}
