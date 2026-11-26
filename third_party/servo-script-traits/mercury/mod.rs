/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the messages Mercury's lanes send to a script thread. One payload type per lane; add
// lanes here, but never grow `ScriptThreadMessage` itself.

/// The `page` lane's messages.
pub mod page;

use std::fmt;

use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

pub use self::page::PageThreadMsg;

/// Every Mercury message to a script thread, wrapped once in `ScriptThreadMessage::Mercury`.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum MercuryScriptThreadMsg {
    /// A message of the `page` lane.
    Page(PageThreadMsg),
}

impl fmt::Debug for MercuryScriptThreadMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Page(message) => write!(formatter, "Page({message:?})"),
        }
    }
}
