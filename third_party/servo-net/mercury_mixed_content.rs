// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: the per-site allow of mixed content and the report of a block to the embedder (EC17). This is
// a new Mercury file; the source is the Mixed Content specification and Mercury's own design, and nothing
// in it is copied from Servo or another project. The hooks are the two calls in `fetch/methods.rs`;
// `MERCURY.md` lists them.

//! Mixed content: the user's allow, and the report of a block.
//!
//! Servo 0.8 blocks mixed content by the algorithms of <https://w3c.github.io/webappsec-mixed-content/>
//! (`should_request_be_blocked_as_mixed_content` and `should_response_be_blocked_as_mixed_content` in
//! `fetch/methods.rs`) but leaves out step 1.3, "the user agent has been instructed to allow mixed
//! content", and tells nobody when it blocks. This file adds both, without changing those two functions:
//!
//! * [`request_is_blocked`] and [`response_is_blocked`] take the specification's answer. When it says
//!   "block", they ask `NetPolicy::mixed_content_allowed` about the page's site (the top level site
//!   that `NetPolicy::sites_of_request` finds; a request whose top-level site is not known is not
//!   allowed, so the answer fails closed). An allowed request goes on and nothing is reported.
//! * When the request is blocked, [`report_blocked`] sends `NetToEmbedderMsg::Mercury(web view,
//!   NetMsg::MixedContentBlocked { url, destination })`, so Mercury can count the blocks of a tab and one
//!   day offer "load unsafe content" in the page's site settings.
//!
//! Exactly one message is sent for each blocked request. The two checks cannot both fire for one fetch:
//! the response check only runs for a response that is not already a network error, and a request that was
//! blocked has one.
//!
//! Known gap, by decision: step 6 of main fetch still upgrades a passive request (image, audio, video) to
//! `https` before the check, also on a site where mixed content is allowed, so an allowed site loads such a
//! resource over `https` when the host answers there. The allow lifts the block, not the upgrade.
//!
//! Drop with the two hooks in `fetch/methods.rs` if Servo implements step 1.3 and a report.

use embedder_traits::{MercuryEmbedderMsg, NetMsg};
use net_traits::request::Request;

use crate::embedder::NetToEmbedderMsg;
use crate::fetch::methods::FetchContext;

/// Whether `request` is blocked as mixed content, given the specification's answer `blocked_by_spec` for
/// the request (`should_request_be_blocked_as_mixed_content`). A block is reported to the embedder.
pub(crate) fn request_is_blocked(
    context: &FetchContext,
    request: &Request,
    blocked_by_spec: bool,
) -> bool {
    blocked(context, request, blocked_by_spec)
}

/// Whether the response to `request` is blocked as mixed content, given the specification's answer
/// `blocked_by_spec` for the response (`should_response_be_blocked_as_mixed_content`). A block is reported
/// to the embedder.
pub(crate) fn response_is_blocked(
    context: &FetchContext,
    request: &Request,
    blocked_by_spec: bool,
) -> bool {
    blocked(context, request, blocked_by_spec)
}

fn blocked(context: &FetchContext, request: &Request, blocked_by_spec: bool) -> bool {
    if !blocked_by_spec {
        return false;
    }
    let policy = &context.state.net_policy;
    // Step 1.3: the user agent has been instructed to allow mixed content (for this page's site).
    if policy.mixed_content_allowed(&policy.sites_of_request(request)) {
        return false;
    }
    report_blocked(context, request);
    true
}

/// Tell the embedder that `request` was blocked as mixed content: one message, naming the web view the
/// request belongs to (none for a request that has no web view, which then reaches the whole `Servo`
/// instance's delegate). The URL is the request's current URL, also when the block came from the response.
pub(crate) fn report_blocked(context: &FetchContext, request: &Request) {
    context.state.embedder_proxy.send(NetToEmbedderMsg::Mercury(
        request.target_webview_id,
        MercuryEmbedderMsg::Net(NetMsg::MixedContentBlocked {
            url: request.current_url(),
            destination: request.destination,
        }),
    ));
}
