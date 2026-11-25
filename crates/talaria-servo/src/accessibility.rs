// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A page's accessibility tree on its way out of the engine (lane a11y-1, item 1).
//!
//! The engine builds a tree for a web view only when told to
//! ([`ServoWebView::set_page_accessible`](crate::ServoWebView::set_page_accessible)), and then
//! sends updates of three kinds of tree through the web view's delegate: the web view's own
//! (a scroll view with one graft), the document's, and an iframe's. The delegate hands each to
//! the sink the window installed ([`ServoRuntime::set_accessibility_sink`]
//! (crate::ServoRuntime::set_accessibility_sink)), which decides what goes on to the window
//! (`mercury_a11y::bridge`) and when (`mercury_a11y::graft`). `talaria-core` never sees a tree:
//! it is the page's text.
//!
//! The sink is one per thread, not one per runtime or per tab: the delegate of every web view,
//! including those a page opens by itself, reaches it without being given anything. Servo's
//! delegate callbacks and `set_accessibility_sink` are all on the thread that runs
//! `spin_event_loop`.

use std::cell::RefCell;
use std::rc::Rc;

use accesskit::TreeUpdate;
use talaria_core::TabId;

/// Called synchronously inside `spin_event_loop` with each tree update of a tab's web view, in
/// the order the engine sent them. The update is the page's text: do not print it.
pub type AccessibilitySink = Rc<dyn Fn(TabId, TreeUpdate)>;

thread_local! {
    static SINK: RefCell<Option<AccessibilitySink>> = const { RefCell::new(None) };
}

pub(crate) fn set_sink(sink: AccessibilitySink) {
    SINK.with(|slot| *slot.borrow_mut() = Some(sink));
}

/// Hands a tab's update to the sink, if there is one. Without a sink the update is dropped
/// (nobody asked for the tree).
pub(crate) fn deliver(tab: TabId, update: TreeUpdate) {
    // The sink is cloned out so that it may install a new one while it runs.
    let sink = SINK.with(|slot| slot.borrow().clone());
    if let Some(sink) = sink {
        sink(tab, update);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use accesskit::{Node, NodeId, Role, Tree, TreeId};

    use super::*;

    fn update() -> TreeUpdate {
        TreeUpdate {
            nodes: vec![(NodeId(0), Node::new(Role::ScrollView))],
            tree: Some(Tree::new(NodeId(0))),
            tree_id: TreeId(accesskit::Uuid::from_u128(7)),
            focus: NodeId(0),
        }
    }

    #[test]
    fn an_update_goes_to_the_sink_with_its_tab_in_order() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&seen);
        set_sink(Rc::new(move |tab, update| {
            log.borrow_mut().push((tab, update.tree_id));
        }));
        deliver(TabId::new(3), update());
        deliver(TabId::new(4), update());
        let seen = seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, TabId::new(3));
        assert_eq!(seen[1].0, TabId::new(4));
        assert_eq!(seen[0].1, update().tree_id);
    }

    #[test]
    fn an_update_with_no_sink_is_dropped_without_a_fuss() {
        // A fresh thread has no sink.
        std::thread::spawn(|| deliver(TabId::new(1), update()))
            .join()
            .expect("delivering to no sink does not panic");
    }

    #[test]
    fn the_sink_may_replace_itself_while_it_runs() {
        let calls = Rc::new(Cell::new(0));
        let counter = Rc::clone(&calls);
        set_sink(Rc::new(move |_, _| {
            counter.set(counter.get() + 1);
            set_sink(Rc::new(|_, _| {}));
        }));
        deliver(TabId::new(1), update());
        deliver(TabId::new(1), update());
        assert_eq!(calls.get(), 1, "the second update went to the new sink");
    }
}
