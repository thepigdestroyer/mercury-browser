// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The data of the dial's fan: `RingBandsData` with the corner's `RingSpec`, where its box sits and
//! the hover caption.
//!
//! The fan is a 768 px square (`FAN_BOX`) centred on the dial, so every coordinate in `data` is
//! relative to the box and the dial is at `(FAN_BOX / 2, FAN_BOX / 2)` in it. [`DialFan::origin`]
//! says where that box lies in the overlay area.
//!
//! The caption (decision D15) is a 186 x 30 px box whose near corner is 24 px from the hovered
//! segment's mid-point toward the dial in both axes (the mockup, for the bottom-right dial:
//! top-left corner at mid-point + (24, 24)); in the other corners the box is mirrored so that it
//! still lies on the dial's side of the point. It is clamped into the fan's box.

use mercury_settings::DialCorner;
use slint::SharedString;

use crate::ring_bands::{RingBandsData, RingOptions};
use crate::ring_geometry::{GlyphMetrics, Point};
use crate::{DialCaption, GroupRow, TabRow};

use super::FAN_BOX;
use super::geometry::{DialGeometry, fan_spec};

/// Width of the caption box.
const CAPTION_WIDTH: f32 = 186.0;
/// Height of the caption box.
const CAPTION_HEIGHT: f32 = 30.0;
/// Distance from the hovered segment's mid-point to the caption's near corner, in x and in y.
const CAPTION_OFFSET: f32 = 24.0;
/// What the caption says for a tab that has neither a title nor a host.
const UNTITLED: &str = "Untitled";

/// Everything the fan of one corner needs.
#[derive(Clone, Debug, PartialEq)]
pub struct DialFan {
    pub corner: DialCorner,
    pub data: RingBandsData,
    /// The top-left corner of the `RingBands` box in the overlay area (the dial's centre minus
    /// `FAN_BOX / 2` in both directions).
    pub origin: Point,
}

impl DialFan {
    pub fn new(
        geometry: &DialGeometry,
        corner: DialCorner,
        groups: &[GroupRow],
        tabs: &[TabRow],
        opts: &RingOptions,
        metrics: &dyn GlyphMetrics,
    ) -> Self {
        let centre = geometry.centre(corner);
        Self {
            corner,
            data: RingBandsData::new(fan_spec(corner), groups, tabs, opts, metrics),
            origin: Point::new(centre.x - FAN_BOX / 2.0, centre.y - FAN_BOX / 2.0),
        }
    }

    /// "Group · Title" at the hovered segment, 24 px toward the dial, clamped into the fan's box;
    /// `None` when no shown tab has the id `hovered_tab`.
    ///
    /// The title is the host when the tab has none and "Untitled" when it has neither; a group
    /// without a name leaves the title alone. `x` and `y` of the result are the top-left corner of
    /// the box in the fan box's coordinates.
    pub fn caption(&self, hovered_tab: i32) -> Option<DialCaption> {
        let segment = self.data.segments.iter().find(|s| s.tab == hovered_tab)?;
        let tab = self.data.tabs.iter().find(|t| t.id == hovered_tab)?;
        let group = self.data.groups.iter().find(|g| g.id == tab.group);

        let title = if !tab.title.is_empty() {
            tab.title.as_str()
        } else if !tab.host.is_empty() {
            tab.host.as_str()
        } else {
            UNTITLED
        };
        let text = match group {
            Some(g) if !g.name.is_empty() => format!("{} · {}", g.name, title),
            _ => title.to_owned(),
        };

        let dial = FAN_BOX / 2.0;
        let (mid_x, mid_y) = (segment.mid_x, segment.mid_y);
        let toward = |mid: f32, size: f32| {
            if mid < dial {
                mid + CAPTION_OFFSET
            } else {
                mid - CAPTION_OFFSET - size
            }
        };
        let x = toward(mid_x, CAPTION_WIDTH).clamp(0.0, FAN_BOX - CAPTION_WIDTH);
        let y = toward(mid_y, CAPTION_HEIGHT).clamp(0.0, FAN_BOX - CAPTION_HEIGHT);
        Some(DialCaption {
            text: SharedString::from(text),
            x,
            y,
        })
    }
}
