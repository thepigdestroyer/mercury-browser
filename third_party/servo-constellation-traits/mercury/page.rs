/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the page lane. Add variants (and the payload types they need) here
// only, never in another lane's file or in a shared enum.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use content_security_policy::sandboxing_directive::SandboxingFlagSet;
use embedder_traits::{UnloadReason, UrlRequest};
use serde::{Deserialize, Serialize};
use servo_base::id::{PipelineId, WebViewId};
use strum::IntoStaticStr;

/// How long a page may keep an unload question open before Mercury lets it unload.
pub const UNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Messages from a script thread to the constellation for the `page` lane.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum PageScriptMsg {
    UnloadChecked { request_id: u64, allowed: bool },
}

impl fmt::Debug for PageScriptMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}

/// Messages from the embedding layer to the constellation for the `page` lane.
#[derive(IntoStaticStr)]
pub enum PageEmbedderMsg {
    StopLoading(WebViewId),
    PermitUnload {
        webview_id: WebViewId,
        request_id: u64,
        reason: UnloadReason,
    },
    ReloadBypassingCache(WebViewId),
    LoadWithPolicy {
        webview_id: WebViewId,
        url_request: UrlRequest,
        scripts_allowed: bool,
    },
    AllowNavigationWithPolicy {
        pipeline_id: PipelineId,
        scripts_allowed: bool,
    },
}

impl fmt::Debug for PageEmbedderMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}

/// The sandboxing flags of a browsing context, with the scripts flag set when scripts are not
/// allowed. Unchanged when they are allowed.
pub fn apply_scripts_policy(flags: SandboxingFlagSet, scripts_allowed: bool) -> SandboxingFlagSet {
    if scripts_allowed {
        flags
    } else {
        flags | SandboxingFlagSet::SANDBOXED_SCRIPTS_BROWSING_CONTEXT_FLAG
    }
}

/// The screen wake locks held by each pipeline.
#[derive(Default)]
pub struct WakeLockLedger {
    per_pipeline: HashMap<PipelineId, u32>,
}

impl WakeLockLedger {
    /// Counts one more lock of `pipeline`. Returns `true` when it is the first lock of all pipelines.
    pub fn acquire(&mut self, pipeline: PipelineId) -> bool {
        let was_idle = self.total() == 0;
        *self.per_pipeline.entry(pipeline).or_insert(0) += 1;
        was_idle
    }

    /// Drops one lock of `pipeline`. Returns `true` when it was the last lock of all pipelines.
    /// A release with no lock held is ignored and returns `false`.
    pub fn release(&mut self, pipeline: PipelineId) -> bool {
        let Some(count) = self.per_pipeline.get_mut(&pipeline) else {
            return false;
        };
        *count -= 1;
        if *count == 0 {
            self.per_pipeline.remove(&pipeline);
        }
        self.total() == 0
    }

    /// Drops every lock of `pipeline`, which has closed or crashed. Returns `true` when all locks
    /// are now gone and some were held.
    pub fn pipeline_gone(&mut self, pipeline: PipelineId) -> bool {
        let had_locks = self.per_pipeline.remove(&pipeline).is_some();
        had_locks && self.total() == 0
    }

    /// The number of locks held by all pipelines.
    pub fn total(&self) -> u32 {
        self.per_pipeline.values().sum()
    }
}

/// The unload questions asked of pages, by request id. Ids start at 1 and increase. The clock is
/// passed in, so the tracker is testable without sleeping.
#[derive(Default)]
pub struct UnloadTracker {
    next_id: u64,
    pending: HashMap<u64, (WebViewId, Duration)>,
}

impl UnloadTracker {
    /// Starts a question about `webview` at time `now`, and returns its request id.
    pub fn begin(&mut self, webview: WebViewId, now: Duration) -> u64 {
        self.next_id += 1;
        self.pending.insert(self.next_id, (webview, now));
        self.next_id
    }

    /// Answers the question `request_id`. `None` when it is unknown or already answered.
    pub fn finish(&mut self, request_id: u64) -> Option<WebViewId> {
        self.pending.remove(&request_id).map(|(webview, _)| webview)
    }

    /// Removes and returns the questions asked more than [`UNLOAD_TIMEOUT`] before `now`.
    pub fn expired(&mut self, now: Duration) -> Vec<(u64, WebViewId)> {
        let expired: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, (_, asked))| now.saturating_sub(*asked) > UNLOAD_TIMEOUT)
            .map(|(id, _)| *id)
            .collect();
        expired
            .into_iter()
            .filter_map(|id| self.pending.remove(&id).map(|(webview, _)| (id, webview)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use servo_base::id::{PipelineNamespace, PipelineNamespaceId, WebViewId};

    use super::*;

    fn install_namespace() {
        PipelineNamespace::install(PipelineNamespaceId(7));
    }

    fn pipeline() -> PipelineId {
        install_namespace();
        PipelineId::new()
    }

    #[test]
    fn apply_scripts_policy_adds_flag_only_when_blocked() {
        let flags = SandboxingFlagSet::SANDBOXED_ORIGIN_BROWSING_CONTEXT_FLAG;
        assert_eq!(apply_scripts_policy(flags, true), flags);
        assert!(
            apply_scripts_policy(flags, false)
                .contains(SandboxingFlagSet::SANDBOXED_SCRIPTS_BROWSING_CONTEXT_FLAG)
        );
        assert!(
            apply_scripts_policy(flags, false)
                .contains(SandboxingFlagSet::SANDBOXED_ORIGIN_BROWSING_CONTEXT_FLAG)
        );
    }

    #[test]
    fn ledger_counts_per_pipeline() {
        let a = pipeline();
        let b = pipeline();
        let mut ledger = WakeLockLedger::default();

        assert!(ledger.acquire(a));
        assert!(!ledger.acquire(b));
        assert_eq!(ledger.total(), 2);
        assert!(!ledger.release(a));
        assert!(ledger.release(b));
        assert_eq!(ledger.total(), 0);
        assert!(!ledger.release(b));

        ledger.acquire(a);
        ledger.acquire(a);
        assert!(!ledger.pipeline_gone(b));
        assert!(ledger.pipeline_gone(a));
        assert_eq!(ledger.total(), 0);
    }

    #[test]
    fn tracker_ids_increase_and_expire() {
        let webview = WebViewId::new(PipelineNamespaceId(7));
        let mut tracker = UnloadTracker::default();
        let first = tracker.begin(webview, Duration::from_secs(0));
        let second = tracker.begin(webview, Duration::from_secs(10));
        assert!(second > first);

        assert_eq!(tracker.finish(first), Some(webview));
        assert_eq!(tracker.finish(first), None);

        assert!(tracker.expired(Duration::from_secs(30)).is_empty());
        let expired = tracker.expired(Duration::from_secs(41));
        assert_eq!(expired, vec![(second, webview)]);
        assert_eq!(tracker.finish(second), None);
    }

    #[test]
    fn page_embedder_debug_names_only() {
        let message = PageEmbedderMsg::PermitUnload {
            webview_id: WebViewId::new(PipelineNamespaceId(7)),
            request_id: 3,
            reason: UnloadReason::Quit,
        };
        assert_eq!(format!("{message:?}"), "PermitUnload");
        let message = PageScriptMsg::UnloadChecked { request_id: 3, allowed: true };
        assert_eq!(format!("{message:?}"), "UnloadChecked");
    }
}
