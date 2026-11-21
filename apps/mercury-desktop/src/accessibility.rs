// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// The shown tab's page, read by assistive technology (lane a11y-1, item 1).
//
// The window's accessibility tree (Slint's) has one node for the page: role `main`, label "Page
// content" (`web-view-focus` in `main-window.slint`). While assistive technology (VoiceOver) is
// attached to the window, the engine builds the shown tab's page tree and the vendored Slint
// backend hangs it from that node (`MercuryPageAccessibility`). This file is the shell around
// that:
//
// - `mercury_a11y::bridge::Bridge` decides, from three facts (technology attached, which tab is
//   shown, which tab was closed), which tab's tree the engine builds and which tree hangs from the
//   page node. It returns commands; this file executes them: the engine is told through
//   `ServoWebView::set_page_accessible`, the window through `set_page_tree`. A tab that is not
//   shown never builds a tree, and the tree of a tab that was switched away is dropped.
// - The tree updates of every tab's web view arrive in the sink installed here, in the order the
//   engine sent them, and only the shown tab's go on to the window
//   (`push_page_update`). The window holds each until its graft is in (`mercury_a11y::graft`).
// - What assistive technology does to the page (press, focus, set a value, scroll) comes back
//   through `on_foreign_action` and goes to the engine; an action that clicks counts as the
//   user's activation of that tab (OD-5), so that a page the user opened with VoiceOver may open
//   a window as it may after a click.
//
// Commands are executed from the engine's pump, where the web views are at hand, never from inside
// a window callback: a callback that arrives while the pump runs would otherwise find the engine
// borrowed. `talaria-core` never sees a tree; no tree or action data is logged (it is the page's).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::Instant;

use accesskit::{ActionRequest, Role};
use i_slint_backend_winit::MercuryPageAccessibility;
use mercury_a11y::bridge::{Bridge, Command, is_activating_action};
use mercury_ui::MainWindow;
use slint::ComponentHandle;
use talaria_core::TabId;
use talaria_servo::ServoWebView;

/// The page node's role and label: what `main-window.slint` gives `web-view-focus`, and what the
/// window looks for. `crates/mercury-ui/tests/a11y_page_node.rs` pins that exactly one such node
/// exists in every state of the window.
pub(crate) const PAGE_ROLE: Role = Role::Main;
pub(crate) const PAGE_LABEL: &str = "Page content";

struct Shell {
    bridge: RefCell<Bridge>,
    /// Commands the bridge returned that the engine's pump has not executed yet.
    pending: RefCell<VecDeque<Command>>,
    window: slint::Weak<MainWindow>,
}

thread_local! {
    /// Where the engine's pump finds the shell (like `ENGINE` in `engine.rs`).
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

fn with_shell(work: impl FnOnce(&Rc<Shell>)) {
    let shell = SHELL.with(|slot| slot.borrow().clone());
    if let Some(shell) = shell {
        work(&shell);
    }
}

/// Connects the window to the engine. Call once, after `connect_servo_runtime`.
pub(crate) fn install(window: &MainWindow) {
    let shell = Rc::new(Shell {
        bridge: RefCell::new(Bridge::new()),
        pending: RefCell::new(VecDeque::new()),
        window: window.as_weak(),
    });
    SHELL.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&shell)));

    let slint_window = window.window();
    slint_window.set_page_graft(PAGE_ROLE, PAGE_LABEL);

    let attached = Rc::downgrade(&shell);
    slint_window.on_activation_changed(move |active| {
        if let Some(shell) = attached.upgrade() {
            shell.activation_changed(active);
        }
    });

    let acted = Rc::downgrade(&shell);
    slint_window.on_foreign_action(move |request| {
        if let Some(shell) = acted.upgrade() {
            shell.forward_action(request);
        }
    });

    let delivered = Rc::downgrade(&shell);
    let installed = crate::engine::with_runtime(|runtime| {
        runtime.set_accessibility_sink(Rc::new(move |tab, update| {
            if let Some(shell) = delivered.upgrade() {
                shell.deliver(tab, update);
            }
        }));
    });
    if !installed {
        log::warn!("page accessibility is off: the engine was not ready to take its sink");
    }
}

/// The engine shows `tab`'s web view now. Call right after `show()`.
pub(crate) fn tab_shown(web_views: &HashMap<TabId, ServoWebView>, tab: TabId) {
    with_shell(|shell| {
        let commands = shell.bridge.borrow_mut().tab_shown(tab.value());
        shell.queue(commands);
        shell.run_pending(web_views);
    });
}

/// The engine hides `tab`'s web view. Call right after `hide()`.
pub(crate) fn tab_hidden(web_views: &HashMap<TabId, ServoWebView>, tab: TabId) {
    with_shell(|shell| {
        let commands = shell.bridge.borrow_mut().tab_hidden(tab.value());
        shell.queue(commands);
        shell.run_pending(web_views);
    });
}

/// `tab`'s web view is closed or about to be replaced.
pub(crate) fn tab_closed(web_views: &HashMap<TabId, ServoWebView>, tab: TabId) {
    with_shell(|shell| {
        let commands = shell.bridge.borrow_mut().tab_closed(tab.value());
        shell.queue(commands);
        shell.run_pending(web_views);
    });
}

/// Executes what the window asked for since the last pump (assistive technology attached or
/// detached). Cheap when there is nothing.
pub(crate) fn run_pending(web_views: &HashMap<TabId, ServoWebView>) {
    with_shell(|shell| shell.run_pending(web_views));
}

impl Shell {
    fn queue(&self, commands: Vec<Command>) {
        self.pending.borrow_mut().extend(commands);
    }

    /// The window's callback: assistive technology attached (`true`) or detached (`false`).
    fn activation_changed(&self, attached: bool) {
        let commands = {
            let mut bridge = self.bridge.borrow_mut();
            if attached {
                bridge.attach()
            } else {
                bridge.detach()
            }
        };
        self.queue(commands);
        // The pump executes them; ask for one, as the window may be covered.
        crate::engine::request_pump();
    }

    /// One update of a tab's page tree, from the engine's delegate.
    fn deliver(&self, tab: TabId, update: accesskit::TreeUpdate) {
        let Some(update) = self.bridge.borrow_mut().update_from(tab.value(), update) else {
            return;
        };
        if let Some(window) = self.window.upgrade() {
            window.window().push_page_update(update);
        }
    }

    /// What assistive technology did to a node of the page. Not logged: it may carry text.
    fn forward_action(&self, request: ActionRequest) {
        if is_activating_action(request.action)
            && let Some(tab) = self.bridge.borrow().active_tab()
        {
            // OD-5: pressing with VoiceOver is the user's gesture on that page.
            crate::activation::note_user_activation(TabId::new(tab), Instant::now());
        }
        let sent = crate::engine::with_runtime(|runtime| {
            runtime.forward_accessibility_action(request);
        });
        if !sent {
            log::debug!(
                "an accessibility action arrived while the engine was busy and was dropped"
            );
        }
    }

    /// Executes the queued commands in order. The engine's answer to an activation (the tab's
    /// tree id) goes back to the bridge, and the commands it returns run before the rest.
    fn run_pending(&self, web_views: &HashMap<TabId, ServoWebView>) {
        loop {
            let Some(command) = self.pending.borrow_mut().pop_front() else {
                break;
            };
            match command {
                Command::Activate(tab) => {
                    let tree = web_views
                        .get(&TabId::new(tab))
                        .and_then(|web_view| web_view.set_page_accessible(true));
                    let follow_up = self.bridge.borrow_mut().activated(tab, tree);
                    let mut pending = self.pending.borrow_mut();
                    for command in follow_up.into_iter().rev() {
                        pending.push_front(command);
                    }
                }
                Command::Deactivate(tab) => {
                    if let Some(web_view) = web_views.get(&TabId::new(tab)) {
                        // The answer (none) is the tree that is gone.
                        let _gone = web_view.set_page_accessible(false);
                    }
                }
                Command::Graft(tree) => {
                    if let Some(window) = self.window.upgrade() {
                        window.window().set_page_tree(tree);
                    }
                }
            }
        }
    }
}
