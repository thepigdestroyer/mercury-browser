---
title: Saved Logins
description: How the mercury-passwords crate stores website logins in the macOS Keychain, generates passwords and reads and writes login CSV files.
---

# Saved logins

`crates/mercury-passwords` is the storage and logic behind Mercury's password manager. It keeps
website logins in the operating system's credential store, generates passwords, and reads and
writes the login CSV files other browsers export. It has no UI and does not detect, offer to
save or fill forms; those parts build on it (see [the password manager in the
browser](#the-password-manager-in-the-browser)).

## The model

| Type | Meaning |
| --- | --- |
| `Origin` | `scheme://host[:port]`: `https`, or `http` for loopback hosts only. Lower-case ASCII host (internationalised names in their `xn--` form), no trailing dot, default port omitted. |
| `LoginKey` | origin + kind (`Form` or `HttpAuth { realm }`) + user name. Two keys are the same login exactly when they are equal. |
| `Password` | A secret in memory: wiped on drop, `Debug` prints `Password(…)`, no `Display`. |
| `SecretBackend` | Where secrets live: `put`, `get`, `delete`, `list` (never returns secrets). |
| `PasswordStore<B>` | Validation and exact-origin lookup on top of a backend. |

A login is offered on **exactly one origin**. `https://example.com`, `https://www.example.com`,
`http://example.com` and `https://example.com:8443` are four different sites (and the third is not
an origin that can hold logins at all). Nothing matches subdomains.

`PasswordStore::save` refuses empty passwords, control characters (NUL, tab, line breaks and the
rest) in user names and realms, and values longer than `MAX_USERNAME_BYTES` (1024),
`MAX_REALM_BYTES` (1024) and `MAX_PASSWORD_BYTES` (4096). An empty user name is allowed. These
rules apply to `save` only: `password` and `remove` accept any key, so every login `all()` lists
can be read and removed. `remove_all` goes on past a login it cannot remove and then reports
`RemoveAllIncomplete` with the number removed and the first error.

`Debug` output of `LoginKey`, `LoginSummary` and `CsvLogin` shows the origin and kind but only
the length of a user name (`<redacted, 5 chars>`), so logging a key does not log who the user is.

## Storage on macOS

`KeychainBackend` keeps one **generic-password** Keychain item per login, written with the
`SecItem` API through the `security-framework` crate. Every call names its keychain explicitly
(`kSecUseKeychain` when adding, `kSecMatchSearchList` when searching, updating and deleting).
`KeychainBackend::login()` resolves the user's default keychain and passes it like any other;
`KeychainBackend::file(path, password)` targets one keychain file and is what tests use.

`file()` refuses, before touching anything, a path with a component beginning with
`login.keychain` (any case) and any path inside `~/Library/Keychains`, `/Library/Keychains` or
`/System/Library/Keychains`. Apple's implementation adds a newly created keychain to the user's
search list, and makes it the default keychain if there is none, when its path contains
`/login.keychain`; other paths are created without changing the search list.

| Attribute | Value |
| --- | --- |
| service (`svce`) | `io.github.thepigdestroyer.mercury-browser.logins` (`KEYCHAIN_SERVICE`) |
| account (`acct`) | a versioned encoding of origin, kind, realm and user name (see below) |
| label (`labl`) | the site and user name, for example `example.com (alice)`; HTTP logins add ` – realm` |
| description (`desc`) | `Mercury Browser web login` or `Mercury Browser HTTP login` |
| data | the password, UTF-8 |
| `cdat`, `mdat` | creation and modification times, kept by the Keychain (one-second resolution) |

The account string lets `list` rebuild every login from attributes alone, without reading a
secret:

```text
account  = "v1" "|" kind "|" origin "|" realm "|" username
kind     = "form" / "http"
origin   = Origin::as_str()     ; canonical, never contains "|" or "%"
realm    = escaped              ; always empty for "form"
username = escaped
escaped  = any text, with each UTF-8 byte of "|", "%" and of control characters written as
           "%" and two upper-case hex digits
```

Only canonical strings decode: a string is accepted if encoding its decoded key gives the same
string back. Items under the service whose account does not decode (another version, someone
else's item) are skipped with a `debug` log line, never reported as errors and never deleted:
`remove_all` only deletes what `list` returned.

A Keychain item as `security dump-keychain` shows it (from a temporary keychain; the secret is
not displayed):

```text
class: "genp"
attributes:
    0x00000007 <blob>="example.com (alice)"
    "acct"<blob>="v1|form|https://example.com||alice"
    "cdat"<timedate>=0x32303236313131353231323635375A00  "20261115212657Z\000"
    "desc"<blob>="Mercury Browser web login"
    "mdat"<timedate>=0x32303236313131353231323635375A00  "20261115212657Z\000"
    "svce"<blob>="io.github.thepigdestroyer.mercury-browser.logins"
```

### What can make macOS ask the user

| Operation | Calls | Can prompt in the application? |
| --- | --- | --- |
| `list` | `SecItemCopyMatching`, attributes only | No: file-keychain attributes are not protected by the item's access list (they are readable even while the keychain is locked). |
| `get` | `SecItemCopyMatching` with `kSecReturnData` | Yes: to unlock a locked keychain, and when the item's access list does not trust this build. |
| `put`, new login | `SecItemAdd` | Only to unlock a locked keychain. |
| `put`, existing login | `SecItemAdd` (duplicate), `SecItemCopyMatching` with data to compare, `SecItemUpdate` if different | Yes, as `get`. |
| `delete` | `SecItemDelete` | No: deleting needs neither the secret nor an unlocked keychain (see below). |

A new item's access list trusts only the program that created it. For an **ad-hoc-signed** build
that trust is pinned to the binary's code-directory hash (`requirement: cdhash H"…"` in
`security dump-keychain -a`), so **every rebuild is a different program to the Keychain**:
reading a password saved by an earlier build shows the system's "wants to use your confidential
information stored in … in your keychain" dialog, once per item, and "Always Allow" lasts only
until the next build. With user interaction disabled the same read fails with
`errSecAuthFailed`, which the crate reports as `PasswordError::Locked`. Releases signed with a
stable Developer ID identity keep a stable designated requirement and do not re-prompt. See
Apple's [TN3137: On Mac keychain APIs and implementations](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains)
for the file-based keychain and its access-list model.

### What a file keychain does and does not protect

In a file-based keychain **only the secret is encrypted**. Any program running as the same user
can, without a prompt and even while the keychain is locked:

- list every saved site and user name (the service, account and label attributes);
- delete any of the items;
- add items under this application's service name, with any account string and any secret.

The access list protects reading the password, nothing else. For the application this means:

- treat what `list`, `all` and `logins_for` return as **untrusted input**: an item may have been
  planted by another program, and its user name or realm can contain anything the encoding allows
  (escaped control characters included). Display it safely; never interpret it.
- **never fill a login into a page merely because an item for that origin exists.** Filling must
  follow a choice the user makes in Mercury's own UI. Otherwise a program that can add one item
  under the service name could get a password of its choosing filled into a site.
- do not rely on a login staying saved: another program can delete it.

### Errors

`PasswordError` never carries a password, user name, realm, account string or CSV field. Keychain
statuses map as follows: `errSecInteractionNotAllowed` → `Locked`; `errSecAuthFailed` →
`Locked` when user interaction is off (locked keychain, or an access list that would have
prompted), otherwise `AccessDenied`; `errSecUserCanceled` → `AccessDenied`; anything else →
`Os { code, message }` with the system's own text for the code.

## Password generator

`generate_password(&GeneratorOptions, &mut impl RandomSource)` draws one character from each
enabled class, fills the rest from their union, then shuffles (Fisher–Yates). Each choice uses
one random byte and rejects bytes at or above `256 - 256 % n`, so there is no modulo bias.
`OsRandom` reads the operating system's generator through `getrandom`.

- Classes: `a–z`, `A–Z`, `0–9`, and the symbols `!#$%&*+-.:=?@^_~`.
- `avoid_ambiguous` removes `0 O o 1 I l |`.
- Length 8 to 128; the default is 20 characters, all classes, ambiguous characters avoided
  (about 114.9 bits by `entropy_bits`).

## CSV import and export

`parse_logins_csv` reads the exports of Chrome, Edge and Brave (`name,url,username,password,note`,
`note` optional), Firefox (`url,username,password,httpRealm,formActionOrigin,guid,timeCreated,timeLastUsed,timePasswordChanged`)
and Safari (`Title,URL,Username,Password,Notes,OTPAuth`, `Notes` optional), told apart by their
header. The Safari header comes from Apple's Safari User Guide as quoted in
[Mozilla bug 1827467](https://bugzilla.mozilla.org/show_bug.cgi?id=1827467); it has not been
checked against a real export. Rows with an unusable URL, an Android-app entry, an empty
password, broken quoting, the wrong number of fields, or a user name, realm or password that
`save` would refuse are skipped and listed by line number with a fixed reason that never
contains a field value. `write_logins_csv` writes Chrome's format.

**Spreadsheet formulas.** A spreadsheet that opens an exported file may run values beginning with
`=`, `+`, `-` or `@` as formulas. The file is deliberately not altered to prevent this (for
example with a leading quote), because a password must round-trip exactly; tell the user not to
open the export in a spreadsheet.

**An exported file holds every password in clear text.** Write one only when the user asks for
it, to the place they chose, as a new file only they can read (mode `0600`), and suggest
deleting it after use. Never write it to a temporary directory or a log.

## Testing without touching real keychains

Keychain tests must never read or change the developer's own keychains, nor show a dialog:

- every test creates its own keychain file in a fresh temporary directory, with an invented
  password, and a guard deletes the directory on every path, including panics;
- before the first Keychain call, the test process disables Keychain user interaction for its
  whole life (`SecKeychain::disable_user_interaction`, held in a static), so an operation that
  would prompt fails instead;
- creating a keychain with `SecKeychainCreate` does not add it to the search list unless its
  path contains `/login.keychain`, which `KeychainBackend::file` refuses; check
  `security list-keychains -d user` before and after a test run all the same;
- the backend contract (`tests/backend_contract.rs`) runs the same suite against `MemoryBackend`
  and a temporary `KeychainBackend`.

Use the `security` tool read-only and only with an explicit path to a temporary keychain (for
example `security dump-keychain <path>`), never with `-d`, which prints secrets.

## The password manager in the browser

`crates/mercury-password-manager` (no Servo, no Slint) holds the logic between the store and the
interface; `apps/mercury-desktop/src/passwords/` connects it to the windows, the engine and macOS.

| Part | Where | What it does |
| --- | --- | --- |
| Backend choice | `passwords::start` | The login keychain only in a normal interactive run of the default profile. `--screenshot`, `--background`, `--profile-dir` and tests keep logins in memory (`AnyBackend::memory`), authenticate with `RefusingAuthenticator` and copy to an in-memory pasteboard: they never touch the login keychain, show a system prompt or change the real clipboard. |
| Settings section | `passwords/section.rs`, `ui/passwords/passwords-section.slint` | List (`login_rows`, untrusted text made safe by `display_text`), search (`filter_rows`), open, copy the user name, show or copy the password, edit (`update_login`: a new user name saves the new key before removing the old), delete (asks first), the generator, CSV import (`import_csv`: never overwrites or reads a saved password) and export (`export_csv` + `write_private_file`: a new `0600` file renamed over the chosen path). The Slint side talks only through the `PasswordManagerUi` global. |
| Store worker | `StoreWorker`, `passwords/mod.rs` | No Keychain call runs on the interface thread: one thread owns the store, jobs after a gesture may make macOS ask (`Interaction::Allowed`), everything else runs with Keychain user interaction off (`Interaction::Never`), a job that panics is reported and the worker goes on, and the list of saved logins (no passwords) is kept in memory. See below. |
| Authentication | `Authenticator`, `AuthGate`, `passwords/local_auth.rs` | Showing, copying and exporting a password first asks `LAContext.evaluatePolicy(.deviceOwnerAuthentication)` (Touch ID or the login password). One success lasts `AUTH_GRACE_PERIOD` (60 s of wall-clock time, so sleep counts) or until the settings window closes. |
| Copying | `Pasteboard`, `CopiedSecret`, `passwords/system_pasteboard.rs` | A password is copied with a request to keep it on this Mac (`NSPasteboardContentsCurrentHostOnly`, which AppKit does not let a program read back), as one item that carries the string and the `org.nspasteboard.ConcealedType` and `TransientType` markers, written with a single `writeObjects:` after `prepareForNewContentsWithOptions:`, and removed after `CLIPBOARD_CLEAR_DELAY` (60 s) or at quit (`post_run`) if the pasteboard has not changed since. |
| Manual fill | `passwords/fill.rs`, `controls/context_menu.rs`, `talaria_core::LoginFill` | See below. |
| HTTP sign-in | `passwords/sign_in.rs`, `SignInSaves`, `page_requests.rs` | See below. |

### The store worker

No Keychain call runs on the interface thread. One thread (`password-store`) owns the store. A
job's result comes back through `slint::invoke_from_event_loop`, and each reply checks again that
what it was for (the opened login, the focused field, the waiting sign-in) is still there. The
section shows "Waiting for your keychain…" meanwhile. The memory store runs jobs inline, so tests
stay synchronous; `StoreWorker::deferred` tests what happens while a job waits.

- **Interaction.** Jobs after a gesture in Mercury's own interface run with
  `Interaction::Allowed` (macOS may ask): opening the Passwords section, reading, copying, editing
  or deleting a login, picking a login in the fill menu, Sign in with a saved password, and saving
  a sign-in the user ticked the save choice for. Start-up and the reload after a saved sign-in run
  with `Interaction::Never`: Keychain user interaction is switched off for the job
  (`KeychainBackend::without_user_interaction`), so a call that would show a dialog fails with
  `Locked`. The list of saved logins (no passwords) is kept in memory, so search, the fill menu and
  the sign-in sheet never call the Keychain.
- **Only this thread calls the Keychain.** `without_user_interaction` is process-wide
  (`SecKeychainSetUserInteractionAllowed`), so a Keychain call on another thread while a quiet job
  runs would lose its dialog. Every Keychain access in the process therefore goes through
  `StoreWorker`. `StoreWorker::spawn` binds the backend to the worker thread
  (`AnyBackend::bind_to_current_thread`), and each Keychain call of `AnyBackend` asserts
  `on_owner_thread()` with `debug_assert!`. **That assertion exists in debug builds only.**
  `debug_assert!` is compiled out of release builds, and the repository's tests run with
  `--release`, so there it never fires; the rule is kept by construction (only `StoreWorker` holds
  the store), not by a check in release. The memory store is not checked. The test of it,
  `a_keychain_call_off_the_owner_thread_is_refused` (macOS, debug assertions only), calls a backend
  for the default keychain from a thread that does not own it: the assertion fires before any
  Keychain call, so it needs no temporary keychain and shows no dialog. It runs only in a
  debug-profile test run (`cargo test -p mercury-password-manager --lib`).
- **A failed job.** Every job runs under `catch_unwind` (on the worker thread, inline and
  deferred alike), and a drop guard switches the quiet flag off again whatever happens. A job that
  panics delivers a marker instead of its result: its reply never runs, the handler set with
  `StoreWorker::set_on_failure` gets `WorkerFailure::JobFailed`, and the worker takes the next job.
  If the worker thread is gone (its channel is closed), `run` drops the waiting reply, so nothing
  leaks, and reports `WorkerFailure::Stopped`; no job runs again in that run. `Passwords::store_failed`
  turns these into "Something went wrong with the saved logins." and "Saved logins are
  unavailable until Mercury Browser restarts." in the Passwords section of the settings window
  (`STORE_FAILED`, `STORE_STOPPED` in `section.rs`). With that window closed a failure is only
  logged, and a sign-in sheet or fill that waited for the job stays as it was.
- **A list that could not be read.** If the list of saved logins failed to load (a locked
  keychain at launch, where the start-up job may not ask), it is read again with
  `Interaction::Allowed` the next time the user makes a gesture that needs it: opening the
  Passwords section, a right-click over a password field, or Sign in with an empty password on a
  sign-in sheet. The menu opened by that right-click offers nothing; the next one offers what the
  read found. See "HTTP sign-in" below for the sheet.

### Manual fill

Servo 0.8 has no autofill hooks (research R6), so filling is user-started and top-frame only:

1. A right-click focuses the clicked field before Servo opens its context menu. When the menu
   arrives, `talaria-servo` looks for a pending input-method control of type `Password` in the
   **same pipeline** as the menu and, if there is one, sends
   `BrowserEvent::PasswordFieldMenu { menu, field }` after the menu's `ControlShown`.
2. The context menu then starts with "Fill login…" if form logins are saved for the exact origin
   of the tab's **top-level** URL (`fill_candidates`). Choosing it turns the menu into the list of
   those logins.
3. Picking one (a click or Return in Mercury's own menu: the user gesture) dismisses the engine's
   menu and, if the same field still has focus and the page still has that origin, queues
   `BrowserCommand::FillLogin`. `ServoWebView::fill_login` evaluates `talaria-core`'s
   `login_fill.js` with the values as escaped string literals. `evaluate_javascript` only runs in
   the top-level document; the script fills only if `location.origin` is the login's origin and
   `document.activeElement` is one of that document's own password inputs (focus in any frame
   makes the frame element the active element). It sets the value, finds the user-name partner
   (the last editable text-like input before it in its form) and fires `input` and `change`. The
   status word is logged; the values never are.

Never filled: on page load, without the menu, into a frame, on a page whose origin cannot hold
logins (plain HTTP other than loopback). The page can read a filled value, like any value the
user types. Save-on-submit and frame-aware fill wait for the engine patch (R7 B2).

**Diagnostics.** A build with the `passwords-diagnostics` feature reads
`MERCURY_PASSWORDS_DIAG_FILL="<origin> <user> <password>"` in automated runs: it seeds that login
in the run's memory store and fills the focused field each time a page finishes loading, as if the
user had picked it. `tests/pages/login-form.html` and `login-iframe.html` (see
`tests/pages/README-dialogs.md`) check the fill and the refusal in a frame with `--screenshot`.

### HTTP sign-in

A page can raise the sign-in sheet without a gesture, so the sheet starts with the **user name**
only of the most recently changed HTTP login saved for the exact origin that asked (realm `""`:
Servo 0.8 does not report it), from the in-memory list. The saved password is read when the user
presses Sign in with that user name and the password field empty; the request is answered when the
keychain replies, if it is still waiting.

**Never for a proxy:** a proxy's challenge carries the destination's URL, so filling it would hand
the destination's login to the proxy. Proxy sheets start empty and offer nothing to save.

Unless the settings turn saving off or list the origin under "Never save passwords for", the sheet
offers "Save the user name and password in Mercury Browser" (in memory-only runs: "Remember the user
name and password until Mercury Browser quits"). With it ticked and a password typed, the login is
saved once the sign-in has worked: the tab starts a new document and finishes loading it without
asking the same origin again (`SignInSaves`; with wrong credentials Servo asks again first).

**Saving.** The user asked for that save (ticked the choice and pressed Sign in), so it runs with
`Interaction::Allowed`: macOS may unlock the keychain or ask about an item another build saved. If
it still fails, `Couldn't save the login for <site>.` appears under the toolbar
(`PasswordSaveNotice` in `main-window.slint`, shown while the toolbar is) with **Retry**, which
saves the same login again with `Interaction::Allowed`, and **Dismiss**, which forgets it unsaved.
Only the last failed save is kept (`Passwords::failed_save`). A save that worked reloads the list
with `Interaction::Never`.

**Reading.** Sign in with the saved user name and the password field empty reads the saved
password on the worker with `Interaction::Allowed` (`read_saved_sign_in`). If it cannot be read (a
locked or refusing keychain, or the login is gone), the sheet stays and says "The saved password
could not be read; type it to sign in." (`SAVED_PASSWORD_UNREADABLE` in `page_requests.rs`). If the
list of saved logins failed to load at launch, Sign in with an empty password lists it again first
(with `Interaction::Allowed`, `with_saved_sign_ins_listed`) and, if it still cannot be listed,
shows the same message instead of signing in with an empty password for a login it cannot see.

**Empty password.** An empty password field with a matching saved user name always means "use the
saved password": the sheet does not send what was typed. A user name with a saved login therefore
cannot be signed in with nothing typed as the password, and a saved login whose password is
genuinely empty cannot be re-entered empty from the sheet; typing any password sends that one
instead. `PasswordStore::save` refuses empty passwords, so such a login can only have been put in
the keychain by another program.

**Follow-up:** Keychain items are not per profile (one service name), which is why `--profile-dir`
profiles keep logins in memory. Per-profile items (a profile id in the account prefix or the
service name, in `mercury-passwords`) would let them persist.

### Not yet verified

The tests use fake backends, a fake authenticator, an in-memory pasteboard and, for
`mercury-password-manager`, a temporary keychain with user interaction switched off, so none of
these has run for real:

- **Keychain dialogs.** The unlock and "allow access" dialogs that `Interaction::Allowed` permits
  (on Sign in, on the save after a sign-in, on Retry, on opening the section) have not been seen
  on a Mac.
- **Touch ID** and the login-password fallback behind `LAContext.evaluatePolicy`.
- **The real pasteboard.** `passwords/system_pasteboard.rs` has tests that write to a private
  `NSPasteboard` (`pasteboardWithUniqueName`, never the general one) and read it back: one item
  with the string, `ConcealedType` and `TransientType`; a plain copy without the markers; each
  copy replacing the last. Not verified: that `CurrentHostOnly` is still in force after the
  `writeObjects:` (AppKit has no call to read it back, and no second device was paired to see
  Universal Clipboard refuse the item), the general pasteboard itself, and that a clipboard
  manager honours the two markers. The user guide therefore says Mercury *asks* macOS to keep
  a copied password on this Mac, not that it is guaranteed.
- **A real HTTP save.** A sign-in against a real server whose login is then saved to the real
  login keychain, and the "Couldn't save" notice and Retry on a real failure.
- **Linux.** `cargo check -p mercury-desktop --all-targets` for Linux has not been run (it cannot
  be compiled on the development Mac); the macOS-only code is behind `cfg(target_os = "macos")`.

## Other platforms

On platforms other than macOS the crate builds with `MemoryBackend` only, which keeps nothing
after the process ends. Every macOS-specific line is behind `cfg(target_os = "macos")`.
