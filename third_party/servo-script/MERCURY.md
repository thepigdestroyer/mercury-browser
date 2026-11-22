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

## Changes: Media Source Extensions (lane med-1, MSE by Plan B, 20 Nov 2026)

Behind `dom_media_source_enabled` (off by default; `MERCURY_DEV_MSE=1` turns it on for development, see
`docs/developer-guide/media-source.md`). The DOM objects are carried from servo/servo#47269
(decomoraes, MPL-2.0; the pull request was closed unmerged on 16 Aug 2026 after its author said it was written with an LLM, so Servo never reviewed the carried code, and Mercury re-checked it: see the guide: `MediaSource`, `SourceBuffer`, `SourceBufferList`, the object URL store, the
`HTMLMediaElement` attach and detach steps) and **thinned**: Mercury's own Servo-free crate
`crates/mercury-mse` owns every spec algorithm that needs no DOM object (readyState, duration,
`endOfStream`, the buffered intersection, abort, remove, `changeType`, the append window, the timestamp
offset, the mode, the track buffers, the byte-stream parsers, quota and eviction), and a GStreamer source
element in `crates/mercury-mse-gst` feeds the player. The DOM calls `MediaSourceBackend` and turns its
`MseEvent`s into events and media element notifications. The PR's `bytestream.rs`, its parser and its
`push_data` data plane are not carried; nothing is ever pushed to the player.

1. **`dom/mse/` (new directory):** `mediasource.rs`, `sourcebuffer.rs`, `sourcebufferlist.rs`, `mod.rs`, and
   `mediaelement.rs` (Mercury's own element side, declared by `htmlmediaelement.rs`, see item 5).
   Each file's header names the PR. `MediaSource` creates its backend through
   `ServoMedia::create_media_source` with a `GenericCallback` that only queues a media element task (the
   media element's own player-callback pattern, with a weak reference so the object stays collectable),
   and handles the `MseEvent`s there. `SourceBuffer` keeps the JavaScript-visible `updating` flag as the
   specification's tasks publish it (cleared by the task that handles `AppendDone`, `AppendError`,
   `RemoveDone`), the last known attribute values for a removed buffer, and empty `audioTracks` and
   `videoTracks`. Left out: `textTracks` (this Servo's `TextTrackList` needs a media element), the
   contents of the track lists, `ManagedMediaSource`, `BufferedChangeEvent`, `MediaSourceHandle`, MSE in
   workers (`canConstructInDedicatedWorker` is false).
2. **`dom/mod.rs`:** `pub(crate) mod mse;` and its glob re-export (the bindings look interfaces up as
   `crate::dom::<lowercase name>::<Name>`).
3. **`dom/globalscope/globalscope.rs`:** `media_source_url_store` and `track_media_source_url`,
   `media_source_for_url`, `forget_media_source_url` (a `MediaSource` never leaves its thread, so unlike a
   blob URL its entry cannot live in the file manager). Carried from the PR.
4. **`dom/url/url.rs`:** `URL.createObjectURL(MediaSource)` (`CreateObjectURL_`) and `revokeObjectURL` for
   it. Carried from the PR.
5. **`dom/html/embedded_content/htmlmediaelement.rs`** (hot file, every hunk marked `Mercury (MED-1)`;
   the line numbers are approximate (they date from the first MED-1 commit and moved when the `media_source_*`
   block left the file): `grep -n "Mercury (MED-1)"` finds every hook, for the next Servo bump):
   * 36, 63, 93: imports. 134-155: `SrcObject::MediaSource` and the union conversion
     (`MediaStreamOrMediaSourceOrBlob`; the IDL union gained `MediaSource`). 300-304, 475: the
     `media_source` field. 1353-1360: `load_from_src_attribute` makes `Resource::MediaSource` for a URL in
     the object URL store. 1669-1690: `resource_fetch_algorithm` attaches first (failure steps when the
     source is not closed or was attached before), then creates the player, and detaches if that fails.
     1746-1752: the new `Resource::MediaSource` and `SrcObject::MediaSource` arms fetch nothing. 1840:
     `in_error_state` is `pub(crate)`.
   * The `media_source_*` functions are **not in this file**: they are in the new `dom/mse/mediaelement.rs`
     (an `impl HTMLMediaElement` that this file declares as a child module with one `#[path =
     "../../mse/mediaelement.rs"] mod mediaelement_mse;` line, so it sees the element's private items and no
     upstream item changed its visibility), and a three-line comment marks the place where they used to be
     (after `in_error_state`). They are `media_source_for_resource`,
     `queue_media_source_stop_delaying_the_load_event`, `detach_media_source`,
     `media_source_position_changed`, `media_source_seeking`, `media_source_update_ready_state` (readiness
     from `element_ready_state` around the position: HAVE_METADATA outside the buffered ranges, HAVE_ENOUGH_DATA
     once ended, HAVE_CURRENT_DATA at the end), `media_source_metadata_received` (the "once enough of the
     media data has been fetched" steps), `media_source_init_received`, `media_source_duration_changed`
     (idempotent), `media_source_end_of_stream`, `media_source_error`. The first seven are `pub(super)`, the rest
     `pub(crate)`.
   * 2498-2503: `seek` calls `media_source_seeking` after setting the new position. 2565-2573, 2636-2670:
     `create_media_player` (`StreamType::MediaSource`, `connect_media_source` as the player's first call, no
     download buffering). 2713: `reset_media_player` detaches the MediaSource after the player is dropped.
   * 2887, 3133, 3300-3315: `playback_metadata_updated` and `playback_duration_changed` ignore the media
     engine for a MediaSource element; `playback_state_changed` never upgrades readiness from the
     pipeline's state. 3338, 3966: `seekable` and `Buffered` come from the backend. 3735: `GetSrcObject`.
     4247: the `Resource::MediaSource` variant. 4784: `PositionChanged` tells the backend and re-evaluates the
     readiness.
6. **`servo-script-bindings` (see its `MERCURY.md`):** `MediaSource.webidl`, `SourceBuffer.webidl`,
   `SourceBufferList.webidl`, `URL.webidl` (the overload), `HTMLMediaElement.webidl` (the union),
   `Bindings.conf` (`cx` lists and `weakReferenceable` for `MediaSource`).

### MED-1b (22 Nov 2026): the two blockers, `getVideoPlaybackQuality`, track lists

7. **`dom/mse/` (Mercury's own files):** `stall.rs` (the `StallController` of `crates/mercury-mse`, which owns
   the guard and the "re-check pending" flag, and a clock for it, kept on the `MediaSource` so the element
   struct gets no field), `tracks.rs`
   (the contents of `SourceBuffer.audioTracks`/`videoTracks` and of the element's lists: `AudioTrack` and
   `VideoTrack` per `TrackInfo`, `addtrack`/`removetrack`; lists are rebuilt with `clear` and `add` because the
   upstream lists have no `remove`), `videoplaybackquality.rs` (the `VideoPlaybackQuality` object). In the
   carried files: `sourcebuffer.rs` keeps a `SourceBufferOps` (the pure operation state of
   `crates/mercury-mse/src/source_buffer_ops.rs`: pending operation, `updating`, the events of each
   transition, `abort()` refused while a removal is pending) and maps its `SbEvent`s to tasks; it takes the
   `OpId` of every completion event (`operation_finished(op)`, `operation_failed(op)`); `mediasource.rs`
   passes it, calls `init_received`/`remove_tracks`, owns the `StallState`. `mediaelement.rs`:
   `media_source_update_ready_state` now ends in `media_source_check_stall` (the raw element facts go to
   `StallController::step`; the DOM keeps the pipeline pause and resume through `update_media_state` and a
   re-check thread for the delay the controller returns, at most 260 ms), `media_source_stalled`,
   `media_source_position_changed` (drops the player's position while seeking: `forward_position`),
   `media_state_decision` and `media_waiting_fires` (the two decisions of item 8, from
   `crates/mercury-mse/src/media_state.rs`), `player_video_playback_quality`.
8. **`htmlmediaelement.rs`, hunks marked `Mercury (MED-1b)` (`grep -n "MED-1b"`):** a `use` line for
   `MediaStateAction`; in `update_media_state` (about lines 500-535) the play and pause branches test
   `media_state_decision(is_playing)` (`media_state_action` of `mercury-mse`) instead of the published
   conditions: with a MediaSource attached the pipeline is not resumed while `media_source_stalled()`, and an
   element that is playing and may play on is no longer paused (published: `else if is_playing && pause()`
   paused it whenever it was called while playing). **This first hunk is gated on a MediaSource being
   attached:** without one `media_state_action(.., media_source_attached = false)` is the published decision
   byte for byte (a table test compares all 16 inputs), so a plain `<video>`/`<audio>` is unchanged. **F6
   (`sdd/deferred-hands-on.md` section F, item F6: pause, resume, seek, loop, ended and a stalled network source
   on a plain file, in a window) is the gate for removing the gate** (pass `true` instead of
   `media_source.get().is_some()` in `media_state_decision`). In `change_ready_state` (about line 1120) the
   `waiting` condition is `media_waiting_fires(old, new, was_potentially_playing)` (`waiting_fires` of
   `mercury-mse`); published, it tested `!self.error.get().is_none()`, so `waiting` fired only for an element
   with an error. This second hunk is the specified behaviour and is **not** gated; `waiting_fires` has table
   tests over every input, plain media included. **Drop both at B1 if Servo has fixed them itself** (check
   then); if only the second is fixed nothing doubles, because the guard fires no events. They stay Mercury
   patches: nothing is sent to Servo.
9. **`htmlvideoelement.rs`:** `GetVideoPlaybackQuality` (one method) and its `use` line, `Mercury (MED-1b)`.
10. **`servo-script-bindings`:** `VideoPlaybackQuality.webidl`, the `getVideoPlaybackQuality()` partial in
    `HTMLVideoElement.webidl`, `Bindings.conf` (`cx` for `GetVideoPlaybackQuality`), all behind
    `dom_media_source_enabled`. **Drop when** Servo 0.9 has `getVideoPlaybackQuality`, track list content on
    `SourceBuffer` or Igalia's MSE (servo/servo#44403).

**Not run in a window** (the screen was locked): the DOM compiles and its pure logic is tested in
`crates/mercury-mse`; the pages `tests/pages/mse-*.html` are written and unrun. Dropped when a Servo release
has its own MSE (servo/servo#44403) and Mercury moves to it.

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
