// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Shared plain types of the ring geometry. Frozen: the submodules are written in parallel
//! against exactly these definitions (see the conventions in `ring_geometry.rs`).

/// A position or an offset, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An ellipse by its **half-axes** (radii): `rx` horizontal, `ry` vertical. The SPEC's
/// "470 × 290 px" Orbit ellipse is `Ellipse { rx: 470.0, ry: 290.0 }` (measured on
/// `orbit-1-overview`: the drawn ellipse is 940 px wide).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    pub rx: f32,
    pub ry: f32,
}

/// The point at `radius` and `deg` (clockwise from 12 o'clock) around `centre`.
pub fn point_on_circle(centre: Point, radius: f32, deg: f32) -> Point {
    let rad = deg.to_radians();
    Point::new(centre.x + radius * rad.sin(), centre.y - radius * rad.cos())
}

/// `deg` folded into `[0, 360)`.
pub fn normalise_deg(deg: f32) -> f32 {
    let folded = deg.rem_euclid(360.0);
    // `rem_euclid` can return exactly 360.0 for tiny negative inputs after rounding.
    if folded >= 360.0 { 0.0 } else { folded }
}

/// The size and shape of one ring of tab segments. `Default` is the hub ring's popover
/// (SPEC §4.3): a 516 px box, segment band radius 180 and width 30, label band 24.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingSpec {
    /// The ring's centre in the component's box.
    pub centre: Point,
    /// Radius of the tab band's centre line.
    pub radius: f32,
    /// Width of the tab band.
    pub band_width: f32,
    /// Space between the tab band's outer edge and the label band.
    pub label_gap: f32,
    /// Width of the label band.
    pub label_width: f32,
    /// Where the arc begins, in degrees clockwise from 12 o'clock. For a full ring (span 360)
    /// the reserve is centred on this angle.
    pub start_deg: f32,
    /// The angle the ring covers: 360 for a ring, less for the dial's fan.
    pub span_deg: f32,
    /// Empty angle kept free (half at each end of the arc), for the pinned beads at 12 o'clock.
    pub reserve_deg: f32,
    /// Extra angle between two groups (a full ring has one gap per group, a fan one fewer).
    pub group_gap_deg: f32,
    /// Angle between two neighbouring tab segments of the same group.
    pub tab_gap_deg: f32,
    /// How far a group's label band is drawn inside the group's extent, at each end.
    pub label_inset_deg: f32,
    /// How far a hovered segment is lifted outward (hit-testing keeps the lifted part).
    pub lift: f32,
}

impl Default for RingSpec {
    fn default() -> Self {
        Self {
            centre: Point::new(258.0, 258.0),
            radius: 180.0,
            band_width: 30.0,
            label_gap: 2.0,
            label_width: 24.0,
            start_deg: 0.0,
            span_deg: 360.0,
            reserve_deg: 24.0,
            group_gap_deg: 5.0,
            tab_gap_deg: 1.2,
            label_inset_deg: 0.4,
            lift: 10.0,
        }
    }
}

impl RingSpec {
    /// Inner edge of the tab band.
    pub fn inner_radius(&self) -> f32 {
        self.radius - self.band_width / 2.0
    }
    /// Outer edge of the tab band.
    pub fn outer_radius(&self) -> f32 {
        self.radius + self.band_width / 2.0
    }
    /// Radius of the label band's centre line (where label glyphs sit).
    pub fn label_radius(&self) -> f32 {
        self.outer_radius() + self.label_gap + self.label_width / 2.0
    }
    /// Inner edge of the label band.
    pub fn label_inner_radius(&self) -> f32 {
        self.outer_radius() + self.label_gap
    }
    /// Outer edge of the label band.
    pub fn label_outer_radius(&self) -> f32 {
        self.label_inner_radius() + self.label_width
    }
}

/// The angles of one tab segment. Tabs are numbered flat, in ring order (group by group).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabArc {
    /// Flat index of the tab in ring order.
    pub tab: usize,
    /// Index of the tab's group in the layout's group list.
    pub group: usize,
    pub index_in_group: usize,
    /// The tab's whole slot, before the gap between segments is taken off.
    pub slot_start_deg: f32,
    pub slot_end_deg: f32,
    /// What is drawn (and hit): the slot inset by half the tab gap at each end.
    pub start_deg: f32,
    pub end_deg: f32,
}

/// The angles of one group: its tabs' slots from the first to the last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupArc {
    pub group: usize,
    /// Flat index of the group's first tab.
    pub first_tab: usize,
    /// A group with no tabs takes no angle and no gap.
    pub tab_count: usize,
    pub start_deg: f32,
    pub end_deg: f32,
    /// The label band's extent: the group's extent inset by `RingSpec::label_inset_deg`.
    pub label_start_deg: f32,
    pub label_end_deg: f32,
}

/// Every tab's and group's angles for one ring, computed once from a `RingSpec` and the number
/// of tabs in each group (`layout::RingLayout::new`). Hit-testing and path building read it.
#[derive(Clone, Debug, PartialEq)]
pub struct RingLayout {
    pub spec: RingSpec,
    /// The angle of one tab slot (equal for every tab in the ring).
    pub pitch_deg: f32,
    pub tabs: Vec<TabArc>,
    pub groups: Vec<GroupArc>,
}

/// What a pointer position is over. `Tab` carries the flat tab index, `Group` the group index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Hit {
    Tab(usize),
    Group(usize),
    /// Inside the tab band's inner edge: the centre disc and the empty ring around it.
    Centre,
    /// The reserve at 12 o'clock where the pinned beads sit, at the tab band's radii.
    Reserve,
    /// Nothing interactive: beyond the label band, a group gap, or a label band over no group.
    Outside,
}

/// One arc path for the Slint `Path` element: a tab segment (`tab` is the flat tab index) or a
/// group's label band (`tab` is the group index).
#[derive(Clone, Debug, PartialEq)]
pub struct ArcSegment {
    pub tab: usize,
    /// SVG path commands in ring-local coordinates, a closed filled shape (see `paths.rs`).
    pub commands: String,
    /// The middle of the shape: where flags and tooltips anchor.
    pub mid: Point,
}

/// What a label glyph is, so the Slint side can pick its colour from the palette (Rust knows no
/// colours).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphRole {
    /// A letter of the group's name.
    Name,
    /// The "⌥" and the key letter.
    Key,
    /// A digit of the tab count.
    Count,
}

/// One glyph of a curved label, placed on the label band.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphPlacement {
    /// One character (a grapheme is not split further; "⌥" is one glyph).
    pub text: String,
    /// The centre of the glyph's box, ring-local; the Slint `Text` is centred on it.
    pub centre: Point,
    /// Rotation in degrees, clockwise, around the glyph box's centre (Slint `transform-rotation`).
    pub angle_deg: f32,
    pub size: f32,
    pub weight: u16,
    pub role: GlyphRole,
}
