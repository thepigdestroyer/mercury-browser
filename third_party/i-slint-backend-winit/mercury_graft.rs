// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: new file (lane a11y-1, item 1). Not part of the published crate. It is Mercury's own
// code and copies no Slint source, so it is under Mercury's licence, PolyForm (see MERCURY.md); every
// other file of this crate keeps its Slint notices. The glue that hangs a web page's AccessKit tree from one node of
// the window's tree. The logic that decides *when* a subtree may be pushed is `mercury_a11y::graft`
// (a Mercury crate, tested without winit); this file only calls it. The idea of a graft queue is
// servoshell's (`ports/servoshell/desktop/gui.rs`, v0.8.0, MPL-2.0); no line of it is used.

//! Hangs a page's accessibility tree from the window's tree.
//!
//! The embedder (Mercury's desktop app) marks one node of the window's tree, by role and label,
//! as the *page node* ([`MercuryPageAccessibility::set_page_graft`]), tells the window which
//! tree the engine built for the shown tab ([`set_page_tree`](MercuryPageAccessibility::set_page_tree)),
//! and hands it that tree's updates ([`push_page_update`](MercuryPageAccessibility::push_page_update)).
//! The window pushes them to the platform's accessibility adapter in the order AccessKit
//! accepts (graft before subtree), routes the actions of assistive technology that target the
//! page back to the embedder, and says when assistive technology attaches or detaches.
//!
//! A page's tree is the page's text: nothing here logs a node, and every call that hands page
//! data to AccessKit is guarded, so that a tree AccessKit refuses switches the page off for the
//! window and does not take the application down.
//!
//! Nothing here runs unless the embedder calls it: a window that never does behaves as the
//! published backend.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeUpdate};
use i_slint_core::api::Window;
use i_slint_core::input::FocusReason;
use i_slint_core::items::ItemRc;
use i_slint_core::timers::Timer;
use i_slint_core::window::WindowInner;
use mercury_a11y::graft::{Drained, GraftState, Offered, Route, guarded};

use super::{AccessKitAdapter, DeferredAccessKitAction, NodeCollection};
use crate::WinitWindowAdapter;

mod sealed {
    pub trait Sealed {}
}

/// What the embedder can do with a window's page accessibility. Implemented for
/// [`slint::Window`](i_slint_core::api::Window) only (the trait is sealed); every method does
/// nothing when the window is not backed by this backend. The calls may be made before the
/// window's winit window exists: the settings are kept and used when it does.
pub trait MercuryPageAccessibility: sealed::Sealed {
    /// The node of the window's tree the page hangs from: the one with this role and this label
    /// (the first in tree order if several match).
    fn set_page_graft(&self, role: Role, label: &str);

    /// The tree of the page that is shown now (`None`: no page). The previous one is forgotten:
    /// updates it still has queued are dropped, late ones refused.
    fn set_page_tree(&self, tree: Option<TreeId>);

    /// An update of the page's trees (the web view's, the document's, an iframe's: each by its
    /// own `tree_id`). Kept until AccessKit may take it; dropped unless assistive technology is
    /// attached. Never reaches the adapter before the graft that names its tree.
    fn push_page_update(&self, update: TreeUpdate);

    /// Called with `true` when assistive technology attached to the window (its first request
    /// for the tree), with `false` when it detached. A second `true` without a `false` between
    /// is delivered as `false`, `true`. Also `false`, `true` when the window dropped the page's
    /// updates it could not place (the page must be activated again), and `false` when it
    /// switched the page off after AccessKit refused a tree. Called from the event loop, never
    /// from inside the adapter.
    fn on_activation_changed(&self, callback: impl Fn(bool) + 'static);

    /// Called with an action request that assistive technology made for a node of the page's
    /// trees (`target_tree` is not the window's own). A `Focus` request also moves the window's
    /// keyboard focus to the page node first. Called from the event loop, never from inside
    /// the adapter.
    fn on_foreign_action(&self, callback: impl Fn(ActionRequest) + 'static);
}

impl sealed::Sealed for Window {}

impl MercuryPageAccessibility for Window {
    fn set_page_graft(&self, role: Role, label: &str) {
        with_page(self, |page| {
            page.inner.borrow_mut().marker = Some((role, label.to_owned()));
            run_later(page, |adapter| adapter.reload_tree());
        });
    }

    fn set_page_tree(&self, tree: Option<TreeId>) {
        with_page(self, |page| {
            {
                let mut inner = page.inner.borrow_mut();
                if inner.tree == tree {
                    return;
                }
                if let Some(old) = inner.tree.take() {
                    inner.graft.forget_tree(old);
                }
                inner.tree = tree;
                inner.graft_node = None;
            }
            run_later(page, |adapter| adapter.reload_tree());
        });
    }

    fn push_page_update(&self, update: TreeUpdate) {
        with_page(self, |page| {
            let queued = {
                let mut inner = page.inner.borrow_mut();
                inner.attached && !inner.disabled && inner.graft.offer(update) == Offered::Queued
            };
            if queued {
                schedule_flush(page);
            }
        });
    }

    fn on_activation_changed(&self, callback: impl Fn(bool) + 'static) {
        with_page(self, |page| page.inner.borrow_mut().on_activation = Some(Rc::new(callback)));
    }

    fn on_foreign_action(&self, callback: impl Fn(ActionRequest) + 'static) {
        with_page(self, |page| page.inner.borrow_mut().on_action = Some(Rc::new(callback)));
    }
}

/// Everything a window keeps for its page: it lives as long as the window adapter, so it can be
/// set before the winit window (and the AccessKit adapter that belongs to it) exists, and it
/// survives the window being suspended and created again.
pub(crate) struct PageShared {
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Role and label of the page node.
    marker: Option<(Role, String)>,
    /// The tree the page node grafts.
    tree: Option<TreeId>,
    /// What may be pushed when (`mercury_a11y::graft`).
    graft: GraftState,
    /// The page node of the tree built last.
    graft_node: Option<NodeId>,
    /// Assistive technology is attached (its first request came, it was not deactivated since).
    attached: bool,
    /// A tree was refused and the page switched off, until assistive technology attaches again.
    disabled: bool,
    /// A flush is queued in the event loop.
    flush_scheduled: bool,
    window: Option<Weak<WinitWindowAdapter>>,
    on_activation: Option<Rc<dyn Fn(bool)>>,
    on_action: Option<Rc<dyn Fn(ActionRequest)>>,
}

impl PageShared {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self { inner: RefCell::new(Inner::default()) })
    }

    /// An AccessKit adapter was made for the window: it knows nothing yet, so nothing the page
    /// state holds applies to it. Assistive technology that was attached to the old one is told
    /// it is gone.
    pub(super) fn adapter_created(self: &Rc<Self>, window: Weak<WinitWindowAdapter>) {
        let notify = {
            let mut inner = self.inner.borrow_mut();
            inner.window = Some(window);
            let was_attached = std::mem::replace(&mut inner.attached, false);
            inner.disabled = false;
            inner.flush_scheduled = false;
            inner.graft.clear();
            inner.graft_node = None;
            inner.tree = None;
            was_attached.then(|| inner.on_activation.clone()).flatten()
        };
        if let Some(callback) = notify {
            callbacks_later(callback, &[false]);
        }
    }

    /// A full tree is about to be built: the page node is found again.
    pub(super) fn begin_build(&self) {
        self.inner.borrow_mut().graft_node = None;
    }

    /// If `node` (whose id is `id`) is the page node, hangs the page's tree from it. Returns
    /// whether it did: the node then has no children of its own, its only child comes from the
    /// grafted tree.
    pub(super) fn mark_page_node(&self, id: NodeId, node: &mut Node) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some((role, label)) = &inner.marker else { return false };
        if node.role() != *role || node.label() != Some(label.as_str()) {
            return false;
        }
        if inner.graft_node.is_some_and(|first| first != id) {
            // A second node that looks like the page node stays a plain node: AccessKit takes a
            // tree from one graft only.
            return false;
        }
        inner.graft_node = Some(id);
        let Some(tree) = inner.tree else { return false };
        node.set_tree_id(tree);
        true
    }

    /// Records an update of the window's own tree that is about to be pushed.
    fn record_root(&self, update: &mut TreeUpdate) {
        self.inner.borrow_mut().graft.push_root(update);
    }

    fn take_ready(&self) -> Drained {
        self.inner.borrow_mut().graft.take_ready()
    }

    /// Whether data of the page may be in what the adapter is given: then every call is guarded.
    fn guard_needed(&self) -> bool {
        let inner = self.inner.borrow();
        inner.tree.is_some() || inner.disabled
    }

    /// AccessKit refused a tree: the page is switched off for this window (D8 of the lane
    /// brief). One line, no content.
    fn switch_off(self: &Rc<Self>) {
        let callback = {
            let mut inner = self.inner.borrow_mut();
            if inner.disabled {
                return;
            }
            inner.disabled = true;
            inner.flush_scheduled = false;
            inner.graft.clear();
            inner.graft_node = None;
            inner.tree = None;
            inner.on_activation.clone()
        };
        i_slint_core::debug_log!(
            "Mercury: AccessKit refused the page's accessibility tree; page content is not given to assistive technology for this window until it attaches again"
        );
        // The graft leaves the adapter's tree with the next full update, and the embedder stops
        // building the tab's tree.
        run_later(self, |adapter| adapter.reload_tree());
        if let Some(callback) = callback {
            callbacks_later(callback, &[false]);
        }
    }
}

/// Runs `work` with the window's PageShared, if the window is backed by this backend.
fn with_page<R>(window: &Window, work: impl FnOnce(&Rc<PageShared>) -> R) -> Option<R> {
    let adapter = WindowInner::from_pub(window).window_adapter();
    let internal = adapter.internal(i_slint_core::InternalToken)?;
    let adapter = (internal as &dyn core::any::Any).downcast_ref::<WinitWindowAdapter>()?;
    Some(work(&adapter.mercury_page))
}

/// Runs `work` on the window's AccessKit adapter from the event loop, as Slint does for its own
/// deferred work (`AccessKitAdapter::invoke_later`): never from inside a call that may hold the
/// adapter. Does nothing while the window has no AccessKit adapter.
fn run_later(page: &Rc<PageShared>, work: impl FnOnce(&mut AccessKitAdapter) + 'static) {
    let Some(window) = page.inner.borrow().window.clone() else { return };
    run_later_on(window, Box::new(work));
}

fn run_later_on(window: Weak<WinitWindowAdapter>, work: Box<dyn FnOnce(&mut AccessKitAdapter)>) {
    Timer::single_shot(Default::default(), move || {
        let mut work = Some(work);
        let mut busy = false;
        let retry_window = window.clone();
        // Not called at all when the window is gone or has no AccessKit adapter: the work is
        // dropped then.
        WinitWindowAdapter::with_access_kit_adapter_from_weak_window_adapter(window, |cell| {
            match cell.try_borrow_mut() {
                Ok(mut adapter) => {
                    if let Some(work) = work.take() {
                        work(&mut adapter);
                    }
                }
                // Borrowed right now (a call into the adapter is on the stack): a turn later.
                Err(_) => busy = true,
            }
        });
        if busy && let Some(work) = work {
            run_later_on(retry_window, work);
        }
    });
}

/// Calls the embedder's activation callback with each state in turn, from the event loop.
fn callbacks_later(callback: Rc<dyn Fn(bool)>, states: &'static [bool]) {
    Timer::single_shot(Default::default(), move || {
        for state in states {
            callback(*state);
        }
    });
}

fn schedule_flush(page: &Rc<PageShared>) {
    {
        let mut inner = page.inner.borrow_mut();
        if inner.flush_scheduled {
            return;
        }
        inner.flush_scheduled = true;
    }
    run_later(page, |adapter| {
        adapter.page.inner.borrow_mut().flush_scheduled = false;
        let page = adapter.page.clone();
        flush(&mut adapter.inner, &page);
    });
}

/// Pushes what the state may release now, in order, each guarded.
fn flush(inner: &mut accesskit_winit::Adapter, page: &Rc<PageShared>) {
    let drained = page.take_ready();
    for update in drained.updates {
        if guarded(|| inner.update_if_active(|| update)).is_err() {
            page.switch_off();
            return;
        }
    }
    if drained.reset {
        // A queue overflowed: the engine must send the tab's tree again, from the start.
        let callback = page.inner.borrow().on_activation.clone();
        if let Some(callback) = callback {
            callbacks_later(callback, &[false, true]);
        }
    }
}

/// `accesskit_winit::Adapter::update_if_active` for every update of the window's own tree: the
/// update is recorded (the grafts it adds or removes, a focus that cannot be followed yet),
/// pushed, and what was waiting for it follows. Guarded while page data may be in the tree.
pub(super) fn update_if_active(
    inner: &mut accesskit_winit::Adapter,
    page: &Rc<PageShared>,
    make: impl FnOnce() -> TreeUpdate,
) {
    let push = |inner: &mut accesskit_winit::Adapter| {
        inner.update_if_active(|| {
            let mut update = make();
            page.record_root(&mut update);
            update
        })
    };
    if !page.guard_needed() {
        push(inner);
        return;
    }
    if guarded(|| push(inner)).is_err() {
        page.switch_off();
        return;
    }
    flush(inner, page);
}

/// The host window is about to gain or lose focus (a `Focused` event is about to reach the
/// platform adapter). `accesskit_consumer` 0.38 rebuilds its focus then from the *local* id of the
/// node it has focused, used as an id of the window's own tree; a node of a page's tree has an id
/// the window's tree does not have, and the consumer panics ("Focused ID ... is not in the node
/// list"). So while the window's focus is on the page node, it is moved to the window's root for
/// the moment of the change; the rebuild this backend does after every `Focused` event puts it
/// back (`mercury-a11y`'s `tests/graft.rs` pins the defect and the remedy; drop this when the
/// consumer is fixed).
pub(super) fn before_host_focus_change(
    inner: &mut accesskit_winit::Adapter,
    page: &Rc<PageShared>,
    window_root: NodeId,
) {
    if !page.inner.borrow().graft.root_focus_is_in_a_page() {
        return;
    }
    update_if_active(inner, page, || TreeUpdate {
        nodes: Vec::new(),
        tree: None,
        tree_id: TreeId::ROOT,
        focus: window_root,
    });
}

/// Assistive technology asked for the window's tree (again): the state starts over.
pub(super) fn attached(page: &Rc<PageShared>) -> Option<DeferredAccessKitAction> {
    let mut inner = page.inner.borrow_mut();
    let was_attached = std::mem::replace(&mut inner.attached, true);
    inner.disabled = false;
    inner.flush_scheduled = false;
    inner.graft.clear();
    inner.graft_node = None;
    inner.tree = None;
    let callback = inner.on_activation.clone()?;
    // A second request without a detach in between means the adapter's tree is new again: the
    // page's tree the embedder holds belongs to the old one.
    let states: &'static [bool] = if was_attached { &[false, true] } else { &[true] };
    Some(DeferredAccessKitAction::Mercury(Deferred::Activation(callback, states)))
}

/// Assistive technology detached.
pub(super) fn detached(page: &Rc<PageShared>) -> Option<DeferredAccessKitAction> {
    let mut inner = page.inner.borrow_mut();
    let was_attached = std::mem::replace(&mut inner.attached, false);
    inner.disabled = false;
    inner.flush_scheduled = false;
    inner.graft.clear();
    inner.graft_node = None;
    inner.tree = None;
    let callback = inner.on_activation.clone().filter(|_| was_attached)?;
    Some(DeferredAccessKitAction::Mercury(Deferred::Activation(callback, &[false])))
}

/// An action request for a node that is not the window's own: for the embedder, if the tree is
/// one the adapter holds. A `Focus` also moves the window's keyboard focus to the page node, so
/// that typing then goes to the page.
pub(super) fn foreign_action(
    nodes: &NodeCollection,
    request: ActionRequest,
) -> Option<DeferredAccessKitAction> {
    let (callback, graft_node) = {
        let inner = nodes.page.inner.borrow();
        if inner.disabled || !inner.attached || inner.graft.route(&request) != Route::Foreign {
            return None;
        }
        (inner.on_action.clone()?, inner.graft_node)
    };
    let focus = if request.action == Action::Focus {
        graft_node.and_then(|id| nodes.item_rc_for_node_id(id))
    } else {
        None
    };
    Some(DeferredAccessKitAction::Mercury(Deferred::Foreign { focus, callback, request }))
}

/// Work for the embedder, done after the adapter is released.
pub enum Deferred {
    Activation(Rc<dyn Fn(bool)>, &'static [bool]),
    Foreign { focus: Option<ItemRc>, callback: Rc<dyn Fn(ActionRequest)>, request: ActionRequest },
}

impl Deferred {
    pub fn invoke(&self, window: &Window) {
        match self {
            Deferred::Activation(callback, states) => {
                for state in *states {
                    callback(*state);
                }
            }
            Deferred::Foreign { focus, callback, request } => {
                if let Some(item) = focus {
                    // As the pointer would: typing goes to the page from now on.
                    WindowInner::from_pub(window).set_focus_item(
                        item,
                        true,
                        FocusReason::PointerClick,
                    );
                }
                callback(request.clone());
            }
        }
    }
}
