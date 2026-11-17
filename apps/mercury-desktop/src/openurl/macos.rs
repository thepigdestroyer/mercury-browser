//! Links and files macOS sends to Mercury Browser: a link clicked in Mail or Slack arrives as a
//! get-URL Apple event (`kInternetEventClass`/`kAEGetURL`), a file opened from Finder as
//! `application:openURLs:` (AppKit turns the open-documents event into that call, or into
//! `application:openFiles:` for a delegate without it).
//!
//! winit 0.30's application delegate implements neither, so these methods are added to its class
//! at run time through `macos_app_delegate::add_delegate_method`, as the terminate handler is:
//! only methods the delegate does not answer already, and the delegate is assigned again so that
//! AppKit sees them. The get-URL handler installed here receives
//! the events that arrive before the event loop runs; once AppKit has finished launching, its own
//! get-URL handler takes over and calls `application:openURLs:` (observed with events sent to
//! this process), so both paths end in `super::deliver`, which queues the URLs until the
//! controller exists.
//!
//! Also: the system's handler for a page's `mailto:` or `tel:` link (`WorkspaceOpener`, used by
//! normal runs only), and the diagnostics that send Apple events to this process only.

use std::ffi::CStr;
use std::panic::AssertUnwindSafe;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Imp, Sel};
use objc2::{MainThreadMarker, Message, class, msg_send, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;
use url::Url;

use super::external::ExternalOpener;
use crate::macos_app_delegate::add_delegate_method;

const fn four_char_code(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

/// `kInternetEventClass` and `kAEGetURL` (`'GURL'`, AppleEvents.h / AERegistry.h).
const INTERNET_EVENT_CLASS: u32 = four_char_code(b"GURL");
const GET_URL: u32 = four_char_code(b"GURL");
/// `kCoreEventClass` and `kAEOpenDocuments`.
const CORE_EVENT_CLASS: u32 = four_char_code(b"aevt");
const OPEN_DOCUMENTS: u32 = four_char_code(b"odoc");
/// `keyDirectObject`.
const DIRECT_OBJECT: u32 = four_char_code(b"----");
/// `NSApplicationDelegateReplySuccess`.
const REPLY_SUCCESS: usize = 0;
/// `NSAppleEventSendNoReply`.
const SEND_NO_REPLY: usize = 1;

/// `void method:(id)a second:(id)b`.
const TWO_OBJECTS: &CStr = c"v@:@@";

type TwoObjectMethod =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject);

/// Call on the main thread once the backend (and with it winit's application delegate) exists,
/// before the event loop runs.
pub(crate) fn receive_urls_from_other_apps() {
    keep_command_line_arguments_from_appkit();
    // SAFETY: each function's signature matches its type encoding: no result, `self`, `_cmd`
    // and two objects; none reads the delegate's state or unwinds.
    let two =
        |method: TwoObjectMethod| unsafe { std::mem::transmute::<TwoObjectMethod, Imp>(method) };
    for (selector, imp) in [
        (sel!(application:openURLs:), two(open_urls)),
        (sel!(application:openFiles:), two(open_files)),
        (
            sel!(mercuryHandleGetURLEvent:withReplyEvent:),
            two(handle_get_url_event),
        ),
    ] {
        if let Err(error) = add_delegate_method(selector, imp, TWO_OBJECTS) {
            log::warn!("openurl: {selector} was not added to the application delegate: {error:?}");
        }
    }
    match application_delegate() {
        Some(delegate) => install_get_url_handler(&delegate),
        None => {
            log::warn!("openurl: links from other apps are not handled: no application delegate")
        }
    }
}

/// Once the delegate answers `application:openFiles:`, AppKit hands it every command-line
/// argument that does not start with `-` as a file to open (tried: `https://a.test/x.html` came
/// back as the file `<current directory>/https:/a.test/x.html`). The command line is Mercury's
/// own (`cli.rs`); `NSTreatUnknownArgumentsAsOpen` = NO turns that off, as Chromium does. It is
/// registered in the registration domain, which lives in memory for this run and is never
/// written to the user's preferences.
fn keep_command_line_arguments_from_appkit() {
    // SAFETY: `standardUserDefaults` and `registerDefaults:` with an `NSDictionary` of
    // strings, and the dictionary constructor, are documented with these types.
    unsafe {
        let key = NSString::from_str("NSTreatUnknownArgumentsAsOpen");
        let value = NSString::from_str("NO");
        let defaults: Retained<AnyObject> = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let registration: Retained<AnyObject> = msg_send![
            class!(NSDictionary),
            dictionaryWithObject: &*value,
            forKey: &*key
        ];
        let _: () = msg_send![&defaults, registerDefaults: &*registration];
    }
}

fn application_delegate() -> Option<Retained<AnyObject>> {
    let mtm = MainThreadMarker::new()?;
    let delegate = NSApplication::sharedApplication(mtm).delegate()?;
    let delegate: &AnyObject = delegate.as_ref();
    Some(delegate.retain())
}

fn install_get_url_handler(delegate: &AnyObject) {
    // SAFETY: `setEventHandler:andSelector:forEventClass:andEventID:` takes an object, a
    // selector and two 32-bit four-character codes; the selector was added to the handler's
    // class with the signature the manager calls.
    unsafe {
        let manager: Retained<AnyObject> =
            msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
        let _: () = msg_send![
            &manager,
            setEventHandler: delegate,
            andSelector: sel!(mercuryHandleGetURLEvent:withReplyEvent:),
            forEventClass: INTERNET_EVENT_CLASS,
            andEventID: GET_URL
        ];
    }
    log::debug!("openurl: get-URL Apple events are handled");
}

unsafe extern "C-unwind" fn handle_get_url_event(
    _this: *mut AnyObject,
    _cmd: Sel,
    event: *mut AnyObject,
    _reply: *mut AnyObject,
) {
    // Nothing may unwind into AppKit.
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the Apple Event Manager passes the event being handled.
        let Some(event) = (unsafe { event.as_ref() }) else {
            return;
        };
        // SAFETY: Foundation messages with their documented types.
        let url = unsafe {
            let object: Option<Retained<AnyObject>> =
                msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
            object.and_then(|object| {
                let text: Option<Retained<NSString>> = msg_send![&object, stringValue];
                text
            })
        };
        match url {
            Some(url) => super::deliver(vec![url.to_string()], "a get-URL Apple event"),
            None => log::warn!("openurl: a get-URL Apple event carried no URL"),
        }
    }));
}

unsafe extern "C-unwind" fn open_urls(
    _this: *mut AnyObject,
    _cmd: Sel,
    _application: *mut AnyObject,
    urls: *mut AnyObject,
) {
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: AppKit passes an `NSArray<NSURL *>`.
        let urls = unsafe { strings_of(urls, |url| msg_send![url, absoluteString]) };
        super::deliver(urls, "application:openURLs:");
    }));
}

unsafe extern "C-unwind" fn open_files(
    _this: *mut AnyObject,
    _cmd: Sel,
    application: *mut AnyObject,
    paths: *mut AnyObject,
) {
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: AppKit passes an `NSArray<NSString *>` of paths; `fileURLWithPath:` takes one.
        let urls = unsafe {
            strings_of(paths, |path| {
                let url: Option<Retained<AnyObject>> =
                    msg_send![class!(NSURL), fileURLWithPath: path];
                url.and_then(|url| msg_send![&url, absoluteString])
            })
        };
        super::deliver(urls, "application:openFiles:");
        // SAFETY: AppKit expects this reply to `application:openFiles:`; it takes an
        // `NSApplicationDelegateReply`.
        if let Some(application) = unsafe { application.as_ref() } {
            let _: () = unsafe { msg_send![application, replyToOpenOrPrint: REPLY_SUCCESS] };
        }
    }));
}

/// The strings `string_of` makes of each element of an `NSArray`.
///
/// # Safety
/// `array` is null or an `NSArray`, and `string_of` sends only messages its elements answer.
unsafe fn strings_of(
    array: *mut AnyObject,
    string_of: impl Fn(&AnyObject) -> Option<Retained<NSString>>,
) -> Vec<String> {
    // SAFETY: the caller's contract.
    let Some(array) = (unsafe { array.as_ref() }) else {
        return Vec::new();
    };
    // SAFETY: `count` and `objectAtIndex:` are `NSArray` messages with these types.
    let count: usize = unsafe { msg_send![array, count] };
    (0..count)
        .filter_map(|index| {
            let element: Retained<AnyObject> = unsafe { msg_send![array, objectAtIndex: index] };
            string_of(&element).map(|string| string.to_string())
        })
        .collect()
}

fn ns_url(url: &Url) -> Option<Retained<AnyObject>> {
    let string = NSString::from_str(url.as_str());
    // SAFETY: `URLWithString:` takes a string and returns a URL or nil.
    unsafe { msg_send![class!(NSURL), URLWithString: &*string] }
}

/// The name of the app LaunchServices would open `url` with. A lookup only: nothing is opened.
pub(crate) fn app_for_url(url: &Url) -> Option<String> {
    let url = ns_url(url)?;
    // SAFETY: `sharedWorkspace`, `URLForApplicationToOpenURL:` and `lastPathComponent` are
    // documented with these types.
    let name: Option<Retained<NSString>> = unsafe {
        let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
        let application: Option<Retained<AnyObject>> =
            msg_send![&workspace, URLForApplicationToOpenURL: &*url];
        let application = application?;
        msg_send![&application, lastPathComponent]
    };
    let name = name?.to_string();
    Some(name.strip_suffix(".app").unwrap_or(&name).to_owned())
}

/// Opens links in the app the system chooses for them. Normal runs only.
pub(crate) struct WorkspaceOpener;

impl ExternalOpener for WorkspaceOpener {
    fn app_for(&self, url: &Url) -> Option<String> {
        app_for_url(url)
    }

    fn open(&self, url: &Url) -> bool {
        let Some(ns_url) = ns_url(url) else {
            return false;
        };
        // SAFETY: `openURL:` takes a URL and returns a `BOOL`.
        let opened: Bool = unsafe {
            let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
            msg_send![&workspace, openURL: &*ns_url]
        };
        opened.as_bool()
    }
}

/// Diagnostics (written to standard error: at `start` the log is not set up yet): sends this
/// process the Apple events another app would send for `urls`: a
/// get-URL event for each web URL, one open-documents event for the `file:` URLs. They are
/// addressed to the current process, so the Apple Event Manager hands them straight to the
/// handler without serialisation (AEMach.h): they never leave the process. Never sent to any
/// other process.
pub(crate) fn send_open_events_to_this_process(urls: &[String]) {
    let (files, others): (Vec<&String>, Vec<&String>) =
        urls.iter().partition(|url| url.starts_with("file:"));
    // SAFETY: Foundation messages with their documented types: `AEReturnID` is `i16`
    // (`kAutoGenerateReturnID` = -1), `AETransactionID` is `i32` (`kAnyTransactionID` = 0), the
    // timeout is in seconds; `insertDescriptor:atIndex:` with index 0 appends.
    unsafe {
        let target: Retained<AnyObject> =
            msg_send![class!(NSAppleEventDescriptor), currentProcessDescriptor];
        let event = |class: u32, id: u32| -> Retained<AnyObject> {
            msg_send![
                class!(NSAppleEventDescriptor),
                appleEventWithEventClass: class,
                eventID: id,
                targetDescriptor: &*target,
                returnID: -1i16,
                transactionID: 0i32
            ]
        };
        let send = |event: &AnyObject, what: &str| {
            let mut error: *mut AnyObject = std::ptr::null_mut();
            let reply: Option<Retained<AnyObject>> = msg_send![
                event,
                sendEventWithOptions: SEND_NO_REPLY,
                timeout: 10.0f64,
                error: &mut error
            ];
            if reply.is_none() {
                let description: Option<Retained<NSString>> = error
                    .as_ref()
                    .and_then(|error| msg_send![error, localizedDescription]);
                eprintln!(
                    "mercury-browser: diagnostics: sending {what} to this process failed: {}",
                    description.map_or_else(|| "unknown error".to_owned(), |d| d.to_string())
                );
            } else {
                eprintln!("mercury-browser: diagnostics: sent {what} to this process");
            }
        };
        for url in others {
            let get_url = event(INTERNET_EVENT_CLASS, GET_URL);
            let string = NSString::from_str(url);
            let direct: Retained<AnyObject> =
                msg_send![class!(NSAppleEventDescriptor), descriptorWithString: &*string];
            let _: () =
                msg_send![&get_url, setParamDescriptor: &*direct, forKeyword: DIRECT_OBJECT];
            send(&get_url, "a get-URL Apple event");
        }
        if !files.is_empty() {
            let open = event(CORE_EVENT_CLASS, OPEN_DOCUMENTS);
            let list: Retained<AnyObject> =
                msg_send![class!(NSAppleEventDescriptor), listDescriptor];
            for file in files {
                let Some(url) = Url::parse(file).ok().as_ref().and_then(ns_url) else {
                    continue;
                };
                let item: Retained<AnyObject> =
                    msg_send![class!(NSAppleEventDescriptor), descriptorWithFileURL: &*url];
                let _: () = msg_send![&list, insertDescriptor: &*item, atIndex: 0isize];
            }
            let _: () = msg_send![&open, setParamDescriptor: &*list, forKeyword: DIRECT_OBJECT];
            send(&open, "an open-documents Apple event");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_character_codes_are_big_endian() {
        assert_eq!(INTERNET_EVENT_CLASS, 0x4755_524c);
        assert_eq!(OPEN_DOCUMENTS, 0x6f64_6f63);
        assert_eq!(DIRECT_OBJECT, 0x2d2d_2d2d);
    }
}
