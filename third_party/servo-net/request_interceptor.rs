/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use embedder_traits::{GenericEmbedderProxy, WebResourceRequest, WebResourceResponseMsg};
// Mercury: `debug` for the rewrite log lines (EC37); `Destination` is no longer used here, the main-frame test
// moved to `mercury_rewrite::is_top_level_document` (D17).
use log::{debug, error};
use net_traits::http_status::HttpStatus;
use net_traits::request::Request;
use net_traits::response::{Response, ResponseBody};
use net_traits::{NetworkError, ResourceFetchTimingContainer};

use crate::embedder::NetToEmbedderMsg;
// Mercury: frame identity and the rewrite hook (EC38, EC37); the policy that judges the request.
use crate::mercury_rewrite::{self, FrameIdentity};
use crate::net_policy::NetPolicy;

#[derive(Clone)]
pub struct RequestInterceptor {
    embedder_proxy: GenericEmbedderProxy<NetToEmbedderMsg>,
}

impl RequestInterceptor {
    pub fn new(embedder_proxy: GenericEmbedderProxy<NetToEmbedderMsg>) -> RequestInterceptor {
        RequestInterceptor { embedder_proxy }
    }

    pub(crate) async fn intercept_request(
        &self,
        request: &mut Request,
        response: &mut Option<Response>,
        timing_info: &ResourceFetchTimingContainer,
        // Mercury: the policy judges the request's frame identity and the rewrite (EC38, EC37).
        policy: &NetPolicy,
    ) {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        // Mercury: the main-frame flag, the site, the initiator and the third-party flag of the request.
        let identity = FrameIdentity::of(request, policy);
        let web_resource_request = WebResourceRequest {
            method: request.method.clone(),
            // Mercury: the URL of this hop, not the first URL of the chain (the embedder must see what a
            // rewrite would replace).
            url: request.current_url(),
            headers: request.headers.clone(),
            destination: request.destination,
            referrer_url: request.referrer.to_url().cloned(),
            is_for_main_frame: identity.is_for_main_frame,
            is_redirect: request.redirect_count > 0,
            // Mercury: frame identity (EC38).
            top_level_site: identity.top_level_site,
            initiator_origin: identity.initiator_origin,
            is_third_party: identity.is_third_party,
        };

        self.embedder_proxy
            .send(NetToEmbedderMsg::WebResourceRequested(
                request.target_webview_id,
                web_resource_request,
                sender,
            ));

        // TODO: use done_chan and run in CoreResourceThreadPool.
        let mut accumulated_body = Vec::new();
        while let Some(message) = receiver.recv().await {
            match message {
                WebResourceResponseMsg::Start(webresource_response) => {
                    let timing = timing_info.inner().clone();
                    let mut response_override =
                        Response::new(webresource_response.url.into(), timing);
                    response_override.headers = webresource_response.headers;
                    response_override.status = HttpStatus::new(
                        webresource_response.status_code,
                        webresource_response.status_message,
                    );
                    *response = Some(response_override);
                },
                WebResourceResponseMsg::SendBodyData(data) => {
                    accumulated_body.push(data);
                },
                WebResourceResponseMsg::FinishLoad => {
                    if accumulated_body.is_empty() {
                        break;
                    }
                    let Some(response) = response.as_mut() else {
                        error!("Received unexpected FinishLoad message");
                        break;
                    };
                    *response.body.lock() =
                        ResponseBody::Done(accumulated_body.into_iter().flatten().collect());
                    break;
                },
                WebResourceResponseMsg::CancelLoad => {
                    *response = Some(Response::network_error(NetworkError::LoadCancelled));
                    break;
                },
                WebResourceResponseMsg::DoNotIntercept => break,
                // Mercury: change the request, then let it go on (EC37). Terminal like `DoNotIntercept`: the
                // embedder is not asked again about this hop. A request whose answer is already decided (a
                // block of an earlier check, or a response the embedder started) is left as it is.
                WebResourceResponseMsg::Rewrite(rewrite) => {
                    if response.is_some() {
                        debug!(
                            "Mercury: a rewrite of a request that is already answered is ignored"
                        );
                    } else if let Ok(applied) = mercury_rewrite::apply(request, rewrite, policy) {
                        debug!(
                            "Mercury: request rewritten (url changed: {}, origin changed: {}, headers set: {}, headers removed: {})",
                            applied.url_changed,
                            applied.origin_changed,
                            applied.headers_set,
                            applied.headers_removed
                        );
                    }
                    break;
                },
            }
        }
    }
}
