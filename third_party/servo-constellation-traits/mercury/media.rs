/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the media lane. Add variants (and the payload types they need) here
// only, never in another lane's file or in a shared enum.

use std::fmt;

use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

/// Messages from a script thread to the constellation for the `media` lane.
///
/// The lane covers MSE, EME, autoplay, capture and WebRTC. Empty until it adds its variants.
/// `Debug` prints only the variant name, never a payload.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum MediaScriptMsg {}

impl fmt::Debug for MediaScriptMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}

/// Messages from the embedding layer to the constellation for the `media` lane.
///
/// The lane covers MSE, EME, autoplay, capture and WebRTC. Empty until it adds its variants.
/// `Debug` prints only the variant name, never a payload.
#[derive(IntoStaticStr)]
pub enum MediaEmbedderMsg {}

impl fmt::Debug for MediaEmbedderMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}
