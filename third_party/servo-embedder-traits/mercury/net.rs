/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the net lane. Add variants (and the payload types they need) here
// only, never in another lane's file or in a shared enum.

use std::fmt;

use content_security_policy::Destination;
use serde::{Deserialize, Serialize};
use servo_url::ServoUrl;
use strum::IntoStaticStr;

/// Messages from the engine to Mercury for the `net` lane (network policy, cookies, privacy).
///
/// Messages that come from the network thread rather than from script or the constellation reach the
/// embedder as `NetToEmbedderMsg::Mercury` in `servo-net`, which carries a [`MercuryEmbedderMsg`] and
/// ends in the same delivery function as the other lanes' messages.
///
/// `Debug` prints only the variant name, never a payload.
///
/// [`MercuryEmbedderMsg`]: crate::MercuryEmbedderMsg
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum NetMsg {
    /// A request was blocked as mixed content: a secure page asked for something that is not
    /// potentially trustworthy, and the user has not allowed mixed content on the page's site (EC17).
    /// Sent once for each blocked request, naming the web view the request belongs to when it has one.
    MixedContentBlocked {
        /// The URL of the blocked request (of the request, not of a response that was blocked after
        /// it was answered).
        url: ServoUrl,
        /// What the blocked request was for (script, image, fetch and so on).
        destination: Destination,
    },
}

impl fmt::Debug for NetMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}

/// How the embedder changes a request before it goes on: the payload of
/// `WebResourceResponseMsg::Rewrite` (EC37).
///
/// The network thread applies it with the rules of `servo-net/mercury_rewrite.rs` (the scheme must stay,
/// the origin must stay unless the request is a top-level navigation, only `http` and `https`, no
/// credential, cookie or connection headers, bounded sizes) and refuses the whole rewrite, leaving the
/// request as it was, when any part breaks a rule. An empty rewrite changes nothing.
///
/// `Debug` shows only which parts are set, never a URL or a header value.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct WebResourceRewrite {
    /// The new URL of the request's current hop. Must keep the scheme and, unless the request is a
    /// top-level navigation, the origin.
    pub url: Option<ServoUrl>,
    /// Headers to set (replacing any value the request has), as (name, value).
    pub set_headers: Vec<(String, String)>,
    /// Names of headers to remove. Removals are done before the headers are set.
    pub remove_headers: Vec<String>,
}

impl fmt::Debug for WebResourceRewrite {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("WebResourceRewrite")
            .field("url", &self.url.as_ref().map(|_| "<set>"))
            .field("set_headers", &self.set_headers.len())
            .field("remove_headers", &self.remove_headers.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_content_message_debug_is_the_variant_name_only() {
        let message = NetMsg::MixedContentBlocked {
            url: ServoUrl::parse("http://insecure.example/script.js?token=secret").unwrap(),
            destination: Destination::Script,
        };
        assert_eq!(format!("{message:?}"), "MixedContentBlocked");
        let wrapped = crate::MercuryEmbedderMsg::Net(message);
        assert_eq!(format!("{wrapped:?}"), "Net::MixedContentBlocked");
    }

    #[test]
    fn mixed_content_message_is_send_and_static() {
        fn assert_send_static<T: Send + 'static>() {}
        assert_send_static::<NetMsg>();
        assert_send_static::<WebResourceRewrite>();
    }

    #[test]
    fn an_empty_rewrite_is_the_default_and_sets_nothing() {
        let rewrite = WebResourceRewrite::default();
        assert!(rewrite.url.is_none());
        assert!(rewrite.set_headers.is_empty());
        assert!(rewrite.remove_headers.is_empty());
    }

    #[test]
    fn rewrite_debug_shows_no_url_and_no_header_value() {
        let rewrite = WebResourceRewrite {
            url: Some(ServoUrl::parse("https://a.example/?session=hunter2").unwrap()),
            set_headers: vec![("X-Token".to_owned(), "hunter2".to_owned())],
            remove_headers: vec!["X-Other".to_owned()],
        };
        let printed = format!("{rewrite:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("a.example"), "{printed}");
        assert!(!printed.contains("X-Token"), "{printed}");
        assert!(printed.contains("set_headers: 1"), "{printed}");
        assert!(printed.contains("remove_headers: 1"), "{printed}");
    }

    #[test]
    fn the_net_message_did_not_make_the_wrapper_large() {
        // `EmbedderMsg` and `NetToEmbedderMsg` carry `(Option<WebViewId>, MercuryEmbedderMsg)`; the
        // scaffold's own test pins 32 bytes for that pair, and this variant (a URL pointer and a
        // one-byte destination) has to fit.
        #[cfg(target_pointer_width = "64")]
        {
            let size = std::mem::size_of::<(
                Option<servo_base::id::WebViewId>,
                crate::MercuryEmbedderMsg,
            )>();
            assert!(size <= 32, "{size} bytes");
        }
    }
}
