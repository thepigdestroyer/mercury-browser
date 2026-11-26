/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this whole directory is Mercury's own (K0 engine base, R14 section 3.4). It holds the
// one message variant, `EmbedderMsg::Mercury`, through which every Mercury lane sends its own
// messages from the engine to the embedder, so that a lane adds variants in its own file here and
// never in a shared enum. Nothing in upstream Servo refers to it.

//! Messages from the engine to Mercury, one sub-enum per lane.
//!
//! [`EmbedderMsg::Mercury`](crate::EmbedderMsg::Mercury) carries a [`MercuryEmbedderMsg`]. Each
//! lane owns exactly one file in this directory (`a11y.rs`, `core.rs`, `media.rs`, `net.rs`,
//! `page.rs`, `text.rs`) and adds its variants, and the payload types they need, only there. The
//! public items of every lane file are re-exported from this module and from the crate root, so
//! they are reachable as `embedder_traits::*` and (through the `servo` crate) as `servo::*`.
//!
//! The lane files are private modules on purpose: names such as `core` and `net` must not become
//! public names of the crate, where a glob re-export would shadow the `core` and `net` crates.
//!
//! The `Debug` implementations in this directory print only the variant path, for example
//! `Core::Probe`, and never a payload: what a page sent must not reach a log.

mod a11y;
mod clipboard;
mod core;
mod forms;
mod media;
mod net;
mod page;
mod text;

use std::fmt;

use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

pub use self::a11y::*;
pub use self::clipboard::*;
pub use self::core::*;
pub use self::forms::*;
pub use self::media::*;
pub use self::net::*;
pub use self::page::*;
pub use self::text::*;

/// A message from the engine to Mercury, grouped by the lane that owns it.
///
/// Sent as [`EmbedderMsg::Mercury`](crate::EmbedderMsg::Mercury) and delivered to
/// `WebViewDelegate::notify_mercury_message` (or, without a web view, to
/// `ServoDelegate::notify_mercury_message`) in the `servo` crate. Like `EmbedderMsg` itself it
/// crosses the process boundary, so every payload must be `Serialize + Deserialize`.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum MercuryEmbedderMsg {
    /// Scaffold and test messages that belong to no lane.
    Core(CoreMsg),
    /// Accessibility (the `a11y` lane).
    A11y(A11yMsg),
    /// Media: MSE, EME, autoplay, capture and WebRTC (the `media` lane).
    Media(MediaMsg),
    /// Network and privacy messages that need the embedder (the `net` lane).
    Net(NetMsg),
    /// Page and document features: storage, locks, viewport (the `page` lane).
    Page(PageMsg),
    /// Text input and editing: IME, composition, spelling (the `text` lane).
    Text(TextMsg),
}

impl fmt::Debug for MercuryEmbedderMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let lane: &'static str = self.into();
        let variant: &'static str = match self {
            Self::Core(message) => message.into(),
            Self::A11y(message) => message.into(),
            Self::Media(message) => message.into(),
            Self::Net(message) => message.into(),
            Self::Page(message) => message.into(),
            Self::Text(message) => message.into(),
        };
        write!(formatter, "{lane}::{variant}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_static<T: Send + 'static>() {}

    #[test]
    fn every_lane_type_is_send_and_static() {
        assert_send_static::<MercuryEmbedderMsg>();
        assert_send_static::<CoreMsg>();
        assert_send_static::<A11yMsg>();
        assert_send_static::<MediaMsg>();
        assert_send_static::<NetMsg>();
        assert_send_static::<PageMsg>();
        assert_send_static::<TextMsg>();
    }

    #[test]
    fn debug_prints_only_the_variant_path() {
        let message = MercuryEmbedderMsg::Core(CoreMsg::Probe(0xDEAD_BEEF));
        assert_eq!(format!("{message:?}"), "Core::Probe");
        assert_eq!(format!("{:?}", CoreMsg::Probe(7)), "Probe");
    }

    #[test]
    fn the_variant_is_named_for_its_lane() {
        let message = MercuryEmbedderMsg::Core(CoreMsg::Probe(1));
        let lane: &'static str = (&message).into();
        assert_eq!(lane, "Core");
    }

    /// The size of `EmbedderMsg` in Servo 0.8.0 on a 64-bit target, measured on 7f1a78e before the
    /// Mercury variant existed: its largest variant is `ShowEmbedderControl` (248 bytes with its tag).
    /// Every message in the channel is this big, so the new variant must not grow the enum; the
    /// allowance is one pointer.
    #[cfg(target_pointer_width = "64")]
    const EMBEDDER_MSG_SIZE_IN_SERVO_0_8: usize = 248;

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_mercury_variant_did_not_grow_the_embedder_message() {
        use std::mem::size_of;

        let size = size_of::<crate::EmbedderMsg>();
        assert!(
            size <= EMBEDDER_MSG_SIZE_IN_SERVO_0_8 + size_of::<usize>(),
            "EmbedderMsg is {size} bytes; Servo 0.8.0 had {EMBEDDER_MSG_SIZE_IN_SERVO_0_8}"
        );
        // The wrapper pair is 40 bytes on the scaffold's own layout (measured on HEAD before the text
        // lane's clipboard variants); the boxed text variants keep it from growing further.
        assert!(size_of::<(Option<servo_base::id::WebViewId>, MercuryEmbedderMsg)>() <= 40);
    }

    #[test]
    fn the_engine_message_wrapper_names_only_itself() {
        let message = crate::EmbedderMsg::Mercury(
            None,
            MercuryEmbedderMsg::Core(CoreMsg::Probe(0xDEAD_BEEF)),
        );
        assert_eq!(format!("{message:?}"), "Mercury");
    }
}
