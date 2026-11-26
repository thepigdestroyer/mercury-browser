// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Orbit overview's glue (SPEC §4.6, §6.1; orbit-view brief §6 item 5): opening takes a fresh
//! picture of the active tab, keeps the pictures as `slint::Image`s while Orbit is open, runs the
//! machine (`mercury_ui::orbit_view`), turns its effects into controller group commands and
//! closes. The pure parts (state machine, layout, drag resolver) are in `mercury-ui`.
//!
//! # What `layout-switch` and `input-hooks` call
//!
//! - [`OrbitHost::for_desktop`] makes the host from the browser controller (rows, group commands,
//!   pictures); `layout-switch` passes the closures that need the window (activate a tab, hide the
//!   overlay and restore the page, sync the tab strip after a group command, the group colours).
//! - [`OrbitWindow`] is implemented by `layout-switch` on `MainWindow` (the one `OrbitView`
//!   instance): its setters and one `connect` that routes every callback of the view to the
//!   [`OrbitEvents`] it is given. `OrbitPreview` has the same surface (the tests below implement it
//!   on that window).
//! - [`OrbitController::open`] (⌘E, "See everything"), [`OrbitController::close`],
//!   [`OrbitController::handle_key`] (`input-hooks`: Esc, ⌘↑ and the rest while Orbit is up),
//!   [`OrbitController::rows_changed`] (the sync tick: tabs or groups changed) and
//!   [`OrbitController::refresh`] (the window was resized).
//!
//! # What `input-hooks` must know about text fields
//!
//! `handle_key` asks the window ([`OrbitWindow::text_field_focused`]) whether the search field or
//! the new-group popover's name field has the keyboard focus. While one has, it answers `false`
//! (the key is not taken, so it reaches the field) for `N`, `/`, typed characters, the arrows and
//! Tab, and for ⏎ while the popover is open (the popover confirms itself). Escape and the
//! ⌘-keys still act. A hook that runs before Slint sees the key can therefore call `handle_key`
//! for every key while Orbit is open without eating what is typed.
//!
//! # How a frame is made
//!
//! The machine ([`OrbitMachine`]) holds the level, the focus and the drag. After every event the
//! glue compares its [`View`] with the one last drawn and, if it differs (or a group command or a
//! notice made the picture stale), builds a frame with [`OrbitData::build`], fills the persistent
//! models and hands the window a [`Frame`] of single values (made by [`OrbitModels::frame`]). A
//! pointer move during a drag only moves the ghost. After the first build of an open the window is
//! told to play the 320 ms open animation ([`OrbitWindow::play_open`]).
//!
//! # Pictures (brief D4)
//!
//! `open` asks the host to capture the active tab (`CapturePicture`), builds at once and asks for
//! the active tab's picture. A capture lands on the main thread some milliseconds later with no
//! signal that Orbit can hear (`tab_snapshots.rs` is not edited), so one timer asks for the active
//! tab's picture a second time after [`REFRESH_DELAY`]; the old picture stays on screen until the
//! new one is there. If that ask brought no new picture (none, or the same pixels) one more ask
//! follows [`LATE_REFRESH_DELAY`] later (about 750 ms after opening); never more than two asks
//! per open. Every other tab's picture is asked for once, the first time it is shown.
//! Everything is dropped on close.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use mercury_ui::orbit_view::data::{
    Frame, Options, OrbitData, OrbitModels, popover_colour_name, relative_age,
};
use mercury_ui::orbit_view::drag::{
    self, Decision, DropTarget, GroupOp, OrbitPayload, Source, drop_target_of, lit_target,
    source_of, target_from_id,
};
use mercury_ui::orbit_view::keys::key_from_text;
use mercury_ui::orbit_view::{
    Effect, Key, KeyContext, Mods, OrbitInput, OrbitMachine, Stage, Target, View,
};
use mercury_ui::ring_bands::{RingBandsData, RingColours};
use mercury_ui::{GroupRow, OrbitDragView, RingDropTarget, RingHit, RingHitKind, TabRow};
use slint::{Color, DataTransfer, Image, Timer, TimerMode};
use talaria_core::{
    BrowserController, GroupColour, GroupError, GroupId, GroupPreset, MAX_GROUPS, TabId,
};

use crate::rendering::BrowserRenderingContext;
use crate::tab_strip::{id_for_ui, id_from_ui};

/// How long after opening the active tab's picture is asked for the second time: a capture is a
/// pixel read (8 to 22 ms) and a shrink on the worker thread, and its result is posted to the main
/// thread.
pub(crate) const REFRESH_DELAY: Duration = Duration::from_millis(250);

/// How long after the second ask the third and last one comes, if the second brought no new
/// picture (a capture that is late under load or while the window is covered): 750 ms after
/// opening.
pub(crate) const LATE_REFRESH_DELAY: Duration = Duration::from_millis(500);

/// The nonce of the next Orbit that opens, unique in the process (so a payload of another window's
/// Orbit is as foreign as one of an older open).
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

// ---------------------------------------------------------------------------------------------
// The host and the window: what the glue needs from outside
// ---------------------------------------------------------------------------------------------

/// What the window shows besides the rows.
#[allow(dead_code)] // used by layout-switch
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Environment {
    /// The dark scheme is in use (the label colours of the rows are already resolved).
    pub dark: bool,
    /// Animations off (SPEC §7): every duration of the view is 0 ms.
    pub reduce_motion: bool,
}

/// A group command on the controller; an `Err` is the controller's refusal.
pub(crate) type GroupOpFn = dyn Fn(&GroupOp) -> Result<(), GroupError>;

/// The rendering context of the window, once there is one.
pub(crate) type ContextFn = dyn Fn() -> Option<Rc<BrowserRenderingContext>>;

/// The closures the glue calls. [`OrbitHost::for_desktop`] makes the real one; the tests make one
/// from fakes.
pub(crate) struct OrbitHost {
    /// Take a fresh picture of the active tab now (`tab_snapshots::capture_active_now`).
    pub capture_active: Box<dyn Fn()>,
    /// The picture of a tab (`TabRow.id`), if there is one (`tab_snapshots::image_for`). It copies
    /// pixels: the glue calls it at most once per tab per open, and twice for the active tab.
    pub snapshot: Box<dyn Fn(i32) -> Option<Image>>,
    /// The rows now.
    pub rows: Box<dyn Fn() -> OrbitInput>,
    /// Runs a group command on the controller. An `Err` becomes a notice, never a panic.
    pub group_op: Box<GroupOpFn>,
    /// Makes this tab the active tab (called after `close`).
    pub activate: Box<dyn Fn(i32)>,
    /// Leaves Orbit: hides the overlay and gives the page its focus back. The glue has already
    /// torn its own state down.
    pub close: Box<dyn Fn()>,
    /// The scheme and the motion setting, asked on every build.
    pub environment: Box<dyn Fn() -> Environment>,
}

/// The one `OrbitView` of the application, seen from the glue. `layout-switch` implements it on
/// `MainWindow` (each method is a setter, a getter or a callback registration of the view); the
/// tests implement it on `OrbitPreview` and on a fake. All methods take `&self`.
pub(crate) trait OrbitWindow {
    /// The size of the overlay in logical pixels (the whole window).
    fn stage(&self) -> Stage;
    /// The glyph colours of the ring labels, read from the window's live `ChromePalette`.
    fn ring_colours(&self) -> RingColours;
    /// Gives the view the persistent models (once, from `OrbitController::new`).
    fn set_models(&self, models: &OrbitModels);
    /// Sets every single value of the view (including the search field's placeholder).
    fn set_frame(&self, frame: &Frame);
    /// Sets the search field's text.
    fn set_search_text(&self, text: &str);
    /// Sets the pinch progress (-1 to 1); the view animates the change (160 ms).
    fn set_pinch_progress(&self, progress: f32);
    fn focus_search(&self);
    /// Gives the overlay's key scope the keyboard focus (when Orbit opens).
    fn focus_view(&self);
    /// Whether the search field or the new-group popover's name field has the keyboard focus (the
    /// view's `text-field-focused`): typed keys are text then, not commands.
    fn text_field_focused(&self) -> bool;
    /// Plays the 320 ms open animation (the view's `play-open()`); called once per open, after
    /// the first build.
    fn play_open(&self);
    /// Cancels the drag `RingBands` has in flight, if any (the view's `cancel-ring-drag()`).
    /// Called after the machine's own drag is gone, on Esc, on a cancelled drag and when Orbit
    /// closes; the ring may answer with `drag-cancelled`, which the glue ignores once the
    /// machine has no drag.
    fn cancel_ring_drag(&self);
    /// Routes every callback of the view to `events` (once, from `OrbitController::new`). Every
    /// closure handed to Slint starts with `InSlintCallback::enter()`.
    fn connect(&self, events: OrbitEvents);
}

// ---------------------------------------------------------------------------------------------
// The controller
// ---------------------------------------------------------------------------------------------

/// Orbit's glue: owns the machine, the pictures and the models. Cheap to clone out of
/// [`OrbitController::events`] for the window's callbacks.
pub(crate) struct OrbitController {
    inner: Rc<Inner>,
}

#[allow(dead_code)] // used by layout-switch and input-hooks
impl OrbitController {
    /// Connects to the window: gives it the models and routes its callbacks here. Orbit starts
    /// closed.
    pub(crate) fn new(host: OrbitHost, window: Rc<dyn OrbitWindow>) -> Self {
        let inner = Rc::new_cyclic(|me| Inner::new(me.clone(), host, Rc::clone(&window)));
        window.set_models(&inner.models);
        window.connect(OrbitEvents {
            inner: Rc::downgrade(&inner),
        });
        Self { inner }
    }

    /// What the window's callbacks call.
    pub(crate) fn events(&self) -> OrbitEvents {
        OrbitEvents {
            inner: Rc::downgrade(&self.inner),
        }
    }

    /// Opens at level 1: asks for a fresh picture of the active tab, then builds. A second call
    /// while open does nothing.
    pub(crate) fn open(&self) {
        self.inner.open();
    }

    /// Leaves Orbit (the same as the "Back to the page" button): drops every picture and calls the
    /// host's `close`. Does nothing when closed.
    pub(crate) fn close(&self) {
        self.inner.close_with(None);
    }

    pub(crate) fn is_open(&self) -> bool {
        self.inner.open.get()
    }

    /// A key while Orbit is up (`input-hooks`). `false` when Orbit is closed, so the key goes on
    /// to the page; `true` when open and the key is one of Orbit's (the overlay is modal: a key
    /// that changes nothing is still taken, it must not reach the page). `false` too for a key
    /// that is text while the search field or the popover's name field has the focus (`N`, `/`,
    /// characters, arrows, Tab, and ⏎ with the popover open): it must reach the field. Escape and
    /// the ⌘-keys act whatever has the focus.
    pub(crate) fn handle_key(&self, key: Key, mods: Mods) -> bool {
        self.inner.handle_key(key, mods)
    }

    /// Sets the search text, as if it had been typed.
    pub(crate) fn set_search(&self, text: &str) {
        self.inner.set_search(text);
    }

    /// A click on `target`, for callers that hit-test themselves.
    pub(crate) fn click(&self, target: Target) {
        self.inner.click(target);
    }

    /// The tabs or groups changed while Orbit is open: reads the rows and reconciles.
    pub(crate) fn rows_changed(&self) {
        self.inner.rows_changed();
    }

    /// The window changed size: lays everything out again.
    pub(crate) fn refresh(&self) {
        self.inner.refresh();
    }

    /// The nonce of the open Orbit (its drags carry it).
    pub(crate) fn session(&self) -> u64 {
        self.inner.machine.borrow().session()
    }
}

/// What the callbacks of the `OrbitView` call. Holds the controller weakly (the window owns the
/// closures that own this, the controller owns the window). Every method enters the Slint callback
/// scope itself, so a window implementation cannot forget it.
#[derive(Clone)]
pub(crate) struct OrbitEvents {
    inner: Weak<Inner>,
}

#[allow(dead_code)] // used by layout-switch (the view's callbacks)
impl OrbitEvents {
    fn with<R>(&self, otherwise: R, call: impl FnOnce(&Inner) -> R) -> R {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        match self.inner.upgrade() {
            Some(inner) => call(&inner),
            None => otherwise,
        }
    }

    /// `-1`: the pointer left every planet.
    pub(crate) fn planet_hovered(&self, group: i32) {
        self.with((), |inner| {
            inner.hover(target_from_id(group, Target::Planet));
        });
    }

    pub(crate) fn planet_clicked(&self, group: i32) {
        self.with((), |inner| inner.click(Target::Planet(group)));
    }

    /// `-1`: the pointer left every preview.
    pub(crate) fn preview_hovered(&self, tab: i32) {
        self.with((), |inner| {
            inner.hover(target_from_id(tab, Target::Preview));
        });
    }

    pub(crate) fn preview_clicked(&self, tab: i32) {
        self.with((), |inner| inner.click(Target::Preview(tab)));
    }

    pub(crate) fn chip_clicked(&self, group: i32) {
        self.with((), |inner| inner.click(Target::Chip(group)));
    }

    pub(crate) fn card_clicked(&self) {
        self.with((), |inner| inner.click(Target::Card));
    }

    pub(crate) fn neighbour_clicked(&self, tab: i32) {
        self.with((), |inner| inner.click(Target::Neighbour(tab)));
    }

    pub(crate) fn backdrop_clicked(&self) {
        self.with((), |inner| inner.click(Target::Backdrop));
    }

    /// The dashed "New group" ring was clicked: the same as `N`.
    pub(crate) fn new_group_clicked(&self) {
        // A click is not typing: it works whatever has the keyboard focus.
        self.with((), |inner| {
            inner.run_key(Key::N, Mods::NONE, false);
        });
    }

    /// "Back to the page".
    pub(crate) fn back_clicked(&self) {
        self.with((), |inner| inner.close_with(None));
    }

    pub(crate) fn search_edited(&self, text: &str) {
        self.with((), |inner| inner.search_edited(text));
    }

    /// A key the view did not use itself: the text of the key and the modifiers (Command,
    /// Shift, Alt, Control). `true`: handled.
    pub(crate) fn key_pressed(
        &self,
        text: &str,
        command: bool,
        shift: bool,
        alt: bool,
        control: bool,
    ) -> bool {
        self.with(false, |inner| {
            let Some(key) = key_from_text(text) else {
                return false;
            };
            let mods = Mods {
                command,
                shift,
                alt,
                control,
            };
            inner.handle_key(key, mods)
        })
    }

    pub(crate) fn pinch_started(&self, x: f32, y: f32) {
        self.with((), |inner| inner.pinch_started(x, y));
    }

    pub(crate) fn pinch_updated(&self, scale: f32) {
        self.with((), |inner| inner.pinch_updated(scale));
    }

    pub(crate) fn pinch_ended(&self) {
        self.with((), Inner::pinch_ended);
    }

    pub(crate) fn pinch_cancelled(&self) {
        self.with((), Inner::pinch_cancelled);
    }

    /// A tab was lifted out of a ring (`RingBands`' own drag).
    pub(crate) fn ring_drag_started(&self, _group: i32, tab: i32) {
        self.with((), |inner| inner.drag_started(Source::Tab(tab)));
    }

    /// The pointer of a ring drag moved (stage coordinates, also outside the ring).
    pub(crate) fn ring_drag_moved(&self, x: f32, y: f32) {
        self.with((), |inner| inner.ring_drag_moved(x, y));
    }

    /// A ring drag was released over a ring: `slot` is the insertion slot.
    pub(crate) fn ring_dropped(&self, group: i32, slot: i32) {
        self.with((), |inner| inner.ring_dropped(group, slot));
    }

    /// A ring drag ended anywhere else: the last pointer position says where.
    pub(crate) fn ring_drag_cancelled(&self) {
        self.with((), Inner::ring_drag_cancelled);
    }

    pub(crate) fn popover_name_edited(&self, name: &str) {
        self.with((), |inner| inner.popover_name_edited(name));
    }

    pub(crate) fn popover_colour_chosen(&self, index: i32) {
        self.with((), |inner| inner.popover_colour_chosen(index));
    }

    pub(crate) fn popover_confirmed(&self) {
        self.with((), Inner::popover_confirmed);
    }

    pub(crate) fn popover_cancelled(&self) {
        self.with((), |inner| {
            inner.handle_key(Key::Escape, Mods::NONE);
        });
    }

    /// What a ring-local point of the ring of `group` is over (forwarded to `RingBandsData`).
    pub(crate) fn ring_hit_test(&self, group: i32, x: f32, y: f32, hovered: i32) -> RingHit {
        let outside = RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        };
        self.with(outside.clone(), |inner| {
            inner
                .with_ring(group, |ring| ring.hit(x, y, hovered))
                .unwrap_or(outside)
        })
    }

    /// Where a tab dragged out of a ring of `group` would land (forwarded to `RingBandsData`).
    pub(crate) fn ring_drop_target(
        &self,
        group: i32,
        x: f32,
        y: f32,
        dragging: i32,
    ) -> RingDropTarget {
        self.with(RingDropTarget::default(), |inner| {
            inner
                .with_ring(group, |ring| ring.drop_target(x, y, dragging))
                .unwrap_or_default()
        })
    }

    /// What a `DragArea` carries: `kind` is `"tab"` or `"group"`. An empty `DataTransfer` makes
    /// the item undraggable (a pinned tab, the loose planet, an id that is gone).
    pub(crate) fn payload_for(&self, kind: &str, id: i32) -> DataTransfer {
        self.with(DataTransfer::default(), |inner| inner.payload_for(kind, id))
    }

    /// Whether a `DropArea` should accept `data`: `kind` is `"group"` (a planet, a chip, the
    /// centre ring), `"new-group"` or `"empty"`; `slot` is the insertion slot, -1 for the end.
    pub(crate) fn payload_accepts(
        &self,
        data: &DataTransfer,
        kind: &str,
        id: i32,
        slot: i32,
    ) -> bool {
        self.with(false, |inner| inner.payload_accepts(data, kind, id, slot))
    }

    /// A `DropArea` received `data`. `true`: the drop was taken.
    pub(crate) fn payload_dropped(
        &self,
        data: &DataTransfer,
        kind: &str,
        id: i32,
        slot: i32,
    ) -> bool {
        self.with(false, |inner| inner.payload_dropped(data, kind, id, slot))
    }

    /// A `DragArea` drag is over a drop target; (`x`, `y`) is the pointer in stage coordinates.
    pub(crate) fn drag_over(&self, kind: &str, id: i32, x: f32, y: f32) {
        self.with((), |inner| inner.drag_over(kind, id, x, y));
    }

    /// A `DragArea` drag began: `kind` is `"tab"` (a preview) or `"group"` (a planet).
    pub(crate) fn drag_source_started(&self, kind: &str, id: i32) {
        self.with((), |inner| {
            if let Some(source) = source_of(kind, id) {
                inner.drag_started(source);
            }
        });
    }

    /// A `DragArea` drag ended; `accepted` is whether a drop took it.
    pub(crate) fn drag_source_finished(&self, accepted: bool) {
        self.with((), |inner| inner.drag_source_finished(accepted));
    }
}

/// Whether `key` is text while a text field has the focus (so it must reach the field).
fn is_text_key(key: Key, mods: Mods) -> bool {
    !mods.command
        && matches!(
            key,
            Key::N
                | Key::Slash
                | Key::Char(_)
                | Key::Up
                | Key::Down
                | Key::Left
                | Key::Right
                | Key::Tab
        )
}

// ---------------------------------------------------------------------------------------------
// The state
// ---------------------------------------------------------------------------------------------

struct Inner {
    me: Weak<Inner>,
    host: OrbitHost,
    window: Rc<dyn OrbitWindow>,
    models: OrbitModels,
    machine: RefCell<OrbitMachine>,
    input: RefCell<Rc<OrbitInput>>,
    /// The last frame's plain data: its layout answers hit tests and its rings the ring tests.
    data: RefCell<Option<Rc<OrbitData>>>,
    frame: RefCell<Frame>,
    /// The machine's view when the last frame was built.
    last_view: RefCell<View>,
    /// The pictures of this open: one entry per tab that was asked for (`None`: it has none).
    images: RefCell<HashMap<i32, Option<Image>>>,
    search_text: RefCell<String>,
    notice: RefCell<String>,
    /// The pointer of the drag in flight (stage coordinates), for the ghost.
    pointer: Cell<Option<(f32, f32)>>,
    open: Cell<bool>,
    /// The frame is out of date for a reason the machine's view does not show.
    stale: Cell<bool>,
    /// How many times the active tab's picture was asked for again in this open (0 to 2).
    refresh_asks: Cell<u8>,
    building: Cell<bool>,
    refresh_timer: Timer,
    late_refresh_timer: Timer,
}

impl Inner {
    fn new(me: Weak<Inner>, host: OrbitHost, window: Rc<dyn OrbitWindow>) -> Self {
        Self {
            me,
            host,
            window,
            models: OrbitModels::new(),
            machine: RefCell::new(OrbitMachine::default()),
            input: RefCell::new(Rc::new(OrbitInput::default())),
            data: RefCell::new(None),
            frame: RefCell::new(Frame::default()),
            last_view: RefCell::new(View::default()),
            images: RefCell::new(HashMap::new()),
            search_text: RefCell::new(String::new()),
            notice: RefCell::new(String::new()),
            pointer: Cell::new(None),
            open: Cell::new(false),
            stale: Cell::new(false),
            refresh_asks: Cell::new(0),
            building: Cell::new(false),
            refresh_timer: Timer::default(),
            late_refresh_timer: Timer::default(),
        }
    }

    // -- Opening and closing ----------------------------------------------------------------

    fn open(&self) {
        if self.open.replace(true) {
            return;
        }
        self.forget_everything();
        // A pinch that closed the last Orbit may have left the view's progress at -1 or 1.
        self.window.set_pinch_progress(0.0);
        let input = Rc::new((self.host.rows)());
        *self.input.borrow_mut() = Rc::clone(&input);
        let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        *self.machine.borrow_mut() = OrbitMachine::new(session);
        let effects = self.machine.borrow_mut().open(&input);
        // `CapturePicture` comes first: the capture is asked for before the first build.
        self.apply(effects);
        // The active tab's picture is the one that gets refreshed once the capture has landed.
        self.picture(input.active_tab);
        self.rebuild();
        self.window.focus_view();
        self.window.play_open();
        self.start_refresh_timer();
    }

    /// Leaves Orbit; with `Some(tab)` activates that tab afterwards.
    fn close_with(&self, activate: Option<i32>) {
        if !self.open.replace(false) {
            return;
        }
        self.refresh_timer.stop();
        self.late_refresh_timer.stop();
        *self.machine.borrow_mut() = OrbitMachine::default();
        self.window.cancel_ring_drag();
        // Whatever ended Orbit (a pinch out of level 1, a pinch in on the card, Esc), the view's
        // pinch scale goes back to 1.
        self.window.set_pinch_progress(0.0);
        self.forget_everything();
        // Empty models and a blank frame, so that no picture stays alive in the view.
        let empty = OrbitData::build(
            self.window.stage(),
            &OrbitInput::default(),
            &View::default(),
            &|_| None,
            &Options::default(),
        );
        self.models.update(&empty);
        self.window.set_frame(&Frame::default());
        self.window.set_search_text("");
        (self.host.close)();
        if let Some(tab) = activate {
            (self.host.activate)(tab);
        }
    }

    fn forget_everything(&self) {
        self.images.borrow_mut().clear();
        self.search_text.borrow_mut().clear();
        self.notice.borrow_mut().clear();
        self.pointer.set(None);
        self.stale.set(false);
        self.refresh_asks.set(0);
        *self.data.borrow_mut() = None;
        *self.input.borrow_mut() = Rc::new(OrbitInput::default());
        *self.last_view.borrow_mut() = View::default();
        *self.frame.borrow_mut() = Frame::default();
    }

    fn start_refresh_timer(&self) {
        self.start_refresh(&self.refresh_timer, REFRESH_DELAY);
    }

    fn start_refresh(&self, timer: &Timer, delay: Duration) {
        let me = self.me.clone();
        timer.start(TimerMode::SingleShot, delay, move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(inner) = me.upgrade() {
                inner.refresh_active_picture();
            }
        });
    }

    /// The active tab's picture, asked for again: at most twice per open (after
    /// [`REFRESH_DELAY`], and once more after [`LATE_REFRESH_DELAY`] if the first ask brought no
    /// new picture). The old picture stays until a new one is there.
    fn refresh_active_picture(&self) {
        let asked = self.refresh_asks.get();
        if !self.open.get() || asked >= 2 {
            return;
        }
        self.refresh_asks.set(asked + 1);
        let active = self.input.borrow().active_tab;
        let before = self.images.borrow().get(&active).cloned().flatten();
        let after = (self.host.snapshot)(active);
        let landed = after
            .as_ref()
            .is_some_and(|image| !same_picture(before.as_ref(), image));
        if let Some(image) = after {
            self.images.borrow_mut().insert(active, Some(image));
            self.stale.set(true);
            self.settle();
        }
        if asked == 0 && !landed {
            self.start_refresh(&self.late_refresh_timer, LATE_REFRESH_DELAY);
        }
    }

    /// The picture of a tab: asked from the host once per open, then kept.
    fn picture(&self, tab: i32) -> Option<Image> {
        let known = self.images.borrow().get(&tab).cloned();
        if let Some(image) = known {
            return image;
        }
        let image = (self.host.snapshot)(tab);
        self.images.borrow_mut().insert(tab, image.clone());
        image
    }

    // -- Building the frame -----------------------------------------------------------------

    /// Builds again if the machine's view or something the view does not show has changed.
    fn settle(&self) {
        if !self.open.get() {
            return;
        }
        let mut view = self.machine.borrow().view();
        view.pinch = None;
        let mut last = self.last_view.borrow().clone();
        last.pinch = None;
        if self.stale.get() || view != last {
            self.rebuild();
        }
    }

    fn rebuild(&self) {
        if !self.open.get() {
            return;
        }
        if self.building.replace(true) {
            // Called from inside a build (a property write fired a callback): once more after it.
            self.stale.set(true);
            return;
        }
        for _ in 0..3 {
            self.stale.set(false);
            self.build_once();
            if !self.stale.get() || !self.open.get() {
                break;
            }
        }
        self.building.set(false);
    }

    fn build_once(&self) {
        let view = self.machine.borrow().view();
        let input = Rc::clone(&self.input.borrow());
        let environment = (self.host.environment)();
        let options = Options {
            dark: environment.dark,
            colours: self.window.ring_colours(),
            ..Options::default()
        };
        let snapshots = |tab: i32| self.picture(tab);
        let data = OrbitData::build(self.window.stage(), &input, &view, &snapshots, &options);
        self.models.update(&data);
        *self.data.borrow_mut() = Some(Rc::new(data));
        *self.last_view.borrow_mut() = view.clone();
        self.push_frame(&view, &input, environment);
    }

    fn push_frame(&self, view: &View, input: &OrbitInput, environment: Environment) {
        let notice = self.notice.borrow().clone();
        let mut frame = self
            .models
            .frame(view, input, environment.reduce_motion, &notice);
        self.place_ghost(&mut frame.drag);
        *self.frame.borrow_mut() = frame.clone();
        self.window.set_frame(&frame);
    }

    fn place_ghost(&self, drag: &mut OrbitDragView) {
        if let (true, Some((x, y))) = (drag.active, self.pointer.get()) {
            drag.ghost_x = x;
            drag.ghost_y = y;
        }
    }

    /// Moves the ghost without a rebuild.
    fn move_ghost(&self) {
        let mut frame = self.frame.borrow().clone();
        if !frame.drag.active {
            return;
        }
        self.place_ghost(&mut frame.drag);
        *self.frame.borrow_mut() = frame.clone();
        self.window.set_frame(&frame);
    }

    // -- Effects ----------------------------------------------------------------------------

    /// Does what the machine asked. Never builds: the caller calls `settle` afterwards.
    fn apply(&self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::CapturePicture => (self.host.capture_active)(),
                Effect::Close { activate } => {
                    self.close_with(activate);
                    return;
                }
                Effect::Springback => self.window.set_pinch_progress(0.0),
                Effect::ZoomedTo(_) => {
                    self.window.set_pinch_progress(0.0);
                    self.stale.set(true);
                }
                Effect::Resolve(decision) => self.resolve(&decision),
                Effect::Apply(op) => self.run_op(&op),
                Effect::OpenNewGroupPopover { .. } => self.stale.set(true),
                Effect::FocusSearch => self.window.focus_search(),
                Effect::ClearSearch => {
                    self.search_text.borrow_mut().clear();
                    self.window.set_search_text("");
                    self.stale.set(true);
                }
            }
        }
    }

    fn resolve(&self, decision: &Decision) {
        match decision.op() {
            Some(op) => self.run_op(&op),
            None if *decision == Decision::Reject => self.say("That can't be dropped there."),
            None => {}
        }
    }

    /// Runs a group command; the rows are read again after it, a refusal is a notice and a log
    /// line.
    fn run_op(&self, op: &GroupOp) {
        match (self.host.group_op)(op) {
            Ok(()) => self.refresh_rows(),
            Err(error) => {
                log::warn!("orbit: {op:?} was refused: {error}");
                self.say(&notice_text(error));
            }
        }
    }

    fn say(&self, text: &str) {
        *self.notice.borrow_mut() = text.to_owned();
        self.stale.set(true);
    }

    /// Clears the notice at the next user action.
    fn clear_notice(&self) {
        if !self.notice.borrow().is_empty() {
            self.notice.borrow_mut().clear();
            self.stale.set(true);
        }
    }

    fn refresh_rows(&self) {
        let input = Rc::new((self.host.rows)());
        *self.input.borrow_mut() = Rc::clone(&input);
        // A tab that is gone does not keep its picture.
        self.images
            .borrow_mut()
            .retain(|tab, _| input.tab(*tab).is_some());
        let dragging = self.machine.borrow().view().drag.is_some();
        let effects = self.machine.borrow_mut().rows_changed(&input);
        if dragging && self.machine.borrow().view().drag.is_none() {
            // The dragged tab or group is gone: the ring's drag has nothing to land.
            self.pointer.set(None);
            self.window.cancel_ring_drag();
        }
        self.stale.set(true);
        self.apply(effects);
    }

    // -- Events -----------------------------------------------------------------------------

    fn hover(&self, target: Target) {
        if !self.open.get() {
            return;
        }
        self.machine.borrow_mut().hover(target);
        self.settle();
    }

    fn click(&self, target: Target) {
        if !self.open.get() {
            return;
        }
        self.clear_notice();
        let effects = self.machine.borrow_mut().click(target);
        self.apply(effects);
        self.settle();
    }

    /// A key from outside the view (`input-hooks`) or from its key scope.
    fn handle_key(&self, key: Key, mods: Mods) -> bool {
        if !self.open.get() {
            return false;
        }
        let text_field_focused = self.window.text_field_focused();
        if text_field_focused {
            // The popover confirms itself on ⏎ (its own callback): the hook must not eat it.
            let popover = self.machine.borrow().view().popover.is_some();
            let enter_for_the_popover = key == Key::Enter && !mods.command && popover;
            if is_text_key(key, mods) || enter_for_the_popover {
                return false;
            }
        }
        self.run_key(key, mods, text_field_focused)
    }

    /// Runs a key on the machine; `text_input_focused` is what the machine is told about the
    /// keyboard.
    fn run_key(&self, key: Key, mods: Mods, text_input_focused: bool) -> bool {
        if !self.open.get() {
            return false;
        }
        self.clear_notice();
        let dragging = self.machine.borrow().view().drag.is_some();
        let effects = self
            .machine
            .borrow_mut()
            .key(key, mods, KeyContext { text_input_focused });
        if dragging && self.machine.borrow().view().drag.is_none() {
            // Esc, layer 1: the machine's drag is gone, so the ring's must go too (its
            // `drag-cancelled` then finds nothing to drop).
            self.pointer.set(None);
            self.window.cancel_ring_drag();
        }
        if let Key::Char(c) = key
            && effects.contains(&Effect::FocusSearch)
        {
            // The machine does not touch the field: the typed character starts the search.
            let text = format!("{}{c}", self.search_text.borrow());
            self.window.set_search_text(&text);
            self.search_edited(&text);
        }
        self.apply(effects);
        self.settle();
        true
    }

    fn set_search(&self, text: &str) {
        if !self.open.get() {
            return;
        }
        self.window.set_search_text(text);
        self.search_edited(text);
    }

    fn search_edited(&self, text: &str) {
        if !self.open.get() {
            return;
        }
        *self.search_text.borrow_mut() = text.to_owned();
        self.machine.borrow_mut().search_changed(text);
        self.settle();
    }

    fn rows_changed(&self) {
        if !self.open.get() {
            return;
        }
        self.refresh_rows();
        self.settle();
    }

    fn refresh(&self) {
        self.rebuild();
    }

    // -- Pinch ------------------------------------------------------------------------------

    fn pinch_started(&self, x: f32, y: f32) {
        if !self.open.get() {
            return;
        }
        let target = self
            .data
            .borrow()
            .as_ref()
            .map_or(Target::Backdrop, |data| data.layout.hit(x, y));
        self.machine.borrow_mut().pinch_started(target);
    }

    /// The view scales the stage with the pinch itself; the machine is asked at `ended`.
    fn pinch_updated(&self, scale: f32) {
        if self.open.get() {
            self.machine.borrow_mut().pinch_updated(scale);
        }
    }

    fn pinch_ended(&self) {
        if !self.open.get() {
            return;
        }
        let effects = self.machine.borrow_mut().pinch_ended();
        self.apply(effects);
        self.settle();
    }

    fn pinch_cancelled(&self) {
        if !self.open.get() {
            return;
        }
        let effects = self.machine.borrow_mut().pinch_cancelled();
        self.apply(effects);
        self.settle();
    }

    // -- Drag and drop ----------------------------------------------------------------------

    fn drag_started(&self, source: Source) {
        if !self.open.get() {
            return;
        }
        self.clear_notice();
        self.machine.borrow_mut().drag_started(source);
        self.settle();
    }

    /// The target the machine shows as lit: only one the drop would change something on.
    fn lit(&self, target: Option<DropTarget>) -> Option<DropTarget> {
        let (source, session) = {
            let machine = self.machine.borrow();
            (machine.view().drag?.source, machine.session())
        };
        lit_target(source, session, target, &self.input.borrow())
    }

    fn drag_over(&self, kind: &str, id: i32, x: f32, y: f32) {
        if !self.open.get() {
            return;
        }
        self.pointer.set(Some((x, y)));
        let lit = self.lit(drop_target_of(kind, id, -1));
        self.machine.borrow_mut().drag_over(lit);
        self.settle();
        self.move_ghost();
    }

    fn ring_drag_moved(&self, x: f32, y: f32) {
        if !self.open.get() {
            return;
        }
        self.pointer.set(Some((x, y)));
        let lit = self.lit(self.drop_target_at(x, y));
        self.machine.borrow_mut().drag_over(lit);
        self.settle();
        self.move_ghost();
    }

    /// A ring drag was released over a ring at `slot`.
    fn ring_dropped(&self, group: i32, slot: i32) {
        if !self.open.get() {
            return;
        }
        let target = drop_target_of("group", group, slot);
        self.release_over(target);
    }

    /// `RingBands` reports `drag-cancelled` for a release anywhere but over its own ring: the last
    /// pointer position says what that was. Nothing happens when the machine's drag is already
    /// gone (Esc cancelled it).
    fn ring_drag_cancelled(&self) {
        if !self.open.get() || self.machine.borrow().view().drag.is_none() {
            return;
        }
        let target = self
            .pointer
            .get()
            .and_then(|(x, y)| self.drop_target_at(x, y));
        self.release_over(target);
    }

    /// Releases the drag over `target` (`None`: over nothing, which cancels).
    fn release_over(&self, target: Option<DropTarget>) {
        self.machine.borrow_mut().drag_over(target);
        let effects = self.machine.borrow_mut().drag_released();
        self.pointer.set(None);
        self.apply(effects);
        self.settle();
    }

    fn drag_source_finished(&self, accepted: bool) {
        if !self.open.get() {
            return;
        }
        if !accepted {
            self.machine.borrow_mut().drag_cancelled();
            self.window.cancel_ring_drag();
        }
        self.pointer.set(None);
        self.settle();
    }

    /// What the stage point is over for a drop (`OrbitData::drop_target_at`), for the tab being
    /// dragged, if the drag is of a tab.
    fn drop_target_at(&self, x: f32, y: f32) -> Option<DropTarget> {
        let data = self.data.borrow().clone()?;
        let dragging = match self.machine.borrow().view().drag.map(|drag| drag.source) {
            Some(Source::Tab(tab)) => tab,
            _ => -1,
        };
        data.drop_target_at(x, y, dragging)
    }

    fn with_ring<R>(&self, group: i32, call: impl FnOnce(&RingBandsData) -> R) -> Option<R> {
        let data = self.data.borrow().clone()?;
        ring_of(&data, group).map(call)
    }

    fn payload_for(&self, kind: &str, id: i32) -> DataTransfer {
        let input = Rc::clone(&self.input.borrow());
        let machine = self.machine.borrow();
        let Some(source) = source_of(kind, id).filter(|source| machine.can_drag(*source)) else {
            return DataTransfer::default();
        };
        let payload = OrbitPayload {
            session: machine.session(),
            source,
        };
        payload.to_data_transfer(&drag::drag_text(source, &input))
    }

    /// The payload in `data`, if it is this Orbit's.
    fn decode(&self, data: &DataTransfer) -> Option<OrbitPayload> {
        if !self.open.get() {
            return None;
        }
        OrbitPayload::from_data_transfer(data, self.machine.borrow().session()).ok()
    }

    fn payload_accepts(&self, data: &DataTransfer, kind: &str, id: i32, slot: i32) -> bool {
        let (Some(payload), Some(target)) = (self.decode(data), drop_target_of(kind, id, slot))
        else {
            return false;
        };
        drag::can_accept(&payload, target, &self.input.borrow())
    }

    fn payload_dropped(&self, data: &DataTransfer, kind: &str, id: i32, slot: i32) -> bool {
        if !self.payload_accepts(data, kind, id, slot) {
            return false;
        }
        let (Some(payload), Some(target)) = (self.decode(data), drop_target_of(kind, id, slot))
        else {
            return false;
        };
        if self.machine.borrow().view().drag.is_none() {
            self.machine.borrow_mut().drag_started(payload.source);
        }
        self.release_over(Some(target));
        true
    }

    // -- The new-group popover --------------------------------------------------------------

    fn popover_name_edited(&self, name: &str) {
        if !self.open.get() {
            return;
        }
        let colour = self
            .machine
            .borrow()
            .view()
            .popover
            .map(|popover| popover.colour)
            .unwrap_or_default();
        self.machine.borrow_mut().popover_edited(name, &colour);
        // The field already shows what was typed: no new frame for every key.
        *self.last_view.borrow_mut() = self.machine.borrow().view();
    }

    fn popover_colour_chosen(&self, index: i32) {
        if !self.open.get() {
            return;
        }
        let Some(colour) = popover_colour_name(index) else {
            return;
        };
        let name = self
            .machine
            .borrow()
            .view()
            .popover
            .map(|popover| popover.name)
            .unwrap_or_default();
        self.machine.borrow_mut().popover_edited(&name, &colour);
        self.settle();
    }

    fn popover_confirmed(&self) {
        if !self.open.get() {
            return;
        }
        let (name, colour) = self
            .machine
            .borrow()
            .view()
            .popover
            .map(|popover| (popover.name, popover.colour))
            .unwrap_or_default();
        let effects = self.machine.borrow_mut().popover_confirmed(&name, &colour);
        self.stale.set(true);
        self.apply(effects);
        self.settle();
    }
}

/// The ring data of the planet (level 1) or the centre ring (levels 2 and 3) of `group`.
fn ring_of(data: &OrbitData, group: i32) -> Option<&RingBandsData> {
    data.planets
        .iter()
        .find(|planet| planet.row.group == group)
        .or_else(|| {
            data.centre
                .as_ref()
                .filter(|centre| centre.row.group == group)
        })
        .map(|planet| &planet.ring)
}

/// Whether `after` holds the same pixels as `before`. `Image` compares by buffer address and every
/// `tab_snapshots::image_for` makes a new buffer, so the pixels are compared (the pictures are
/// small). A picture whose pixels cannot be read counts as different.
fn same_picture(before: Option<&Image>, after: &Image) -> bool {
    let Some(before) = before else {
        return false;
    };
    if before.size() != after.size() {
        return false;
    }
    match (before.to_rgba8(), after.to_rgba8()) {
        (Some(before), Some(after)) => before.as_bytes() == after.as_bytes(),
        _ => false,
    }
}

/// The one line a refused group command shows in the view (the log has the details).
pub(crate) fn notice_text(error: GroupError) -> String {
    match error {
        GroupError::TabPinned => "Pinned tabs can't be put in a group.".to_owned(),
        GroupError::TooManyGroups => format!("A window holds at most {MAX_GROUPS} groups."),
        GroupError::NotFound | GroupError::TabNotFound => {
            "That tab or group is no longer there.".to_owned()
        }
        GroupError::NoTabs => "A group needs at least one tab.".to_owned(),
        GroupError::EmptyName => "A group needs a name.".to_owned(),
        GroupError::KeyTaken { .. } => "Another group already has that key.".to_owned(),
        GroupError::IdsExhausted => "No more group ids are available.".to_owned(),
    }
}

// ---------------------------------------------------------------------------------------------
// The real host: rows and group commands against the browser controller
// ---------------------------------------------------------------------------------------------

/// The colours Orbit's rows need, from the appearance settings (`mercury-appearance` knows the
/// presets, the tints that stay legible and the shades of a group's tabs; the desktop crate does
/// not depend on it, so `layout-switch` implements this next to the code that applies the look).
pub(crate) trait RowStyle {
    /// The dark scheme is in use.
    fn dark(&self) -> bool;
    /// The mark, the label wash and the deep fill of a group colour on the chrome.
    fn swatch(&self, colour: GroupColour) -> GroupSwatch;
    /// The shade of the tab at `index` in a group of this colour.
    fn shade(&self, colour: GroupColour, index: usize) -> Color;
}

/// The three colours of a `GroupRow`.
#[allow(dead_code)] // used by layout-switch
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GroupSwatch {
    pub colour: Color,
    pub tint: Color,
    pub deep: Color,
}

/// What `layout-switch` gives [`OrbitHost::for_desktop`].
#[allow(dead_code)] // used by layout-switch
pub(crate) struct DesktopHost {
    pub controller: Rc<RefCell<BrowserController>>,
    /// The rendering context, once there is one (it is made when the first frame is shown).
    pub rendering_context: Box<ContextFn>,
    pub style: Rc<dyn RowStyle>,
    /// The sun's title ("Personal").
    pub profile_name: String,
    /// The sun's subtitle without the count ("Window 1").
    pub window_label: String,
    /// Makes the tab the active one and shows it (the controller's `select_tab` and the sync).
    pub activate: Box<dyn Fn(TabId)>,
    /// Hides the overlay and gives the page its focus back.
    pub close: Box<dyn Fn()>,
    /// A group command changed the controller: sync the window (the tab strip, the hub).
    pub controller_changed: Box<dyn Fn()>,
    pub environment: Box<dyn Fn() -> Environment>,
}

impl OrbitHost {
    /// The host that reads the rows from the controller, runs group commands on it and takes the
    /// pictures from `tab_snapshots`.
    #[allow(dead_code)] // used by layout-switch
    pub(crate) fn for_desktop(parts: DesktopHost) -> Self {
        let DesktopHost {
            controller,
            rendering_context,
            style,
            profile_name,
            window_label,
            activate,
            close,
            controller_changed,
            environment,
        } = parts;
        let capture_controller = Rc::clone(&controller);
        let rows_controller = Rc::clone(&controller);
        Self {
            capture_active: Box::new(move || {
                let Some(context) = rendering_context() else {
                    return;
                };
                let active = capture_controller.borrow().active_tab_id();
                crate::tab_snapshots::capture_active_now(&context, active);
            }),
            snapshot: Box::new(|tab| id_from_ui(tab).and_then(crate::tab_snapshots::image_for)),
            rows: Box::new(move || {
                rows_from_controller(
                    &rows_controller.borrow(),
                    style.as_ref(),
                    &profile_name,
                    &window_label,
                    SystemTime::now(),
                )
            }),
            group_op: Box::new(move |op| {
                let result = apply_group_op(&mut controller.borrow_mut(), op, SystemTime::now());
                if result.is_ok() {
                    controller_changed();
                }
                result
            }),
            activate: Box::new(move |tab| {
                if let Some(tab) = id_from_ui(tab) {
                    activate(tab);
                }
            }),
            close,
            environment,
        }
    }
}

/// A group id as the view carries it (`GroupRow.id`); `None` past `i32::MAX` (never in practice:
/// ids come from a counter capped well below).
pub(crate) fn group_id_for_ui(group: GroupId) -> Option<i32> {
    i32::try_from(group.value()).ok()
}

/// The rows Orbit shows, from the controller: groups in their order with their keys, counts and
/// flags, tabs in strip order, loose and pinned tabs folded into the "Loose tabs" planet.
///
/// Private tabs do not exist yet. When they do, they must be left out here (no row, so no
/// capture, no picture, no card) and from `capture_active` below (review F10).
pub(crate) fn rows_from_controller(
    controller: &BrowserController,
    style: &dyn RowStyle,
    profile_name: &str,
    window_label: &str,
    now: SystemTime,
) -> OrbitInput {
    let active = controller.active_tab_id();
    let active_group = controller.group_of(active);
    let mut shades: HashMap<GroupId, (GroupColour, usize)> = HashMap::new();
    let groups: Vec<GroupRow> = controller
        .groups()
        .iter()
        .filter_map(|group| {
            let id = group_id_for_ui(group.id)?;
            let swatch = style.swatch(group.colour);
            let summary = controller.group_summary(group.id).unwrap_or_default();
            shades.insert(group.id, (group.colour, 0));
            Some(GroupRow {
                id,
                name: group.name.as_str().into(),
                colour: swatch.colour,
                tint: swatch.tint,
                deep: swatch.deep,
                key_label: group
                    .key
                    .map(|key| key.letter().to_string())
                    .unwrap_or_default()
                    .into(),
                count: i32::try_from(summary.tabs).unwrap_or(i32::MAX),
                last_used: relative_age(now, group.last_active).into(),
                active: active_group == Some(group.id),
                any_loading: summary.any_loading,
                any_audible: summary.any_audible,
                any_unread: summary.any_unread,
            })
        })
        .collect();
    let slate = GroupColour::Preset(GroupPreset::Slate);
    let tabs: Vec<TabRow> = controller
        .tabs()
        .filter_map(|tab| {
            let id = id_for_ui(tab.id());
            if id < 0 {
                return None;
            }
            let state = tab.state();
            let status = controller.tab_status(tab.id()).unwrap_or_default();
            let (group, colour, index) = match tab.group().and_then(|group| {
                let (colour, next) = shades.get_mut(&group)?;
                let index = *next;
                *next += 1;
                Some((group, *colour, index))
            }) {
                Some((group, colour, index)) => {
                    (group_id_for_ui(group).unwrap_or(-1), colour, index)
                }
                None => (-1, slate, 0),
            };
            let favicon = state
                .page_icon()
                .map_or_else(Image::default, crate::ui_sync::slint_image_from_page_icon);
            Some(TabRow {
                id,
                group,
                index_in_group: i32::try_from(index).unwrap_or(0),
                title: crate::pdf::tab_title(tab.id(), state.page_title()).into(),
                host: host_of(state.url()).into(),
                shade: style.shade(colour, index),
                favicon,
                active: tab.id() == active,
                pinned: tab.is_pinned(),
                loading: status.loading,
                progress: 0.0,
                audible: status.audible,
                muted: status.muted,
                unread: status.unread,
                crashed: status.crashed,
                capture: !status.capture.is_empty(),
                matches_query: true,
            })
        })
        .collect();
    OrbitInput::new(
        &groups,
        &tabs,
        profile_name,
        window_label,
        id_for_ui(active),
        style.dark(),
    )
}

/// The host part of a page address, or the address itself when it has none.
fn host_of(address: &str) -> String {
    url::Url::parse(address)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| address.to_owned())
}

/// Runs a group command of the machine on the controller (the one `match` of the brief): a move
/// between groups, a tab made loose, a new group from tabs, a reordered group. Ids that do not
/// fit are the controller's "not found".
pub(crate) fn apply_group_op(
    controller: &mut BrowserController,
    op: &GroupOp,
    now: SystemTime,
) -> Result<(), GroupError> {
    let tab_of = |tab: i32| id_from_ui(tab).ok_or(GroupError::TabNotFound);
    let group_of = |group: i32| {
        u64::try_from(group)
            .map(GroupId::new)
            .map_err(|_| GroupError::NotFound)
    };
    match op {
        GroupOp::MoveTab { tab, group, index } => {
            controller.move_tab_to_group(tab_of(*tab)?, Some(group_of(*group)?), *index)
        }
        GroupOp::Ungroup { tab } => controller.ungroup_tab(tab_of(*tab)?),
        GroupOp::CreateGroup { tabs, name, colour } => {
            let tabs = tabs
                .iter()
                .map(|tab| tab_of(*tab))
                .collect::<Result<Vec<_>, _>>()?;
            let colour = colour.as_deref().and_then(GroupColour::parse);
            controller
                .create_group(&tabs, name.as_deref(), colour, now)
                .map(|_| ())
        }
        GroupOp::ReorderGroup { group, index } => controller
            .reorder_group(group_of(*group)?, *index)
            .map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use mercury_ui::orbit_view::LOOSE_GROUP_ID;
    use mercury_ui::orbit_view::fixtures::{orbit_input, skeleton_picture, tab_id};
    use mercury_ui::{OrbitLevel, OrbitPreview};
    use slint::{ComponentHandle, Model, SharedString};
    use talaria_core::{BrowserEvent, GroupKey, TabActivation};

    use super::*;

    type Log = Rc<RefCell<Vec<String>>>;

    // ------------------------------------------------------------------------------------
    // Fakes: a window that records what it is given, and a host that records what it is asked
    // ------------------------------------------------------------------------------------

    struct FakeWindow {
        log: Log,
        events: RefCell<Option<OrbitEvents>>,
        frames: RefCell<Vec<Frame>>,
        search_text: RefCell<String>,
        focus_search: Cell<u32>,
        cancel_ring_drag: Cell<u32>,
        pinch_progress: Cell<f32>,
        /// What `text_field_focused` answers.
        text_focused: Cell<bool>,
        play_open: Cell<u32>,
    }

    impl FakeWindow {
        fn new(log: &Log) -> Rc<Self> {
            Rc::new(Self {
                log: Rc::clone(log),
                events: RefCell::new(None),
                frames: RefCell::new(Vec::new()),
                search_text: RefCell::new(String::new()),
                focus_search: Cell::new(0),
                cancel_ring_drag: Cell::new(0),
                pinch_progress: Cell::new(0.0),
                text_focused: Cell::new(false),
                play_open: Cell::new(0),
            })
        }

        fn frame(&self) -> Frame {
            self.frames
                .borrow()
                .last()
                .cloned()
                .expect("a frame was set")
        }

        fn events(&self) -> OrbitEvents {
            self.events.borrow().clone().expect("the glue connected")
        }
    }

    impl OrbitWindow for FakeWindow {
        fn stage(&self) -> Stage {
            Stage::MOCKUP
        }

        fn ring_colours(&self) -> RingColours {
            RingColours::default()
        }

        fn set_models(&self, _models: &OrbitModels) {
            self.log.borrow_mut().push("models".to_owned());
        }

        fn set_frame(&self, frame: &Frame) {
            self.log.borrow_mut().push("frame".to_owned());
            self.frames.borrow_mut().push(frame.clone());
        }

        fn set_search_text(&self, text: &str) {
            *self.search_text.borrow_mut() = text.to_owned();
        }

        fn set_pinch_progress(&self, progress: f32) {
            self.pinch_progress.set(progress);
        }

        fn focus_search(&self) {
            self.focus_search.set(self.focus_search.get() + 1);
        }

        fn cancel_ring_drag(&self) {
            self.cancel_ring_drag.set(self.cancel_ring_drag.get() + 1);
        }

        fn focus_view(&self) {
            self.log.borrow_mut().push("focus-view".to_owned());
        }

        fn text_field_focused(&self) -> bool {
            self.text_focused.get()
        }

        fn play_open(&self) {
            self.play_open.set(self.play_open.get() + 1);
            self.log.borrow_mut().push("play-open".to_owned());
        }

        fn connect(&self, events: OrbitEvents) {
            *self.events.borrow_mut() = Some(events);
        }
    }

    struct Fakes {
        log: Log,
        input: Rc<RefCell<OrbitInput>>,
        snapshots: Rc<RefCell<HashMap<i32, u32>>>,
        /// The picture the host gives from now on instead of the first one (a capture landed).
        later_picture: Rc<RefCell<Option<Image>>>,
        /// The host has no picture at all (no capture landed).
        no_pictures: Rc<Cell<bool>>,
        ops: Rc<RefCell<Vec<GroupOp>>>,
        refuse: Rc<Cell<Option<GroupError>>>,
        rows_calls: Rc<Cell<u32>>,
    }

    impl Fakes {
        fn log(&self) -> Vec<String> {
            self.log.borrow().clone()
        }

        fn count(&self, entry: &str) -> usize {
            self.log
                .borrow()
                .iter()
                .filter(|line| *line == entry)
                .count()
        }

        fn ops(&self) -> Vec<GroupOp> {
            self.ops.borrow().clone()
        }
    }

    fn fake_host() -> (Fakes, OrbitHost) {
        let fakes = Fakes {
            log: Rc::new(RefCell::new(Vec::new())),
            input: Rc::new(RefCell::new(orbit_input(false))),
            snapshots: Rc::new(RefCell::new(HashMap::new())),
            later_picture: Rc::new(RefCell::new(None)),
            no_pictures: Rc::new(Cell::new(false)),
            ops: Rc::new(RefCell::new(Vec::new())),
            refuse: Rc::new(Cell::new(None)),
            rows_calls: Rc::new(Cell::new(0)),
        };
        let picture = skeleton_picture(false);
        let host = OrbitHost {
            capture_active: {
                let log = Rc::clone(&fakes.log);
                Box::new(move || log.borrow_mut().push("capture".to_owned()))
            },
            snapshot: {
                let snapshots = Rc::clone(&fakes.snapshots);
                let later = Rc::clone(&fakes.later_picture);
                let none = Rc::clone(&fakes.no_pictures);
                Box::new(move |tab| {
                    *snapshots.borrow_mut().entry(tab).or_default() += 1;
                    if none.get() {
                        return None;
                    }
                    Some(later.borrow().clone().unwrap_or_else(|| picture.clone()))
                })
            },
            rows: {
                let input = Rc::clone(&fakes.input);
                let calls = Rc::clone(&fakes.rows_calls);
                Box::new(move || {
                    calls.set(calls.get() + 1);
                    input.borrow().clone()
                })
            },
            group_op: {
                let ops = Rc::clone(&fakes.ops);
                let refuse = Rc::clone(&fakes.refuse);
                Box::new(move |op| {
                    ops.borrow_mut().push(op.clone());
                    refuse.get().map_or(Ok(()), Err)
                })
            },
            activate: {
                let log = Rc::clone(&fakes.log);
                Box::new(move |tab| log.borrow_mut().push(format!("activate:{tab}")))
            },
            close: {
                let log = Rc::clone(&fakes.log);
                Box::new(move || log.borrow_mut().push("close".to_owned()))
            },
            environment: Box::new(Environment::default),
        };
        (fakes, host)
    }

    fn setup() -> (OrbitController, Rc<FakeWindow>, Fakes) {
        crate::settings::tests::init_slint();
        let (fakes, host) = fake_host();
        let window = FakeWindow::new(&fakes.log);
        let orbit = OrbitController::new(host, window.clone());
        (orbit, window, fakes)
    }

    /// Opened, with the first frame built.
    fn opened() -> (OrbitController, Rc<FakeWindow>, Fakes) {
        let (orbit, window, fakes) = setup();
        orbit.open();
        (orbit, window, fakes)
    }

    fn elapse(time: Duration) {
        i_slint_backend_testing::mock_elapsed_time(time);
        slint::platform::update_timers_and_animations();
    }

    /// The ids of the mockup: Research is group 1 (tabs 101 to 110), Lisbon trip group 2.
    const RESEARCH: i32 = 1;
    const LISBON: i32 = 2;
    const READING: i32 = 3;
    const ACTIVE: i32 = 101;
    const ACCESSKIT_GUIDE: i32 = 104;

    // ------------------------------------------------------------------------------------
    // Opening, pictures, closing
    // ------------------------------------------------------------------------------------

    #[test]
    fn opening_asks_for_the_capture_once_and_before_the_first_build() {
        let (orbit, window, fakes) = setup();
        assert!(!orbit.is_open());
        assert_eq!(fakes.count("frame"), 0, "nothing is built while closed");

        orbit.open();
        orbit.open();

        assert!(orbit.is_open());
        assert_eq!(fakes.count("capture"), 1, "a second open does nothing");
        let log = fakes.log();
        let capture = log.iter().position(|line| line == "capture");
        let first_build = log.iter().position(|line| line == "frame");
        assert!(capture.is_some() && capture < first_build, "{log:?}");
        assert_eq!(window.frame().level, OrbitLevel::Groups);
        assert_eq!(window.frame().container_label, "Orbit, every group");
        assert_eq!(fakes.count("focus-view"), 1);
        assert_eq!(window.play_open.get(), 1, "the open animation plays once");
        let play = log.iter().position(|line| line == "play-open");
        assert!(first_build < play, "after the first build: {log:?}");
    }

    #[test]
    fn the_open_animation_plays_once_per_open() {
        let (orbit, window, _fakes) = setup();
        assert_eq!(window.play_open.get(), 0, "not while closed");

        orbit.open();
        orbit.open();
        assert_eq!(window.play_open.get(), 1, "a second open does nothing");
        window.events().planet_clicked(RESEARCH);
        orbit.rows_changed();
        assert_eq!(
            window.play_open.get(),
            1,
            "levels and rows do not replay it"
        );

        orbit.close();
        assert_eq!(window.play_open.get(), 1);
        orbit.open();
        assert_eq!(window.play_open.get(), 2);
    }

    /// How many times the host was asked for a tab's picture.
    fn asks(fakes: &Fakes, tab: i32) -> u32 {
        fakes.snapshots.borrow().get(&tab).copied().unwrap_or(0)
    }

    #[test]
    fn the_active_tabs_picture_is_asked_for_again_once_and_every_other_tab_once() {
        let (_orbit, window, fakes) = opened();
        let events = window.events();
        events.planet_clicked(RESEARCH);
        events.preview_clicked(ACCESSKIT_GUIDE);
        events.neighbour_clicked(ACCESSKIT_GUIDE + 1);
        assert_eq!(
            asks(&fakes, ACTIVE),
            1,
            "the active tab's picture is fetched when Orbit opens"
        );

        // The capture landed: the second ask brings a new picture, so there is no third.
        *fakes.later_picture.borrow_mut() = Some(skeleton_picture(true));
        elapse(REFRESH_DELAY + Duration::from_millis(10));
        elapse(LATE_REFRESH_DELAY * 4);
        events.planet_hovered(READING);
        events.planet_hovered(-1);

        assert_eq!(asks(&fakes, ACTIVE), 2, "refreshed exactly once");
        for (tab, asked) in fakes
            .snapshots
            .borrow()
            .iter()
            .filter(|(tab, _)| **tab != ACTIVE)
        {
            assert_eq!(*asked, 1, "tab {tab} was asked for more than once");
        }
    }

    #[test]
    fn a_picture_that_did_not_change_is_asked_for_once_more_at_about_750_ms_and_then_never() {
        let (_orbit, _window, fakes) = opened();
        assert_eq!(asks(&fakes, ACTIVE), 1);

        elapse(REFRESH_DELAY - Duration::from_millis(10));
        assert_eq!(asks(&fakes, ACTIVE), 1, "not before 250 ms");
        elapse(Duration::from_millis(20));
        assert_eq!(asks(&fakes, ACTIVE), 2, "the first re-ask");

        elapse(LATE_REFRESH_DELAY - Duration::from_millis(20));
        assert_eq!(asks(&fakes, ACTIVE), 2, "not before about 750 ms");
        elapse(Duration::from_millis(40));
        assert_eq!(asks(&fakes, ACTIVE), 3, "the late ask");

        elapse(Duration::from_secs(30));
        assert_eq!(asks(&fakes, ACTIVE), 3, "bounded: two asks after the open");
    }

    #[test]
    fn a_picture_that_never_lands_is_still_asked_for_at_most_twice_after_the_open() {
        let (orbit, _window, fakes) = setup();
        fakes.no_pictures.set(true);
        orbit.open();

        elapse(REFRESH_DELAY + Duration::from_millis(10));
        elapse(LATE_REFRESH_DELAY + Duration::from_millis(10));
        elapse(Duration::from_secs(30));

        assert_eq!(asks(&fakes, ACTIVE), 3);
        assert!(orbit.is_open());
    }

    #[test]
    fn closing_before_the_late_ask_cancels_it_and_a_new_open_starts_the_count_again() {
        let (orbit, _window, fakes) = opened();
        elapse(REFRESH_DELAY + Duration::from_millis(10));
        assert_eq!(asks(&fakes, ACTIVE), 2);

        orbit.close();
        elapse(Duration::from_secs(30));
        assert_eq!(asks(&fakes, ACTIVE), 2, "no ask after close");

        orbit.open();
        assert_eq!(asks(&fakes, ACTIVE), 3);
        elapse(REFRESH_DELAY + Duration::from_millis(10));
        elapse(LATE_REFRESH_DELAY + Duration::from_millis(10));
        assert_eq!(asks(&fakes, ACTIVE), 5, "one open ask and two more");
    }

    #[test]
    fn closing_drops_every_picture_and_blanks_the_view() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.planet_clicked(RESEARCH);
        assert!(!orbit.inner.images.borrow().is_empty());

        orbit.close();

        assert!(!orbit.is_open());
        assert!(orbit.inner.images.borrow().is_empty());
        assert_eq!(orbit.inner.models.previews().row_count(), 0);
        assert_eq!(orbit.inner.models.planets().row_count(), 0);
        assert!(!window.frame().has_centre && !window.frame().card.visible);
        assert_eq!(window.frame().container_label, "");
        assert_eq!(fakes.count("close"), 1);

        orbit.close();
        assert_eq!(fakes.count("close"), 1, "closing twice tells the host once");
        elapse(REFRESH_DELAY * 2);
        assert_eq!(
            fakes.snapshots.borrow().get(&ACTIVE),
            Some(&1),
            "no refresh after close"
        );
    }

    #[test]
    fn opening_again_asks_for_the_pictures_again() {
        let (orbit, _window, fakes) = opened();
        orbit.close();
        orbit.open();

        assert_eq!(fakes.count("capture"), 2);
        assert_eq!(fakes.snapshots.borrow().get(&ACTIVE), Some(&2));
    }

    #[test]
    fn leaving_with_a_tab_closes_first_and_activates_after() {
        let (_orbit, window, fakes) = opened();
        let events = window.events();
        events.planet_clicked(RESEARCH);
        events.preview_clicked(ACCESSKIT_GUIDE);
        assert_eq!(window.frame().level, OrbitLevel::Tab);

        events.card_clicked();

        let log = fakes.log();
        assert_eq!(log[log.len() - 2..], ["close", "activate:104"]);
        assert_eq!(fakes.count("close"), 1);
    }

    #[test]
    fn back_to_the_page_closes_without_activating() {
        let (orbit, window, fakes) = opened();

        window.events().back_clicked();

        assert!(!orbit.is_open());
        assert_eq!(fakes.count("close"), 1);
        assert!(fakes.log().iter().all(|line| !line.starts_with("activate")));
    }

    #[test]
    fn the_session_changes_on_every_open_and_an_old_payload_is_refused() {
        let (orbit, window, _fakes) = opened();
        let events = window.events();
        let first = orbit.session();
        let payload = events.payload_for("tab", ACTIVE);
        assert!(events.payload_accepts(&payload, "group", LISBON, -1));

        orbit.close();
        assert!(
            !events.payload_accepts(&payload, "group", LISBON, -1),
            "nothing is accepted while closed"
        );
        orbit.open();

        assert_ne!(orbit.session(), first);
        assert!(!events.payload_accepts(&payload, "group", LISBON, -1));
        let fresh = events.payload_for("tab", ACTIVE);
        assert!(events.payload_accepts(&fresh, "group", LISBON, -1));
    }

    // ------------------------------------------------------------------------------------
    // Keys
    // ------------------------------------------------------------------------------------

    #[test]
    fn escape_is_not_taken_while_closed_and_leaves_orbit_when_open() {
        let (orbit, _window, fakes) = setup();
        assert!(!orbit.handle_key(Key::Escape, Mods::NONE));
        assert!(!orbit.handle_key(Key::Enter, Mods::NONE));

        orbit.open();
        assert!(orbit.handle_key(Key::Escape, Mods::NONE));

        assert!(!orbit.is_open());
        assert_eq!(fakes.count("close"), 1);
        assert!(!orbit.handle_key(Key::Escape, Mods::NONE));
    }

    #[test]
    fn escape_goes_through_its_layers_search_first_then_the_page() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.search_edited("servo");
        assert!(window.frame().query_active);

        assert!(orbit.handle_key(Key::Escape, Mods::NONE));

        assert!(orbit.is_open(), "the first Escape only clears the search");
        assert!(!window.frame().query_active);
        assert_eq!(*window.search_text.borrow(), "");
        assert_eq!(fakes.count("close"), 0);
        assert!(orbit.handle_key(Key::Escape, Mods::NONE));
        assert_eq!(fakes.count("close"), 1);
    }

    #[test]
    fn typing_a_character_starts_the_search_with_it() {
        let (orbit, window, _fakes) = opened();

        assert!(orbit.handle_key(Key::Char('x'), Mods::NONE));

        assert_eq!(window.focus_search.get(), 1);
        assert_eq!(*window.search_text.borrow(), "x");
        assert!(window.frame().query_active);

        assert!(orbit.handle_key(Key::Slash, Mods::NONE));
        assert_eq!(window.focus_search.get(), 2);
        assert_eq!(*window.search_text.borrow(), "x", "/ only moves the focus");
    }

    #[test]
    fn set_search_fills_the_field_and_marks_the_query_active() {
        let (orbit, window, _fakes) = opened();

        orbit.set_search("lisbon");

        assert_eq!(*window.search_text.borrow(), "lisbon");
        assert!(window.frame().query_active);
        orbit.set_search("  ");
        assert!(!window.frame().query_active, "a blank query is no query");
    }

    #[test]
    fn command_up_climbs_one_level_and_leaves_orbit_from_the_first() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.planet_clicked(RESEARCH);
        events.preview_clicked(ACCESSKIT_GUIDE);
        assert_eq!(window.frame().level, OrbitLevel::Tab);

        assert!(orbit.handle_key(Key::Up, Mods::COMMAND));
        assert_eq!(window.frame().level, OrbitLevel::Group);
        assert!(orbit.handle_key(Key::Up, Mods::COMMAND));
        assert_eq!(window.frame().level, OrbitLevel::Groups);
        assert!(orbit.is_open());
        assert!(orbit.handle_key(Key::Up, Mods::COMMAND));

        assert!(!orbit.is_open());
        assert_eq!(fakes.count("close"), 1);
    }

    #[test]
    fn enter_zooms_into_the_focused_group() {
        let (orbit, window, _fakes) = opened();

        assert!(orbit.handle_key(Key::Enter, Mods::NONE));

        assert_eq!(window.frame().level, OrbitLevel::Group);
        assert_eq!(
            window.frame().focus_tab,
            ACTIVE,
            "the active tab is first at level 2"
        );
    }

    #[test]
    fn new_group_clicked_is_the_n_key() {
        let (orbit, window, _fakes) = opened();

        window.events().new_group_clicked();

        let view = orbit.inner.machine.borrow().view();
        assert_eq!(view.popover.map(|popover| popover.tab), Some(ACTIVE));
        assert!(orbit.handle_key(Key::Escape, Mods::NONE));
        assert!(orbit.inner.machine.borrow().view().popover.is_none());
        assert!(orbit.is_open(), "Esc closes the popover first");
    }

    #[test]
    fn while_a_text_field_has_the_focus_the_text_keys_are_not_taken_and_change_nothing() {
        let (orbit, window, fakes) = opened();
        window.text_focused.set(true);
        let frames = window.frames.borrow().len();
        let arrow = |key: slint::platform::Key| char::from(key).to_string();

        for key in [
            Key::N,
            Key::Slash,
            Key::Char('x'),
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Tab,
        ] {
            assert!(!orbit.handle_key(key, Mods::NONE), "{key:?}");
        }
        assert!(!orbit.handle_key(
            Key::Left,
            Mods {
                alt: true,
                ..Mods::NONE
            }
        ));
        let events = window.events();
        assert!(!events.key_pressed("n", false, false, false, false));
        assert!(!events.key_pressed(
            &arrow(slint::platform::Key::UpArrow),
            false,
            false,
            false,
            false
        ));

        assert!(
            orbit.inner.machine.borrow().view().popover.is_none(),
            "N typed into a field does not open the popover"
        );
        assert_eq!(
            window.focus_search.get(),
            0,
            "'/' and letters do not steal the focus"
        );
        assert_eq!(*window.search_text.borrow(), "");
        assert_eq!(window.frames.borrow().len(), frames, "nothing was rebuilt");
        assert_eq!(fakes.count("close"), 0);
        assert!(orbit.is_open());
    }

    #[test]
    fn without_a_focused_text_field_n_opens_the_popover_and_the_arrows_are_taken() {
        let (orbit, window, _fakes) = opened();
        assert!(!window.text_focused.get());

        assert!(orbit.handle_key(Key::N, Mods::NONE));

        assert!(orbit.inner.machine.borrow().view().popover.is_some());
        assert!(orbit.handle_key(Key::Right, Mods::NONE), "modal: taken");
    }

    #[test]
    fn escape_and_the_command_keys_still_act_while_a_text_field_has_the_focus() {
        let (orbit, window, fakes) = opened();
        window.events().planet_clicked(RESEARCH);
        window.text_focused.set(true);
        window.events().search_edited("servo");

        assert!(
            orbit.handle_key(Key::Escape, Mods::NONE),
            "Esc clears the search"
        );
        assert!(!window.frame().query_active);
        assert!(
            orbit.handle_key(Key::Up, Mods::COMMAND),
            "⌘↑ climbs a level"
        );
        assert_eq!(window.frame().level, OrbitLevel::Groups);
        assert!(orbit.handle_key(Key::Escape, Mods::NONE));

        assert!(!orbit.is_open());
        assert_eq!(fakes.count("close"), 1);
    }

    #[test]
    fn enter_in_the_popovers_name_field_is_left_to_the_field() {
        let (orbit, window, _fakes) = opened();
        window.events().new_group_clicked();
        assert!(orbit.inner.machine.borrow().view().popover.is_some());

        window.text_focused.set(true);
        assert!(!orbit.handle_key(Key::Enter, Mods::NONE));
        window.text_focused.set(false);
        assert!(orbit.handle_key(Key::Enter, Mods::NONE), "modal: taken");
    }

    #[test]
    fn clicking_new_group_opens_the_popover_even_while_the_search_field_has_the_focus() {
        let (orbit, window, _fakes) = opened();
        window.text_focused.set(true);

        window.events().new_group_clicked();

        assert!(orbit.inner.machine.borrow().view().popover.is_some());
    }

    #[test]
    fn key_pressed_reports_whether_orbit_took_the_key() {
        let (orbit, window, fakes) = opened();
        let events = window.events();

        assert!(
            !events.key_pressed(
                &char::from(slint::platform::Key::F1).to_string(),
                false,
                false,
                false,
                false
            ),
            "F1 is not Orbit's"
        );
        assert!(events.key_pressed("\n", false, false, false, false));
        assert_eq!(window.frame().level, OrbitLevel::Group);
        let up = char::from(slint::platform::Key::UpArrow).to_string();
        assert!(events.key_pressed(&up, true, false, false, false), "⌘↑");
        assert_eq!(window.frame().level, OrbitLevel::Groups);
        assert!(events.key_pressed("\u{1b}", false, false, false, false));
        assert!(!orbit.is_open());
        assert_eq!(fakes.count("close"), 1);
        assert!(
            !events.key_pressed("\u{1b}", false, false, false, false),
            "closed: not taken"
        );
    }

    // ------------------------------------------------------------------------------------
    // Levels, hover, pinch, rows
    // ------------------------------------------------------------------------------------

    #[test]
    fn clicks_move_through_the_levels_and_the_container_label_says_where() {
        let (_orbit, window, _fakes) = opened();
        let events = window.events();

        events.planet_clicked(RESEARCH);
        assert_eq!(window.frame().level, OrbitLevel::Group);
        assert_eq!(window.frame().container_label, "Orbit, Research, 10 tabs");

        events.preview_clicked(ACCESSKIT_GUIDE);
        assert_eq!(window.frame().level, OrbitLevel::Tab);
        assert_eq!(
            window.frame().container_label,
            "Orbit, AccessKit guide, tab 4 of 10"
        );

        events.backdrop_clicked();
        assert_eq!(window.frame().level, OrbitLevel::Group, "outside the card");
        events.chip_clicked(LISBON);
        assert_eq!(window.frame().container_label, "Orbit, Lisbon trip, 8 tabs");
    }

    #[test]
    fn hover_reaches_the_machine_and_minus_one_clears_it() {
        let (orbit, window, _fakes) = opened();
        let events = window.events();
        let hover = || orbit.inner.machine.borrow().view().hover;

        events.planet_hovered(LISBON);
        assert_eq!(hover(), Some(Target::Planet(LISBON)));
        events.planet_hovered(-1);
        assert_eq!(hover(), None);
        events.planet_clicked(RESEARCH);
        events.preview_hovered(ACCESSKIT_GUIDE);
        assert_eq!(hover(), Some(Target::Preview(ACCESSKIT_GUIDE)));
        events.preview_hovered(-1);
        assert_eq!(hover(), None);
    }

    #[test]
    fn a_pinch_that_does_not_commit_springs_back() {
        let (orbit, window, _fakes) = opened();
        let events = window.events();
        window.pinch_progress.set(0.7);
        let frames = window.frames.borrow().len();

        events.pinch_started(720.0, 470.0);
        events.pinch_updated(1.1);
        events.pinch_ended();

        assert!(orbit.is_open());
        assert_eq!(
            window.pinch_progress.get(),
            0.0,
            "the view animates back to 0"
        );
        assert_eq!(
            window.frames.borrow().len(),
            frames,
            "a pinch that goes nowhere builds nothing"
        );
        assert_eq!(window.frame().level, OrbitLevel::Groups);
    }

    #[test]
    fn a_cancelled_pinch_springs_back() {
        let (orbit, window, _fakes) = opened();
        let events = window.events();
        window.pinch_progress.set(0.5);

        events.pinch_started(10.0, 10.0);
        events.pinch_updated(1.3);
        events.pinch_cancelled();

        assert!(orbit.is_open());
        assert_eq!(window.pinch_progress.get(), 0.0);
        assert!(orbit.inner.machine.borrow().view().pinch.is_none());
    }

    #[test]
    fn a_pinch_in_on_nothing_springs_back_and_a_pinch_out_leaves_orbit() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        // Far outside every planet and the sun.
        events.pinch_started(2.0, 2.0);
        events.pinch_updated(2.0);
        events.pinch_ended();
        assert!(
            orbit.is_open(),
            "no target under the pointer: nothing to zoom into"
        );
        assert_eq!(window.frame().level, OrbitLevel::Groups);

        // The view writes the pinch's progress while the fingers move: -1 for this pinch.
        window.pinch_progress.set(-1.0);
        events.pinch_started(720.0, 470.0);
        events.pinch_updated(0.5);
        events.pinch_ended();

        assert!(!orbit.is_open(), "pinching out of level 1 leaves Orbit");
        assert_eq!(fakes.count("close"), 1);
        assert_eq!(
            window.pinch_progress.get(),
            0.0,
            "a pinch that closes must not leave the scale at 0.92"
        );

        // Whatever the view was left with, the next Orbit opens at scale 1.
        window.pinch_progress.set(1.0);
        orbit.open();
        assert_eq!(window.pinch_progress.get(), 0.0, "reopened at scale 1");
    }

    #[test]
    fn a_pinch_in_on_the_card_that_opens_the_tab_also_resets_the_pinch_progress() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.planet_clicked(RESEARCH);
        events.preview_clicked(ACCESSKIT_GUIDE);
        assert_eq!(window.frame().level, OrbitLevel::Tab);
        let card = window.frame().card;
        let (x, y) = (card.x + card.width / 2.0, card.y + card.height / 2.0);

        window.pinch_progress.set(1.0);
        events.pinch_started(x, y);
        events.pinch_updated(1.4);
        events.pinch_ended();

        assert!(
            !orbit.is_open(),
            "pinching in on the card activates the tab"
        );
        assert_eq!(fakes.count("activate:104"), 1);
        assert_eq!(window.pinch_progress.get(), 0.0);
    }

    #[test]
    fn closing_resets_the_pinch_progress_whatever_the_view_was_left_with() {
        let (orbit, window, _fakes) = opened();
        window.pinch_progress.set(0.6);

        orbit.close();

        assert_eq!(window.pinch_progress.get(), 0.0);
    }

    #[test]
    fn rows_that_lose_the_shown_group_bring_the_view_back_to_level_one() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.planet_clicked(RESEARCH);
        assert_eq!(window.frame().level, OrbitLevel::Group);

        let mut input = fakes.input.borrow_mut();
        input.groups.retain(|group| group.id != RESEARCH);
        input.tabs.retain(|tab| tab.group != RESEARCH);
        drop(input);
        orbit.rows_changed();

        assert_eq!(window.frame().level, OrbitLevel::Groups);
        assert_eq!(window.frame().container_label, "Orbit, every group");
        assert!(orbit.inner.input.borrow().group(RESEARCH).is_none());
    }

    #[test]
    fn rows_changed_does_nothing_while_closed() {
        let (orbit, _window, fakes) = setup();

        orbit.rows_changed();
        orbit.refresh();

        assert_eq!(fakes.count("frame"), 0);
    }

    #[test]
    fn a_resize_builds_the_frame_again() {
        let (orbit, window, _fakes) = opened();
        let frames = window.frames.borrow().len();

        orbit.refresh();

        assert_eq!(window.frames.borrow().len(), frames + 1);
    }

    // ------------------------------------------------------------------------------------
    // Drag and drop: what reaches the controller
    // ------------------------------------------------------------------------------------

    fn lisbon_tabs() -> usize {
        orbit_input(false).tabs_of(LISBON).len()
    }

    #[test]
    fn a_tab_dragged_onto_another_planet_is_one_move_to_the_end_of_that_group() {
        let (_orbit, window, fakes) = opened();
        let events = window.events();
        let payload = events.payload_for("tab", ACTIVE);
        assert!(!payload.is_empty());

        events.drag_source_started("tab", ACTIVE);
        events.drag_over("group", LISBON, 700.0, 300.0);
        assert!(
            window.frame().drag.active,
            "the ghost shows during the drag"
        );
        assert!(events.payload_dropped(&payload, "group", LISBON, -1));

        assert_eq!(
            fakes.ops(),
            [GroupOp::MoveTab {
                tab: ACTIVE,
                group: LISBON,
                index: lisbon_tabs()
            }]
        );
        assert!(
            !window.frame().drag.active,
            "the ghost is gone after the drop"
        );
    }

    #[test]
    fn a_tab_dropped_on_a_ring_slot_gets_the_final_place_not_the_slot() {
        let (_orbit, window, fakes) = opened();
        let events = window.events();

        // Tab 101 is first in Research; slot 3 is "between the 3rd and 4th": once it has left
        // its place the final place is 2.
        events.ring_drag_started(RESEARCH, ACTIVE);
        events.ring_dropped(RESEARCH, 3);

        assert_eq!(
            fakes.ops(),
            [GroupOp::MoveTab {
                tab: ACTIVE,
                group: RESEARCH,
                index: 2
            }]
        );
    }

    #[test]
    fn a_tab_dropped_on_its_own_slot_changes_nothing() {
        let (_orbit, window, fakes) = opened();
        let events = window.events();

        events.ring_drag_started(RESEARCH, ACTIVE);
        events.ring_dropped(RESEARCH, 0);
        events.ring_drag_started(RESEARCH, ACTIVE);
        events.ring_dropped(RESEARCH, 1);

        assert!(fakes.ops().is_empty());
        assert!(window.frame().notice.is_empty());
        assert!(!window.frame().drag.active);
    }

    #[test]
    fn a_planet_dragged_onto_another_is_a_reorder_to_that_place() {
        let (_orbit, window, fakes) = opened();
        let events = window.events();
        let payload = events.payload_for("group", RESEARCH);

        events.drag_source_started("group", RESEARCH);
        assert!(events.payload_dropped(&payload, "group", READING, -1));

        assert_eq!(
            fakes.ops(),
            [GroupOp::ReorderGroup {
                group: RESEARCH,
                index: 2
            }]
        );
    }

    #[test]
    fn a_drop_the_resolver_refuses_is_not_taken_and_runs_nothing() {
        let (_orbit, window, fakes) = opened();
        let events = window.events();
        let tab = events.payload_for("tab", ACTIVE);
        let group = events.payload_for("group", RESEARCH);

        assert!(
            !events.payload_accepts(&tab, "group", RESEARCH, -1),
            "its own planet"
        );
        assert!(!events.payload_accepts(&group, "new-group", -1, -1));
        assert!(
            !events.payload_accepts(&group, "group", RESEARCH, -1),
            "onto itself"
        );
        assert!(events.payload_accepts(&tab, "new-group", -1, -1));
        assert!(events.payload_accepts(&tab, "empty", -1, -1));
        assert!(
            !events.payload_accepts(&tab, "nowhere", -1, -1),
            "an unknown kind"
        );
        assert!(!events.payload_dropped(&tab, "group", RESEARCH, -1));

        assert!(fakes.ops().is_empty());
    }

    #[test]
    fn only_an_orbit_payload_of_this_session_is_accepted() {
        let (_orbit, window, _fakes) = opened();
        let events = window.events();
        let forged_text = DataTransfer::from(SharedString::from("mercury-orbit:tab:101"));
        let nothing = DataTransfer::default();
        let mut foreign = DataTransfer::default();
        foreign.set_user_data(Rc::new(42_u32));
        let other_session = OrbitPayload {
            session: u64::MAX,
            source: Source::Tab(ACTIVE),
        }
        .to_data_transfer("Servo layout notes");

        for (name, data) in [
            ("forged text", forged_text),
            ("nothing", nothing),
            ("another type of user data", foreign),
            ("another session", other_session),
        ] {
            assert!(
                !events.payload_accepts(&data, "group", LISBON, -1),
                "{name}"
            );
            assert!(
                !events.payload_dropped(&data, "group", LISBON, -1),
                "{name}"
            );
        }
    }

    #[test]
    fn what_is_read_only_cannot_be_dragged() {
        let (orbit, window, fakes) = setup();
        {
            let mut input = fakes.input.borrow_mut();
            let pinned = tab_id(1, 0);
            input.pinned.push(pinned);
        }
        orbit.open();
        let events = window.events();

        assert!(
            events.payload_for("tab", tab_id(1, 0)).is_empty(),
            "a pinned tab"
        );
        assert!(
            events.payload_for("tab", 99_999).is_empty(),
            "an unknown tab"
        );
        assert!(
            events.payload_for("group", 99_999).is_empty(),
            "an unknown group"
        );
        assert!(events.payload_for("group", LOOSE_GROUP_ID).is_empty());
        assert!(
            events.payload_for("album", ACTIVE).is_empty(),
            "an unknown kind"
        );
        assert!(!events.payload_for("tab", ACTIVE).is_empty());
    }

    #[test]
    fn the_plain_text_of_a_drag_is_the_title_or_the_name() {
        let (_orbit, window, _fakes) = opened();
        let events = window.events();

        let tab = events.payload_for("tab", ACTIVE);
        let group = events.payload_for("group", RESEARCH);

        assert_eq!(tab.plain_text().ok().as_deref(), Some("Servo layout notes"));
        assert_eq!(group.plain_text().ok().as_deref(), Some("Research"));
    }

    #[test]
    fn a_ring_drag_released_over_empty_stage_is_a_new_group_with_its_popover() {
        let (orbit, window, fakes) = opened();
        let events = window.events();

        events.ring_drag_started(LISBON, tab_id(1, 2));
        events.ring_drag_moved(1.0, 1.0);
        events.ring_drag_cancelled();

        // At (1, 1) the stage is empty: the tab is dropped on "empty", a new group.
        let popover = orbit.inner.machine.borrow().view().popover;
        assert_eq!(popover.map(|popover| popover.tab), Some(tab_id(1, 2)));
        assert!(
            fakes.ops().is_empty(),
            "the group is made when the popover is confirmed"
        );
        assert!(!window.frame().drag.active);
    }

    #[test]
    fn a_ring_drag_that_escape_cancelled_is_not_dropped_afterwards() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.ring_drag_started(LISBON, tab_id(1, 2));
        events.ring_drag_moved(1.0, 1.0);

        assert!(
            orbit.handle_key(Key::Escape, Mods::NONE),
            "Esc cancels the drag"
        );
        events.ring_drag_cancelled();

        assert!(orbit.is_open());
        assert!(orbit.inner.machine.borrow().view().popover.is_none());
        assert!(fakes.ops().is_empty());
    }

    #[test]
    fn escape_during_a_drag_cancels_the_machines_drag_and_the_rings_once() {
        let (orbit, window, _fakes) = opened();
        let events = window.events();
        events.ring_drag_started(LISBON, tab_id(1, 2));
        assert_eq!(window.cancel_ring_drag.get(), 0);

        assert!(orbit.handle_key(Key::Escape, Mods::NONE));

        assert_eq!(window.cancel_ring_drag.get(), 1);
        assert!(orbit.inner.machine.borrow().view().drag.is_none());
        assert!(orbit.is_open(), "the first Esc only cancels the drag");
        events.ring_drag_cancelled();
        assert_eq!(
            window.cancel_ring_drag.get(),
            1,
            "the ring's echo is ignored"
        );
    }

    #[test]
    fn escape_without_a_drag_does_not_cancel_the_ring() {
        let (orbit, window, _fakes) = opened();
        window.events().search_edited("servo");

        assert!(orbit.handle_key(Key::Escape, Mods::NONE));

        assert!(orbit.is_open(), "Esc cleared the search");
        assert_eq!(window.cancel_ring_drag.get(), 0);
    }

    #[test]
    fn closing_cancels_the_ring_drag() {
        let (orbit, window, _fakes) = opened();
        let events = window.events();
        events.ring_drag_started(LISBON, tab_id(1, 2));

        orbit.close();

        assert_eq!(window.cancel_ring_drag.get(), 1);
        orbit.close();
        assert_eq!(
            window.cancel_ring_drag.get(),
            1,
            "closed twice, cancelled once"
        );
    }

    #[test]
    fn a_drag_that_ended_without_a_drop_or_lost_its_tab_cancels_the_ring() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.drag_source_started("tab", ACTIVE);
        events.drag_source_finished(false);
        assert_eq!(window.cancel_ring_drag.get(), 1);

        events.ring_drag_started(LISBON, tab_id(1, 2));
        fakes
            .input
            .borrow_mut()
            .tabs
            .retain(|tab| tab.id != tab_id(1, 2));
        orbit.rows_changed();

        assert!(orbit.inner.machine.borrow().view().drag.is_none());
        assert_eq!(window.cancel_ring_drag.get(), 2);
    }

    #[test]
    fn a_drag_source_that_ended_without_a_drop_is_cancelled() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.drag_source_started("tab", ACTIVE);
        events.drag_over("group", LISBON, 10.0, 10.0);
        assert!(window.frame().drag.active);

        events.drag_source_finished(false);

        assert!(orbit.inner.machine.borrow().view().drag.is_none());
        assert!(!window.frame().drag.active);
        assert!(fakes.ops().is_empty());
    }

    #[test]
    fn the_ghost_follows_the_pointer_without_a_rebuild() {
        let (_orbit, window, _fakes) = opened();
        let events = window.events();
        events.drag_source_started("tab", ACTIVE);
        events.drag_over("group", LISBON, 100.0, 200.0);
        let frames = window.frames.borrow().len();

        events.drag_over("group", LISBON, 120.0, 230.0);

        let frame = window.frame();
        assert_eq!((frame.drag.ghost_x, frame.drag.ghost_y), (120.0, 230.0));
        assert_eq!(
            window.frames.borrow().len(),
            frames + 1,
            "one set_frame, no models rebuilt"
        );
    }

    #[test]
    fn a_target_a_drop_would_change_nothing_on_is_not_lit() {
        let (orbit, window, _fakes) = opened();
        let events = window.events();
        events.drag_source_started("tab", ACTIVE);

        events.drag_over("group", RESEARCH, 1.0, 1.0);
        let over = |orbit: &OrbitController| {
            orbit
                .inner
                .machine
                .borrow()
                .view()
                .drag
                .and_then(|drag| drag.over)
        };
        assert_eq!(over(&orbit), None, "its own planet");

        events.drag_over("group", LISBON, 1.0, 1.0);
        assert_eq!(
            over(&orbit),
            Some(DropTarget::Group {
                group: LISBON,
                slot: None
            })
        );
        events.drag_over("new-group", -1, 1.0, 1.0);
        assert_eq!(over(&orbit), Some(DropTarget::NewGroup));
    }

    // ------------------------------------------------------------------------------------
    // Group commands: refusals, the popover
    // ------------------------------------------------------------------------------------

    #[test]
    fn a_refused_command_is_a_one_line_notice_and_the_next_action_clears_it() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        fakes.refuse.set(Some(GroupError::TabPinned));
        events.ring_drag_started(RESEARCH, ACTIVE);
        events.ring_dropped(RESEARCH, 5);

        assert_eq!(fakes.ops().len(), 1);
        assert_eq!(
            window.frame().notice,
            "Pinned tabs can't be put in a group."
        );
        assert!(orbit.is_open(), "a refusal never closes or panics");

        events.backdrop_clicked();
        assert_eq!(window.frame().notice, "");
    }

    #[test]
    fn a_successful_command_reads_the_rows_again() {
        let (_orbit, window, fakes) = opened();
        let events = window.events();
        assert_eq!(
            fakes.rows_calls.get(),
            1,
            "the rows are read when Orbit opens"
        );
        events.ring_drag_started(RESEARCH, ACTIVE);
        let before = fakes.count("frame");

        events.ring_dropped(RESEARCH, 5);
        assert_eq!(fakes.ops().len(), 1);
        assert_eq!(fakes.rows_calls.get(), 2, "and again after the command");
        assert!(fakes.count("frame") > before, "the view was built again");

        fakes.refuse.set(Some(GroupError::NotFound));
        events.ring_drag_started(RESEARCH, ACTIVE);
        events.ring_dropped(RESEARCH, 6);
        assert_eq!(fakes.ops().len(), 2);
        assert_eq!(
            fakes.rows_calls.get(),
            2,
            "a refused command changes nothing to read"
        );
    }

    #[test]
    fn the_new_group_popover_makes_a_group_with_the_name_and_the_chosen_colour() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        let payload = events.payload_for("tab", tab_id(1, 2));
        events.drag_source_started("tab", tab_id(1, 2));
        assert!(events.payload_dropped(&payload, "new-group", -1, -1));
        assert!(orbit.inner.machine.borrow().view().popover.is_some());

        events.popover_name_edited("Plans");
        let frames = window.frames.borrow().len();
        events.popover_name_edited("Plans 2");
        assert_eq!(
            window.frames.borrow().len(),
            frames,
            "typing builds no frame"
        );
        events.popover_colour_chosen(1);
        events.popover_colour_chosen(99);
        events.popover_confirmed();

        assert_eq!(
            fakes.ops(),
            [GroupOp::CreateGroup {
                tabs: vec![tab_id(1, 2)],
                name: Some("Plans 2".to_owned()),
                colour: Some("amber".to_owned()),
            }]
        );
        assert!(orbit.inner.machine.borrow().view().popover.is_none());
    }

    #[test]
    fn cancelling_the_popover_makes_no_group() {
        let (orbit, window, fakes) = opened();
        let events = window.events();
        events.new_group_clicked();
        assert!(orbit.inner.machine.borrow().view().popover.is_some());

        events.popover_cancelled();

        assert!(orbit.inner.machine.borrow().view().popover.is_none());
        assert!(orbit.is_open());
        assert!(fakes.ops().is_empty());
    }

    #[test]
    fn the_rings_forward_their_hit_tests_to_the_ring_data() {
        let (orbit, window, _fakes) = opened();
        let events = window.events();
        let data = orbit
            .inner
            .data
            .borrow()
            .clone()
            .expect("a frame was built");

        assert_eq!(
            events.ring_hit_test(9999, 10.0, 10.0, -1).kind,
            RingHitKind::Outside,
            "a ring that is not shown"
        );
        assert!(!events.ring_drop_target(9999, 10.0, 10.0, -1).valid);
        for planet in &data.planets {
            for (x, y) in [(5.0, 5.0), (60.0, 20.0), (planet.row.ring_size / 2.0, 12.0)] {
                let direct = planet.ring.hit(x, y, -1);
                let forwarded = events.ring_hit_test(planet.row.group, x, y, -1);
                assert_eq!((forwarded.kind, forwarded.id), (direct.kind, direct.id));
                let direct = planet.ring.drop_target(x, y, ACTIVE);
                let forwarded = events.ring_drop_target(planet.row.group, x, y, ACTIVE);
                assert_eq!(
                    (forwarded.valid, forwarded.group, forwarded.index),
                    (direct.valid, direct.group, direct.index)
                );
            }
        }
    }

    // ------------------------------------------------------------------------------------
    // The notices (the pure helpers that moved to `mercury-ui` are tested there)
    // ------------------------------------------------------------------------------------

    #[test]
    fn every_refusal_has_its_own_plain_sentence() {
        let errors = [
            GroupError::NotFound,
            GroupError::TabNotFound,
            GroupError::TabPinned,
            GroupError::NoTabs,
            GroupError::TooManyGroups,
            GroupError::EmptyName,
            GroupError::KeyTaken {
                holder: GroupId::new(1),
            },
            GroupError::IdsExhausted,
        ];
        let texts: Vec<String> = errors.iter().map(|error| notice_text(*error)).collect();

        for text in &texts {
            assert!(text.ends_with('.') && text.chars().next().is_some_and(char::is_uppercase));
        }
        assert_eq!(texts[2], "Pinned tabs can't be put in a group.");
        assert_eq!(texts[4], "A window holds at most 64 groups.");
        assert_eq!(texts[0], texts[1], "both say that the thing is gone");
        let mut unique = texts.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), texts.len() - 1);
    }

    // ------------------------------------------------------------------------------------
    // The real controller: the glue's mapping against real group operations
    // ------------------------------------------------------------------------------------

    struct TestStyle;

    impl RowStyle for TestStyle {
        fn dark(&self) -> bool {
            false
        }

        fn swatch(&self, _colour: GroupColour) -> GroupSwatch {
            GroupSwatch {
                colour: Color::from_rgb_u8(0x11, 0x22, 0x33),
                tint: Color::from_rgb_u8(0xEE, 0xEE, 0xEE),
                deep: Color::from_rgb_u8(0x01, 0x02, 0x03),
            }
        }

        fn shade(&self, _colour: GroupColour, index: usize) -> Color {
            Color::from_rgb_u8(index as u8, 0, 0)
        }
    }

    /// Six tabs: Research holds the first three, Lisbon the next two, the last one is loose.
    fn real_controller() -> (BrowserController, Vec<TabId>, GroupId, GroupId) {
        let mut controller = BrowserController::default();
        let mut tabs = vec![controller.active_tab_id()];
        for _ in 1..6 {
            tabs.push(
                controller
                    .open_tab_with(None, TabActivation::Background)
                    .expect("a blank tab opens"),
            );
        }
        let now = SystemTime::now();
        let research = controller
            .create_group(&tabs[0..3], Some("Research"), None, now)
            .expect("Research is made");
        let lisbon = controller
            .create_group(&tabs[3..5], Some("Lisbon"), None, now)
            .expect("Lisbon is made");
        (controller, tabs, research, lisbon)
    }

    fn ui(group: GroupId) -> i32 {
        group_id_for_ui(group).expect("a small group id")
    }

    fn apply(controller: &mut BrowserController, op: &GroupOp) -> Result<(), GroupError> {
        apply_group_op(controller, op, SystemTime::now())
    }

    #[test]
    fn a_move_between_two_groups_is_one_controller_move_to_the_final_place() {
        let (mut controller, tabs, research, lisbon) = real_controller();

        apply(
            &mut controller,
            &GroupOp::MoveTab {
                tab: id_for_ui(tabs[0]),
                group: ui(lisbon),
                index: 1,
            },
        )
        .expect("the move is allowed");

        assert_eq!(controller.group_tabs(lisbon), [tabs[3], tabs[0], tabs[4]]);
        assert_eq!(controller.group_tabs(research), [tabs[1], tabs[2]]);
    }

    #[test]
    fn a_move_inside_a_group_puts_the_tab_at_the_final_index() {
        let (mut controller, tabs, research, _lisbon) = real_controller();

        apply(
            &mut controller,
            &GroupOp::MoveTab {
                tab: id_for_ui(tabs[0]),
                group: ui(research),
                index: 2,
            },
        )
        .expect("the move is allowed");

        assert_eq!(controller.group_tabs(research), [tabs[1], tabs[2], tabs[0]]);
    }

    #[test]
    fn a_tab_moved_to_the_loose_planet_leaves_its_group() {
        let (mut controller, tabs, _research, lisbon) = real_controller();

        apply(
            &mut controller,
            &GroupOp::Ungroup {
                tab: id_for_ui(tabs[3]),
            },
        )
        .expect("the tab is ungrouped");

        assert_eq!(controller.group_of(tabs[3]), None);
        assert_eq!(controller.group_tabs(lisbon), [tabs[4]]);
    }

    #[test]
    fn a_reordered_group_takes_its_new_place() {
        let (mut controller, _tabs, research, lisbon) = real_controller();
        let order = |controller: &BrowserController| {
            controller
                .groups()
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&controller), [research, lisbon]);

        apply(
            &mut controller,
            &GroupOp::ReorderGroup {
                group: ui(research),
                index: 1,
            },
        )
        .expect("the reorder is allowed");

        assert_eq!(order(&controller), [lisbon, research]);
    }

    #[test]
    fn a_new_group_from_a_tab_has_its_name_and_colour() {
        let (mut controller, tabs, research, lisbon) = real_controller();

        apply(
            &mut controller,
            &GroupOp::CreateGroup {
                tabs: vec![id_for_ui(tabs[5])],
                name: Some("Fresh".to_owned()),
                colour: Some("rose".to_owned()),
            },
        )
        .expect("the group is made");

        let fresh = controller.group_of(tabs[5]).expect("the tab is in a group");
        assert!(fresh != research && fresh != lisbon);
        let group = controller.group(fresh).expect("the group exists");
        assert_eq!(group.name, "Fresh");
        assert_eq!(group.colour.to_name(), "rose");
        assert_eq!(controller.groups().len(), 3);
    }

    #[test]
    fn a_new_group_with_no_name_or_an_unknown_colour_takes_the_controllers_defaults() {
        let (mut controller, tabs, _research, _lisbon) = real_controller();

        apply(
            &mut controller,
            &GroupOp::CreateGroup {
                tabs: vec![id_for_ui(tabs[5])],
                name: None,
                colour: Some("mauve".to_owned()),
            },
        )
        .expect("the group is made");

        let fresh = controller.group_of(tabs[5]).expect("the tab is in a group");
        let group = controller.group(fresh).expect("the group exists");
        assert!(!group.name.is_empty());
        assert_ne!(group.colour.to_name(), "mauve");
    }

    #[test]
    fn refusals_come_back_as_the_controllers_errors_and_change_nothing() {
        let (mut controller, tabs, research, lisbon) = real_controller();
        assert!(controller.set_tab_pinned(tabs[5], true));

        let pinned = apply(
            &mut controller,
            &GroupOp::MoveTab {
                tab: id_for_ui(tabs[5]),
                group: ui(lisbon),
                index: 0,
            },
        );
        let unknown_tab = apply(
            &mut controller,
            &GroupOp::MoveTab {
                tab: 99_999,
                group: ui(lisbon),
                index: 0,
            },
        );
        let negative_tab = apply(&mut controller, &GroupOp::Ungroup { tab: -3 });
        let unknown_group = apply(
            &mut controller,
            &GroupOp::MoveTab {
                tab: id_for_ui(tabs[0]),
                group: 77,
                index: 0,
            },
        );
        let negative_group = apply(
            &mut controller,
            &GroupOp::ReorderGroup {
                group: -2,
                index: 0,
            },
        );
        let pinned_group = apply(
            &mut controller,
            &GroupOp::CreateGroup {
                tabs: vec![id_for_ui(tabs[5])],
                name: None,
                colour: None,
            },
        );

        assert_eq!(pinned, Err(GroupError::TabPinned));
        assert_eq!(unknown_tab, Err(GroupError::TabNotFound));
        assert_eq!(negative_tab, Err(GroupError::TabNotFound));
        assert_eq!(unknown_group, Err(GroupError::NotFound));
        assert_eq!(negative_group, Err(GroupError::NotFound));
        assert_eq!(pinned_group, Err(GroupError::TabPinned));
        assert_eq!(controller.group_tabs(research).len(), 3);
        assert_eq!(controller.group_tabs(lisbon).len(), 2);
        assert_eq!(controller.groups().len(), 2);
    }

    #[test]
    fn the_rows_come_from_the_controller_with_the_loose_planet_last() {
        let (mut controller, tabs, research, _lisbon) = real_controller();
        assert!(controller.handle_event(
            tabs[0],
            BrowserEvent::AddressChanged {
                url: "https://docs.example.org/guide".into()
            }
        ));
        assert!(controller.handle_event(
            tabs[0],
            BrowserEvent::PageTitleChanged {
                title: Some("Notes".into())
            }
        ));
        controller
            .set_group_key(research, GroupKey::from_letter('r'))
            .expect("the key is free");
        let now = SystemTime::now();

        let input = rows_from_controller(&controller, &TestStyle, "Personal", "Window 1", now);

        let names: Vec<_> = input
            .groups
            .iter()
            .map(|group| group.name.to_string())
            .collect();
        assert_eq!(names, ["Research", "Lisbon", "Loose tabs"]);
        let research_row = &input.groups[0];
        assert_eq!(research_row.id, ui(research));
        assert_eq!(research_row.key_label, "R");
        assert_eq!(research_row.count, 3);
        assert!(research_row.active, "the active tab is in Research");
        assert!(!input.groups[1].active);
        assert_eq!(research_row.last_used, "now");
        assert_eq!(research_row.tint, Color::from_rgb_u8(0xEE, 0xEE, 0xEE));
        assert_eq!(input.groups[2].id, LOOSE_GROUP_ID);
        assert_eq!(input.groups[2].count, 1);
        assert_eq!(
            (input.profile_name.as_str(), input.window_label.as_str()),
            ("Personal", "Window 1")
        );

        assert_eq!(input.tabs.len(), 6);
        let first = input.tab(id_for_ui(tabs[0])).expect("the first tab");
        assert_eq!(first.group, ui(research));
        assert_eq!(first.index_in_group, 0);
        assert_eq!(first.title, "Notes");
        assert_eq!(first.host, "docs.example.org");
        assert!(first.active);
        let third = input.tab(id_for_ui(tabs[2])).expect("the third tab");
        assert_eq!(third.index_in_group, 2);
        assert_eq!(
            third.shade,
            Color::from_rgb_u8(2, 0, 0),
            "the shade follows the place"
        );
        let loose = input.tab(id_for_ui(tabs[5])).expect("the loose tab");
        assert_eq!(loose.group, LOOSE_GROUP_ID);
        assert!(input.pinned.is_empty());
        assert_eq!(input.active_tab, id_for_ui(tabs[0]));
    }

    #[test]
    fn a_pinned_tab_is_in_the_loose_planet_and_read_only() {
        let (mut controller, tabs, _research, _lisbon) = real_controller();
        assert!(controller.set_tab_pinned(tabs[5], true));

        let input = rows_from_controller(
            &controller,
            &TestStyle,
            "Personal",
            "Window 1",
            SystemTime::now(),
        );

        assert_eq!(input.pinned, [id_for_ui(tabs[5])]);
        assert!(input.is_pinned(id_for_ui(tabs[5])));
        let row = input.tab(id_for_ui(tabs[5])).expect("the pinned tab");
        assert_eq!(row.group, LOOSE_GROUP_ID);
        assert!(!row.pinned, "ring-bands draws it in Orbit's copy");
    }

    /// The glue, its window, the real controller, its tabs, the two groups and how many times the
    /// window was told that the controller changed.
    type RealOrbit = (
        OrbitController,
        Rc<FakeWindow>,
        Rc<RefCell<BrowserController>>,
        Vec<TabId>,
        (GroupId, GroupId),
        Rc<Cell<u32>>,
    );

    /// The glue on a real controller, through the view's drag callbacks.
    fn real_orbit() -> RealOrbit {
        crate::settings::tests::init_slint();
        let (controller, tabs, research, lisbon) = real_controller();
        let controller = Rc::new(RefCell::new(controller));
        let changed = Rc::new(Cell::new(0));
        let counted = Rc::clone(&changed);
        let host = OrbitHost::for_desktop(DesktopHost {
            controller: Rc::clone(&controller),
            rendering_context: Box::new(|| None),
            style: Rc::new(TestStyle),
            profile_name: "Personal".to_owned(),
            window_label: "Window 1".to_owned(),
            activate: Box::new(|_| {}),
            close: Box::new(|| {}),
            controller_changed: Box::new(move || counted.set(counted.get() + 1)),
            environment: Box::new(Environment::default),
        });
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let window = FakeWindow::new(&log);
        let orbit = OrbitController::new(host, window.clone());
        orbit.open();
        (orbit, window, controller, tabs, (research, lisbon), changed)
    }

    #[test]
    fn dragging_a_tab_to_another_planet_moves_it_in_the_real_controller() {
        let (orbit, window, controller, tabs, (research, lisbon), changed) = real_orbit();
        let events = window.events();
        let dragged = id_for_ui(tabs[0]);
        let payload = events.payload_for("tab", dragged);
        assert!(!payload.is_empty());

        events.drag_source_started("tab", dragged);
        assert!(events.payload_dropped(&payload, "group", ui(lisbon), -1));

        let controller = controller.borrow();
        assert_eq!(controller.group_tabs(lisbon), [tabs[3], tabs[4], tabs[0]]);
        assert_eq!(controller.group_tabs(research), [tabs[1], tabs[2]]);
        assert_eq!(changed.get(), 1, "the window is told once");
        let input = orbit.inner.input.borrow();
        assert_eq!(input.tab(dragged).map(|tab| tab.group), Some(ui(lisbon)));
        assert_eq!(input.group(ui(research)).map(|group| group.count), Some(2));
        assert!(orbit.is_open());
    }

    #[test]
    fn dropping_a_tab_on_a_ring_slot_reorders_inside_its_group_in_the_real_controller() {
        let (_orbit, window, controller, tabs, (research, _lisbon), _changed) = real_orbit();
        let events = window.events();

        events.ring_drag_started(ui(research), id_for_ui(tabs[0]));
        events.ring_dropped(ui(research), 3);

        assert_eq!(
            controller.borrow().group_tabs(research),
            [tabs[1], tabs[2], tabs[0]]
        );
    }

    #[test]
    fn dragging_a_planet_reorders_the_groups_in_the_real_controller() {
        let (_orbit, window, controller, _tabs, (research, lisbon), _changed) = real_orbit();
        let events = window.events();
        let payload = events.payload_for("group", ui(research));

        events.drag_source_started("group", ui(research));
        assert!(events.payload_dropped(&payload, "group", ui(lisbon), -1));

        let order: Vec<_> = controller
            .borrow()
            .groups()
            .iter()
            .map(|group| group.id)
            .collect();
        assert_eq!(order, [lisbon, research]);
    }

    #[test]
    fn a_tab_dropped_on_empty_stage_becomes_a_new_group_once_the_popover_is_confirmed() {
        let (orbit, window, controller, tabs, _groups, changed) = real_orbit();
        let events = window.events();
        let dragged = id_for_ui(tabs[4]);
        let payload = events.payload_for("tab", dragged);

        events.drag_source_started("tab", dragged);
        assert!(events.payload_dropped(&payload, "empty", -1, -1));
        assert_eq!(
            controller.borrow().groups().len(),
            2,
            "not before the popover is confirmed"
        );
        events.popover_name_edited("Fresh");
        events.popover_colour_chosen(3);
        events.popover_confirmed();

        let controller = controller.borrow();
        assert_eq!(controller.groups().len(), 3);
        let fresh = controller.group_of(tabs[4]).expect("the tab is in a group");
        let group = controller.group(fresh).expect("the new group");
        assert_eq!(
            (group.name.as_str(), group.colour.to_name().as_str()),
            ("Fresh", "rose")
        );
        assert_eq!(changed.get(), 1);
        assert_eq!(
            orbit.inner.input.borrow().groups.len(),
            4,
            "Research, Lisbon, the new group and the loose planet"
        );
    }

    #[test]
    fn dragging_a_tab_onto_the_loose_planet_ungroups_it() {
        let (_orbit, window, controller, tabs, (_research, lisbon), _changed) = real_orbit();
        let events = window.events();
        let dragged = id_for_ui(tabs[3]);
        let payload = events.payload_for("tab", dragged);

        events.drag_source_started("tab", dragged);
        assert!(events.payload_dropped(&payload, "group", LOOSE_GROUP_ID, -1));

        assert_eq!(controller.borrow().group_of(tabs[3]), None);
        assert_eq!(controller.borrow().group_tabs(lisbon), [tabs[4]]);
    }

    #[test]
    fn pinned_tabs_and_the_loose_planet_cannot_be_dragged_in_the_real_controller() {
        let (orbit, window, controller, tabs, _groups, _changed) = real_orbit();
        let events = window.events();
        assert!(controller.borrow_mut().set_tab_pinned(tabs[5], true));
        orbit.rows_changed();

        assert!(events.payload_for("tab", id_for_ui(tabs[5])).is_empty());
        assert!(events.payload_for("group", LOOSE_GROUP_ID).is_empty());
        assert!(!events.payload_for("tab", id_for_ui(tabs[0])).is_empty());
    }

    #[test]
    fn a_tab_that_closes_while_orbit_is_open_leaves_the_rows_and_the_picture_cache() {
        let (orbit, _window, controller, tabs, _groups, _changed) = real_orbit();
        let gone = id_for_ui(tabs[2]);
        orbit.inner.images.borrow_mut().insert(gone, None);
        orbit
            .inner
            .images
            .borrow_mut()
            .insert(id_for_ui(tabs[0]), None);
        let outcome = controller.borrow_mut().close_tab(tabs[2]);
        assert!(!matches!(outcome, talaria_core::CloseTabOutcome::NotFound));

        orbit.rows_changed();

        assert!(orbit.inner.input.borrow().tab(gone).is_none());
        assert!(!orbit.inner.images.borrow().contains_key(&gone));
        assert!(
            orbit
                .inner
                .images
                .borrow()
                .contains_key(&id_for_ui(tabs[0]))
        );
    }

    // ------------------------------------------------------------------------------------
    // The real view: `OrbitPreview` implements the window trait the way layout-switch does
    // ------------------------------------------------------------------------------------

    /// `$window.$register(move |args| { <guard>; $call })` with the events cloned in.
    macro_rules! wire {
        ($window:expr, $events:ident, $register:ident, || $call:expr) => {{
            let $events = $events.clone();
            $window.$register(move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                $call
            });
        }};
        ($window:expr, $events:ident, $register:ident, |$($arg:ident),+| $call:expr) => {{
            let $events = $events.clone();
            $window.$register(move |$($arg),+| {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                $call
            });
        }};
    }

    impl OrbitWindow for OrbitPreview {
        fn stage(&self) -> Stage {
            let size = self.window().size();
            let scale = self.window().scale_factor();
            if size.width == 0 || size.height == 0 {
                return Stage::MOCKUP;
            }
            Stage::new(size.width as f32 / scale, size.height as f32 / scale)
        }

        fn ring_colours(&self) -> RingColours {
            RingColours::from_palette(&self.global::<mercury_ui::ChromePalette>())
        }

        fn set_models(&self, models: &OrbitModels) {
            self.set_planets(models.planets());
            self.set_previews(models.previews());
            self.set_chips(models.chips());
            self.set_neighbours(models.neighbours());
            self.set_crumbs(models.crumbs());
        }

        fn set_frame(&self, frame: &Frame) {
            self.set_level(frame.level);
            self.set_centre(frame.centre.clone());
            self.set_has_centre(frame.has_centre);
            self.set_more_previews(frame.more_previews);
            self.set_card(frame.card.clone());
            self.set_sun(frame.sun.clone());
            self.set_new_group(frame.new_group.clone());
            self.set_drag(frame.drag.clone());
            self.set_popover(frame.popover.clone());
            self.set_query_active(frame.query_active);
            self.set_focus_tab(frame.focus_tab);
            self.set_container_label(frame.container_label.clone());
            self.set_notice(frame.notice.clone());
            self.set_reduce_motion(frame.reduce_motion);
            self.set_animate_spinners(frame.animate_spinners);
            self.set_search_placeholder(frame.search_placeholder.clone());
        }

        fn set_search_text(&self, text: &str) {
            OrbitPreview::set_search_text(self, text.into());
        }

        fn set_pinch_progress(&self, progress: f32) {
            OrbitPreview::set_pinch_progress(self, progress);
        }

        fn focus_search(&self) {
            self.invoke_focus_search();
        }

        fn focus_view(&self) {
            self.invoke_focus_view();
        }

        fn text_field_focused(&self) -> bool {
            self.get_text_field_focused()
        }

        fn play_open(&self) {
            self.invoke_play_open();
        }

        fn cancel_ring_drag(&self) {
            self.invoke_cancel_ring_drag();
        }

        fn connect(&self, events: OrbitEvents) {
            wire!(self, events, on_planet_hovered, |group| events
                .planet_hovered(group));
            wire!(self, events, on_planet_clicked, |group| events
                .planet_clicked(group));
            wire!(self, events, on_preview_hovered, |tab| events
                .preview_hovered(tab));
            wire!(self, events, on_preview_clicked, |tab| events
                .preview_clicked(tab));
            wire!(self, events, on_chip_clicked, |group| events
                .chip_clicked(group));
            wire!(self, events, on_card_clicked, || events.card_clicked());
            wire!(self, events, on_neighbour_clicked, |tab| events
                .neighbour_clicked(tab));
            wire!(self, events, on_backdrop_clicked, || events
                .backdrop_clicked());
            wire!(self, events, on_new_group_clicked, || events
                .new_group_clicked());
            wire!(self, events, on_back_clicked, || events.back_clicked());
            wire!(self, events, on_search_edited, |text| events
                .search_edited(&text));
            wire!(
                self,
                events,
                on_key_pressed,
                |text, command, shift, alt, control| {
                    events.key_pressed(&text, command, shift, alt, control)
                }
            );
            wire!(self, events, on_pinch_started, |x, y| events
                .pinch_started(x, y));
            wire!(self, events, on_pinch_updated, |scale| events
                .pinch_updated(scale));
            wire!(self, events, on_pinch_ended, || events.pinch_ended());
            wire!(self, events, on_pinch_cancelled, || events
                .pinch_cancelled());
            wire!(self, events, on_ring_drag_started, |group, tab| {
                events.ring_drag_started(group, tab)
            });
            wire!(self, events, on_ring_drag_moved, |x, y| events
                .ring_drag_moved(x, y));
            wire!(self, events, on_ring_dropped, |group, slot| events
                .ring_dropped(group, slot));
            wire!(self, events, on_ring_drag_cancelled, || events
                .ring_drag_cancelled());
            wire!(self, events, on_popover_name_edited, |name| events
                .popover_name_edited(&name));
            wire!(self, events, on_popover_colour_chosen, |index| {
                events.popover_colour_chosen(index)
            });
            wire!(self, events, on_popover_confirmed, || events
                .popover_confirmed());
            wire!(self, events, on_popover_cancelled, || events
                .popover_cancelled());
            wire!(self, events, on_ring_hit_test, |group, x, y, hovered| {
                events.ring_hit_test(group, x, y, hovered)
            });
            wire!(
                self,
                events,
                on_ring_drop_target,
                |group, x, y, dragging| events.ring_drop_target(group, x, y, dragging)
            );
            wire!(self, events, on_payload_for, |kind, id| events
                .payload_for(&kind, id));
            wire!(self, events, on_payload_accepts, |data, kind, id, slot| {
                events.payload_accepts(&data, &kind, id, slot)
            });
            wire!(self, events, on_payload_dropped, |data, kind, id, slot| {
                events.payload_dropped(&data, &kind, id, slot)
            });
            wire!(self, events, on_drag_over, |kind, id, x, y| events
                .drag_over(&kind, id, x, y));
            wire!(self, events, on_drag_source_started, |kind, id| {
                events.drag_source_started(&kind, id)
            });
            wire!(self, events, on_drag_source_finished, |accepted| {
                events.drag_source_finished(accepted)
            });
        }
    }

    fn preview_orbit() -> (OrbitController, Rc<OrbitPreview>, Fakes) {
        crate::settings::tests::init_slint();
        let (fakes, host) = fake_host();
        let preview = Rc::new(OrbitPreview::new().expect("the preview window"));
        let orbit = OrbitController::new(host, preview.clone());
        orbit.open();
        (orbit, preview, fakes)
    }

    #[test]
    fn the_views_callbacks_reach_the_machine_and_the_view_follows() {
        let (orbit, preview, fakes) = preview_orbit();
        assert_eq!(preview.get_level(), OrbitLevel::Groups);
        assert_eq!(preview.get_container_label(), "Orbit, every group");

        preview.invoke_planet_clicked(RESEARCH);
        assert_eq!(preview.get_level(), OrbitLevel::Group);
        assert_eq!(preview.get_container_label(), "Orbit, Research, 10 tabs");
        assert_eq!(preview.get_focus_tab(), ACTIVE);

        preview.invoke_preview_clicked(ACCESSKIT_GUIDE);
        assert_eq!(preview.get_level(), OrbitLevel::Tab);

        let up = char::from(slint::platform::Key::UpArrow).to_string();
        assert!(preview.invoke_key_pressed(up.into(), true, false, false, false));
        assert_eq!(preview.get_level(), OrbitLevel::Group);

        preview.invoke_search_edited("servo".into());
        assert!(preview.get_query_active());
        assert!(preview.invoke_key_pressed("\u{1b}".into(), false, false, false, false));
        assert!(!preview.get_query_active());
        assert_eq!(preview.get_search_text(), "");
        assert!(orbit.is_open());

        preview.invoke_back_clicked();
        assert!(!orbit.is_open());
        assert_eq!(fakes.count("close"), 1);
    }

    #[test]
    fn opening_plays_the_views_open_animation_and_the_frame_sets_the_search_placeholder() {
        let (orbit, preview, _fakes) = preview_orbit();

        assert!(!preview.get_search_placeholder().is_empty());
        assert!(!preview.text_field_focused(), "nothing is focused yet");
        assert!(
            preview.get_open_progress() < 1.0,
            "the 320 ms animation has started"
        );
        elapse(Duration::from_millis(400));
        assert!(preview.get_open_progress() > 0.999);
        assert!(
            orbit.handle_key(Key::N, Mods::NONE),
            "no text field: N is Orbit's"
        );
    }

    #[test]
    fn the_views_payload_callbacks_answer_for_tabs_and_planets() {
        let (_orbit, preview, fakes) = preview_orbit();

        let tab = preview.invoke_payload_for("tab".into(), ACTIVE);
        assert!(preview.invoke_payload_accepts(tab.clone(), "group".into(), LISBON, -1));
        assert!(!preview.invoke_payload_accepts(tab.clone(), "group".into(), RESEARCH, -1));
        assert!(preview.invoke_payload_for("tab".into(), 99_999).is_empty());
        let text_only = DataTransfer::from(SharedString::from("mercury-orbit:tab:101"));
        assert!(!preview.invoke_payload_accepts(text_only, "group".into(), LISBON, -1));

        preview.invoke_drag_source_started("tab".into(), ACTIVE);
        assert!(preview.invoke_payload_dropped(tab, "group".into(), LISBON, -1));
        assert_eq!(fakes.ops().len(), 1);
    }

    #[test]
    fn the_views_ring_callbacks_answer_outside_for_a_ring_that_is_not_shown() {
        let (_orbit, preview, _fakes) = preview_orbit();

        assert_eq!(
            preview.invoke_ring_hit_test(9999, 10.0, 10.0, -1).kind,
            RingHitKind::Outside
        );
        assert!(!preview.invoke_ring_drop_target(9999, 10.0, 10.0, -1).valid);
    }

    #[test]
    fn the_view_gets_a_planet_per_group_and_the_previews_of_the_shown_group() {
        let (_orbit, preview, _fakes) = preview_orbit();
        assert_eq!(preview.get_planets().row_count(), 5);

        preview.invoke_planet_clicked(RESEARCH);

        assert_eq!(preview.get_previews().row_count(), 10);
        assert!(preview.get_has_centre());
    }

    #[test]
    fn level_three_shows_the_card_of_the_tab_with_its_picture() {
        let (_orbit, preview, _fakes) = preview_orbit();
        preview.invoke_planet_clicked(RESEARCH);

        preview.invoke_preview_clicked(ACCESSKIT_GUIDE);

        let card = preview.get_card();
        assert!(card.visible);
        assert_eq!(card.tab, ACCESSKIT_GUIDE);
        assert!(card.has_picture, "the host's picture reached the card");
    }

    #[test]
    fn a_planets_ring_answers_hit_tests_with_its_tabs() {
        let (orbit, preview, _fakes) = preview_orbit();
        let data = orbit
            .inner
            .data
            .borrow()
            .clone()
            .expect("a frame was built");
        let planet = data.planets.first().expect("a planet");
        assert!(
            !planet.ring.segments.is_empty(),
            "the finished ring-bands draws segments"
        );

        let tab = planet.ring.tabs.first().expect("a tab in the ring").id;
        let mid = &planet.ring.segments[0];
        let hit = preview.invoke_ring_hit_test(planet.row.group, mid.mid_x, mid.mid_y, -1);

        assert_eq!((hit.kind, hit.id), (RingHitKind::Tab, tab));
    }
}
