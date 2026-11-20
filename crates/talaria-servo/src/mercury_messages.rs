// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The router for the engine's Mercury messages.
//!
//! The patched Servo crates (`third_party/servo-embedder-traits/mercury/`) carry every message
//! that a Mercury lane needs from the engine in one variant, `EmbedderMsg::Mercury`, holding a
//! [`servo::MercuryEmbedderMsg`] with one sub-enum per lane. `runtime.rs` forwards both delegate
//! hooks (`WebViewDelegate::notify_mercury_message` and `ServoDelegate::notify_mercury_message`)
//! to [`handle`], and [`handle`] gives each message to its lane.
//!
//! # How a lane takes over its messages
//!
//! 1. Add the variants in the lane's own file in `servo-embedder-traits/mercury/`.
//! 2. Replace the call in the lane's arm below with the lane's own function (preferably in a
//!    module of its own), and nothing else in this file. Each arm is separated from the next by a
//!    comment line, so two lanes' edits never touch adjacent lines.
//!
//! A message that no lane has taken over is only logged at `debug` level. The log shows the
//! variant path (for example `Core::Probe`) and never a payload, because what a page sent must not
//! reach a log.

use servo::{CoreMsg, MercuryEmbedderMsg};
use talaria_core::TabId;

/// Handle one Mercury message from the engine. `tab` is the tab of the web view the message
/// concerns, or `None` for a message that names no web view (or whose web view has gone).
pub(crate) fn handle(tab: Option<TabId>, message: MercuryEmbedderMsg) {
    match message {
        MercuryEmbedderMsg::Core(message) => handle_core(tab, message),
        // The a11y lane: replace this call with the lane's handler.
        MercuryEmbedderMsg::A11y(message) => unhandled(tab, &message),
        // The media lane: replace this call with the lane's handler.
        MercuryEmbedderMsg::Media(message) => unhandled(tab, &message),
        // The net lane: replace this call with the lane's handler.
        MercuryEmbedderMsg::Net(message) => unhandled(tab, &message),
        // The page lane: replace this call with the lane's handler.
        MercuryEmbedderMsg::Page(message) => unhandled(tab, &message),
        // The text lane: replace this call with the lane's handler.
        MercuryEmbedderMsg::Text(message) => unhandled(tab, &message),
    }
}

/// The scaffold's own messages. `Probe` exists so that the types can be constructed in tests; the
/// engine never sends it in the product.
fn handle_core(tab: Option<TabId>, message: CoreMsg) {
    match message {
        CoreMsg::Probe(value) => {
            log::debug!("Mercury engine message Core::Probe({value}) for {tab:?}");
        }
    }
}

/// A message whose lane has not taken it over: one log line with the variant path only.
fn unhandled(tab: Option<TabId>, message: &dyn std::fmt::Debug) {
    log::debug!("unhandled Mercury engine message {message:?} for {tab:?}");
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::*;

    /// Compile-time check that a lane type can cross threads (and the process boundary): it is
    /// `Send + 'static`, serialisable and deserialisable without borrowing. `serde_json`'s
    /// functions carry the `Serialize` and `DeserializeOwned` bounds, so this crate needs no
    /// `serde` dependency of its own.
    macro_rules! assert_lane_types {
        ($($lane_type:ty),+ $(,)?) => {
            $({
                fn is_send_and_static<T: Send + 'static>() {}
                is_send_and_static::<$lane_type>();
                let _ = serde_json::to_vec::<$lane_type>;
                let _ = serde_json::from_reader::<&[u8], $lane_type>;
            })+
        };
    }

    #[test]
    fn every_lane_type_is_send_static_and_serialisable() {
        // The engine to the embedder (`servo-embedder-traits/mercury/`).
        assert_lane_types!(
            servo::EmbedderMsg,
            MercuryEmbedderMsg,
            servo::CoreMsg,
            servo::A11yMsg,
            servo::MediaMsg,
            servo::NetMsg,
            servo::PageMsg,
            servo::TextMsg,
        );
        // Script to the constellation (`servo-constellation-traits/mercury/`).
        assert_lane_types!(
            servo::MercuryScriptToConstellation,
            servo::CoreScriptMsg,
            servo::A11yScriptMsg,
            servo::MediaScriptMsg,
            servo::PageScriptMsg,
            servo::TextScriptMsg,
        );
    }

    #[test]
    fn the_embedder_to_constellation_types_are_send_and_static() {
        // They stay in one process and are not serialised.
        fn is_send_and_static<T: Send + 'static>() {}
        is_send_and_static::<servo::MercuryEmbedderToConstellation>();
        is_send_and_static::<servo::CoreEmbedderMsg>();
        is_send_and_static::<servo::A11yEmbedderMsg>();
        is_send_and_static::<servo::MediaEmbedderMsg>();
        is_send_and_static::<servo::PageEmbedderMsg>();
        is_send_and_static::<servo::TextEmbedderMsg>();
    }

    #[test]
    fn a_core_probe_round_trips_through_serde_json() {
        let message = MercuryEmbedderMsg::Core(CoreMsg::Probe(42));
        let text = serde_json::to_string(&message).expect("a probe serialises");
        assert_eq!(text, r#"{"Core":{"Probe":42}}"#);
        let back: MercuryEmbedderMsg = serde_json::from_str(&text).expect("a probe deserialises");
        assert!(matches!(back, MercuryEmbedderMsg::Core(CoreMsg::Probe(42))));
    }

    #[test]
    fn the_engine_message_wrapper_round_trips_through_serde_json() {
        let message =
            servo::EmbedderMsg::Mercury(None, MercuryEmbedderMsg::Core(CoreMsg::Probe(7)));
        let text = serde_json::to_string(&message).expect("the wrapper serialises");
        assert_eq!(text, r#"{"Mercury":[null,{"Core":{"Probe":7}}]}"#);
        let back: servo::EmbedderMsg =
            serde_json::from_str(&text).expect("the wrapper deserialises");
        assert!(matches!(
            back,
            servo::EmbedderMsg::Mercury(None, MercuryEmbedderMsg::Core(CoreMsg::Probe(7)))
        ));
    }

    #[test]
    fn a_probe_is_handled_without_a_panic_with_or_without_a_tab() {
        handle(None, MercuryEmbedderMsg::Core(CoreMsg::Probe(u64::MAX)));
        handle(
            Some(TabId::new(3)),
            MercuryEmbedderMsg::Core(CoreMsg::Probe(0)),
        );
    }

    #[test]
    fn the_debug_output_of_a_message_is_only_its_variant_path() {
        let message = MercuryEmbedderMsg::Core(CoreMsg::Probe(0xDEAD_BEEF));
        assert_eq!(format!("{message:?}"), "Core::Probe");
        let wrapper = servo::EmbedderMsg::Mercury(None, message);
        assert_eq!(format!("{wrapper:?}"), "Mercury");
    }

    /// The size of `EmbedderMsg` in Servo 0.8.0 on a 64-bit target, before Mercury added its
    /// variant (measured on 7f1a78e). Every message in the channel is this big, whichever variant
    /// it is, so the new variant must not make the enum bigger: the allowance is one pointer, in
    /// case a later Servo moves the niche.
    #[cfg(target_pointer_width = "64")]
    const EMBEDDER_MSG_SIZE_IN_SERVO_0_8: usize = 248;

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_mercury_variant_did_not_grow_the_embedder_message() {
        let size = size_of::<servo::EmbedderMsg>();
        assert!(
            size <= EMBEDDER_MSG_SIZE_IN_SERVO_0_8 + size_of::<usize>(),
            "EmbedderMsg is {size} bytes; Servo 0.8.0 had {EMBEDDER_MSG_SIZE_IN_SERVO_0_8}"
        );
        // The variant itself is small: it fits in the space of the existing largest one.
        assert!(
            size_of::<(Option<servo::WebViewId>, MercuryEmbedderMsg)>() <= size,
            "the Mercury variant is bigger than the whole enum had room for"
        );
    }
}
