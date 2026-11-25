// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What winit 0.30's macOS view does not do for the system's text input (txt-1 lane).
//!
//! winit's `WinitView` (class name `"WinitView"`, winit-0.30.13/src/platform_impl/macos/view.rs)
//! implements `NSTextInputClient`, but only for input methods that compose: it reports the marked
//! text and the commit, drops the clause attributes of the marked text, ignores every replacement
//! range (a `TODO` in its source), commits text only while marked text exists (so the emoji picker
//! and dictation without marked text are lost), answers `selectedRange` with "not found" and
//! `attributedSubstringForProposedRange:` with nil (so the accent menu, reconversion and dictation
//! corrections have nothing to work on). winit offers no way to extend the view, and vendoring
//! winit is out of plan, so this module **patches the class at run time**, the way
//! `macos_app_delegate.rs` extends winit's application delegate.
//!
//! # What is patched
//!
//! Method implementations of `WinitView` itself (never an inherited method: that would change
//! every `NSView`) are replaced with functions that call the original in every case but those
//! below. Type encodings, which the Objective-C runtime does not check for a replacement, are
//! checked against the class's methods before anything is changed; one that differs, or a missing
//! selector, installs nothing and logs **one** warning.
//!
//! | selector | encoding (`x86_64`; `BOOL` is `B` on `arm64`) | replacement |
//! |---|---|---|
//! | `keyDown:` | `v@:@` | notes that a key event is being handled, runs the original |
//! | `insertText:replacementRange:` | `v@:@{_NSRange=QQ}` | see below |
//! | `setMarkedText:selectedRange:replacementRange:` | `v@:@{_NSRange=QQ}{_NSRange=QQ}` | records the clauses and the replacement range for `ime.rs`, runs the original |
//! | `selectedRange` | `{_NSRange=QQ}@:` | answers from the page's text state if there is one |
//! | `attributedSubstringForProposedRange:actualRange:` | `@@:{_NSRange=QQ}^{_NSRange=QQ}` | answers from the page's text state if there is one |
//!
//! `hasMarkedText` (`c@:`, `B@:` on `arm64`) is called, not replaced; it must exist.
//!
//! `insertText:replacementRange:` goes to Mercury, and **not** to the original, in two cases only:
//!
//! * (a) no marked text, no key event being handled, a text that does not start with a control
//!   character: the emoji picker, dictation. winit would drop it. Mercury inserts it.
//! * (b) a replacement range (not `NSNotFound`), no marked text, and the page's text state is
//!   available: the accent menu, a dictation correction. winit would ignore the range; Mercury
//!   replaces that part of the field (the range is cut to the text state's window and names its
//!   `sequence`, so the engine can tell whether it is still current).
//!
//! In every other case, ordinary typing among them, the original runs, so typing stays on winit's
//! proven path. Two deviations from the plain reading of the lane's decision D9, both so that
//! winit's own bookkeeping never gets out of step: while winit has marked text, `insertText` (also
//! with a replacement range) goes to the original, and `selectedRange` and
//! `attributedSubstringForProposedRange:` are answered by the original, because winit's
//! `markedRange` counts from the start of the marked string and mixing it with ranges of the whole
//! field would confuse input methods.
//!
//! # Turning it off
//!
//! `MERCURY_NATIVE_TEXT_INPUT=0` (or `false`, `off`, `no`) in the environment installs nothing:
//! Mercury then behaves as it did before this layer, which is the way to tell whether a text
//! input problem comes from here.
//!
//! # Safety
//!
//! This is the `unsafe` code of the lane, besides the two Carbon calls of `secure_input.rs`. Each replacement is an `extern "C-unwind"`
//! function with the exact signature of the method it replaces; none may unwind into AppKit, so
//! the work that is not the original's is wrapped in `catch_unwind` and, if it panics, the
//! original runs instead. Every object pointer AppKit passes is treated as possibly nil and
//! checked before use, and strings are asked for their text only if they respond to `string`.
//!
//! Nothing here logs a payload: the text of the marked string, of an insertion and of the page
//! must not reach a log.

use std::cell::Cell;
use std::ffi::CStr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Method, Sel};
use objc2::{msg_send, sel};
use objc2_foundation::NSString;
use talaria_core::{ImeClause, ImeClauseKind, Utf16Range};

use crate::ime::{self, NativeIme};

/// The class winit gives its view.
const WINIT_VIEW_CLASS: &CStr = c"WinitView";
/// The environment variable that turns the layer off.
const KILL_SWITCH: &str = "MERCURY_NATIVE_TEXT_INPUT";

/// `NSRange`, declared here so that this module needs no feature of `objc2-foundation` beyond
/// what the application already enables. Same layout and encoding as Foundation's.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NSRange {
    location: usize,
    length: usize,
}

// SAFETY: `{_NSRange=QQ}`, two `NSUInteger`s, as objc2-foundation declares it.
unsafe impl Encode for NSRange {
    const ENCODING: Encoding = Encoding::Struct("_NSRange", &[usize::ENCODING, usize::ENCODING]);
}

// SAFETY: a pointer to the struct above.
unsafe impl RefEncode for NSRange {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

/// `NSNotFound`.
const NS_NOT_FOUND: usize = isize::MAX as usize;

const NOT_FOUND: NSRange = NSRange {
    location: NS_NOT_FOUND,
    length: 0,
};

type KeyDownFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
type InsertTextFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, NSRange);
type SetMarkedTextFn =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, NSRange, NSRange);
type SelectedRangeFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> NSRange;
type SubstringFn =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSRange, *mut NSRange) -> *mut AnyObject;

/// The implementations a patched class had before.
#[derive(Clone, Copy)]
struct Originals {
    key_down: KeyDownFn,
    insert_text: InsertTextFn,
    set_marked_text: SetMarkedTextFn,
    selected_range: SelectedRangeFn,
    attributed_substring: SubstringFn,
}

/// The patched classes (by address) and what each had before. One entry for `WinitView` in the
/// product; one per fake class in the tests.
static PATCHED: Mutex<Vec<(usize, Originals)>> = Mutex::new(Vec::new());
static WARNED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Why a class was left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallError {
    /// The class does not itself implement this selector.
    MissingSelector(&'static str),
    /// The method has other types than the replacement was written for.
    WrongEncoding(&'static str),
    /// The class was patched before.
    AlreadyInstalled,
}

/// Whether the environment value of the kill switch leaves the layer on.
fn enabled(value: Option<&str>) -> bool {
    !value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        )
    })
}

/// Logs a problem once per run, without any payload.
fn warn_once(message: &str) {
    if !WARNED.swap(true, Ordering::Relaxed) {
        log::warn!("native text input is off: {message}");
    }
}

/// Installs the layer on winit's view once the event loop has created it (the class is registered
/// with the first view), unless the kill switch is on. Call once, on the main thread, before the
/// event loop runs; it looks again for a short while if the class does not exist yet.
pub(crate) fn install() {
    install_with_switch(std::env::var(KILL_SWITCH).ok().as_deref());
}

/// `install` with the value of the kill switch given.
fn install_with_switch(switch: Option<&str>) {
    if !enabled(switch) {
        log::info!("native text input: off ({KILL_SWITCH})");
        return;
    }
    install_attempt(0);
}

/// How often and how long `install` looks for the class: the window is created when the event loop
/// starts, a few milliseconds after the call.
const ATTEMPTS: u32 = 100;
#[cfg(not(test))]
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_millis(50);

fn install_attempt(attempt: u32) {
    let Some(class) = AnyClass::get(WINIT_VIEW_CLASS) else {
        if attempt >= ATTEMPTS {
            warn_once("winit's view class was not found");
        } else {
            retry_later(attempt + 1);
        }
        return;
    };
    match install_on(class) {
        Ok(()) => {
            INSTALLED.store(true, Ordering::Relaxed);
            log::info!("native text input: installed");
        }
        Err(InstallError::AlreadyInstalled) => {}
        Err(InstallError::MissingSelector(name)) => {
            warn_once(&format!("winit's view has no `{name}` of its own"));
        }
        Err(InstallError::WrongEncoding(name)) => {
            warn_once(&format!(
                "`{name}` of winit's view has other types than expected"
            ));
        }
    }
}

#[cfg(not(test))]
fn retry_later(attempt: u32) {
    slint::Timer::single_shot(RETRY_AFTER, move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        install_attempt(attempt);
    });
}

/// Tests never wait for a window.
#[cfg(test)]
fn retry_later(_attempt: u32) {}

/// What a replacement must have: the selector and the types (the return type first, then
/// `self`, the selector and the arguments).
struct Spec {
    name: &'static str,
    selector: Sel,
    encodings: Vec<Encoding>,
}

fn specs() -> [Spec; 6] {
    let range = <NSRange as Encode>::ENCODING;
    let range_pointer = <NSRange as RefEncode>::ENCODING_REF;
    [
        Spec {
            name: "keyDown:",
            selector: sel!(keyDown:),
            encodings: vec![
                Encoding::Void,
                Encoding::Object,
                Encoding::Sel,
                Encoding::Object,
            ],
        },
        Spec {
            name: "insertText:replacementRange:",
            selector: sel!(insertText:replacementRange:),
            encodings: vec![
                Encoding::Void,
                Encoding::Object,
                Encoding::Sel,
                Encoding::Object,
                range.clone(),
            ],
        },
        Spec {
            name: "setMarkedText:selectedRange:replacementRange:",
            selector: sel!(setMarkedText:selectedRange:replacementRange:),
            encodings: vec![
                Encoding::Void,
                Encoding::Object,
                Encoding::Sel,
                Encoding::Object,
                range.clone(),
                range.clone(),
            ],
        },
        Spec {
            name: "selectedRange",
            selector: sel!(selectedRange),
            encodings: vec![range.clone(), Encoding::Object, Encoding::Sel],
        },
        Spec {
            name: "attributedSubstringForProposedRange:actualRange:",
            selector: sel!(attributedSubstringForProposedRange:actualRange:),
            encodings: vec![
                Encoding::Object,
                Encoding::Object,
                Encoding::Sel,
                range,
                range_pointer,
            ],
        },
        // Called, not replaced.
        Spec {
            name: "hasMarkedText",
            selector: sel!(hasMarkedText),
            encodings: vec![Bool::ENCODING, Encoding::Object, Encoding::Sel],
        },
    ]
}

/// Whether `method` has exactly the types in `expected`.
fn has_encodings(method: &Method, expected: &[Encoding]) -> bool {
    let Some((return_type, arguments)) = expected.split_first() else {
        return false;
    };
    if method.arguments_count() != arguments.len() {
        return false;
    }
    let return_matches = method
        .return_type()
        .to_str()
        .is_ok_and(|encoding| return_type.equivalent_to_str(encoding));
    return_matches
        && arguments.iter().enumerate().all(|(index, argument)| {
            method.argument_type(index).is_some_and(|encoding| {
                encoding
                    .to_str()
                    .is_ok_and(|encoding| argument.equivalent_to_str(encoding))
            })
        })
}

/// Replaces the five methods of `class` itself (the product's `WinitView`, a fake in the tests).
/// All or nothing: a missing selector or a type that is not the expected one leaves the class
/// as it was.
pub(crate) fn install_on(class: &'static AnyClass) -> Result<(), InstallError> {
    let address = std::ptr::from_ref(class) as usize;
    let mut patched = PATCHED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if patched.iter().any(|(patched, _)| *patched == address) {
        return Err(InstallError::AlreadyInstalled);
    }

    // The class's own methods only: an inherited one belongs to a superclass, which must not be
    // changed.
    let own = class.instance_methods();
    let mut found = Vec::new();
    for spec in specs() {
        let Some(method) = own.iter().find(|method| method.name() == spec.selector) else {
            return Err(InstallError::MissingSelector(spec.name));
        };
        if !has_encodings(method, &spec.encodings) {
            return Err(InstallError::WrongEncoding(spec.name));
        }
        found.push(*method);
    }
    let [
        key_down,
        insert_text,
        set_marked_text,
        selected_range,
        attributed_substring,
        _has_marked,
    ] = found[..]
    else {
        return Err(InstallError::MissingSelector("one of the methods"));
    };

    // SAFETY: each implementation was found under a selector whose types were just checked to be
    // those of the function type it is converted to.
    let originals = unsafe {
        Originals {
            key_down: std::mem::transmute::<Imp, KeyDownFn>(key_down.implementation()),
            insert_text: std::mem::transmute::<Imp, InsertTextFn>(insert_text.implementation()),
            set_marked_text: std::mem::transmute::<Imp, SetMarkedTextFn>(
                set_marked_text.implementation(),
            ),
            selected_range: std::mem::transmute::<Imp, SelectedRangeFn>(
                selected_range.implementation(),
            ),
            attributed_substring: std::mem::transmute::<Imp, SubstringFn>(
                attributed_substring.implementation(),
            ),
        }
    };
    // Known before the first call can arrive.
    patched.push((address, originals));
    // SAFETY: the replacements below have the signatures of the methods they replace (checked
    // above against the encodings) and call the originals recorded in `PATCHED`.
    unsafe {
        key_down.set_implementation(std::mem::transmute::<KeyDownFn, Imp>(replacement_key_down));
        insert_text.set_implementation(std::mem::transmute::<InsertTextFn, Imp>(
            replacement_insert_text,
        ));
        set_marked_text.set_implementation(std::mem::transmute::<SetMarkedTextFn, Imp>(
            replacement_set_marked_text,
        ));
        selected_range.set_implementation(std::mem::transmute::<SelectedRangeFn, Imp>(
            replacement_selected_range,
        ));
        attributed_substring.set_implementation(std::mem::transmute::<SubstringFn, Imp>(
            replacement_attributed_substring,
        ));
    }
    Ok(())
}

/// What the class of `object` (or its nearest patched superclass) had before.
fn originals_of(object: *mut AnyObject) -> Option<Originals> {
    // SAFETY: AppKit passes the receiver of a method of the patched class.
    let object = unsafe { object.as_ref() }?;
    let patched = PATCHED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut class = Some(object.class());
    while let Some(current) = class {
        let address = std::ptr::from_ref(current) as usize;
        if let Some((_, originals)) = patched.iter().find(|(patched, _)| *patched == address) {
            return Some(*originals);
        }
        class = current.superclass();
    }
    None
}

thread_local! {
    /// How many `keyDown:` calls are running on this thread (they do not nest in practice).
    static KEY_EVENTS: Cell<u32> = const { Cell::new(0) };
}

/// Held while the original `keyDown:` runs, also if it unwinds.
struct KeyEventScope;

impl KeyEventScope {
    fn enter() -> Self {
        KEY_EVENTS.with(|depth| depth.set(depth.get() + 1));
        Self
    }
}

impl Drop for KeyEventScope {
    fn drop(&mut self) {
        KEY_EVENTS.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

fn key_event_in_progress() -> bool {
    KEY_EVENTS.with(|depth| depth.get() > 0)
}

// ---------------------------------------------------------------------------------------------
// The replacements.
// ---------------------------------------------------------------------------------------------

/// `keyDown:`: the original runs inside a scope that says a key event is being handled, which is
/// how an `insertText:` from typing is told from one that comes from the emoji picker.
unsafe extern "C-unwind" fn replacement_key_down(
    this: *mut AnyObject,
    selector: Sel,
    event: *mut AnyObject,
) {
    let Some(originals) = originals_of(this) else {
        return;
    };
    let _scope = KeyEventScope::enter();
    // SAFETY: the original method of this class, called with the arguments it was given.
    unsafe { (originals.key_down)(this, selector, event) }
}

/// `insertText:replacementRange:`: cases (a) and (b) of the module documentation go to Mercury,
/// everything else to the original.
unsafe extern "C-unwind" fn replacement_insert_text(
    this: *mut AnyObject,
    selector: Sel,
    string: *mut AnyObject,
    range: NSRange,
) {
    let Some(originals) = originals_of(this) else {
        return;
    };
    let handled = catch_unwind(AssertUnwindSafe(|| {
        insert_text_for_mercury(this, string, range)
    }))
    .unwrap_or(false);
    if !handled {
        // SAFETY: the original method of this class, called with the arguments it was given.
        unsafe { (originals.insert_text)(this, selector, string, range) }
    }
}

/// `setMarkedText:selectedRange:replacementRange:`: the clauses and the replacement range are
/// kept for the `Ime::Preedit` winit reports for this call; the original runs.
unsafe extern "C-unwind" fn replacement_set_marked_text(
    this: *mut AnyObject,
    selector: Sel,
    string: *mut AnyObject,
    selected: NSRange,
    replacement: NSRange,
) {
    let Some(originals) = originals_of(this) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| record_marked_text(string, replacement)));
    // SAFETY: the original method of this class, called with the arguments it was given.
    unsafe { (originals.set_marked_text)(this, selector, string, selected, replacement) }
}

/// `selectedRange`: the selection of the page's focused field, when there is one.
unsafe extern "C-unwind" fn replacement_selected_range(
    this: *mut AnyObject,
    selector: Sel,
) -> NSRange {
    let Some(originals) = originals_of(this) else {
        return NOT_FOUND;
    };
    match catch_unwind(AssertUnwindSafe(|| selection_for_mercury(this))) {
        Ok(Some(range)) => range,
        // SAFETY: the original method of this class, called with the arguments it was given.
        _ => unsafe { (originals.selected_range)(this, selector) },
    }
}

/// `attributedSubstringForProposedRange:actualRange:`: the text of the page's focused field.
unsafe extern "C-unwind" fn replacement_attributed_substring(
    this: *mut AnyObject,
    selector: Sel,
    range: NSRange,
    actual: *mut NSRange,
) -> *mut AnyObject {
    let Some(originals) = originals_of(this) else {
        return std::ptr::null_mut();
    };
    match catch_unwind(AssertUnwindSafe(|| {
        substring_for_mercury(this, range, actual)
    })) {
        Ok(Some(answer)) => answer,
        // SAFETY: the original method of this class, called with the arguments it was given.
        _ => unsafe { (originals.attributed_substring)(this, selector, range, actual) },
    }
}

// ---------------------------------------------------------------------------------------------
// What the replacements do beyond the original.
// ---------------------------------------------------------------------------------------------

/// Whether winit has marked text now (the view's own `hasMarkedText`).
fn has_marked_text(this: &AnyObject) -> bool {
    // SAFETY: `hasMarkedText` exists on the class (checked at installation) and returns `BOOL`.
    let marked: Bool = unsafe { msg_send![this, hasMarkedText] };
    marked.as_bool()
}

/// The characters of an `NSString` or an `NSAttributedString`; `None` for anything else.
fn text_of(string: &AnyObject) -> Option<String> {
    if let Some(string) = string.downcast_ref::<NSString>() {
        return Some(string.to_string());
    }
    if !string.class().responds_to(sel!(string)) {
        return None;
    }
    // SAFETY: `-[NSAttributedString string]` returns an `NSString`; the object responds to it.
    let inner: Option<Retained<NSString>> = unsafe { msg_send![string, string] };
    inner.map(|inner| inner.to_string())
}

fn utf16_range(range: NSRange) -> Utf16Range {
    let start = u32::try_from(range.location).unwrap_or(u32::MAX);
    let end = u32::try_from(range.location.saturating_add(range.length)).unwrap_or(u32::MAX);
    Utf16Range::new(start, end)
}

/// Cases (a) and (b): `true` when Mercury took the text and the original must not run.
fn insert_text_for_mercury(this: *mut AnyObject, string: *mut AnyObject, range: NSRange) -> bool {
    // SAFETY: AppKit passes live objects or nil; nil is refused.
    let (Some(this), Some(string)) = (unsafe { this.as_ref() }, unsafe { string.as_ref() }) else {
        return false;
    };
    let Some(text) = text_of(string) else {
        return false;
    };
    // winit's own rule for what is no text: control characters are keys, not text.
    if text.chars().next().is_none_or(char::is_control) {
        return false;
    }
    // winit commits marked text itself and keeps its own state while it has some.
    if has_marked_text(this) {
        return false;
    }
    if range.location != NS_NOT_FOUND && ime::with_snapshot(|_| ()).is_some() {
        // (b): a replacement of text the page told us about. A range that cannot be mapped to
        // the page's text becomes an insertion at the selection; the engine decides.
        let replacement =
            ime::with_snapshot(|state| state.replacement_for(utf16_range(range))).flatten();
        ime::native_event(NativeIme::InsertText { text, replacement });
        return true;
    }
    if !key_event_in_progress() {
        // (a): not typing.
        ime::native_event(NativeIme::InsertText {
            text,
            replacement: None,
        });
        return true;
    }
    false
}

/// The marked string's clauses and the range of the field it replaces, for `ime.rs`.
fn record_marked_text(string: *mut AnyObject, replacement: NSRange) {
    // SAFETY: AppKit passes a live object or nil; nil is refused.
    let Some(string) = (unsafe { string.as_ref() }) else {
        return;
    };
    let Some((length, runs)) = attribute_runs(string) else {
        return;
    };
    if length == 0 {
        // The marked text is gone (a cancel); there is nothing to describe.
        return;
    }
    let replacement = (replacement.location != NS_NOT_FOUND)
        .then(|| ime::with_snapshot(|state| state.replacement_for(utf16_range(replacement))))
        .flatten()
        .flatten();
    ime::native_event(NativeIme::Marked {
        clauses: clauses_from_runs(&runs, length),
        replacement,
    });
}

/// `selectedRange` from the page's text state.
fn selection_for_mercury(this: *mut AnyObject) -> Option<NSRange> {
    // SAFETY: AppKit passes the receiver.
    let this = unsafe { this.as_ref() }?;
    if has_marked_text(this) {
        return None;
    }
    ime::with_snapshot(|state| {
        let selection = state.absolute_selection();
        NSRange {
            location: selection.start as usize,
            length: selection.len() as usize,
        }
    })
}

/// `attributedSubstringForProposedRange:actualRange:` from the page's text state: `None` leaves
/// the answer to the original, `Some(null)` answers nil (the range is outside the text we have).
fn substring_for_mercury(
    this: *mut AnyObject,
    range: NSRange,
    actual: *mut NSRange,
) -> Option<*mut AnyObject> {
    // SAFETY: AppKit passes the receiver.
    let this = unsafe { this.as_ref() }?;
    if has_marked_text(this) {
        return None;
    }
    let covered = ime::with_snapshot(|state| state.text_in(utf16_range(range)))?;
    let Some((covered, text)) = covered else {
        return Some(std::ptr::null_mut());
    };
    let class = AnyClass::get(c"NSAttributedString")?;
    let string = NSString::from_str(&text);
    // SAFETY: `+alloc` and `-initWithString:` of `NSAttributedString`.
    let allocated: Allocated<AnyObject> = unsafe { msg_send![class, alloc] };
    // SAFETY: as above; `-init…` consumes the allocation.
    let attributed: Option<Retained<AnyObject>> =
        unsafe { msg_send![allocated, initWithString: &*string] };
    let attributed = attributed?;
    // SAFETY: AppKit passes a pointer it owns, or null.
    if let Some(actual) = unsafe { actual.as_mut() } {
        *actual = NSRange {
            location: covered.start as usize,
            length: covered.len() as usize,
        };
    }
    // The caller gets a +0 (autoreleased) object, as for any method that is not `new`, `copy`…
    Some(Retained::autorelease_return(attributed))
}

// ---------------------------------------------------------------------------------------------
// The clauses of marked text.
// ---------------------------------------------------------------------------------------------

/// A run of the marked string with the same attributes, in UTF-16 units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AttributeRun {
    start: u32,
    end: u32,
    /// `NSUnderlineStyleAttributeName` (`"NSUnderline"`).
    underline: Option<i64>,
    /// `NSMarkedClauseSegmentAttributeName` (`"NSMarkedClauseSegment"`): counts the clauses.
    segment: Option<i64>,
}

/// `NSUnderlineStyleThick`: how the input method draws the clause being edited.
const UNDERLINE_THICK: i64 = 0x02;

/// The runs of a string, with the length of the string in UTF-16 units. A plain `NSString` is one
/// run without attributes. `None` if the object is neither kind of string.
fn attribute_runs(string: &AnyObject) -> Option<(u32, Vec<AttributeRun>)> {
    let attributed_class = AnyClass::get(c"NSAttributedString")?;
    let is_attributed: Bool = unsafe { msg_send![string, isKindOfClass: attributed_class] };
    if !is_attributed.as_bool() {
        let text = string.downcast_ref::<NSString>()?.to_string();
        let length = u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX);
        return Some((
            length,
            vec![AttributeRun {
                start: 0,
                end: length,
                underline: None,
                segment: None,
            }],
        ));
    }
    // SAFETY: the object is an `NSAttributedString`.
    let length: usize = unsafe { msg_send![string, length] };
    let length_units = u32::try_from(length).unwrap_or(u32::MAX);
    let mut runs = Vec::new();
    let mut index = 0_usize;
    // More runs than any input method makes: stop rather than loop on a strange answer.
    while index < length && runs.len() < 1024 {
        let mut effective = NSRange {
            location: index,
            length: 0,
        };
        // SAFETY: `index` is inside the string; `effective` is a valid out parameter.
        let attributes: *mut AnyObject = unsafe {
            msg_send![string, attributesAtIndex: index, effectiveRange: &mut effective as *mut NSRange]
        };
        let end = effective
            .location
            .saturating_add(effective.length)
            .min(length);
        if end <= index {
            break;
        }
        runs.push(AttributeRun {
            start: u32::try_from(index).unwrap_or(u32::MAX),
            end: u32::try_from(end).unwrap_or(u32::MAX),
            underline: number_attribute(attributes, "NSUnderline"),
            segment: number_attribute(attributes, "NSMarkedClauseSegment"),
        });
        index = end;
    }
    Some((length_units, runs))
}

/// The integer value of the attribute `name` in `attributes` (an `NSDictionary`), if it is a
/// number.
fn number_attribute(attributes: *mut AnyObject, name: &str) -> Option<i64> {
    // SAFETY: the dictionary comes from `attributesAtIndex:effectiveRange:`, nil or live.
    let attributes = unsafe { attributes.as_ref() }?;
    if !attributes.class().responds_to(sel!(objectForKey:)) {
        return None;
    }
    let key = NSString::from_str(name);
    // SAFETY: `-[NSDictionary objectForKey:]`.
    let value: *mut AnyObject = unsafe { msg_send![attributes, objectForKey: &*key] };
    // SAFETY: nil or a live object of the dictionary.
    let value = unsafe { value.as_ref() }?;
    if !value.class().responds_to(sel!(longLongValue)) {
        return None;
    }
    // SAFETY: `-[NSNumber longLongValue]`.
    let number: i64 = unsafe { msg_send![value, longLongValue] };
    Some(number)
}

/// The clauses of marked text of `length` units made of `runs`. Adjacent runs of one clause (the
/// same `NSMarkedClauseSegment`, or, without segments, the same underline) are one clause. The
/// clause with a thick underline is the one being edited. With several clauses the others are
/// converted text; a single clause is as typed, unless it is being edited (then the input method
/// has begun converting).
fn clauses_from_runs(runs: &[AttributeRun], length: u32) -> Vec<ImeClause> {
    /// Runs gathered into one clause.
    struct Group {
        start: u32,
        end: u32,
        selected: bool,
        /// What makes two runs one clause: the segment, or without segments the underline.
        identity: (Option<i64>, Option<i64>),
    }

    let mut groups: Vec<Group> = Vec::new();
    for run in runs {
        let thick = run
            .underline
            .is_some_and(|style| style & UNDERLINE_THICK != 0);
        let identity = if run.segment.is_some() {
            (None, run.segment)
        } else {
            (run.underline, None)
        };
        match groups.last_mut() {
            Some(last) if last.identity == identity && last.end == run.start => {
                last.end = run.end;
                last.selected |= thick;
            }
            _ => groups.push(Group {
                start: run.start,
                end: run.end,
                selected: thick,
                identity,
            }),
        }
    }
    groups.retain(|group| group.start < group.end && group.end <= length);
    let several = groups.len() > 1;
    groups
        .into_iter()
        .map(|group| ImeClause {
            range: Utf16Range::new(group.start, group.end),
            kind: match (group.selected, several) {
                (true, _) => ImeClauseKind::SelectedConverted,
                (false, true) => ImeClauseKind::Converted,
                (false, false) => ImeClauseKind::Raw,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Once;

    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{AnyThread, ClassType, DefinedClass, define_class};
    use objc2_foundation::NSNumber;
    use talaria_core::{BrowserController, ImeReplacement, TextInputState};

    use super::*;
    use crate::ime::{Input, NativeIme, take_native_events};

    /// What the fake view's original methods did, and what it pretends to hold.
    #[derive(Default)]
    struct FakeState {
        marked_text: Cell<bool>,
        calls: RefCell<Vec<&'static str>>,
    }

    // A fake of winit's view with the same selectors and signatures, and no AppKit window, view
    // or application: a plain NSObject subclass. Its "originals" only note that they ran.
    macro_rules! fake_view {
        ($name:ident, $runtime_name:literal) => {
            define_class!(
                // SAFETY: NSObject may be subclassed; the ivars are plain Rust values.
                #[unsafe(super(NSObject))]
                #[name = $runtime_name]
                #[ivars = FakeState]
                struct $name;

                impl $name {
                    // SAFETY: the signatures are winit's.
                    #[unsafe(method(keyDown:))]
                    fn key_down(&self, _event: &AnyObject) {
                        self.ivars().calls.borrow_mut().push("keyDown");
                        // What `interpretKeyEvents:` does for an ordinary key: text, no range.
                        let text = NSString::from_str("a");
                        let _: () = unsafe {
                            msg_send![self, insertText: &*text, replacementRange: NOT_FOUND]
                        };
                    }

                    #[unsafe(method(insertText:replacementRange:))]
                    fn insert_text(&self, _string: &AnyObject, _range: NSRange) {
                        self.ivars().calls.borrow_mut().push("insertText");
                    }

                    #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
                    fn set_marked_text(
                        &self,
                        _string: &AnyObject,
                        _selected: NSRange,
                        _replacement: NSRange,
                    ) {
                        self.ivars().calls.borrow_mut().push("setMarkedText");
                    }

                    #[unsafe(method(selectedRange))]
                    fn selected_range(&self) -> NSRange {
                        self.ivars().calls.borrow_mut().push("selectedRange");
                        NOT_FOUND
                    }

                    #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
                    fn attributed_substring(
                        &self,
                        _range: NSRange,
                        _actual: *mut NSRange,
                    ) -> Option<Retained<AnyObject>> {
                        self.ivars().calls.borrow_mut().push("attributedSubstring");
                        None
                    }

                    #[unsafe(method(hasMarkedText))]
                    fn has_marked_text(&self) -> Bool {
                        Bool::new(self.ivars().marked_text.get())
                    }
                }
            );

            impl $name {
                fn create() -> Retained<Self> {
                    let this = Self::alloc().set_ivars(FakeState::default());
                    // SAFETY: `-[NSObject init]`.
                    unsafe { msg_send![super(this), init] }
                }

                fn calls(&self) -> Vec<&'static str> {
                    self.ivars().calls.borrow().clone()
                }
            }
        };
    }

    fake_view!(FakeView, "MercuryTextInputTestView");
    // The name winit gives its view, so that `install` can find it as it does in the product.
    fake_view!(FakeWinitView, "WinitView");

    // A class with none of the selectors.
    define_class!(
        // SAFETY: NSObject may be subclassed; no ivars.
        #[unsafe(super(NSObject))]
        #[name = "MercuryTextInputTestBareView"]
        struct BareView;
    );

    // A class with all selectors except `selectedRange`.
    define_class!(
        // SAFETY: NSObject may be subclassed; no ivars.
        #[unsafe(super(NSObject))]
        #[name = "MercuryTextInputTestAlmostView"]
        struct AlmostView;

        impl AlmostView {
            #[unsafe(method(keyDown:))]
            fn key_down(&self, _event: &AnyObject) {}
            #[unsafe(method(insertText:replacementRange:))]
            fn insert_text(&self, _string: &AnyObject, _range: NSRange) {}
            #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
            fn set_marked_text(&self, _string: &AnyObject, _selected: NSRange, _replacement: NSRange) {}
            #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
            fn attributed_substring(
                &self,
                _range: NSRange,
                _actual: *mut NSRange,
            ) -> Option<Retained<AnyObject>> {
                None
            }
            #[unsafe(method(hasMarkedText))]
            fn has_marked_text(&self) -> Bool {
                Bool::NO
            }
        }
    );

    // A class that has every selector, one of them with other types.
    define_class!(
        // SAFETY: NSObject may be subclassed; no ivars.
        #[unsafe(super(NSObject))]
        #[name = "MercuryTextInputTestOddView"]
        struct OddView;

        impl OddView {
            #[unsafe(method(keyDown:))]
            fn key_down(&self, _event: &AnyObject) {}
            // Wrong: a plain integer where an NSRange is expected.
            #[unsafe(method(insertText:replacementRange:))]
            fn insert_text(&self, _string: &AnyObject, _range: usize) {}
            #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
            fn set_marked_text(
                &self,
                _string: &AnyObject,
                _selected: NSRange,
                _replacement: NSRange,
            ) {
            }
            #[unsafe(method(selectedRange))]
            fn selected_range(&self) -> NSRange {
                NOT_FOUND
            }
            #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
            fn attributed_substring(
                &self,
                _range: NSRange,
                _actual: *mut NSRange,
            ) -> Option<Retained<AnyObject>> {
                None
            }
            #[unsafe(method(hasMarkedText))]
            fn has_marked_text(&self) -> Bool {
                Bool::NO
            }
        }
    );

    /// The fake class, patched once for the whole test run by the same function the product uses.
    fn patched_view() -> Retained<FakeView> {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            install_on(FakeView::class()).expect("the fake class can be patched");
        });
        FakeView::create()
    }

    fn insert(view: &AnyObject, text: &str, range: NSRange) {
        let text = NSString::from_str(text);
        let _: () = unsafe { msg_send![view, insertText: &*text, replacementRange: range] };
    }

    fn key_down(view: &AnyObject) {
        let event = NSObject::new();
        let _: () = unsafe { msg_send![view, keyDown: &*event] };
    }

    fn set_marked(view: &AnyObject, string: &AnyObject, replacement: NSRange) {
        let _: () = unsafe {
            msg_send![view, setMarkedText: string, selectedRange: NSRange { location: 0, length: 0 }, replacementRange: replacement]
        };
    }

    fn selected_range(view: &AnyObject) -> NSRange {
        unsafe { msg_send![view, selectedRange] }
    }

    fn substring(view: &AnyObject, range: NSRange) -> (Option<String>, NSRange) {
        let mut actual = NSRange {
            location: 12345,
            length: 678,
        };
        let answer: Option<Retained<AnyObject>> = unsafe {
            msg_send![view, attributedSubstringForProposedRange: range, actualRange: &mut actual as *mut NSRange]
        };
        (answer.and_then(|answer| text_of(&answer)), actual)
    }

    fn attributed(
        text: &str,
        attributes: &[((usize, usize), i64, Option<i64>)],
    ) -> Retained<AnyObject> {
        let class = AnyClass::get(c"NSMutableAttributedString").expect("Foundation is loaded");
        let string = NSString::from_str(text);
        // SAFETY: `+alloc`, `-initWithString:` of `NSMutableAttributedString`.
        let allocated: Allocated<AnyObject> = unsafe { msg_send![class, alloc] };
        let attributed: Retained<AnyObject> =
            unsafe { msg_send![allocated, initWithString: &*string] };
        for ((location, length), underline, segment) in attributes {
            let range = NSRange {
                location: *location,
                length: *length,
            };
            let underline_key = NSString::from_str("NSUnderline");
            let underline_value = NSNumber::new_i64(*underline);
            let _: () = unsafe {
                msg_send![&*attributed, addAttribute: &*underline_key, value: &*underline_value, range: range]
            };
            if let Some(segment) = segment {
                let segment_key = NSString::from_str("NSMarkedClauseSegment");
                let segment_value = NSNumber::new_i64(*segment);
                let _: () = unsafe {
                    msg_send![&*attributed, addAttribute: &*segment_key, value: &*segment_value, range: range]
                };
            }
        }
        attributed
    }

    fn state(sequence: u64, text: &str, text_start: u32, selection: (u32, u32)) -> TextInputState {
        TextInputState {
            document: 1,
            sequence,
            text: text.to_owned(),
            text_start,
            selection: Utf16Range::new(selection.0, selection.1),
            marked: None,
            caret_rect: None,
            multiline: false,
            is_password: false,
        }
    }

    fn range(location: usize, length: usize) -> NSRange {
        NSRange { location, length }
    }

    // ---- the patch ----

    #[test]
    fn typing_inside_a_key_event_reaches_only_the_original() {
        let _guard = ime::test_guard();
        let view = patched_view();

        key_down(&view);

        // The fake `keyDown:` ran, called `insertText:` as `interpretKeyEvents:` would, and the
        // original `insertText:` handled it; Mercury heard nothing.
        assert_eq!(view.calls(), vec!["keyDown", "insertText"]);
        assert_eq!(take_native_events(), Vec::new());
        // The scope ended with the key event.
        assert!(!key_event_in_progress());
    }

    #[test]
    fn an_insertion_outside_a_key_event_with_no_marked_text_reaches_mercury_not_the_original() {
        let _guard = ime::test_guard();
        let view = patched_view();

        insert(&view, "😀", NOT_FOUND);

        assert_eq!(view.calls(), Vec::<&str>::new());
        assert_eq!(
            take_native_events(),
            vec![NativeIme::InsertText {
                text: "😀".to_owned(),
                replacement: None,
            }]
        );
    }

    #[test]
    fn an_insertion_while_winit_has_marked_text_stays_with_winit() {
        let _guard = ime::test_guard();
        let view = patched_view();
        view.ivars().marked_text.set(true);

        insert(&view, "你", NOT_FOUND);
        insert(&view, "é", range(3, 1));

        assert_eq!(view.calls(), vec!["insertText", "insertText"]);
        assert_eq!(take_native_events(), Vec::new());
    }

    #[test]
    fn control_characters_and_empty_text_are_not_insertions() {
        let _guard = ime::test_guard();
        let view = patched_view();

        insert(&view, "\r", NOT_FOUND);
        insert(&view, "\u{7f}", NOT_FOUND);
        insert(&view, "", NOT_FOUND);

        assert_eq!(view.calls(), vec!["insertText", "insertText", "insertText"]);
        assert_eq!(take_native_events(), Vec::new());
    }

    #[test]
    fn a_replacement_range_reaches_mercury_with_the_range_of_the_text_state() {
        let _guard = ime::test_guard();
        let view = patched_view();
        // The window starts at 100 of the field: "hello" is units 100..105.
        ime::force_snapshot(Some(state(7, "hello", 100, (5, 5))));

        // The accent menu replaces the "e" (units 101..102) with "é".
        insert(&view, "é", range(101, 1));

        assert_eq!(view.calls(), Vec::<&str>::new());
        assert_eq!(
            take_native_events(),
            vec![NativeIme::InsertText {
                text: "é".to_owned(),
                replacement: Some(ImeReplacement {
                    range: Utf16Range::new(1, 2),
                    sequence: 7,
                }),
            }]
        );
    }

    #[test]
    fn a_replacement_range_inside_a_key_event_still_goes_to_mercury() {
        // Choosing an accent with a number key is a key event too.
        let _guard = ime::test_guard();
        let view = patched_view();
        ime::force_snapshot(Some(state(7, "hello", 0, (5, 5))));
        let _scope = KeyEventScope::enter();

        insert(&view, "é", range(1, 1));

        assert_eq!(view.calls(), Vec::<&str>::new());
        assert_eq!(take_native_events().len(), 1);
    }

    #[test]
    fn a_replacement_range_outside_the_text_state_becomes_an_insertion() {
        let _guard = ime::test_guard();
        let view = patched_view();
        ime::force_snapshot(Some(state(7, "hello", 100, (5, 5))));

        insert(&view, "é", range(3, 1));

        assert_eq!(
            take_native_events(),
            vec![NativeIme::InsertText {
                text: "é".to_owned(),
                replacement: None,
            }]
        );
    }

    #[test]
    fn a_replacement_range_without_a_text_state_is_an_insertion_outside_key_events_only() {
        let _guard = ime::test_guard();
        let view = patched_view();
        ime::force_snapshot(None);

        insert(&view, "é", range(1, 1));
        assert_eq!(
            take_native_events(),
            vec![NativeIme::InsertText {
                text: "é".to_owned(),
                replacement: None,
            }]
        );

        // Inside a key event with no text state it is typing: the original.
        let _scope = KeyEventScope::enter();
        insert(&view, "é", range(1, 1));
        assert_eq!(view.calls(), vec!["insertText"]);
        assert_eq!(take_native_events(), Vec::new());
    }

    #[test]
    fn set_marked_text_runs_the_original_and_leaves_one_record_for_ime_to_consume_once() {
        let _guard = ime::test_guard();
        let view = patched_view();
        let controller = Rc::new(RefCell::new(BrowserController::default()));
        let tab = controller.borrow().active_tab_id();
        controller.borrow_mut().handle_event(
            tab,
            talaria_core::BrowserEvent::AddressChanged { url: String::new() },
        );
        ime::attach(&controller, || {}, || true);
        // Japanese: two clauses, the second one being edited (thick underline).
        let marked = attributed("にほん語", &[((0, 3), 1, Some(0)), ((3, 1), 2, Some(1))]);

        set_marked(&view, &marked, NOT_FOUND);

        assert_eq!(view.calls(), vec!["setMarkedText"]);
        let expected_clauses = vec![
            ImeClause {
                range: Utf16Range::new(0, 3),
                kind: ImeClauseKind::Converted,
            },
            ImeClause {
                range: Utf16Range::new(3, 4),
                kind: ImeClauseKind::SelectedConverted,
            },
        ];
        assert_eq!(
            take_native_events(),
            vec![NativeIme::Marked {
                clauses: expected_clauses.clone(),
                replacement: None,
            }]
        );
        // winit's Preedit for the same call: ime.rs uses the record, once.
        // "にほん語" is 12 bytes in UTF-8, 4 units in UTF-16.
        ime::handle(
            &controller,
            Input::Preedit {
                text: "にほん語".to_owned(),
                cursor: Some((9, 12)),
            },
        );
        ime::handle(
            &controller,
            Input::Preedit {
                text: "にほん語".to_owned(),
                cursor: Some((9, 12)),
            },
        );
        let mut sent = Vec::new();
        while let Some(command) = controller.borrow_mut().next_command() {
            if let talaria_core::BrowserCommand::ImeInput(talaria_core::ImeInput::SetMarkedText {
                clauses,
                selected,
                ..
            }) = command.command()
            {
                sent.push((clauses.clone(), *selected));
            }
        }
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], (expected_clauses, Some(Utf16Range::new(3, 4))));
        // The second Preedit had no record: one clause over the text, the selected kind.
        assert_eq!(
            sent[1].0,
            vec![ImeClause {
                range: Utf16Range::new(0, 4),
                kind: ImeClauseKind::SelectedRaw,
            }]
        );
    }

    #[test]
    fn set_marked_text_with_a_plain_string_records_one_raw_clause() {
        let _guard = ime::test_guard();
        let view = patched_view();
        let plain = NSString::from_str("ni");

        set_marked(&view, &plain, NOT_FOUND);

        assert_eq!(view.calls(), vec!["setMarkedText"]);
        assert_eq!(
            take_native_events(),
            vec![NativeIme::Marked {
                clauses: vec![ImeClause {
                    range: Utf16Range::new(0, 2),
                    kind: ImeClauseKind::Raw,
                }],
                replacement: None,
            }]
        );
    }

    #[test]
    fn set_marked_text_records_the_replacement_range_against_the_text_state() {
        let _guard = ime::test_guard();
        let view = patched_view();
        ime::force_snapshot(Some(state(4, "abcdef", 0, (2, 4))));
        let plain = NSString::from_str("x");

        // Reconversion: the marked text takes the place of units 2..4.
        set_marked(&view, &plain, range(2, 2));

        let events = take_native_events();
        let [NativeIme::Marked { replacement, .. }] = events.as_slice() else {
            panic!("one marked record expected, got {events:?}");
        };
        assert_eq!(
            *replacement,
            Some(ImeReplacement {
                range: Utf16Range::new(2, 4),
                sequence: 4,
            })
        );
    }

    #[test]
    fn an_empty_marked_string_leaves_no_record() {
        let _guard = ime::test_guard();
        let view = patched_view();
        let empty = NSString::from_str("");

        set_marked(&view, &empty, NOT_FOUND);

        // The original still ran (winit turns it into the empty preedit that cancels).
        assert_eq!(view.calls(), vec!["setMarkedText"]);
        assert_eq!(take_native_events(), Vec::new());
    }

    #[test]
    fn selected_range_and_substring_answer_from_the_text_state() {
        let _guard = ime::test_guard();
        let view = patched_view();
        // Units 10..16 of the field: "ab😀cd" is 6 units; the selection is "😀" (12..14).
        ime::force_snapshot(Some(state(3, "ab😀cd", 10, (2, 4))));

        assert_eq!(selected_range(&view), range(12, 2));
        assert_eq!(
            substring(&view, range(10, 6)),
            (Some("ab😀cd".to_owned()), range(10, 6))
        );
        // Cut to the window.
        assert_eq!(
            substring(&view, range(0, 100)),
            (Some("ab😀cd".to_owned()), range(10, 6))
        );
        // A range that ends inside the pair leaves the pair out.
        assert_eq!(
            substring(&view, range(11, 2)),
            (Some("b".to_owned()), range(11, 1))
        );
        // Outside the window: nil, and the original is not asked.
        assert_eq!(substring(&view, range(0, 5)).0, None);
        assert_eq!(view.calls(), Vec::<&str>::new());
    }

    #[test]
    fn without_a_text_state_the_original_answers() {
        let _guard = ime::test_guard();
        let view = patched_view();
        ime::force_snapshot(None);

        assert_eq!(selected_range(&view), NOT_FOUND);
        assert_eq!(substring(&view, range(0, 3)).0, None);

        assert_eq!(view.calls(), vec!["selectedRange", "attributedSubstring"]);
    }

    #[test]
    fn while_winit_has_marked_text_the_original_answers() {
        let _guard = ime::test_guard();
        let view = patched_view();
        view.ivars().marked_text.set(true);
        ime::force_snapshot(Some(state(3, "hello", 0, (1, 1))));

        assert_eq!(selected_range(&view), NOT_FOUND);
        assert_eq!(substring(&view, range(0, 3)).0, None);

        assert_eq!(view.calls(), vec!["selectedRange", "attributedSubstring"]);
    }

    #[test]
    fn a_password_field_has_no_text_to_answer_with() {
        let _guard = ime::test_guard();
        let view = patched_view();
        let mut password = state(3, "", 0, (0, 0));
        password.is_password = true;
        ime::force_snapshot(Some(password));

        assert_eq!(selected_range(&view), range(0, 0));
        // Nothing to give, and nothing asked of the original: nil.
        assert_eq!(substring(&view, range(0, 5)).0, Some(String::new()));
    }

    // ---- installing ----

    #[test]
    fn a_class_without_the_selectors_is_left_alone_and_nothing_panics() {
        let _guard = ime::test_guard();

        let result = install_on(BareView::class());

        assert_eq!(result, Err(InstallError::MissingSelector("keyDown:")));
        let patched = PATCHED.lock().unwrap();
        let address = std::ptr::from_ref(BareView::class()) as usize;
        assert!(patched.iter().all(|(patched, _)| *patched != address));
    }

    fn address_of(class: &AnyClass, selector: Sel) -> usize {
        class
            .instance_method(selector)
            .expect("the method exists")
            .implementation() as usize
    }

    #[test]
    fn one_missing_selector_changes_none_of_the_others() {
        let _guard = ime::test_guard();
        let class = AlmostView::class();
        let key_down_before = address_of(class, sel!(keyDown:));
        let insert_before = address_of(class, sel!(insertText:replacementRange:));

        let result = install_on(class);

        assert_eq!(result, Err(InstallError::MissingSelector("selectedRange")));
        assert_eq!(address_of(class, sel!(keyDown:)), key_down_before);
        assert_eq!(
            address_of(class, sel!(insertText:replacementRange:)),
            insert_before
        );
    }

    #[test]
    fn a_method_with_other_types_is_refused_before_anything_changes() {
        let _guard = ime::test_guard();
        let class = OddView::class();
        let key_down_before = address_of(class, sel!(keyDown:));

        let result = install_on(class);

        assert_eq!(
            result,
            Err(InstallError::WrongEncoding("insertText:replacementRange:"))
        );
        assert_eq!(address_of(class, sel!(keyDown:)), key_down_before);
    }

    #[test]
    fn a_class_is_patched_once() {
        let _guard = ime::test_guard();
        let _view = patched_view();

        assert_eq!(
            install_on(FakeView::class()),
            Err(InstallError::AlreadyInstalled)
        );
    }

    #[test]
    fn the_kill_switch_reads_its_variable() {
        assert!(enabled(None));
        assert!(enabled(Some("1")));
        assert!(enabled(Some("")));
        assert!(enabled(Some("yes")));
        assert!(!enabled(Some("0")));
        assert!(!enabled(Some(" 0 ")));
        assert!(!enabled(Some("off")));
        assert!(!enabled(Some("FALSE")));
        assert!(!enabled(Some("No")));
    }

    #[test]
    fn a_view_named_like_winits_is_found_by_install_and_the_kill_switch_stops_it() {
        let _guard = ime::test_guard();
        let class = FakeWinitView::class();
        let before = address_of(class, sel!(insertText:replacementRange:));

        // The switch is on: nothing is looked for and nothing changes.
        install_with_switch(Some("0"));
        assert!(!INSTALLED.load(Ordering::Relaxed));
        assert_eq!(
            address_of(class, sel!(insertText:replacementRange:)),
            before
        );
        let view = FakeWinitView::create();
        insert(&view, "😀", NOT_FOUND);
        assert_eq!(view.calls(), vec!["insertText"]);
        assert_eq!(take_native_events(), Vec::new());

        // Without the switch the class is found by its name and patched.
        install_with_switch(None);
        assert!(INSTALLED.load(Ordering::Relaxed));
        assert_ne!(
            address_of(class, sel!(insertText:replacementRange:)),
            before
        );
        let view = FakeWinitView::create();
        insert(&view, "😀", NOT_FOUND);
        assert_eq!(view.calls(), Vec::<&str>::new());
        assert_eq!(take_native_events().len(), 1);
        // Asking again changes nothing and does not warn.
        install_with_switch(None);
    }

    // ---- type encodings ----

    #[test]
    fn the_expected_encodings_are_what_appkit_declares() {
        // `NSRange` is two `NSUInteger`s; on a 64-bit Mac that is `{_NSRange=QQ}`.
        let range = <NSRange as Encode>::ENCODING.to_string();
        assert_eq!(range, "{_NSRange=QQ}");
        let by_name = |name: &str| {
            specs()
                .into_iter()
                .find(|spec| spec.name == name)
                .expect("the spec exists")
                .encodings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("")
        };
        assert_eq!(by_name("keyDown:"), "v@:@");
        assert_eq!(by_name("insertText:replacementRange:"), "v@:@{_NSRange=QQ}");
        assert_eq!(
            by_name("setMarkedText:selectedRange:replacementRange:"),
            "v@:@{_NSRange=QQ}{_NSRange=QQ}"
        );
        assert_eq!(by_name("selectedRange"), "{_NSRange=QQ}@:");
        assert_eq!(
            by_name("attributedSubstringForProposedRange:actualRange:"),
            "@@:{_NSRange=QQ}^{_NSRange=QQ}"
        );
        let has_marked = by_name("hasMarkedText");
        assert!(
            has_marked == "c@:" || has_marked == "B@:",
            "BOOL is c on x86_64 and B on arm64, got {has_marked}"
        );
    }

    #[test]
    fn the_expected_encodings_equal_those_of_the_methods_they_replace() {
        // The fake's methods carry the encodings objc2 derives from the same Rust types winit's
        // view declares; the check `install_on` makes on `WinitView` is this one.
        let class = FakeView::class();
        for spec in specs() {
            let method = class
                .instance_methods()
                .iter()
                .copied()
                .find(|method| method.name() == spec.selector)
                .expect("the fake has the selector");
            assert!(
                has_encodings(method, &spec.encodings),
                "{} differs from {:?}",
                spec.name,
                spec.encodings
            );
        }
    }

    #[test]
    fn appkits_own_description_of_nstextinputclient_has_the_same_types() {
        // AppKit declares `NSTextInputClient`; the description of each of our selectors in that
        // protocol, with the frame sizes and offsets removed, is what the replacements must
        // have. Loading the framework creates no window, view or application object.
        #[link(name = "AppKit", kind = "framework")]
        unsafe extern "C" {}
        let Some(protocol) = objc2::runtime::AnyProtocol::get(c"NSTextInputClient") else {
            panic!("AppKit declares NSTextInputClient once it is loaded");
        };
        for spec in specs() {
            if spec.name == "keyDown:" {
                // An NSResponder method, not part of the protocol.
                continue;
            }
            // SAFETY: a protocol and a selector.
            let description = unsafe {
                objc2::ffi::protocol_getMethodDescription(
                    std::ptr::from_ref(protocol).cast(),
                    spec.selector,
                    Bool::YES,
                    Bool::YES,
                )
            };
            if description.types.is_null() {
                // Optional in the protocol.
                continue;
            }
            // SAFETY: a C string owned by the runtime.
            let types = unsafe { CStr::from_ptr(description.types) }.to_string_lossy();
            let stripped: String = types.chars().filter(|c| !c.is_ascii_digit()).collect();
            let expected = spec
                .encodings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("");
            assert!(
                stripped == expected || (spec.name == "hasMarkedText" && stripped.len() == 3),
                "{}: AppKit says {types}, the replacement is written for {expected}",
                spec.name
            );
        }
    }

    // ---- clauses ----

    fn run(start: u32, end: u32, underline: Option<i64>, segment: Option<i64>) -> AttributeRun {
        AttributeRun {
            start,
            end,
            underline,
            segment,
        }
    }

    #[test]
    fn no_attributes_make_one_raw_clause() {
        assert_eq!(
            clauses_from_runs(&[run(0, 2, None, None)], 2),
            vec![ImeClause {
                range: Utf16Range::new(0, 2),
                kind: ImeClauseKind::Raw,
            }]
        );
    }

    #[test]
    fn a_thin_underline_is_raw_and_a_thick_one_is_the_clause_being_edited() {
        assert_eq!(
            clauses_from_runs(&[run(0, 3, Some(1), None)], 3)[0].kind,
            ImeClauseKind::Raw
        );
        assert_eq!(
            clauses_from_runs(&[run(0, 3, Some(2), None)], 3)[0].kind,
            ImeClauseKind::SelectedConverted
        );
        // A double underline is not a thick one.
        assert_eq!(
            clauses_from_runs(&[run(0, 3, Some(9), None)], 3)[0].kind,
            ImeClauseKind::Raw
        );
    }

    #[test]
    fn several_clauses_are_converted_except_the_one_being_edited() {
        let clauses = clauses_from_runs(
            &[
                run(0, 2, Some(1), Some(0)),
                run(2, 3, Some(2), Some(1)),
                run(3, 5, Some(1), Some(2)),
            ],
            5,
        );

        assert_eq!(
            clauses,
            vec![
                ImeClause {
                    range: Utf16Range::new(0, 2),
                    kind: ImeClauseKind::Converted,
                },
                ImeClause {
                    range: Utf16Range::new(2, 3),
                    kind: ImeClauseKind::SelectedConverted,
                },
                ImeClause {
                    range: Utf16Range::new(3, 5),
                    kind: ImeClauseKind::Converted,
                },
            ]
        );
    }

    #[test]
    fn runs_of_one_segment_are_one_clause() {
        // Another attribute (a font, say) split the clause into two runs.
        let clauses = clauses_from_runs(
            &[
                run(0, 1, Some(1), Some(7)),
                run(1, 3, Some(1), Some(7)),
                run(3, 4, Some(1), Some(8)),
            ],
            4,
        );

        assert_eq!(
            clauses
                .iter()
                .map(|clause| clause.range)
                .collect::<Vec<_>>(),
            vec![Utf16Range::new(0, 3), Utf16Range::new(3, 4)]
        );
    }

    #[test]
    fn without_segments_equal_underlines_merge_and_different_ones_split() {
        let clauses = clauses_from_runs(
            &[
                run(0, 1, Some(1), None),
                run(1, 2, Some(1), None),
                run(2, 3, Some(2), None),
            ],
            3,
        );

        assert_eq!(
            clauses
                .iter()
                .map(|clause| clause.range)
                .collect::<Vec<_>>(),
            vec![Utf16Range::new(0, 2), Utf16Range::new(2, 3)]
        );
    }

    #[test]
    fn runs_beyond_the_length_are_dropped() {
        assert_eq!(
            clauses_from_runs(&[run(0, 9, Some(1), None)], 3),
            Vec::new()
        );
    }

    #[test]
    fn attribute_runs_read_a_real_attributed_string() {
        let string = attributed("日本語ab", &[((0, 3), 1, Some(0)), ((3, 2), 2, Some(1))]);

        let (length, runs) = attribute_runs(&string).expect("an attributed string");

        assert_eq!(length, 5);
        assert_eq!(
            runs,
            vec![run(0, 3, Some(1), Some(0)), run(3, 5, Some(2), Some(1))]
        );
    }

    #[test]
    fn attribute_runs_of_a_string_without_attributes_are_one_run() {
        let plain = NSString::from_str("a😀");

        let (length, runs) = attribute_runs(&plain).expect("a string");

        assert_eq!(length, 3);
        assert_eq!(runs, vec![run(0, 3, None, None)]);
        // Something that is no string at all.
        let object = NSObject::new();
        assert!(attribute_runs(&object).is_none());
    }

    #[test]
    fn text_of_reads_both_kinds_of_string_and_refuses_the_rest() {
        let plain = NSString::from_str("é");
        let attributed = attributed("é", &[]);
        let object = NSObject::new();

        assert_eq!(text_of(&plain).as_deref(), Some("é"));
        assert_eq!(text_of(&attributed).as_deref(), Some("é"));
        assert_eq!(text_of(&object), None);
    }
}
