# Patched servo-script 0.8.0 (and the media-capture patch)

This directory vendors `servo-script` 0.8.0, Servo's DOM and script engine glue,
under its original MPL-2.0 license. It is the crate exactly as published on
crates.io (checksum
`b6c3e52be4165b3f98b992f384156b049d30b769004a96cf52903ea27cd32f9b`), built from
https://github.com/servo/servo at commit
`4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/script`.

The crate was published without a licence file. Its `Cargo.toml` declares
`license = "MPL-2.0"` and every source file carries the MPL-2.0 header; the
MPL-2.0 text in the repository's `LICENSES/MPL-2.0.txt` applies to this
directory.

It is the home of Mercury's engine patches. The first one, camera and
microphone capture, spans four vendored crates: this one,
`servo-script-bindings`, `servo-media-streams` and `servo-media-gstreamer`.
`third_party/patches/<crate>.patch` is each crate's difference from the
published crate (`mercury-servo-patches diff <crate>` prints it from the
directory as it is now), and `make servo-patches-check` fails if the two
disagree. Together they hold every change, and only the changes listed here and
in the other three directories' `MERCURY.md`. Every changed file is marked with
a `Mercury:` comment (`grep -rn "Mercury:" third_party/servo-script`). How to
re-vendor on a Servo upgrade: `docs/developer-guide/vendored-patches.md`. The design and the engine
facts behind it: `.superpowers/research/R8-media-capture.md`.

## Changes: camera and microphone (`getUserMedia`)

In Servo 0.8.0 `getUserMedia` opens the camera and microphone without asking
anyone, `enumerateDevices` returns device names without permission, a
cross-origin frame can ask in its embedder's name, and nothing ever releases a
device (there is no `MediaStreamTrack.stop()`). Mercury turns capture on only
with these changes.

### 1. Ask before opening a device — `dom/media/mediadevices.rs`

- **What:** `GetUserMedia` rejects `{audio: false, video: false}` with a
  `TypeError`, a document that is not fully active with `InvalidStateError`,
  and an insecure context with `NotAllowedError`. It then asks for
  "microphone" and "camera" through `request_permission_to_use_async` — both
  requests at once, so the embedder can show them together — and opens no
  device (not even the device-monitor probe) before every requested kind is
  granted. The first refusal rejects with `NotAllowedError`; a missing device
  after the grant rejects with `NotFoundError` and stops a source already
  opened for the other kind. The tracks are "capture tracks", registered with
  the document's `MediaDevices`. An answer that arrives after the document
  stopped being fully active, or after its capture was stopped (unload or
  teardown; `MediaDevices::capture_generation`), opens nothing, records
  nothing in the permission results `stop_all_capture` cleared (a document
  restored from the history would otherwise capture without asking and
  without the browser's marks), and rejects with `AbortError`
  (`settle_answer`, with a unit test in the same file).
- **Upstream:** `mediadevices.rs:49-72`.
- **Drop when:** Servo asks for permission in `getUserMedia`.

### 2. Expose device ids and labels only with permission — `dom/media/mediadevices.rs`

- **What:** `EnumerateDevices` gives ids and labels only for kinds whose
  permission is granted (camera: `videoinput`; microphone: `audioinput`,
  `audiooutput`). With neither granted nothing is probed and one entry with
  empty fields stands for each input kind.
- **Upstream:** `mediadevices.rs:75-111`; `servo-media-gstreamer`
  `device_monitor.rs:36-46` uses the device name as its id.
- **Drop when:** Servo implements "device information can be exposed".

### 3. Bare constraint numbers are ideal values — `dom/media/mediadevices.rs`

- **What:** `{width: 1280}` and `{frameRate: 30}` are ignored instead of being
  turned into exact caps that exclude every device without that mode; `exact`,
  `min` and `max` still apply.
- **Upstream:** `mediadevices.rs:128-160`.

### 4. `MediaStreamTrack.stop()` and `readyState` — `dom/media/mediastreamtrack.rs`

- **What:** `readyState` ("live"/"ended") and `stop()`. Stopping a capture
  track releases its source once no live clone uses it; clones of capture
  tracks are registered too. (WebIDL in `servo-script-bindings`.)
- **Upstream:** `mediastreamtrack.rs:57-75`.

### 5. Release devices when the document goes away — `dom/document/document.rs`, `dom/window/window.rs`, `dom/navigator/navigator.rs`

- **What:** the unloading document cleanup steps (navigation, reload, frames)
  and `Window::clear_js_runtime_common` (pipeline exit: tab closed or
  discarded) call `Window::stop_media_capture`, which ends every capture track,
  stops their sources and forgets the document's camera and microphone
  answers, so a page restored from the history asks again. New accessors
  `Window::maybe_media_devices` and `Navigator::maybe_media_devices`.
- **Upstream:** `document.rs:871-901`, `window.rs:570-578`, `navigator.rs:187`.

### 6. Camera and microphone are `'self'` in frames — `dom/document/document.rs`

- **What:** `Document::allowed_to_use_feature` returns false for `Camera` and
  `Microphone` unless the document is the top-level one or same origin with it
  (the default allowlist of those policy-controlled features). Permission
  queries, requests and `getUserMedia` in a cross-origin frame are denied
  without a prompt. `allow=` on an iframe is not honoured. Servo labels every
  permission request with the top-level origin, so this also keeps a
  cross-origin frame from asking in its embedder's name.
- **Workers:** `descriptor_permission_state` (`dom/permission/permissions.rs`)
  returns `Denied` for `Camera` and `Microphone` in any global that is not a
  `Window`. A worker may belong to a frame of another origin, and the rule
  above looks only at documents; without this, a cross-origin frame's worker
  could prompt (`navigator.permissions.request`) in the top-level page's name.
  Same-origin dedicated and shared workers are refused too: their `query()`
  says `denied` where the spec would say `prompt` or `granted`. That is
  deliberate and harmless (workers cannot call `getUserMedia`); a worker
  cannot tell which frame it serves, so do not narrow the gate to
  cross-origin workers.
- **Upstream:** `document.rs:3608-3625`; `permissions.rs:288-337`.
- **Drop when:** Servo implements Permissions Policy and reports the asking frame.

### 7. (in servo-media-gstreamer) Showing a capture stream no longer crashes the tab

See `third_party/servo-media-gstreamer/MERCURY.md`, change 3.

## Changes: post-0.8 Servo changes and K0 fixes (lane K0, 19 Nov 2026)

Post-0.8 changes from servo/servo, taken so that `document.execCommand`, contenteditable editing, the caret and WebGL2
behave like current Servo, plus one notification fix and SpiderMonkey 153.7.0. Each hunk starts with a
`Mercury: backport of servo/servo#N (merge commit)` comment. Upstream's tests (`tests/wpt`) are not vendored;
`tests/pages/compat-editing.html` and `tests/pages/webgl2-textures.html` exercise the changes. All are dropped at the
Servo bump that contains them (B1; a bump that contains the pull request turns the hunk into a no-op conflict to
resolve in favour of upstream).

### 8. Editing backports (ED9)

Editing is on in Mercury (`dom_exec_command_enabled`).

- **servo/servo#49541** (`e35a02ebd`, "Use saturating addition in contenteditable code to avoid overflow"),
  `dom/range/range.rs` (the live-range update after `CharacterData::replaceData`, about line 1631): `offset + count` is
  `offset.saturating_add(count)`. A huge `count` (`text.replaceData(9, 4294967295, ..)` with a live range) panicked the script
  thread by u32 overflow in debug builds. Test: the "range with huge offsets" line of `compat-editing.html`.
- **servo/servo#47459** (`189034510`, "Implement `outdent` editing command"): `dom/execcommand/commands/outdent.rs` (new,
  upstream MPL header), `commands/{mod,delete,indent}.rs`, `basecommand.rs`, `execcommands.rs`, `contenteditable/node.rs`.
  Adds `outdent`, moves `normalize_sublists` from `indent.rs` to `Node::normalize_sublists` and uses it in the delete
  command (steps 7.1 and 7.2 were TODOs). Test: "outdent a nested list" and "outdent a list item out of its list". **Known gap,
  the same in servo/servo main** (compared on 20 Nov 2026): the node list in `execute_outdent_command` keeps a node only if it has
  no editable descendant, and `traverse_preorder` includes the node itself, so an editable text node or paragraph is never kept;
  only `ol`, `ul` and `li` are. `outdent` therefore returns true and changes nothing for `<blockquote>text</blockquote>` or
  `<blockquote><p>text</p></blockquote>` (upstream's `tests/wpt/meta/editing/run/outdent.html.ini` lists those cases as FAIL).
  The page reports it as an INFO line ("known gap"). Also not mapped upstream: `italic` to the input type `formatItalic`
  (`mapped_value_of_command` in `execcommands.rs` has no `Italic` arm), so `beforeinput.inputType` is empty for it.
- **servo/servo#49649** (`10c279fae`, "Rename `DomSlice` `r()` to `as_ref_slice()`"): the 6 call sites in this crate
  (`dom/document/document_event_handler.rs`, `dom/gamepad/gamepad.rs`, `dom/node/node.rs`, `dom/execcommand/{commands/outdent.rs,
  contenteditable/node.rs, contenteditable/selection.rs}`); the trait is in `servo-script-bindings` (`root.rs`). A rename,
  taken because the next backports call the new name. Mercury's own code does not use `DomSlice`.
- **servo/servo#48253** (`cd332167d`, "Render caret inside of contenteditable elements"): `dom/selection.rs` (a `paints_caret`
  flag and `should_paint_caret`: a text-input shadow host, or a collapsed selection whose editing host is the focused element
  while `dom_exec_command_enabled` is on; `paints_caret_for_layout`), `dom/node/layout_dom.rs` (`text_node_paints_caret`),
  `dom/characterdata/characterdata.rs` and `dom/html/form_controls/input_type/text_input_widget.rs` (`set_text_run_selection`
  takes the new flag), `dom/html/htmlelement.rs` (adding or removing `contenteditable` marks the selection dirty). Needs the
  `servo-layout` and `servo-layout-api` hunks. Only the active page's focused host draws the caret (upstream rule: no caret in
  a window without system focus, so a `--screenshot` run shows none: look in a normal window). Test: the caret box at
  the top of `compat-editing.html`, whose `document.hasFocus()` line says which kind of window ran it.
- **servo/servo#49797** (`8d0f0d77f`, "Implement logic for collapsed line break"): `dom/execcommand/contenteditable/node.rs`
  (`Node::is_collapsed_line_break`), `commands/insertlinebreak.rs` (step 10: an extra `br` after a collapsed one),
  `commands/inserttext.rs` (step 14.1) and `is_collapsed_block_prop`. Test: "insertLineBreak at line end".
- **servo/servo#49818** (`d1eff56d7`, "Insert line break when paragraph container is disallowed child"):
  `commands/insertparagraph.rs` step 11.5.1 runs insertLineBreak instead of doing nothing (w3c/editing#552). Test:
  "insertParagraph in an inline editing host".
- **servo/servo#49884, #49917, #49920** (`ba4852dce`, `824dae490`, `d9656e7a2`, "Insert newline characters in pre-formatted
  styling for insertlinebreak", "Check styling of parent if start container is a text node", "Fix collapsed line break
  consideration for parent block nodes"): hand-ported, not applied with `git apply`, because their context contains
  servo/servo#49552 ("pass `JSContext` to `Element::style`", 37 files), which Mercury does not take. The code is the same in
  0.8's forms (`element.style()`, `is_collapsed_line_break(cx.no_gc())`, the `computed_values::white_space_collapse` import);
  `commands/insertlinebreak.rs` and `contenteditable/node.rs` (`move_preserving_ranges<Move, Return>`,
  `is_collapsed_line_break`). Test: "insertLineBreak in white-space: pre-wrap". Take all three or none: revert
  `c812a3f`, `e844e14` and `23c80bf` together (the commit message of the last one names a wrong merge hash; the right one is
  above). Drop with the bump that contains #49552 and these three.

### 9. WebGL2 conformance backports (ED10) — `dom/webgl/`

- **servo/servo#49632** (`e690242e4`), `webglrenderingcontext.rs`, `getParameter` (0.8.0 line about 2363): the four texture
  binding constants share one arm; a constant that the context's WebGL version does not have (for example
  `TEXTURE_BINDING_2D_ARRAY` on WebGL1) sets `INVALID_ENUM` and returns null, where 0.8 panicked on `unwrap`.
- **servo/servo#49912** (`43753d9b4`), `validations/tex_image_2d.rs`, `TexStorageValidator` (line about 783):
  `texStorage2D` and `texStorage3D` reject a compressed format whose extension is not enabled (`INVALID_ENUM`) and a depth or
  depth-stencil format for `TEXTURE_3D` (`INVALID_OPERATION`).
- **servo/servo#49436** (`c2c5117bf`), `webgltexture.rs`, `WebGLTexture::storage` (line about 626): only a `TEXTURE_3D` halves its
  depth with each level; a 2D array keeps its layer count. This is the script half of the zero-initialisation in `servo-webgl`.
- Tested by `tests/pages/webgl2-textures.html`. Not taken: #49404 and #49902 (they need `servo-canvas-traits`, not vendored).
  Consequence: `framebufferTextureLayer` on a `TEXTURE_2D_ARRAY` texture is rejected with `INVALID_OPERATION` in 0.8.

### 10. Show a notification that has no icon, image or badge (EF4) — `dom/serviceworker/notification.rs`

- **What:** in `Notification::fetch_resources_and_show_when_ready` (about line 884), after the `pending_requests` list is built:
  `if pending_requests.is_empty() { self.show(); return; }`. An unparsable icon URL also gives an empty list and shows at once,
  which matches the spec's "on failure, continue".
- **Why:** `show()` was called only from the fetch-completion path, so a notification with nothing to fetch (the common case)
  was never handed to the embedder and never fired `show`. `show()` is safe there: it sends `EmbedderMsg::ShowNotification` and
  queues the `show` event as a task, so the event still fires after the constructor returns; `pending_request_ids` is only
  inserted into together with a `pending_requests` entry, so none dangles. The idea is that of patch 0002 of Zervo
  (github.com/Goddv/Zervo, `patches/servo/0002-script-show-notification-without-resources.patch`); written from its
  description, not copied (credit in `THIRD-PARTY-NOTICES.md`).
- **Tested by:** `tests/pages/notification-no-icon.html` (PASS, FAIL or SKIP; SKIP when the permission cannot be granted).
- **Drop when:** the Servo bump that contains the same fix (check `Notification` in `servo-script` at B1; not fixed in 0.8.0).

### 11. SpiderMonkey 153.7.0 (`mozjs` 0.27.0) — `Cargo.toml`

The `js` dependency (package `mozjs`) is `=0.27.0` instead of `=0.26.13`, with a `# Mercury:` comment (servo/servo#49907, for
SpiderMonkey's security fixes); the same line is changed in `servo-script-bindings` and `servo-script-webcrypto`. See
`third_party/servo-script-webcrypto/MERCURY.md`. Drop at the bump that contains it (B1).

## Changes: page accessibility (lane a11y-1, 21 Nov 2026)

### 12. What an assistive technology can do to a page (EE5) — new `dom/document/accessibility_actions.rs`, `dom/window/window.rs`

- **What:** upstream performs one accessibility action, an untrusted `click`, in `Window::handle_accessibility_actions`.
  The new file performs the 16 actions that `mercury_a11y::actions::SUPPORTED_ACTIONS` advertises (Focus, Blur, Click,
  SetValue, ReplaceSelectedText, ScrollIntoView, ScrollToPoint, ScrollUp/Down/Left/Right, Increment, Decrement, Expand,
  Collapse, ShowContextMenu) with an exhaustive `match` on `accesskit::Action`. `Click` also sets the window's last
  activation time (decision OD-5/D6: assistive-technology actions that click count as user activation; the action
  channel is embedder-only, so a page cannot grant itself activation). `window.rs` has one hook: the body of
  `handle_accessibility_actions` is one call, and the `use accesskit::Action` it needed is gone. No page content is
  logged.
- **Why:** VoiceOver's "activate", "set value", "scroll to" and so on have to do something on a page.
- **Upstream:** `dom/window/window.rs` `handle_accessibility_actions` (about line 4021 in 0.8.0).
- **Tested by:** unit tests of the pure helpers in the file; `crates/talaria-servo/tests/a11y_actions.rs`
  (`headless_servo_*`, ignored, run by `make test-a11y-pages` where Servo can render).
- **Drop when:** upstream Servo performs the actions itself.

### 13. When a node's accessibility properties are stale (EE3, EE4) — new `dom/document/accessibility_attributes.rs`, `dom/element/element.rs`, `dom/html/form_controls/{htmlinputelement,htmltextareaelement,htmlselectelement}.rs`, `dom/html/embedded_content/htmliframeelement.rs`

- **What:** upstream damages the accessibility tree for a `role` change and a text change only. The new file decides which
  attribute changes (`aria-*`, `alt`, `title`, `for`, `id`, `value`, `hidden`, `disabled`, ...), which native state flags
  and which value changes (typed text, a range's number, a select's selection) damage the node. Hook lines: one call in
  `Element::attribute_mutated`, one in `Element::set_state`, one `damage_for_value_change` call in each
  place a form control's value changes (`handle_text_content_changed`, `value_changed`, `update_shadow_tree`), and one
  `add_pending_accessibility_damage` where an `<iframe>` gets a new pipeline (its node grafts that document's tree).
  `dom/document/mod.rs` declares the two modules. All cost nothing while no assistive technology is attached
  (`add_pending_accessibility_damage` records only for an active document).
- **Why:** names, states and values are computed from the DOM; a stale answer is a wrong reading.
- **Tested by:** `headless_servo_state_changes_reach_the_tree` and the frames tests in `crates/talaria-servo/tests/`.
- **Drop when:** upstream servo#46241 and its follow-ups damage the tree for the same changes.
## Changes: text, lane txt-1 (21 Nov 2026)

Typed highlights (EB7), IME composition (EB11) and two post-0.8 selection fixes. Nothing here is a backport of a pull request
unless it says so.

### 14. Selection on replaced elements (servo/servo#49613) — `dom/node/layout_dom.rs`, `dom/selection.rs`, `dom/selection_range.rs`

Merge commit `af5f54ba7` (sabbCodes, 14 Nov 2026). `replaced_is_selected` also requires a non-collapsed range;
`FlatTreeSelection::is_collapsed` and `SelectionRange::is_collapsed` are new; the element branch of `set_node_selection` follows.
Needs the `servo-layout` hunk. Drop at the Servo bump that contains #49613 (B1).

### 15. UA widgets are not selected (servo/servo#49885) — `dom/selection.rs`

Hand-port (merge commit `afc9224b1`, Martin Robinson, 13 Nov 2026): `VisibleSelectionTraversal` skips the subtree of an element whose
shadow root is a UA widget (`Element` joins the `use crate::dom::{..}` line). Needs the `user-agent.css` hunk of `servo-layout`.
Drop at the bump that contains #49885 (B1).

### 16. Typed highlights (EB7) — new `dom/highlights.rs`; `dom/mod.rs`, `dom/document/document.rs`, `dom/window/window.rs`, `dom/node/node.rs`, `dom/node/layout_dom.rs`, `dom/characterdata/characterdata.rs`, `layout_dom/servo_layout_node.rs`

- **What:** a `HighlightSet` of live `Range`s per `Document` (field, initialiser and accessor `Document::highlights()` in
  `document.rs`: `#[dom_struct]` forbids public fields, so a sibling module cannot read the field). Once per reflow, right after the
  selection update (`window.rs`: one hook line, `Document::update_highlights`), it resolves the text nodes each range covers and
  pushes each node's highlights to its text run (`CharacterData::set_text_run_highlights`; a new display list, no relayout). It
  keeps the per-node result for layout to read when it builds a run (`LayoutDom<Node>::highlights_for_text_node`); the text of a text
  control gets its composition from the control (`composition_for_layout`). `NodeFlags::OVERLAPS_HIGHLIGHT` (bit 4) lets layout ask
  only the nodes that have one. API: `set(kind, ranges)`, `clear(kind)`, `clear_all()`; the find, text-fragment and spell-check
  lanes are its next producers.
- **Why:** composition underlines inside `contenteditable` (EB11) and, later, find in page, text fragments and spell check.
- **Upstream:** none. **Tested by:** the pure parts in `servo-layout-api` and `servo-layout`; the script side by the compile and
  by `servo/tests/input_methods.rs` (composition in an editing host registers its range here). **Drop when:** never as a whole.
- **Cost:** `update` recomputes per reflow while the set is not empty; producers so far are small ranges. Not measured.

### 17. IME composition and the text input state (EB11) — new `dom/document/text_input_state.rs`, `dom/execcommand/composition.rs`; `dom/document/{document_event_handler,editing,document,mod}.rs`, `dom/execcommand/execcommands.rs`, `dom/html/form_controls/{text_input,text_control,htmlinputelement,htmltextareaelement}.rs`, `dom/html/form_controls/input_type/text_input_widget.rs`

- **What:** `ImeEvent::{Commit, SetMarkedText, InsertText}` (see `servo-embedder-traits/MERCURY.md`) and the events in the order
  Chromium fires them (`compositionstart`, `compositionupdate`, a non-cancelable `beforeinput`, `input` with `insertCompositionText`
  and `isComposing`, then `compositionend` exactly once; `Commit("")` cancels). `ImeState` (`text_input_state.rs`) keeps the
  composition, resolves replacement ranges (honoured only for the last `sequence` sent, an unchanged text window, on character
  boundaries; never in a password field, never outside the editable) and sends `TextMsg::InputState` when the focused editable's
  text, selection, composition or caret rectangle changes (`None` when focus leaves an editable). **Text controls:** `TextInput`
  keeps a `composition` range; updates replace it instead of selecting the inserted text; `EndComposition(text)` now applies the
  final text (it used to drop it); the marked range reaches layout through `composition_for_layout` and
  `TextInputWidget::set_text_run_highlights` (`Composition`, `CompositionSelected`). **Editing hosts:** the refusal of
  `StartComposition`/`EndComposition` at `editing.rs:397-400` is gone; `inserttext` runs through `exec_command_with_input_events`
  with the `insertCompositionText`/`insertReplacementText` event parameters (`composition.rs`), and the marked text is a live DOM
  `Range` registered as `Composition` in the highlight set. **Hooks:** `document_event_handler.rs` (field `ime`, the new
  `handle_ime_event` arms, `handle_ime_commit` of #49400, a flush after keyboard, IME, mouse and touch events), and
  `Document::update_the_rendering` (`document.rs`: one line after the reflow, `flush_text_input_state_after_reflow`).
- **Decisions:** a cancel fires `beforeinput` and `input` (data `""`) before `compositionend`, so React and friends see the value
  change; `Dismissed` ends an open composition with `compositionend` and keeps the text; `InsertText`/`Commit` during a
  composition finish it with that text. The caret rectangle is an estimate (layout has no query for a range's box; it is exact for
  monospace text), measured once for a newly focused editable and while composing, and after a reflow; it is relative to the frame
  like `ShowEmbedderControl`'s rectangle. The text of an editing host's state is the plain text of the block that holds the
  selection. **Cost:** a flush first compares a *stamp* (a text control keeps a content counter bumped at the two places that write
  its text, plus its edit point, selection origin and composition; an editing host's block has the DOM version number of
  `inclusive_descendants_version`, plus the selection and marked ranges) with the last one, and returns before it copies any
  text when nothing changed and the caret rectangle needs no new measurement; so a reflow caused elsewhere on the page (an
  animation) costs one comparison, whatever the size of the text. A state costs one copy of the control's text per change.
  `SetMarkedText` and `InsertText` force a style reflow once each (`is_still_rendered`: an event listener may have hidden the
  element); that is one per keystroke of an input method, not per frame. **Document id:** every state carries the document's
  pipeline id and "nothing focused" is the separate `TextMsg::InputCleared(id)`, so the browser can ignore the late "cleared" of a
  page after the state of an iframe's field arrived. A composition left behind by a focus change between two input method events
  ends with its own `compositionend` before the next event is handled.
- **Why:** Chinese, Japanese and Korean input, dead keys, the emoji picker, dictation and the accent menu (R14 EB11, P0).
- **Upstream:** the Commit part is servo/servo#49400 (merge commit `13fbe153e`, drop at the bump that has it); the rest is Mercury's.
  **Tested by:** `servo/tests/mercury_ime.rs` (25 tests; `make engine-tests`, see `servo/MERCURY.md`) and the `#[cfg(test)]` modules of
  `composition.rs` and `text_input_state.rs` (`make engine-tests` runs `cargo test -p servo-script --lib`); nothing was seen in a
  window. **Unverified against Chromium or WPT:** the order of the composition events (on a commit there is no `compositionupdate`
  that carries the final text, and a commit of the text that is already marked still fires a second `beforeinput`/`input`) and
  the order of `compositionend` and `blur` (here `compositionend` can follow `blur`, from the blur handler's flush): run WPT
  `uievents/composition` and `editing/` composition tests and align, or soften the wording. **Drop when:** never as a whole; if a bump brings composition for editing hosts, `composition.rs` and the host half of
  `ImeState` go.

## Warnings

Cargo does not cap lints for path dependencies, so compiling this crate in
Mercury shows rustc warnings that crates.io builds hide. With Mercury's feature
set (no `gamepad`, `webgpu`, testing hooks) and toolchain, the crate as
published gives 16; each is silenced at its own site, so that a new warning —
from a Mercury change or a Servo upgrade — still shows:

| Site | Warning | Silenced with |
|---|---|---|
| `dom/document/document_event_handler.rs` `use crate::dom::types::{…}` | unused import `GlobalScope` | `#[allow(unused_imports)]` on that `use` |
| `dom/html/embedded_content/htmlvideoelement.rs` `use script_bindings::error::Fallible` | unused import (used with `webgpu`) | `#[allow(unused_imports)]` |
| `dom/document/document.rs` `queue_document_completion` | unused variable `cx` | `#[allow(unused_variables)]` on the function |
| `tasks/task_manager.rs` `gamepad_task_source` | never used (only `gamepad` uses it) | `#[cfg(feature = "gamepad")]` on the definition |
| `dom/animations/animationtimeline.rs`, `documenttimeline.rs` `advance_specific`; `dom/bindings/weakref.rs` `MutableWeakRef::set`; `dom/canvas/imagedata.rs` `get_snapshot`; `dom/document/document.rs` `advance_animation_timeline_for_testing`; `dom/html/embedded_content/htmlcanvaselement.rs` `get_or_init_webgpu_context`, `is_valid`; `dom/window/window.rs` `gc`, `advance_animation_clock`; `dom/worklet/worklet.rs` `perform_a_worklet_task`; `event_loop/script_thread.rs` `handle_tick_all_animations_for_testing` | never used | `#[allow(dead_code)]` on the item |
| `dom/mod.rs` `mod types`; `dom/bindings/mod.rs` `mod Bindings` | unfulfilled `#[expect(unused)]` / `#[expect(non_camel_case_types)]` | `allow` instead of `expect` |

Cargo also reports `backtrace` as an unused dependency of the crate (it is used
only with `webgl_backtrace` and the testing hooks); `[lints.cargo]
unused_dependencies = "allow"` at the end of `Cargo.toml` silences that one
lint, which concerns only this manifest's dependency list.

The other three vendored crates compile without warnings.

## Removing this directory

Remove the `[patch.crates-io]` entry and this directory once a Servo release
contains equivalents of every change listed here and in the other vendored
crates' `MERCURY.md`, or once Mercury no longer needs them.
