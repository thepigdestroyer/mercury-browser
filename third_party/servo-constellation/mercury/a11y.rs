/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the a11y lane. Implement handlers here only.

//! Accessibility in the constellation: the iframes of a page (a11y-1, EE7).
//!
//! Upstream Servo activates accessibility for the **top-level** document of a `WebView` only
//! (`Constellation::set_accessibility_active()` and `set_frame_tree_for_webview()` send
//! `SetAccessibilityActive` to `active_top_level_pipeline_id`). A document inside an `<iframe>` has
//! its own pipeline, its own layout and its own accessibility tree (`TreeId::from(pipeline_id)`),
//! and builds none until it is told to. This file tells it.
//!
//! # The rule
//!
//! While a `WebView` is active for accessibility, **every document in the active frame tree below
//! the top-level one** (the browsing contexts that are fully active: the iframes of the active
//! document, and theirs) is activated, and a document that leaves the tree is deactivated:
//!
//! * [`Constellation::sync_accessibility_of_child_documents`] computes the set of documents that
//!   should be active, compares it with the set that was told to be, and sends the difference. It
//!   is called wherever the set can change: when the `WebView` is activated or deactivated
//!   (`set_accessibility_active()`), and every time the frame tree changes
//!   (`set_frame_tree_for_webview()`: an iframe's document enters the tree when its navigation
//!   completes (`change_session_history()`), a history traversal swaps documents, and a new
//!   top-level document replaces the old tree).
//! * [`Constellation::forget_accessibility_of_child_document`] drops a pipeline from the set when
//!   it is closed (`close_pipeline()`). No message is sent: the pipeline's exit message follows
//!   on the same channel, and a message that fails to send is treated as a crashed script thread.
//!
//! A pipeline is created before it is in the frame tree (it is a *pending change* until its
//! document is activated, `handle_script_new_iframe()` and `new_pipeline()`), and a document that
//! is not in the tree does not build a tree. Activating it at the moment it enters the tree, which
//! always goes through `set_frame_tree_for_webview()`, is the earliest moment that matters, and
//! needs no hook at creation.
//!
//! # The epoch
//!
//! The epoch a child document is activated with is the epoch of its top-level document with a flag
//! ([`embedder_traits::epoch_for_child_document`]). Layout sends it back with every update, and
//! `WebView::process_accessibility_tree_update()` reads the flag: an update of a child document
//! is passed on as it is and is **never** grafted as the page (it may arrive before the
//! top-level document's own first update), and it is dropped when the top-level document it
//! belonged to has been replaced.
//!
//! # Ordering
//!
//! The messages to a child's script thread and to the top-level one are not ordered with respect
//! to each other (they can be different processes, and layout sends its updates straight to the
//! embedder), so the first tree of a child may reach the embedder before the update of its parent
//! that grafts it. That is the embedder's to handle: it holds a tree until a graft names it
//! (`mercury_a11y::graft`).

use embedder_traits::epoch_for_child_document;
use layout_api::ScriptThreadFactory;
use script_traits::ScriptThreadMessage;
use servo_base::Epoch;
use servo_base::id::{PipelineId, WebViewId};
use servo_config::pref;
use servo_constellation_traits::{A11yEmbedderMsg, A11yScriptMsg, ServiceWorkerManagerFactory};

use crate::constellation::Constellation;

/// What [`Constellation::sync_accessibility_of_child_documents`] sends.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ChildActivationPlan {
    /// The documents to deactivate, with the epoch they were activated with: they left the active
    /// frame tree, the `WebView` is no longer active, or they were activated for another
    /// top-level document (they are in `activate` as well then).
    pub(crate) deactivate: Vec<(PipelineId, Epoch)>,
    /// The documents to activate.
    pub(crate) activate: Vec<PipelineId>,
}

/// Compare the documents that were told to build an accessibility tree (`active`, each with the
/// epoch it was told) with the ones that should (`wanted`, all to be told `epoch`).
///
/// A document that is active with another epoch is deactivated and activated again, so that it
/// starts from a full tree for the current top-level document.
pub(crate) fn plan_child_activation(
    active: &[(PipelineId, Epoch)],
    wanted: &[PipelineId],
    epoch: Epoch,
) -> ChildActivationPlan {
    let mut plan = ChildActivationPlan::default();
    for &(pipeline_id, activated_with) in active {
        if activated_with != epoch || !wanted.contains(&pipeline_id) {
            plan.deactivate.push((pipeline_id, activated_with));
        }
    }
    for &pipeline_id in wanted {
        let up_to_date = active
            .iter()
            .any(|&(id, activated_with)| id == pipeline_id && activated_with == epoch);
        if !up_to_date && !plan.activate.contains(&pipeline_id) {
            plan.activate.push(pipeline_id);
        }
    }
    plan
}

impl<STF, SWF> Constellation<STF, SWF>
where
    STF: ScriptThreadFactory,
    SWF: ServiceWorkerManagerFactory,
{
    /// Handle a message of the `a11y` lane from the embedding layer. The enum is still empty.
    pub(super) fn handle_a11y_embedder_message(&mut self, message: A11yEmbedderMsg) {
        match message {}
    }

    /// Handle a message of the `a11y` lane from the script thread of `source_pipeline_id` in
    /// `webview_id`. The enum is still empty.
    pub(super) fn handle_a11y_script_message(
        &mut self,
        _webview_id: WebViewId,
        _source_pipeline_id: PipelineId,
        message: A11yScriptMsg,
    ) {
        match message {}
    }

    /// Bring the accessibility of the child (iframe) documents of `webview_id` in line with the
    /// `WebView`: see the module documentation. Calling it when nothing changed sends nothing.
    pub(crate) fn sync_accessibility_of_child_documents(&mut self, webview_id: WebViewId) {
        if !pref!(accessibility_enabled) {
            return;
        }
        let Some(webview) = self.webviews.get(&webview_id) else {
            return;
        };
        let accessibility_active = webview.accessibility_active;
        let has_top_level_document = webview.active_top_level_pipeline_id.is_some();
        let epoch = epoch_for_child_document(webview.active_top_level_pipeline_epoch);

        // A document that has gone is not in the map any more: forget it, whichever `WebView`.
        self.a11y_active_children
            .retain(|pipeline_id, _| self.pipelines.contains_key(pipeline_id));

        let wanted: Vec<PipelineId> = if accessibility_active && has_top_level_document {
            self.fully_active_browsing_contexts_iter(webview_id)
                .filter(|browsing_context| browsing_context.parent_pipeline_id.is_some())
                .map(|browsing_context| browsing_context.pipeline_id)
                .collect()
        } else {
            Vec::new()
        };
        let active: Vec<(PipelineId, Epoch)> = self
            .a11y_active_children
            .iter()
            .filter(|(pipeline_id, _)| {
                self.pipelines
                    .get(pipeline_id)
                    .is_some_and(|pipeline| pipeline.webview_id == webview_id)
            })
            .map(|(&pipeline_id, &activated_with)| (pipeline_id, activated_with))
            .collect();

        let plan = plan_child_activation(&active, &wanted, epoch);
        for (pipeline_id, activated_with) in plan.deactivate {
            self.a11y_active_children.remove(&pipeline_id);
            self.send_message_to_pipeline(
                pipeline_id,
                ScriptThreadMessage::SetAccessibilityActive(pipeline_id, false, activated_with),
                "Set accessibility inactive in a child document after closure",
            );
        }
        for pipeline_id in plan.activate {
            if self.send_message_to_pipeline(
                pipeline_id,
                ScriptThreadMessage::SetAccessibilityActive(pipeline_id, true, epoch),
                "Set accessibility active in a child document after closure",
            ) {
                self.a11y_active_children.insert(pipeline_id, epoch);
            }
        }
    }

    /// A pipeline is being closed: it is no longer told apart from the ones that were never
    /// activated. Nothing is sent to it (see the module documentation).
    pub(crate) fn forget_accessibility_of_child_document(&mut self, pipeline_id: PipelineId) {
        self.a11y_active_children.remove(&pipeline_id);
    }
}

#[cfg(test)]
mod tests {
    use servo_base::id::{PipelineNamespace, PipelineNamespaceId};

    use super::*;

    fn pipelines<const N: usize>() -> [PipelineId; N] {
        PipelineNamespace::install(PipelineNamespaceId(1));
        std::array::from_fn(|_| PipelineId::new())
    }

    fn child_epoch(top_level: u32) -> Epoch {
        epoch_for_child_document(Epoch(top_level))
    }

    #[test]
    fn nothing_active_and_nothing_wanted_sends_nothing() {
        assert_eq!(
            plan_child_activation(&[], &[], child_epoch(1)),
            ChildActivationPlan::default()
        );
    }

    #[test]
    fn a_document_that_enters_the_tree_is_activated_once() {
        let [a, b] = pipelines();
        let epoch = child_epoch(1);
        let plan = plan_child_activation(&[], &[a, b], epoch);
        assert_eq!(plan.activate, vec![a, b]);
        assert!(plan.deactivate.is_empty());

        // The same tree again: nothing more to send.
        let plan = plan_child_activation(&[(a, epoch), (b, epoch)], &[a, b], epoch);
        assert_eq!(plan, ChildActivationPlan::default());
    }

    #[test]
    fn a_document_that_leaves_the_tree_is_deactivated_with_the_epoch_it_was_activated_with() {
        let [a, b] = pipelines();
        let epoch = child_epoch(3);
        let plan = plan_child_activation(&[(a, epoch), (b, epoch)], &[b], epoch);
        assert_eq!(plan.deactivate, vec![(a, epoch)]);
        assert!(plan.activate.is_empty());
    }

    #[test]
    fn a_web_view_that_is_not_active_deactivates_every_child() {
        let [a, b] = pipelines();
        let epoch = child_epoch(2);
        let plan = plan_child_activation(&[(a, epoch), (b, epoch)], &[], epoch);
        assert_eq!(plan.deactivate.len(), 2);
        assert!(plan.activate.is_empty());
    }

    #[test]
    fn a_document_activated_for_an_older_top_level_document_starts_over() {
        let [a] = pipelines();
        let old = child_epoch(1);
        let new = child_epoch(2);
        let plan = plan_child_activation(&[(a, old)], &[a], new);
        assert_eq!(plan.deactivate, vec![(a, old)]);
        assert_eq!(plan.activate, vec![a]);
    }

    #[test]
    fn a_swap_deactivates_the_old_document_and_activates_the_new_one() {
        let [old_frame, new_frame, other] = pipelines();
        let epoch = child_epoch(5);
        let plan = plan_child_activation(
            &[(old_frame, epoch), (other, epoch)],
            &[new_frame, other],
            epoch,
        );
        assert_eq!(plan.deactivate, vec![(old_frame, epoch)]);
        assert_eq!(plan.activate, vec![new_frame]);
    }

    #[test]
    fn a_pipeline_that_is_listed_twice_is_activated_once() {
        let [a] = pipelines();
        let plan = plan_child_activation(&[], &[a, a], child_epoch(1));
        assert_eq!(plan.activate, vec![a]);
    }
}
