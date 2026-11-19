/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the net lane. Add variants (and the payload types they need) here
// only, never in another lane's file or in a shared enum.

use std::fmt;

use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

/// Messages from the engine to Mercury for the `net` lane (network policy, cookies, privacy).
///
/// Empty until that lane adds its variants. Messages that come from the network thread rather than
/// from script or the constellation keep using `NetToEmbedderMsg` in `servo-net`.
///
/// `Debug` prints only the variant name, never a payload.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum NetMsg {}

impl fmt::Debug for NetMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}
