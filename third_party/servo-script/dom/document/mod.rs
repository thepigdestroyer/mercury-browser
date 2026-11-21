/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: a11y-1 items 4 and 5: when the accessibility tree is damaged, and what an assistive technology's
// actions do.
pub(crate) mod accessibility_actions;
pub(crate) mod accessibility_attributes;
pub(crate) mod accessibility_data;
mod animation_manager;
#[expect(clippy::module_inception, reason = "The interface name is Document")]
pub(crate) mod document;
pub(crate) mod document_embedder_controls;
pub(crate) mod document_event_handler;
pub(crate) mod documentfragment;
pub(crate) mod documentorshadowroot;
pub(crate) mod documenttype;
pub(crate) mod domimplementation;
pub(crate) mod domparser;
pub(crate) mod editing;
pub(crate) mod focus;
mod iframe_collection;
mod interactive_element_command;
mod tree_ordered_index_map;
pub(crate) mod visibilitystateentry;
pub(crate) mod websocket;

pub(crate) use self::document::*;
