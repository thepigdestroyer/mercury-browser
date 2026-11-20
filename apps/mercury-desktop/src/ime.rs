// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The input-method (IME) composition state machine (txt-1 lane).
//!
//! What winit reports about the input method (`Ime::Enabled`, `Preedit`, `Commit`, `Disabled`) and
//! what the AppKit layer adds (`macos_text_input.rs`: clauses, replacement ranges, text that comes
//! without a composition) go in; the commands for the page (`ImeInput`) come out. [`Machine`] is
//! pure: no window, no controller, no clock, so every rule below is tested without a window.
//!
//! # Rules
//!
//! * A non-empty `Preedit` is marked text: `SetMarkedText`, starting a composition if none is
//!   open. winit gives the cursor in UTF-8 bytes; the page wants UTF-16 units, so the cursor is
//!   converted (clamped, never inside a character). The clauses and the replacement range come
//!   from the record AppKit's `setMarkedText:` left just before (see [`NativeIme::Marked`]), or,
//!   without one, a single clause over the whole text.
//! * `Preedit("")` is **held**. winit macOS sends it right before a `Commit` (the commit's first
//!   half) and also alone when the user cancels (Escape) or deletes the last marked character.
//!   `Commit(text)` after it is `Commit(text)`; if nothing follows by the end of the event-loop
//!   turn ([`flush`], run by a zero-delay timer) it was a cancel: `Commit("")`.
//! * `Commit` with no composition (the emoji picker through winit, a dead key committing) is
//!   `Commit` to the active tab. Final text with an empty string and no composition is dropped.
//! * `Disabled`, the window losing focus, a sheet shown over the page, and the active tab
//!   changing end the composition with `Dismissed` and forget everything.
//! * Every command goes to the tab that **started** the composition, whichever tab is active
//!   when it is sent.
//!
//! # The signal for the key switcher
//!
//! [`composition_active`] is **the** signal for `option_keys.rs` (SPEC section 5.1): while it is
//! `true`, an input method owns the keyboard (Option and Shift keys select candidates, move
//! clauses and type dead keys) and no key may be taken as a shortcut or switcher key.
//! [`composition_tab`] says which tab composes. It is `true` from the first marked text until the
//! composition ends, including the short wait for a held `Preedit("")` to turn out to be a
//! cancel, and `false` after every ending (commit, cancel, dismissal, a tab switch). Read it on the
//! main thread inside the key's event handler; the value is an atomic, so it is also safe to read
//! elsewhere. A key event that arrives while it is `true` is already marked `is_composing` for the
//! page by `keyboard.rs`.
//!
//! # Wiring
//!
//! [`attach`] (called once from `keyboard.rs`) gives the module the controller, a way to redraw
//! and the question "may the page take text now". Without it the entry points that need them
//! ([`native_event`], [`snapshot`]) do nothing. Commands that cannot be queued at once because
//! the controller is borrowed wait in a backlog and are sent by a zero-delay timer.
//!
//! Nothing here logs a payload: what was typed must not reach a log.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};

use talaria_core::{
    BrowserCommand, BrowserController, ImeClause, ImeClauseKind, ImeInput, ImeReplacement, TabId,
    TextInputState, Utf16Range, utf16_len, utf16_offset_of_byte,
};

/// What winit reports about the input method. `cursor` is in UTF-8 bytes of `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Input {
    Enabled,
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    Commit(String),
    Disabled,
}

/// What the AppKit layer knows that winit's events do not carry.
// Only the macOS layer reports these; the machine itself is portable.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum NativeIme {
    /// `setMarkedText:` was called: the clauses of the marked text (ranges inside it, UTF-16
    /// units) and the range of the field it replaces. The record belongs to the next non-empty
    /// `Preedit`, which winit reports for the same call.
    Marked {
        clauses: Vec<ImeClause>,
        replacement: Option<ImeReplacement>,
    },
    /// Text that winit drops: `insertText:` with no marked text, outside a key press (the emoji
    /// picker, dictation) or with a replacement range (the accent menu, a dictation correction).
    InsertText {
        text: String,
        replacement: Option<ImeReplacement>,
    },
}

impl fmt::Debug for NativeIme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Sizes only, never the text.
        match self {
            Self::Marked {
                clauses,
                replacement,
            } => formatter
                .debug_struct("Marked")
                .field("clauses", clauses)
                .field("replacement", replacement)
                .finish(),
            Self::InsertText { text, replacement } => formatter
                .debug_struct("InsertText")
                .field("text_bytes", &text.len())
                .field("replacement", replacement)
                .finish(),
        }
    }
}

/// One command for the page of one tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Command {
    pub(crate) tab: TabId,
    pub(crate) input: ImeInput,
}

impl Command {
    fn new(tab: TabId, input: ImeInput) -> Self {
        Self { tab, input }
    }
}

/// The clauses and replacement of one `setMarkedText:` call.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeRecord {
    clauses: Vec<ImeClause>,
    replacement: Option<ImeReplacement>,
}

impl NativeRecord {
    /// Whether the clauses can belong to marked text of `length` UTF-16 units: they are not
    /// empty, each lies inside the text, and together they reach its end.
    fn fits(&self, length: u32) -> bool {
        !self.clauses.is_empty()
            && self
                .clauses
                .iter()
                .all(|clause| clause.range.start <= clause.range.end && clause.range.end <= length)
            && self
                .clauses
                .iter()
                .map(|clause| clause.range.end)
                .max()
                .is_some_and(|end| end == length)
    }
}

/// The composition state. See the module documentation for the rules.
#[derive(Debug, Default)]
pub(crate) struct Machine {
    /// The tab that started the open composition.
    composing: Option<TabId>,
    /// A `Preedit("")` arrived while composing and nothing has told whether it was a commit's
    /// first half or a cancel.
    held_empty: bool,
    /// The record of the latest `setMarkedText:`, waiting for its `Preedit`.
    pending: Option<NativeRecord>,
}

impl Machine {
    pub(crate) fn is_active(&self) -> bool {
        self.composing.is_some()
    }

    pub(crate) fn tab(&self) -> Option<TabId> {
        self.composing
    }

    /// Whether a `Preedit("")` waits for [`Machine::flush`].
    pub(crate) fn holds_empty(&self) -> bool {
        self.held_empty
    }

    /// Whether a composition is open in a tab other than `active`.
    pub(crate) fn composing_elsewhere(&self, active: TabId) -> bool {
        self.composing.is_some_and(|tab| tab != active)
    }

    /// One event from winit. `active` is the active tab now; it only matters for a composition
    /// that starts or text that comes without one.
    pub(crate) fn input(&mut self, active: TabId, input: Input) -> Vec<Command> {
        match input {
            Input::Enabled => {
                // A record from before the input method was switched on belongs to nothing.
                self.pending = None;
                Vec::new()
            }
            Input::Preedit { text, cursor } => self.preedit(active, text, cursor),
            Input::Commit(text) => self.commit(active, text),
            Input::Disabled => self.dismiss(),
        }
    }

    fn preedit(
        &mut self,
        active: TabId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> Vec<Command> {
        if text.is_empty() {
            // Held: a commit or a cancel follows. Without a composition there is nothing to end.
            self.held_empty = self.composing.is_some();
            return Vec::new();
        }
        self.held_empty = false;
        let tab = *self.composing.get_or_insert(active);
        let length = utf16_len(&text);
        let selected = cursor.map(|(start, end)| {
            let (start, end) = (start.min(end), start.max(end));
            Utf16Range::new(
                utf16_offset_of_byte(&text, start),
                utf16_offset_of_byte(&text, end),
            )
        });
        let (clauses, replacement) = match self.pending.take().filter(|record| record.fits(length))
        {
            Some(record) => (record.clauses, record.replacement),
            None => (vec![whole_text_clause(length, selected)], None),
        };
        vec![Command::new(
            tab,
            ImeInput::SetMarkedText {
                text,
                selected,
                clauses,
                replacement,
            },
        )]
    }

    fn commit(&mut self, active: TabId, text: String) -> Vec<Command> {
        self.held_empty = false;
        match self.composing.take() {
            // Also an empty text: that is a cancel.
            Some(tab) => vec![Command::new(tab, ImeInput::Commit(text))],
            None if text.is_empty() => Vec::new(),
            None => vec![Command::new(active, ImeInput::Commit(text))],
        }
    }

    /// Resolves a held `Preedit("")`: nothing followed it in this turn, so the user cancelled.
    pub(crate) fn flush(&mut self) -> Vec<Command> {
        if !std::mem::take(&mut self.held_empty) {
            return Vec::new();
        }
        match self.composing.take() {
            Some(tab) => vec![Command::new(tab, ImeInput::Commit(String::new()))],
            None => Vec::new(),
        }
    }

    /// Ends the composition without a result: `Disabled`, focus lost, a sheet shown. Forgets
    /// everything.
    pub(crate) fn dismiss(&mut self) -> Vec<Command> {
        self.held_empty = false;
        self.pending = None;
        self.composing
            .take()
            .map(|tab| Command::new(tab, ImeInput::Dismissed))
            .into_iter()
            .collect()
    }

    /// The active tab is `active`: a composition of another tab ends.
    pub(crate) fn active_tab_changed(&mut self, active: TabId) -> Vec<Command> {
        if self.composing_elsewhere(active) {
            self.dismiss()
        } else {
            Vec::new()
        }
    }

    /// One report of the AppKit layer.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn native(&mut self, active: TabId, event: NativeIme) -> Vec<Command> {
        match event {
            NativeIme::Marked {
                clauses,
                replacement,
            } => {
                self.pending = Some(NativeRecord {
                    clauses,
                    replacement,
                });
                Vec::new()
            }
            NativeIme::InsertText { text, replacement } => {
                // A cancel that was still waiting for the end of its turn comes first. AppKit
                // inserts only when it has no marked text, so a composition still open here is
                // out of step with it: end that one too.
                let mut commands = self.flush();
                commands.extend(self.dismiss());
                if !text.is_empty() {
                    commands.push(Command::new(
                        active,
                        ImeInput::InsertText { text, replacement },
                    ));
                }
                commands
            }
        }
    }
}

/// One clause over the whole marked text; the selected kind when the input method has selected
/// something inside it.
fn whole_text_clause(length: u32, selected: Option<Utf16Range>) -> ImeClause {
    ImeClause {
        range: Utf16Range::new(0, length),
        kind: if selected.is_some_and(|range| !range.is_empty()) {
            ImeClauseKind::SelectedRaw
        } else {
            ImeClauseKind::Raw
        },
    }
}

// ---------------------------------------------------------------------------------------------
// The one machine of the browser window, and the entry points that use it.
// ---------------------------------------------------------------------------------------------

/// What [`attach`] gives the module.
struct Link {
    controller: Weak<RefCell<BrowserController>>,
    redraw: Box<dyn Fn()>,
    accepts_text: Box<dyn Fn() -> bool>,
}

static COMPOSITION_ACTIVE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static MACHINE: RefCell<Machine> = RefCell::new(Machine::default());
    static LINK: RefCell<Option<Link>> = const { RefCell::new(None) };
    /// Commands that could not be queued because the controller was borrowed.
    static BACKLOG: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
    static DRAIN_SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static FLUSH_SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static DISMISS_SCHEDULED: Cell<bool> = const { Cell::new(false) };
}

/// Whether an input method composition is open: the signal for `option_keys.rs`. See the module
/// documentation.
pub(crate) fn composition_active() -> bool {
    COMPOSITION_ACTIVE.load(Ordering::Relaxed)
}

/// The tab whose page has the open composition (main thread only).
pub(crate) fn composition_tab() -> Option<TabId> {
    MACHINE.with(|machine| machine.borrow().tab())
}

/// Whether a composition is open in `tab`: what makes a key event of that tab `is_composing`.
pub(crate) fn composing_in(tab: TabId) -> bool {
    composition_tab() == Some(tab)
}

fn with_machine<R>(action: impl FnOnce(&mut Machine) -> R) -> R {
    MACHINE.with(|machine| {
        let mut machine = machine.borrow_mut();
        let result = action(&mut machine);
        COMPOSITION_ACTIVE.store(machine.is_active(), Ordering::Relaxed);
        result
    })
}

/// Gives the module what it needs beyond a controller handed to each call: the controller (held
/// weakly), a way to redraw the window, and whether the page may take text now (it has the
/// keyboard, no sheet covers it and no field of the window's own has the text focus).
pub(crate) fn attach(
    controller: &Rc<RefCell<BrowserController>>,
    redraw: impl Fn() + 'static,
    accepts_text: impl Fn() -> bool + 'static,
) {
    LINK.with(|link| {
        *link.borrow_mut() = Some(Link {
            controller: Rc::downgrade(controller),
            redraw: Box::new(redraw),
            accepts_text: Box::new(accepts_text),
        });
    });
}

/// One event from winit, for the page of the active tab (or of the tab that composes).
pub(crate) fn handle(controller: &RefCell<BrowserController>, input: Input) {
    let active = controller.borrow().active_tab_id();
    let (commands, holds_empty) = with_machine(|machine| {
        let commands = machine.input(active, input);
        (commands, machine.holds_empty())
    });
    send(controller, commands);
    if holds_empty {
        schedule_flush();
    }
}

/// Ends the composition without a result: a sheet is shown, the focus left the page or the
/// window.
pub(crate) fn dismiss_composition(controller: &RefCell<BrowserController>) {
    let commands = with_machine(Machine::dismiss);
    send(controller, commands);
}

/// Resolves a held empty preedit: run at the end of the event-loop turn in which it arrived.
pub(crate) fn flush(controller: &RefCell<BrowserController>) {
    let commands = with_machine(Machine::flush);
    send(controller, commands);
}

/// The active tab is `active`; a composition in another tab ends (a zero-delay timer does it, so
/// the caller, a state update, need not borrow the controller mutably). Cheap when nothing
/// composes.
pub(crate) fn active_tab_changed(active: TabId) {
    if !MACHINE.with(|machine| machine.borrow().composing_elsewhere(active)) {
        return;
    }
    if DISMISS_SCHEDULED.replace(true) {
        return;
    }
    run_after_this_turn(|| {
        DISMISS_SCHEDULED.set(false);
        let Some(controller) = linked_controller() else {
            return;
        };
        let Ok(active) = controller
            .try_borrow()
            .map(|controller| controller.active_tab_id())
        else {
            // Borrowed right now: the next state update asks again.
            return;
        };
        let commands = with_machine(|machine| machine.active_tab_changed(active));
        send(&controller, commands);
    });
}

/// One report of the AppKit layer (`macos_text_input.rs`), on the main thread.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn native_event(event: NativeIme) {
    #[cfg(test)]
    NATIVE_LOG.with(|log| log.borrow_mut().push(event.clone()));
    let Some(controller) = linked_controller() else {
        return;
    };
    if matches!(event, NativeIme::InsertText { .. }) && !text_accepted() {
        // The text belongs to a field of the window (the address bar), not to the page.
        return;
    }
    let Ok(active) = controller
        .try_borrow()
        .map(|controller| controller.active_tab_id())
    else {
        return;
    };
    let commands = with_machine(|machine| machine.native(active, event));
    send(&controller, commands);
}

/// Reads the text state of the active tab's focused editable element, while the page may take
/// text. The native layer answers `selectedRange` and `attributedSubstring` from it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn with_snapshot<R>(read: impl FnOnce(&TextInputState) -> R) -> Option<R> {
    #[cfg(test)]
    if let Some(forced) = TEST_SNAPSHOT.with(|forced| forced.borrow().clone()) {
        return forced.as_ref().map(read);
    }
    if !text_accepted() {
        return None;
    }
    let controller = linked_controller()?;
    let controller = controller.try_borrow().ok()?;
    controller.active_state().text_input().map(read)
}

/// The latest text state of the active tab's focused editable element (a copy; the native layer
/// reads it in place with [`with_snapshot`]). Part of the module's interface for other lanes.
#[allow(dead_code)]
pub(crate) fn snapshot() -> Option<TextInputState> {
    with_snapshot(Clone::clone)
}

fn linked_controller() -> Option<Rc<RefCell<BrowserController>>> {
    LINK.with(|link| link.borrow().as_ref()?.controller.upgrade())
}

fn text_accepted() -> bool {
    LINK.with(|link| {
        link.borrow()
            .as_ref()
            .is_some_and(|link| (link.accepts_text)())
    })
}

fn redraw() {
    LINK.with(|link| {
        if let Some(link) = link.borrow().as_ref() {
            (link.redraw)();
        }
    });
}

/// Queues the commands for their tabs. A controller that is borrowed at the moment keeps them in
/// the backlog for a timer.
fn send(controller: &RefCell<BrowserController>, commands: Vec<Command>) {
    if commands.is_empty() {
        return;
    }
    BACKLOG.with(|backlog| backlog.borrow_mut().extend(commands));
    if !drain_backlog(controller) {
        schedule_drain();
    }
}

/// Queues everything in the backlog; `false` when the controller is borrowed and some of it is
/// still waiting.
fn drain_backlog(controller: &RefCell<BrowserController>) -> bool {
    let Ok(mut controller) = controller.try_borrow_mut() else {
        return false;
    };
    let commands = BACKLOG.with(|backlog| std::mem::take(&mut *backlog.borrow_mut()));
    for command in commands {
        controller.queue_command_for(command.tab, BrowserCommand::ImeInput(command.input));
    }
    true
}

fn schedule_drain() {
    if DRAIN_SCHEDULED.replace(true) {
        return;
    }
    run_after_this_turn(|| {
        DRAIN_SCHEDULED.set(false);
        let Some(controller) = linked_controller() else {
            return;
        };
        if drain_backlog(&controller) {
            redraw();
        } else {
            schedule_drain();
        }
    });
}

fn schedule_flush() {
    if FLUSH_SCHEDULED.replace(true) {
        return;
    }
    run_after_this_turn(|| {
        FLUSH_SCHEDULED.set(false);
        let Some(controller) = linked_controller() else {
            return;
        };
        flush(&controller);
        redraw();
    });
}

/// Runs `task` once the event loop has finished what it is doing now.
#[cfg(not(test))]
fn run_after_this_turn(task: impl FnOnce() + 'static) {
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        task();
    });
}

/// Tests run what is due by hand (`flush`, `drain_backlog`).
#[cfg(test)]
fn run_after_this_turn(_task: impl FnOnce() + 'static) {}

// ---------------------------------------------------------------------------------------------
// Test support.
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
thread_local! {
    static NATIVE_LOG: RefCell<Vec<NativeIme>> = const { RefCell::new(Vec::new()) };
    /// `Some(state)` replaces what [`with_snapshot`] reads.
    static TEST_SNAPSHOT: RefCell<Option<Option<TextInputState>>> = const { RefCell::new(None) };
}

/// Every test that reaches the one machine of this thread (or [`composition_active`]) holds this
/// guard: the atomic is shared by all test threads. It starts the test from a clean state.
#[cfg(test)]
pub(crate) fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    with_machine(|machine| *machine = Machine::default());
    LINK.with(|link| *link.borrow_mut() = None);
    BACKLOG.with(|backlog| backlog.borrow_mut().clear());
    NATIVE_LOG.with(|log| log.borrow_mut().clear());
    TEST_SNAPSHOT.with(|forced| *forced.borrow_mut() = None);
    guard
}

/// The reports [`native_event`] has received on this thread, oldest first.
#[cfg(test)]
pub(crate) fn take_native_events() -> Vec<NativeIme> {
    NATIVE_LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

/// Makes [`with_snapshot`] read `state` (`None`: no editable element has focus).
#[cfg(test)]
pub(crate) fn force_snapshot(state: Option<TextInputState>) {
    TEST_SNAPSHOT.with(|forced| *forced.borrow_mut() = Some(state));
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAB: TabId = TabId::new(1);
    const OTHER_TAB: TabId = TabId::new(2);

    fn preedit(text: &str, cursor: Option<(usize, usize)>) -> Input {
        Input::Preedit {
            text: text.to_owned(),
            cursor,
        }
    }

    fn commit(text: &str) -> Input {
        Input::Commit(text.to_owned())
    }

    fn inputs(commands: &[Command]) -> Vec<&ImeInput> {
        commands.iter().map(|command| &command.input).collect()
    }

    fn raw(range: (u32, u32)) -> ImeClause {
        ImeClause {
            range: Utf16Range::new(range.0, range.1),
            kind: ImeClauseKind::Raw,
        }
    }

    fn marked(text: &str, selected: Option<(u32, u32)>, clauses: Vec<ImeClause>) -> ImeInput {
        ImeInput::SetMarkedText {
            text: text.to_owned(),
            selected: selected.map(|(start, end)| Utf16Range::new(start, end)),
            clauses,
            replacement: None,
        }
    }

    #[test]
    fn the_chinese_sequence_marks_then_commits_after_the_held_empty_preedit() {
        let mut machine = Machine::default();
        machine.input(TAB, Input::Enabled);

        let first = machine.input(TAB, preedit("n", Some((1, 1))));
        assert_eq!(
            first,
            vec![Command::new(
                TAB,
                marked("n", Some((1, 1)), vec![raw((0, 1))])
            )]
        );
        assert!(machine.is_active());
        let second = machine.input(TAB, preedit("ni", Some((2, 2))));
        assert_eq!(
            second,
            vec![Command::new(
                TAB,
                marked("ni", Some((2, 2)), vec![raw((0, 2))])
            )]
        );

        // winit: Preedit("") right before the Commit.
        assert!(machine.input(TAB, preedit("", None)).is_empty());
        assert!(machine.holds_empty());
        assert!(machine.is_active());
        let done = machine.input(TAB, commit("你"));

        assert_eq!(
            done,
            vec![Command::new(TAB, ImeInput::Commit("你".to_owned()))]
        );
        assert!(!machine.is_active());
        assert!(!machine.holds_empty());
        // Nothing is left for the end of the turn.
        assert!(machine.flush().is_empty());
    }

    #[test]
    fn a_dead_key_marks_the_accent_and_commits_the_letter() {
        let mut machine = Machine::default();

        let first = machine.input(TAB, preedit("´", Some((2, 2))));
        // "´" is two bytes in UTF-8 and one unit in UTF-16.
        assert_eq!(
            first,
            vec![Command::new(
                TAB,
                marked("´", Some((1, 1)), vec![raw((0, 1))])
            )]
        );
        machine.input(TAB, preedit("", None));
        let done = machine.input(TAB, commit("é"));

        assert_eq!(
            done,
            vec![Command::new(TAB, ImeInput::Commit("é".to_owned()))]
        );
        assert!(!machine.is_active());
    }

    #[test]
    fn japanese_clauses_and_the_replacement_come_from_the_native_record() {
        let mut machine = Machine::default();
        let clauses = vec![
            ImeClause {
                range: Utf16Range::new(0, 2),
                kind: ImeClauseKind::Converted,
            },
            ImeClause {
                range: Utf16Range::new(2, 3),
                kind: ImeClauseKind::SelectedConverted,
            },
        ];
        let replacement = Some(ImeReplacement {
            range: Utf16Range::new(4, 7),
            sequence: 11,
        });
        // AppKit calls setMarkedText: first; winit reports the Preedit for the same call.
        assert!(
            machine
                .native(
                    TAB,
                    NativeIme::Marked {
                        clauses: clauses.clone(),
                        replacement,
                    },
                )
                .is_empty()
        );

        // "日本語": 9 bytes, 3 units; the selected clause is the last character.
        let commands = machine.input(TAB, preedit("日本語", Some((6, 9))));

        assert_eq!(
            commands,
            vec![Command::new(
                TAB,
                ImeInput::SetMarkedText {
                    text: "日本語".to_owned(),
                    selected: Some(Utf16Range::new(2, 3)),
                    clauses,
                    replacement,
                },
            )]
        );
        // The record was used once: the next Preedit has no record again.
        let next = machine.input(TAB, preedit("日本", Some((6, 6))));
        assert_eq!(
            next,
            vec![Command::new(
                TAB,
                marked("日本", Some((2, 2)), vec![raw((0, 2))])
            )]
        );
    }

    #[test]
    fn a_native_record_that_does_not_fit_the_text_is_ignored() {
        let mut machine = Machine::default();
        machine.native(
            TAB,
            NativeIme::Marked {
                clauses: vec![raw((0, 5))],
                replacement: Some(ImeReplacement {
                    range: Utf16Range::new(0, 1),
                    sequence: 1,
                }),
            },
        );

        let commands = machine.input(TAB, preedit("ni", Some((2, 2))));

        assert_eq!(
            commands,
            vec![Command::new(
                TAB,
                marked("ni", Some((2, 2)), vec![raw((0, 2))])
            )]
        );
    }

    #[test]
    fn a_selected_range_without_a_record_makes_the_single_clause_selected() {
        let mut machine = Machine::default();

        let commands = machine.input(TAB, preedit("ab", Some((0, 2))));

        assert_eq!(
            commands,
            vec![Command::new(
                TAB,
                marked(
                    "ab",
                    Some((0, 2)),
                    vec![ImeClause {
                        range: Utf16Range::new(0, 2),
                        kind: ImeClauseKind::SelectedRaw,
                    }],
                ),
            )]
        );
    }

    #[test]
    fn escape_cancels_when_the_empty_preedit_stays_alone_until_the_flush() {
        let mut machine = Machine::default();
        machine.input(TAB, preedit("ni", Some((2, 2))));

        assert!(machine.input(TAB, preedit("", None)).is_empty());
        assert!(machine.holds_empty());
        assert!(machine.is_active());
        let cancel = machine.flush();

        assert_eq!(
            cancel,
            vec![Command::new(TAB, ImeInput::Commit(String::new()))]
        );
        assert!(!machine.is_active());
        assert!(!machine.holds_empty());
        assert!(machine.flush().is_empty());
    }

    #[test]
    fn a_new_preedit_after_the_empty_one_continues_the_composition() {
        let mut machine = Machine::default();
        machine.input(TAB, preedit("a", Some((1, 1))));
        machine.input(TAB, preedit("", None));

        let commands = machine.input(OTHER_TAB, preedit("b", Some((1, 1))));

        // Still the tab that started it; nothing was cancelled.
        assert_eq!(
            commands,
            vec![Command::new(
                TAB,
                marked("b", Some((1, 1)), vec![raw((0, 1))])
            )]
        );
        assert!(!machine.holds_empty());
        assert!(machine.flush().is_empty());
    }

    #[test]
    fn a_commit_without_a_composition_is_a_commit_to_the_active_tab() {
        let mut machine = Machine::default();

        let commands = machine.input(OTHER_TAB, commit("😀"));

        assert_eq!(
            commands,
            vec![Command::new(OTHER_TAB, ImeInput::Commit("😀".to_owned()))]
        );
        assert!(!machine.is_active());
        // An empty commit with nothing to end says nothing.
        assert!(machine.input(TAB, commit("")).is_empty());
    }

    #[test]
    fn an_empty_preedit_with_no_composition_holds_nothing() {
        let mut machine = Machine::default();

        assert!(machine.input(TAB, preedit("", None)).is_empty());

        assert!(!machine.holds_empty());
        assert!(machine.flush().is_empty());
    }

    #[test]
    fn disabling_the_input_method_dismisses_the_tab_that_composes() {
        let mut machine = Machine::default();
        machine.input(TAB, preedit("her", None));

        // Another tab is active by now; the dismissal still goes to the composing one.
        let commands = machine.input(OTHER_TAB, Input::Disabled);

        assert_eq!(commands, vec![Command::new(TAB, ImeInput::Dismissed)]);
        assert!(!machine.is_active());
        assert!(machine.input(TAB, Input::Disabled).is_empty());
    }

    #[test]
    fn focus_lost_and_a_sheet_dismiss_and_forget_a_held_preedit() {
        let mut machine = Machine::default();
        machine.input(TAB, preedit("ni", Some((2, 2))));
        machine.input(TAB, preedit("", None));

        let commands = machine.dismiss();

        assert_eq!(commands, vec![Command::new(TAB, ImeInput::Dismissed)]);
        assert!(!machine.is_active());
        assert!(!machine.holds_empty());
        assert!(machine.flush().is_empty());
        assert!(machine.dismiss().is_empty());
    }

    #[test]
    fn a_tab_switch_dismisses_the_composition_of_the_tab_that_was_left() {
        let mut machine = Machine::default();
        machine.input(TAB, preedit("ni", Some((2, 2))));

        // Still in the same tab: nothing happens.
        assert!(machine.active_tab_changed(TAB).is_empty());
        assert!(machine.is_active());
        let commands = machine.active_tab_changed(OTHER_TAB);

        assert_eq!(commands, vec![Command::new(TAB, ImeInput::Dismissed)]);
        assert!(!machine.is_active());
        // What follows belongs to the new tab.
        let next = machine.input(OTHER_TAB, commit("你"));
        assert_eq!(
            next,
            vec![Command::new(OTHER_TAB, ImeInput::Commit("你".to_owned()))]
        );
    }

    #[test]
    fn a_stale_native_record_is_dropped_when_the_input_method_is_enabled_again() {
        let mut machine = Machine::default();
        machine.native(
            TAB,
            NativeIme::Marked {
                clauses: vec![ImeClause {
                    range: Utf16Range::new(0, 2),
                    kind: ImeClauseKind::SelectedConverted,
                }],
                replacement: None,
            },
        );

        machine.input(TAB, Input::Enabled);
        let commands = machine.input(TAB, preedit("ab", Some((2, 2))));

        assert_eq!(
            commands,
            vec![Command::new(
                TAB,
                marked("ab", Some((2, 2)), vec![raw((0, 2))])
            )]
        );
    }

    #[test]
    fn a_native_record_does_not_survive_a_dismissal() {
        let mut machine = Machine::default();
        machine.native(
            TAB,
            NativeIme::Marked {
                clauses: vec![ImeClause {
                    range: Utf16Range::new(0, 2),
                    kind: ImeClauseKind::SelectedConverted,
                }],
                replacement: None,
            },
        );
        machine.dismiss();

        let commands = machine.input(TAB, preedit("ab", Some((2, 2))));

        assert_eq!(
            commands,
            vec![Command::new(
                TAB,
                marked("ab", Some((2, 2)), vec![raw((0, 2))])
            )]
        );
    }

    #[test]
    fn the_cursor_is_converted_from_utf8_bytes_to_utf16_units() {
        let mut machine = Machine::default();

        // "a😀b": a = byte 0, the emoji = bytes 1..5 (units 1..3), b = byte 5 (unit 3).
        let after_emoji = machine.input(TAB, preedit("a😀b", Some((5, 5))));
        let ImeInput::SetMarkedText { selected, .. } = &after_emoji[0].input else {
            panic!("marked text expected");
        };
        assert_eq!(*selected, Some(Utf16Range::collapsed(3)));

        // A cursor in the middle of the emoji is at its start, not inside the pair.
        let inside = machine.input(TAB, preedit("a😀b", Some((3, 3))));
        let ImeInput::SetMarkedText { selected, .. } = &inside[0].input else {
            panic!("marked text expected");
        };
        assert_eq!(*selected, Some(Utf16Range::collapsed(1)));

        // Past the end: the end. Reversed: put in order.
        let past = machine.input(TAB, preedit("a😀b", Some((99, 99))));
        let ImeInput::SetMarkedText { selected, .. } = &past[0].input else {
            panic!("marked text expected");
        };
        assert_eq!(*selected, Some(Utf16Range::collapsed(4)));
        let reversed = machine.input(TAB, preedit("a😀b", Some((5, 1))));
        let ImeInput::SetMarkedText { selected, .. } = &reversed[0].input else {
            panic!("marked text expected");
        };
        assert_eq!(*selected, Some(Utf16Range::new(1, 3)));

        // No cursor at all stays none.
        let none = machine.input(TAB, preedit("ab", None));
        let ImeInput::SetMarkedText { selected, .. } = &none[0].input else {
            panic!("marked text expected");
        };
        assert_eq!(*selected, None);
    }

    #[test]
    fn text_without_a_composition_goes_to_the_active_tab_as_an_insertion() {
        let mut machine = Machine::default();
        let replacement = Some(ImeReplacement {
            range: Utf16Range::new(3, 4),
            sequence: 8,
        });

        let emoji = machine.native(
            OTHER_TAB,
            NativeIme::InsertText {
                text: "😀".to_owned(),
                replacement: None,
            },
        );
        let accent = machine.native(
            OTHER_TAB,
            NativeIme::InsertText {
                text: "é".to_owned(),
                replacement,
            },
        );

        assert_eq!(
            emoji,
            vec![Command::new(
                OTHER_TAB,
                ImeInput::InsertText {
                    text: "😀".to_owned(),
                    replacement: None,
                },
            )]
        );
        assert_eq!(
            accent,
            vec![Command::new(
                OTHER_TAB,
                ImeInput::InsertText {
                    text: "é".to_owned(),
                    replacement,
                },
            )]
        );
        assert!(!machine.is_active());
    }

    #[test]
    fn an_insertion_first_resolves_a_cancel_that_is_still_waiting() {
        let mut machine = Machine::default();
        machine.input(TAB, preedit("ni", Some((2, 2))));
        machine.input(TAB, preedit("", None));

        let commands = machine.native(
            TAB,
            NativeIme::InsertText {
                text: "😀".to_owned(),
                replacement: None,
            },
        );

        assert_eq!(
            inputs(&commands),
            vec![
                &ImeInput::Commit(String::new()),
                &ImeInput::InsertText {
                    text: "😀".to_owned(),
                    replacement: None,
                },
            ]
        );
        assert!(!machine.is_active());
        assert!(!machine.holds_empty());
    }

    #[test]
    fn an_insertion_while_a_composition_is_open_ends_that_composition_first() {
        let mut machine = Machine::default();
        machine.input(TAB, preedit("ni", Some((2, 2))));

        let commands = machine.native(
            OTHER_TAB,
            NativeIme::InsertText {
                text: "x".to_owned(),
                replacement: None,
            },
        );

        assert_eq!(commands[0], Command::new(TAB, ImeInput::Dismissed));
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[1].tab, OTHER_TAB);
        assert!(!machine.is_active());
    }

    #[test]
    fn an_empty_insertion_says_nothing() {
        let mut machine = Machine::default();

        let commands = machine.native(
            TAB,
            NativeIme::InsertText {
                text: String::new(),
                replacement: None,
            },
        );

        assert!(commands.is_empty());
    }

    #[test]
    fn composition_active_follows_every_transition_and_is_false_after_every_ending() {
        let _guard = test_guard();
        let controller = RefCell::new(BrowserController::default());
        let tab = controller.borrow().active_tab_id();
        assert!(!composition_active());
        assert_eq!(composition_tab(), None);

        // Marked, then committed.
        handle(&controller, preedit("n", Some((1, 1))));
        assert!(composition_active());
        assert_eq!(composition_tab(), Some(tab));
        assert!(composing_in(tab));
        handle(&controller, preedit("", None));
        assert!(
            composition_active(),
            "a held preedit is still a composition"
        );
        handle(&controller, commit("你"));
        assert!(!composition_active());
        assert_eq!(composition_tab(), None);

        // Marked, then cancelled by the flush.
        handle(&controller, preedit("n", Some((1, 1))));
        handle(&controller, preedit("", None));
        assert!(composition_active());
        flush(&controller);
        assert!(!composition_active());

        // Marked, then disabled.
        handle(&controller, preedit("n", Some((1, 1))));
        assert!(composition_active());
        handle(&controller, Input::Disabled);
        assert!(!composition_active());

        // Marked, then dismissed (focus lost, a sheet).
        handle(&controller, preedit("n", Some((1, 1))));
        assert!(composition_active());
        dismiss_composition(&controller);
        assert!(!composition_active());
        assert!(!composing_in(tab));
    }

    #[test]
    fn handle_queues_the_commands_for_the_tab_that_started_the_composition() {
        let _guard = test_guard();
        let controller = RefCell::new(BrowserController::default());
        let composing_tab = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, composing_tab);
        handle(&controller, preedit("her", None));
        controller.borrow_mut().open_tab();

        handle(&controller, Input::Disabled);

        let commands = queued(&controller);
        assert!(commands.iter().all(|(tab, _)| *tab == composing_tab));
        assert_eq!(
            commands.last().map(|(_, input)| input),
            Some(&ImeInput::Dismissed)
        );
        assert!(!composition_active());
    }

    #[test]
    fn commands_wait_in_the_backlog_while_the_controller_is_borrowed() {
        let _guard = test_guard();
        let controller = RefCell::new(BrowserController::default());
        let tab = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, tab);

        {
            let _busy = controller.borrow_mut();
            // Reaches the machine, but cannot be queued.
            let commands = with_machine(|machine| machine.input(tab, preedit("n", Some((1, 1)))));
            send(&controller, commands);
        }
        assert!(queued(&controller).is_empty());

        assert!(drain_backlog(&controller));
        assert_eq!(queued(&controller).len(), 1);
        // Nothing is sent twice.
        assert!(drain_backlog(&controller));
        assert!(queued(&controller).is_empty());
    }

    #[test]
    fn native_events_are_ignored_without_a_link_and_logged_for_tests() {
        let _guard = test_guard();
        let event = NativeIme::InsertText {
            text: "😀".to_owned(),
            replacement: None,
        };

        native_event(event.clone());

        assert_eq!(take_native_events(), vec![event]);
        assert!(!composition_active());
        assert_eq!(snapshot(), None);
    }

    #[test]
    fn a_native_insertion_reaches_the_page_only_while_it_accepts_text() {
        let _guard = test_guard();
        let controller = Rc::new(RefCell::new(BrowserController::default()));
        let tab = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, tab);
        let accepts = Rc::new(Cell::new(false));
        attach(&controller, || {}, {
            let accepts = Rc::clone(&accepts);
            move || accepts.get()
        });
        let insertion = || NativeIme::InsertText {
            text: "😀".to_owned(),
            replacement: None,
        };

        // The address bar has the text focus: the emoji is not for the page.
        native_event(insertion());
        assert!(queued(&controller).is_empty());

        accepts.set(true);
        native_event(insertion());
        assert_eq!(
            queued(&controller),
            vec![(
                tab,
                ImeInput::InsertText {
                    text: "😀".to_owned(),
                    replacement: None,
                }
            )]
        );
    }

    #[test]
    fn the_snapshot_is_the_active_tabs_text_state_while_the_page_accepts_text() {
        let _guard = test_guard();
        let controller = Rc::new(RefCell::new(BrowserController::default()));
        let tab = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, tab);
        let state = TextInputState {
            sequence: 3,
            text: "hello".to_owned(),
            text_start: 0,
            selection: Utf16Range::collapsed(5),
            marked: None,
            caret_rect: None,
            multiline: false,
            is_password: false,
        };
        controller.borrow_mut().handle_event(
            tab,
            talaria_core::BrowserEvent::TextInputState(Some(state.clone())),
        );
        let accepts = Rc::new(Cell::new(false));
        attach(&controller, || {}, {
            let accepts = Rc::clone(&accepts);
            move || accepts.get()
        });

        assert_eq!(snapshot(), None);
        accepts.set(true);
        assert_eq!(snapshot(), Some(state));
        assert_eq!(with_snapshot(|state| state.sequence), Some(3));
        // While the controller is borrowed elsewhere there is no answer, not a panic.
        let busy = controller.borrow_mut();
        assert_eq!(snapshot(), None);
        drop(busy);
    }

    #[test]
    fn a_tab_switch_in_the_ui_ends_the_composition_when_the_timer_runs() {
        let _guard = test_guard();
        let controller = Rc::new(RefCell::new(BrowserController::default()));
        let composing_tab = controller.borrow().active_tab_id();
        mark_engine_ready(&controller, composing_tab);
        attach(&controller, || {}, || true);
        handle(&controller, preedit("ni", Some((2, 2))));
        let other = controller.borrow_mut().open_tab();
        controller.borrow_mut().select_tab(other);

        // What the timer runs.
        let commands = with_machine(|machine| machine.active_tab_changed(other));
        send(&controller, commands);

        assert!(!composition_active());
        let sent = queued(&controller);
        assert_eq!(
            sent.last().map(|(tab, input)| (*tab, input.clone())),
            Some((composing_tab, ImeInput::Dismissed))
        );
    }

    fn mark_engine_ready(controller: &RefCell<BrowserController>, tab: TabId) {
        let handled = controller.borrow_mut().handle_event(
            tab,
            talaria_core::BrowserEvent::AddressChanged { url: String::new() },
        );
        assert!(handled);
    }

    /// Everything queued so far, as (tab, input) pairs.
    fn queued(controller: &RefCell<BrowserController>) -> Vec<(TabId, ImeInput)> {
        std::iter::from_fn(|| controller.borrow_mut().next_command())
            .filter_map(|command| match command.command() {
                BrowserCommand::ImeInput(input) => Some((command.tab_id(), input.clone())),
                _other => None,
            })
            .collect()
    }
}
