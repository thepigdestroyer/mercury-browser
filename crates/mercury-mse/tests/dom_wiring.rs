// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Tripwire for the DOM wiring of the med-1b review fixes (findings 1, 2, 4, 5). The decisions are pure
//! code in this crate and are tested there (`ops_state`, `ops_backend`, `stall_controller`,
//! `media_state_table`, `position_gate_backward_seek`); what no unit test can see is whether the vendored
//! DOM calls them. These tests read the DOM sources as text and fail when one of the calls goes missing
//! or the old inline logic comes back. They check that the wiring exists, not that the page behaves:
//! `tests/pages/mse-abort.html`, `mse-seek.html` and `mse-underrun.html` are the window-level proof.
//!
//! The DOM tree is `<repo>/third_party/servo-script`; `MERCURY_DOM_ROOT` overrides the repository root
//! (used to run the tests against an older tree, as a negative control).

use std::path::PathBuf;

fn root() -> PathBuf {
    match std::env::var_os("MERCURY_DOM_ROOT") {
        Some(root) => PathBuf::from(root),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
    }
}

fn dom_source(relative: &str) -> String {
    let path = root().join("third_party/servo-script/dom").join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The text of the function that starts with `signature`, up to the next function of the same impl.
fn body_of(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("no `{signature}` in the source"));
    let rest = &source[start + signature.len()..];
    let end = ["\n    fn ", "\n    pub(", "\n    pub fn ", "\n    #[", "\n    ///"]
        .iter()
        .filter_map(|marker| rest.find(marker))
        .min()
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

fn code_lines(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_source_buffer_keeps_its_operation_state_in_source_buffer_ops() {
    let source = code_lines(&dom_source("mse/sourcebuffer.rs"));
    assert!(source.contains("SourceBufferOps"), "SourceBuffer must own a SourceBufferOps");
    for call in [
        ".begin_append(",
        ".begin_remove(",
        ".check_abort(",
        ".abort()",
        ".abort_pending()",
        ".detach()",
        ".finished(",
        ".failed(",
    ] {
        assert!(source.contains(call), "sourcebuffer.rs no longer calls `{call}`");
    }
    assert!(
        !source.contains("OpTracker") && !source.contains("updating: Cell<bool>"),
        "the DOM keeps its own tracker or `updating` flag again: the state belongs to SourceBufferOps"
    );
    assert!(
        !source.contains("updating.set("),
        "something sets `updating` directly: every transition goes through SourceBufferOps"
    );
}

#[test]
fn abort_asks_the_state_machine_before_the_backend() {
    let source = dom_source("mse/sourcebuffer.rs");
    let abort = code_lines(&body_of(&source, "fn Abort(&self)"));
    let check = abort.find(".check_abort(").expect("Abort() does not call check_abort");
    let backend = abort.find(".abort(self.id)").expect("Abort() does not call the backend's abort");
    assert!(check < backend, "a refused abort() must not reach the backend's abort");
    assert!(
        abort.contains("InvalidState") || abort.contains("abort_refused"),
        "Abort() does not turn the refusal into an InvalidStateError"
    );
}

#[test]
fn remove_and_append_register_their_operations() {
    let source = dom_source("mse/sourcebuffer.rs");
    let remove = code_lines(&body_of(&source, "fn Remove(&self"));
    assert!(remove.contains(".begin_remove("), "Remove() must call begin_remove (abort() throws while it runs)");
    let append = code_lines(&body_of(&source, "fn AppendBuffer(&self"));
    assert!(append.contains(".begin_append("), "AppendBuffer() must call begin_append");
}

#[test]
fn the_position_is_not_forwarded_to_the_backend_while_seeking() {
    let source = dom_source("mse/mediaelement.rs");
    let body = code_lines(&body_of(&source, "fn media_source_position_changed("));
    assert!(body.contains("forward_position("), "media_source_position_changed must go through forward_position");
    assert!(body.contains("seeking"), "forward_position needs the element's `seeking` flag");
    assert!(body.contains(".set_position("), "the position still has to reach the backend when not seeking");
}

#[test]
fn the_stall_check_hands_raw_facts_to_the_controller() {
    let source = dom_source("mse/mediaelement.rs");
    let body = code_lines(&body_of(&source, "fn media_source_check_stall("));
    assert!(body.contains("ElementSnapshot"), "the stall check must build an ElementSnapshot");
    assert!(body.contains(".step("), "the stall check must call StallController::step");
    assert!(
        !body.contains("StallInput") && !body.contains("StallGuard"),
        "the DOM builds the guard's input again: that belongs to StallController"
    );
    assert!(body.contains("recheck_in_ms"), "the re-check delay comes from the controller's answer");
    assert!(body.contains("PipelineChange"), "the pipeline pause and resume follow the controller's answer");
    let recheck = code_lines(&body_of(&source, "fn schedule_media_source_stall_recheck("));
    assert!(recheck.contains("recheck_fired"), "the re-check timer must tell the controller it fired");
    let stall = code_lines(&dom_source("mse/stall.rs"));
    assert!(stall.contains("StallController") && !stall.contains("recheck_pending"));
}

#[test]
fn update_media_state_and_waiting_use_the_pure_decisions() {
    let element = code_lines(&dom_source("html/embedded_content/htmlmediaelement.rs"));
    let update = body_of(&element, "fn update_media_state(&self)");
    assert!(update.contains("media_state_decision("), "update_media_state must use media_state_decision");
    assert!(
        !update.contains("is_potentially_playing()"),
        "update_media_state tests the conditions inline again"
    );
    assert!(
        element.contains("media_waiting_fires("),
        "change_ready_state must use media_waiting_fires"
    );
    assert!(
        !element.contains("!self.error.get().is_none()"),
        "the published `waiting` condition (fired only with an error) is back"
    );
}

#[test]
fn the_update_media_state_change_applies_only_with_a_media_source() {
    let source = dom_source("mse/mediaelement.rs");
    let body = code_lines(&body_of(&source, "fn media_state_decision("));
    assert!(body.contains("media_state_action("));
    assert!(
        body.contains("self.media_source.get().is_some()"),
        "the last argument of media_state_action must be 'a MediaSource is attached' until F6 has passed \
         (MERCURY.md item 8)"
    );
}
