/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the core lane (the scaffold itself). Add variants (and the payload
// types they need) here only, never in another lane's file or in a shared enum.

use std::fmt;

use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

/// Messages from a script thread to the constellation that belong to no feature lane.
///
/// `Debug` prints only the variant name, never a payload.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum CoreScriptMsg {
    /// Test only: nothing sends it in the product. It lets the scaffold's types be constructed,
    /// serialised and routed in tests while every lane's enum is still empty. The constellation
    /// only traces it.
    Probe(u64),
}

impl fmt::Debug for CoreScriptMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}

/// Messages from the embedding layer to the constellation that belong to no feature lane.
///
/// `Debug` prints only the variant name, never a payload.
#[derive(IntoStaticStr)]
pub enum CoreEmbedderMsg {
    /// Test only: nothing sends it in the product. It lets the scaffold's types be constructed
    /// and routed in tests while every lane's enum is still empty. The constellation only traces
    /// it.
    Probe(u64),
}

impl fmt::Debug for CoreEmbedderMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}
