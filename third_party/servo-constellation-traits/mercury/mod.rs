/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this whole directory is Mercury's own (K0 engine base, R14 section 3.4). It holds the
// two message variants, `ScriptToConstellationMessage::Mercury` and
// `EmbedderToConstellationMessage::Mercury`, through which every Mercury lane sends its own
// messages to the constellation, so that a lane adds variants in its own file here and never in a
// shared enum. Nothing in upstream Servo refers to it.

//! Messages to the `Constellation` from Mercury's lanes, one sub-enum per lane and direction.
//!
//! * [`MercuryScriptToConstellation`] travels from a script thread to the constellation, so it can
//!   cross a process boundary and is `Serialize + Deserialize`.
//! * [`MercuryEmbedderToConstellation`] travels from the embedding layer to the constellation in
//!   the same process and is a plain enum.
//!
//! Each lane owns one file in this directory (`a11y.rs`, `core.rs`, `media.rs`, `page.rs`,
//! `text.rs`) that defines its `<Lane>ScriptMsg` and `<Lane>EmbedderMsg` and the payload types
//! they need. The public items of every lane file are re-exported from the crate root. The lane
//! files are private modules on purpose, so that `core` never becomes a public name of the crate;
//! this module itself is public so that the `servo` crate can re-export all of its items at once.
//! The constellation handles each lane in the matching file of its own `mercury/` directory.
//!
//! The `Debug` implementations here print only the variant path, for example `Core::Probe`, and
//! never a payload: what a page sent must not reach a log. The network lane has no constellation
//! messages; its messages go to the embedder (`embedder_traits::NetMsg`).

mod a11y;
mod core;
mod media;
mod page;
mod text;

use std::fmt;

use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

pub use self::a11y::*;
pub use self::core::*;
pub use self::media::*;
pub use self::page::*;
pub use self::text::*;

/// A message from a script thread to the constellation, grouped by the lane that owns it.
///
/// Sent as [`ScriptToConstellationMessage::Mercury`](crate::ScriptToConstellationMessage::Mercury).
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum MercuryScriptToConstellation {
    /// Scaffold and test messages that belong to no lane.
    Core(CoreScriptMsg),
    /// Accessibility (the `a11y` lane).
    A11y(A11yScriptMsg),
    /// Media: MSE, EME, autoplay, capture and WebRTC (the `media` lane).
    Media(MediaScriptMsg),
    /// Page and document features: storage, locks, viewport (the `page` lane).
    Page(PageScriptMsg),
    /// Text input and editing: IME, composition, spelling (the `text` lane).
    Text(TextScriptMsg),
}

impl fmt::Debug for MercuryScriptToConstellation {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let lane: &'static str = self.into();
        let variant: &'static str = match self {
            Self::Core(message) => message.into(),
            Self::A11y(message) => message.into(),
            Self::Media(message) => message.into(),
            Self::Page(message) => message.into(),
            Self::Text(message) => message.into(),
        };
        write!(formatter, "{lane}::{variant}")
    }
}

/// A message from the embedding layer to the constellation, grouped by the lane that owns it.
///
/// Sent as
/// [`EmbedderToConstellationMessage::Mercury`](crate::EmbedderToConstellationMessage::Mercury).
#[derive(IntoStaticStr)]
pub enum MercuryEmbedderToConstellation {
    /// Scaffold and test messages that belong to no lane.
    Core(CoreEmbedderMsg),
    /// Accessibility (the `a11y` lane).
    A11y(A11yEmbedderMsg),
    /// Media: MSE, EME, autoplay, capture and WebRTC (the `media` lane).
    Media(MediaEmbedderMsg),
    /// Page and document features: storage, locks, viewport (the `page` lane).
    Page(PageEmbedderMsg),
    /// Text input and editing: IME, composition, spelling (the `text` lane).
    Text(TextEmbedderMsg),
}

impl fmt::Debug for MercuryEmbedderToConstellation {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let lane: &'static str = self.into();
        let variant: &'static str = match self {
            Self::Core(message) => message.into(),
            Self::A11y(message) => message.into(),
            Self::Media(message) => message.into(),
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
        assert_send_static::<MercuryScriptToConstellation>();
        assert_send_static::<MercuryEmbedderToConstellation>();
        assert_send_static::<CoreScriptMsg>();
        assert_send_static::<CoreEmbedderMsg>();
        assert_send_static::<A11yScriptMsg>();
        assert_send_static::<A11yEmbedderMsg>();
        assert_send_static::<MediaScriptMsg>();
        assert_send_static::<MediaEmbedderMsg>();
        assert_send_static::<PageScriptMsg>();
        assert_send_static::<PageEmbedderMsg>();
        assert_send_static::<TextScriptMsg>();
        assert_send_static::<TextEmbedderMsg>();
    }

    #[test]
    fn debug_prints_only_the_variant_path() {
        let script = MercuryScriptToConstellation::Core(CoreScriptMsg::Probe(0xDEAD_BEEF));
        assert_eq!(format!("{script:?}"), "Core::Probe");
        let embedder = MercuryEmbedderToConstellation::Core(CoreEmbedderMsg::Probe(0xDEAD_BEEF));
        assert_eq!(format!("{embedder:?}"), "Core::Probe");
        assert_eq!(format!("{:?}", CoreScriptMsg::Probe(7)), "Probe");
        assert_eq!(format!("{:?}", CoreEmbedderMsg::Probe(7)), "Probe");
    }

    #[test]
    fn the_constellation_messages_name_only_themselves() {
        let script = crate::ScriptToConstellationMessage::Mercury(
            MercuryScriptToConstellation::Core(CoreScriptMsg::Probe(0xDEAD_BEEF)),
        );
        assert_eq!(format!("{script:?}"), "ScriptMsg::Mercury");
        let embedder = crate::EmbedderToConstellationMessage::Mercury(
            MercuryEmbedderToConstellation::Core(CoreEmbedderMsg::Probe(0xDEAD_BEEF)),
        );
        assert_eq!(format!("{embedder:?}"), "ConstellationMsg::Mercury");
    }
}
