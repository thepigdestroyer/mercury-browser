//! What other lanes provide to the import window.

use mercury_ui::ImportWindow;

/// Bookmarks lane: "Import bookmarks from a file…" (a Netscape bookmark file, through the
/// native open panel). The button in the import window is disabled until this is `true`.
pub(crate) const IMPORT_FROM_FILE_AVAILABLE: bool = false;

/// Bookmarks lane: ask for a bookmark file and import it. Called from the import window when no
/// other browser was found.
pub(crate) fn import_bookmarks_from_file(_window: &ImportWindow) {}
