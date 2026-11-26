// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub ring popover's side of the browser (hub-ring lane, SPEC `design/ui-final/SPEC.md` §4.3):
//! turns the controller's groups and tabs into the rows of `mercury_ui::hub_ring::HubRingInput`
//! and carries out what the ring asks for through `HubRingHost`. The search, the key navigation,
//! the drawing and the controller are in `mercury-ui` (brief D2); this file is only the adapter.
//!
//! Nothing instantiates it until the layout-switch lane puts `RingPopover` and `HubButton` into
//! `MainWindow` and calls [`install`] (brief D7). That lane builds the [`DesktopActions`] with
//! [`main_window_actions`] and calls [`rows_with`] (or [`rows`]) and `HubRing::set_input` whenever
//! the tabs, the groups, the scheme or the settings change.
//!
//! The rows follow the conventions of the ring's own fixtures (`ring_bands::fixtures`):
//!
//! - a group row exists for every group of the window's `GroupBook`, in the book's order, and one
//!   last pseudo group "Loose" (`LOOSE_GROUP_ID`) while a tab that is neither in a group nor pinned
//!   exists; `GroupRow.count` is the number of tabs that get a segment (shown tabs), so it always
//!   equals the number of `TabRow`s of that group that are not pinned;
//! - every tab of the window has a `TabRow`. A pinned tab is never in a group, so its row says
//!   `LOOSE_GROUP_ID` and `index_in_group` 0; the ring draws it as a bead, not a segment.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::SystemTime;

use mercury_appearance::Colour;
use mercury_appearance::group_colour::{
    self, GroupColour as PaletteColour, MIN_TEXT_CONTRAST, Swatch, tab_shade,
};
use mercury_settings::AppearanceSettings;
use mercury_ui::hub_ring::{HubRing, HubRingHost, HubRingInput, LOOSE_GROUP_ID, TabMedia};
use mercury_ui::ring_bands::RingColours;
use mercury_ui::{ChromePalette, GroupRow, HubRingBridge, MainWindow, TabRow};
use slint::{Color, ComponentHandle, Global, SharedString};
use talaria_core::{
    BrowserCommand, BrowserController, CloseTabOutcome, GroupColour, GroupId, GroupKey,
    GroupSummary, MediaPlaybackState, MediaPosition, MediaSessionAction, PinnedCloseOutcome, Tab,
    TabActivation, TabId, load_progress_estimate,
};

use crate::ui_sync::{slint_image_from_page_icon, sync_ui};

/// How long ⌥ is held before the key look starts when the settings are not at hand
/// (`appearance.tab_groups.keys.hold_ms`'s default).
const DEFAULT_HOLD_MS: u32 = 400;

/// `TabRow.progress` of a loading tab whose progress is not known.
const UNKNOWN_PROGRESS: f32 = -1.0;

/// What the rows need besides the controller: the settings the ring reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowSettings {
    /// `HubRingInput.hold_ms`: how long ⌥ is down before the key look.
    pub(crate) hold_ms: u32,
    /// `HubRingInput.orbit_available`: "See everything" is drawn.
    pub(crate) orbit_available: bool,
    /// `HubRingInput.reduce_motion`.
    pub(crate) reduce_motion: bool,
    /// Whether to fetch the tab snapshots. Each one is a copy of up to 480 x 300 pixels, so a
    /// caller that rebuilds the rows on every tick and does not need new pictures (the ring is
    /// closed) can leave them out. Without them the ring shows its placeholder cards.
    pub(crate) previews: bool,
}

impl Default for RowSettings {
    fn default() -> Self {
        Self {
            hold_ms: DEFAULT_HOLD_MS,
            orbit_available: false,
            reduce_motion: false,
            previews: true,
        }
    }
}

impl RowSettings {
    /// The hold time from `appearance.tab_groups.keys.hold_ms` (checked by
    /// `mercury_appearance::resolve`: a value out of range is the default 400), and whether Orbit
    /// exists. The other fields keep their defaults.
    pub(crate) fn from_appearance(appearance: &AppearanceSettings, orbit_available: bool) -> Self {
        let (resolved, _repairs) = mercury_appearance::resolve(appearance);
        Self {
            hold_ms: u32::try_from(resolved.tab_groups.key_hold.as_millis())
                .unwrap_or(DEFAULT_HOLD_MS),
            orbit_available,
            ..Self::default()
        }
    }
}

/// The window's live colours, which the rows' swatches and the ring's label colours are computed
/// from. Nothing here is designed against one palette: the surface, the ink and the accent are
/// whatever the window's `ChromePalette` holds now, in the scheme it shows now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowStyle {
    /// The scheme the window shows (`ChromePalette.dark`).
    pub(crate) dark: bool,
    /// `ChromePalette.popup-background`: what the ring is drawn on, the surface of the tints.
    pub(crate) popup_background: Colour,
    /// `ChromePalette.text`: the ink read on the tints.
    pub(crate) text: Colour,
    /// The ring's key and count colours (`RingColours::from_palette`); sent as
    /// `HubRingInput.colours`.
    pub(crate) ring: RingColours,
}

impl RowStyle {
    /// Reads the window's live `ChromePalette`; build it again (cheap) whenever the theme, the
    /// scheme or the palette may have changed, that is for every `rows` call.
    pub(crate) fn from_window<C>(window: &C) -> Self
    where
        C: ComponentHandle,
        for<'a> ChromePalette<'a>: Global<'a, C>,
    {
        let palette = window.global::<ChromePalette>();
        Self {
            dark: palette.get_dark(),
            popup_background: appearance_colour(palette.get_popup_background()),
            text: appearance_colour(palette.get_text()),
            ring: RingColours::from_palette(&palette),
        }
    }
}

/// A window colour as the appearance crate's `Colour` (opaque: a group swatch is never drawn
/// translucent).
fn appearance_colour(colour: Color) -> Colour {
    Colour::rgb(colour.red(), colour.green(), colour.blue())
}

// ---------------------------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------------------------

/// The rows of the ring for the window's current groups and tabs: every tab (pinned too), the
/// group rows (loose tabs get the pseudo group `LOOSE_GROUP_ID`), swatches from the live `style`,
/// flags from `TabStatus`, snapshots from `tab_snapshots::image_for`. `now` makes "2 min ago".
/// Settings are the defaults: see [`rows_with`].
///
/// layout-switch: this is `rows(controller, now)` plus one argument, the [`RowStyle`] of the
/// window (`RowStyle::from_window(&window)`), so that no colour here is a palette's own.
pub(crate) fn rows(
    controller: &BrowserController,
    now: SystemTime,
    style: &RowStyle,
) -> HubRingInput {
    rows_with(controller, now, style, &RowSettings::default())
}

/// [`rows`] with the settings of the ring.
///
/// Load progress: Servo reports none, so a loading tab's `progress` is the easing estimate
/// (`load_progress_estimate`) of the time since this function first saw the tab loading, and
/// -1.0 (unknown) on that first sight. It is as exact as the calls are frequent: call it when a
/// load starts (every `sync_ui` does) for the estimate to start with the load.
pub(crate) fn rows_with(
    controller: &BrowserController,
    now: SystemTime,
    style: &RowStyle,
    settings: &RowSettings,
) -> HubRingInput {
    let dark = style.dark;
    let active_tab = controller.active_tab_id();
    let mut groups = Vec::new();
    let mut bases: HashMap<i32, PaletteColour> = HashMap::new();
    let mut last_active = Vec::new();

    for group in controller.groups() {
        let Some(id) = group_ring_id(group.id) else {
            continue;
        };
        let members = controller.group_tabs(group.id);
        if members.is_empty() {
            continue;
        }
        let base = palette_colour(group.colour);
        let swatch = swatch_for(base, style);
        let summary = controller.group_summary(group.id).unwrap_or_default();
        groups.push(GroupRow {
            id,
            name: group.name.as_str().into(),
            colour: slint_colour(swatch.colour),
            tint: slint_colour(swatch.tint),
            deep: slint_colour(swatch.deep),
            key_label: key_label(group.key),
            count: to_count(members.len()),
            last_used: ago(now, group.last_active).into(),
            active: members.contains(&active_tab),
            any_loading: summary.any_loading,
            any_audible: summary.any_audible,
            any_unread: summary.any_unread,
        });
        bases.insert(id, base);
        if let Some(tab) = controller
            .group_tabs_by_recency(group.id)
            .first()
            .copied()
            .and_then(ring_id)
        {
            last_active.push((id, tab));
        }
    }

    let loose = controller.loose_tabs();
    if !loose.is_empty() {
        let base = slate();
        let swatch = swatch_for(base, style);
        let summary =
            GroupSummary::from_statuses(loose.iter().filter_map(|tab| controller.tab_status(*tab)));
        groups.push(GroupRow {
            id: LOOSE_GROUP_ID,
            name: "Loose".into(),
            colour: slint_colour(swatch.colour),
            tint: slint_colour(swatch.tint),
            deep: slint_colour(swatch.deep),
            key_label: SharedString::new(),
            count: to_count(loose.len()),
            // Loose tabs have no activity stamp (the book stamps groups only).
            last_used: SharedString::new(),
            active: loose.contains(&active_tab),
            any_loading: summary.any_loading,
            any_audible: summary.any_audible,
            any_unread: summary.any_unread,
        });
        bases.insert(LOOSE_GROUP_ID, base);
    }

    // The mark of a pinned tab's bead: the slate swatch's `deep`, as the appearance crate makes it
    // (white reads on it at 4.5:1 in every scheme), not the lightened `deep` of the group rows.
    let pinned_mark = slint_colour(mark_for(slate(), style));
    let open: HashSet<TabId> = controller.tabs().map(Tab::id).collect();
    let mut tabs = Vec::new();
    let mut media = Vec::new();
    let mut previews = Vec::new();
    let mut per_group: HashMap<i32, i32> = HashMap::new();

    LOAD_CLOCK.with_borrow_mut(|clock| {
        clock.forget_closed(&open);
        for tab in controller.tabs() {
            let Some(id) = ring_id(tab.id()) else {
                continue;
            };
            let state = tab.state();
            let status = controller.tab_status(tab.id()).unwrap_or_default();
            let group = ring_group_of(controller, tab);
            let base = bases.get(&group).copied().unwrap_or_else(slate);
            let (index_in_group, shade) = if tab.is_pinned() {
                (0, pinned_mark)
            } else {
                let next = per_group.entry(group).or_insert(0);
                let index = *next;
                *next += 1;
                let shade_index = usize::try_from(index).unwrap_or(0);
                (index, slint_colour(tab_shade(base.base(), shade_index)))
            };
            let host = host_of(state.url());
            let title = crate::pdf::tab_title(tab.id(), state.page_title());
            tabs.push(TabRow {
                id,
                group,
                index_in_group,
                title: display_title(&title, &host).into(),
                host: host.into(),
                shade,
                favicon: state
                    .page_icon()
                    .map(slint_image_from_page_icon)
                    .unwrap_or_default(),
                active: tab.id() == active_tab,
                pinned: tab.is_pinned(),
                loading: status.loading,
                progress: clock.progress(tab.id(), status.loading, now),
                audible: status.audible,
                muted: status.muted,
                unread: status.unread,
                crashed: status.crashed,
                capture: !status.capture.is_empty(),
                matches_query: true,
            });
            // Every tab with a media session stays controllable: a paused tab keeps its Play
            // button, a muted one its Unmute.
            let session = state.media();
            if status.audible || status.muted || session.playback() != MediaPlaybackState::None {
                media.push(media_for(
                    id,
                    session.playback(),
                    status.muted,
                    session.position(),
                ));
            }
            if settings.previews
                && let Some(image) = crate::tab_snapshots::image_for(tab.id())
            {
                previews.push((id, image));
            }
        }
    });

    HubRingInput {
        groups,
        tabs,
        media,
        previews,
        last_active,
        dark,
        colours: Some(style.ring),
        orbit_available: settings.orbit_available,
        reduce_motion: settings.reduce_motion,
        hold_ms: settings.hold_ms,
    }
}

/// A tab's id as the ring carries it (`TabRow.id`, an `int`). `None` for the id that does not fit
/// (one per opened tab: it never happens in practice); such a tab has no row.
fn ring_id(tab: TabId) -> Option<i32> {
    i32::try_from(tab.value()).ok()
}

/// A group's id as the ring carries it: `GroupId` never exceeds `i32::MAX` (`MAX_GROUP_ID`).
fn group_ring_id(group: GroupId) -> Option<i32> {
    i32::try_from(group.value()).ok()
}

fn to_count(count: usize) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

/// The ring's group of a tab: its group while the group exists, else the pseudo group. Pinned
/// tabs are in no group.
fn ring_group_of(controller: &BrowserController, tab: &Tab) -> i32 {
    if tab.is_pinned() {
        return LOOSE_GROUP_ID;
    }
    tab.group()
        .and_then(|group| controller.group(group))
        .and_then(|group| group_ring_id(group.id))
        .unwrap_or(LOOSE_GROUP_ID)
}

/// The group's colour in the appearance crate's terms: the preset of the same name, or the custom
/// colour.
fn palette_colour(colour: GroupColour) -> PaletteColour {
    match colour {
        GroupColour::Preset(preset) => {
            group_colour::preset(preset.name()).map_or_else(slate, PaletteColour::Preset)
        }
        GroupColour::Custom(rgb) => {
            PaletteColour::custom(Colour::rgb(rgb.red, rgb.green, rgb.blue))
        }
    }
}

/// The colour of the loose-tabs pseudo group (and of pinned tabs' beads): the slate preset.
fn slate() -> PaletteColour {
    group_colour::preset("slate").map_or_else(
        || PaletteColour::custom(Colour::hex(0x007c_8088)),
        PaletteColour::Preset,
    )
}

/// The swatch of a group colour for the ring's group rows: it sits on the popover, so the surface
/// is the live `popup-background` and the ink the live `text` of the style.
///
/// `tint` is the appearance crate's. Its `deep` is "the colour darkened until white reads on it":
/// it has no dark half, and the ring draws the group's name in it on the popover (ring-bands D4),
/// where in the dark scheme a dark colour would not be read. So for the dark scheme `deep` is the
/// colour lightened until it reads on the live surface at 4.5:1 (the colour itself when it
/// already does). A name is not a bead mark: see [`mark_for`] for what white is drawn on.
fn swatch_for(colour: PaletteColour, style: &RowStyle) -> Swatch {
    let surface = style.popup_background;
    let swatch = colour.swatch(surface, style.text);
    if !style.dark {
        return swatch;
    }
    // themes lane: move into mercury-appearance (Swatch::for_scheme(surface, ink, dark)); this loop
    // is the stop-gap.
    let deep = (0..=255_u32)
        .map(|step| swatch.colour.lighter(f64::from(step) / 255.0))
        .find(|candidate| candidate.contrast_ratio(surface) >= MIN_TEXT_CONTRAST)
        .unwrap_or(Colour::WHITE);
    Swatch { deep, ..swatch }
}

/// A fill that white text reads on at 4.5:1 in either scheme: the appearance crate's own `deep`
/// of the colour (never the lightened one of [`swatch_for`]). The bead of a pinned tab is drawn
/// with it, with a white initial.
fn mark_for(colour: PaletteColour, style: &RowStyle) -> Colour {
    colour.swatch(style.popup_background, style.text).deep
}

fn slint_colour(colour: Colour) -> Color {
    Color::from_rgb_u8(colour.r, colour.g, colour.b)
}

/// The key as the ring prints it: the capital letter, or nothing.
fn key_label(key: Option<GroupKey>) -> SharedString {
    key.map(|key| key.letter().to_string())
        .unwrap_or_default()
        .into()
}

/// "now", "N min ago", "1 hour ago", "N hours ago", "yesterday" (24 to 48 hours), "N days ago",
/// from the time since `then`. A `then` in the future (a clock that ran backwards) is "now".
pub(crate) fn ago(now: SystemTime, then: SystemTime) -> String {
    let seconds = now
        .duration_since(then)
        .map_or(0, |elapsed| elapsed.as_secs());
    match seconds {
        0..=59 => "now".to_owned(),
        60..=3_599 => format!("{} min ago", seconds / 60),
        3_600..=7_199 => "1 hour ago".to_owned(),
        7_200..=86_399 => format!("{} hours ago", seconds / 3_600),
        86_400..=172_799 => "yesterday".to_owned(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// The host of an address, or nothing for an address without one (`about:`, `file:`, a new tab).
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_default()
}

/// What the ring calls a tab: its title, else its host, else "Untitled".
fn display_title(title: &str, host: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        title.to_owned()
    } else if !host.is_empty() {
        host.to_owned()
    } else {
        "Untitled".to_owned()
    }
}

/// The sound state of a tab that plays or is muted. A pause command exists for every tab whose
/// page has a media session (`BrowserCommand::MediaSessionAction`, which pauses and plays from
/// script when it is not sent through the engine), so `can_pause` is whether media has played.
fn media_for(
    tab: i32,
    playback: MediaPlaybackState,
    muted: bool,
    position: Option<MediaPosition>,
) -> TabMedia {
    TabMedia {
        tab,
        playing: playback == MediaPlaybackState::Playing,
        paused: playback == MediaPlaybackState::Paused,
        can_pause: playback != MediaPlaybackState::None,
        muted,
        position: position.and_then(whole_seconds),
    }
}

/// (position, duration) in whole seconds, when the engine gave a finite, positive duration.
fn whole_seconds(position: MediaPosition) -> Option<(u32, u32)> {
    let (at, duration) = (position.position(), position.duration());
    if !at.is_finite() || !duration.is_finite() || duration <= 0.0 || at < 0.0 {
        return None;
    }
    let seconds = |value: f64| value.round().clamp(0.0, f64::from(u32::MAX)) as u32;
    Some((seconds(at.min(duration)), seconds(duration)))
}

/// When each tab was first seen loading, for the progress estimate.
#[derive(Debug, Default)]
struct LoadClock {
    first_seen: HashMap<TabId, SystemTime>,
}

impl LoadClock {
    /// `TabRow.progress`: 0 for a tab that is not loading, [`UNKNOWN_PROGRESS`] the first time it
    /// is seen loading, then the estimate (0 to 0.9) of the time since.
    fn progress(&mut self, tab: TabId, loading: bool, now: SystemTime) -> f32 {
        if !loading {
            self.first_seen.remove(&tab);
            return 0.0;
        }
        match self.first_seen.get(&tab) {
            Some(&since) => {
                let elapsed = now.duration_since(since).unwrap_or_default();
                f32::from(load_progress_estimate(elapsed)) / 100.0
            }
            None => {
                self.first_seen.insert(tab, now);
                UNKNOWN_PROGRESS
            }
        }
    }

    fn forget_closed(&mut self, open: &HashSet<TabId>) {
        self.first_seen.retain(|tab, _| open.contains(tab));
    }
}

thread_local! {
    static LOAD_CLOCK: RefCell<LoadClock> = RefCell::new(LoadClock::default());
}

// ---------------------------------------------------------------------------------------------
// Moving a tab
// ---------------------------------------------------------------------------------------------

/// What the strip looks like to a move: the tabs in strip order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StripTab {
    id: TabId,
    pinned: bool,
    group: Option<GroupId>,
}

fn strip_of(controller: &BrowserController) -> Vec<StripTab> {
    controller
        .tabs()
        .map(|tab| StripTab {
            id: tab.id(),
            pinned: tab.is_pinned(),
            group: tab.group(),
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MoveTarget {
    Group(GroupId),
    Loose,
}

impl MoveTarget {
    /// `GroupRow.id` to a target: `LOOSE_GROUP_ID` is the loose tabs; any other id that is not a
    /// group id (the ring's -1, zero) is none.
    fn from_ring(group: i32) -> Option<Self> {
        if group == LOOSE_GROUP_ID {
            return Some(Self::Loose);
        }
        u64::try_from(group)
            .ok()
            .filter(|value| *value > 0)
            .map(|value| Self::Group(GroupId::new(value)))
    }
}

/// The arguments of `BrowserController::move_tab_to_group`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MovePlan {
    group: Option<GroupId>,
    index: usize,
}

/// Turns the ring's request (put `tab` in `target` at `slot`) into the controller's arguments.
///
/// `slot` is the insertion slot as `RingDropTarget.index` has it: an index into the target's shown
/// tabs *as the ring shows them now*, the moving tab itself included when it is already there (0
/// is before the first tab, the tab count after the last). `BrowserController::move_tab_to_group`
/// wants where the tab ends up instead: inside a group, its final index among the group's tabs;
/// out of every group, its index in the strip without the tab (after the pinned tabs). So, inside
/// the tab's own target, a slot after the tab counts one less, and the slots just before and just
/// after the tab (it would stay where it is) are no move at all, `None`. A slot past the end is the
/// end. `None` also for a pinned or unknown tab and an unknown group (the controller refuses them).
///
/// The loose tabs are the unpinned tabs in no group; they are scattered through the strip between
/// the groups' runs, so the final loose index is turned into the strip place of the loose tab it
/// goes before (the controller never lands a tab inside a run).
fn plan_move(strip: &[StripTab], tab: TabId, target: MoveTarget, slot: usize) -> Option<MovePlan> {
    let moving = strip.iter().find(|candidate| candidate.id == tab)?;
    if moving.pinned {
        return None;
    }
    if let MoveTarget::Group(group) = target
        && !strip.iter().any(|candidate| candidate.group == Some(group))
    {
        return None;
    }
    let in_target = |candidate: &StripTab| {
        !candidate.pinned
            && match target {
                MoveTarget::Group(group) => candidate.group == Some(group),
                MoveTarget::Loose => candidate.group.is_none(),
            }
    };
    let shown: Vec<&StripTab> = strip
        .iter()
        .filter(|candidate| in_target(candidate))
        .collect();
    let slot = slot.min(shown.len());
    let final_index = match shown.iter().position(|candidate| candidate.id == tab) {
        Some(from) if slot == from || slot == from + 1 => return None,
        Some(from) if slot > from => slot - 1,
        _ => slot,
    };
    match target {
        MoveTarget::Group(group) => Some(MovePlan {
            group: Some(group),
            index: final_index,
        }),
        MoveTarget::Loose => {
            let rest: Vec<&StripTab> = strip
                .iter()
                .filter(|candidate| candidate.id != tab)
                .collect();
            let loose: Vec<usize> = rest
                .iter()
                .enumerate()
                .filter(|(_, candidate)| in_target(candidate))
                .map(|(place, _)| place)
                .collect();
            let index = match loose.get(final_index) {
                Some(&place) => place,
                None => loose.last().map_or(rest.len(), |last| last + 1),
            };
            Some(MovePlan { group: None, index })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The host
// ---------------------------------------------------------------------------------------------

/// What a request changed, for the window to show. The caller of [`install`] decides what each
/// one costs (see [`main_window_actions`]); the host has no window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    /// A tab was selected, muted, moved or opened: sync the window and redraw.
    Shown,
    /// Like `Shown`, and a command was queued for a page (pause or play): the engine must also
    /// be asked to pump.
    CommandQueued,
    /// A tab was closed (or was the last one: `finish_closing_tab` quits).
    Closed(CloseTabOutcome),
    /// Closing the active pinned tab with only pinned tabs left opened a new tab: like Cmd+W
    /// (`tab_strip::step_off_pinned_tab`) the web view loses focus and the address bar gets it.
    OpenedBlankTab,
}

/// The parts of what the ring asks for that need the window or other state of the application,
/// handed in by the caller (layout-switch) so the host stays free of `MainWindow`.
pub(crate) struct DesktopActions {
    /// Called after the host changed the controller, with no borrow of it held.
    pub(crate) changed: Rc<dyn Fn(Change)>,
    /// "See everything": opens Orbit. `None` while Orbit does not exist (the button is then not
    /// drawn, `HubRingInput.orbit_available`), and the request does nothing.
    pub(crate) open_orbit: Option<Rc<dyn Fn()>>,
}

impl DesktopActions {
    /// Actions that only report changes; no Orbit.
    pub(crate) fn new(changed: impl Fn(Change) + 'static) -> Self {
        Self {
            changed: Rc::new(changed),
            open_orbit: None,
        }
    }
}

/// The application's own way to show a change in the main window: what the tab strip, the tab
/// search and the shortcuts do after the same commands (`sync_ui` and a redraw; the quit rule of
/// `finish_closing_tab`; Cmd+W's focus change after a new tab).
pub(crate) fn main_window_actions(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    web_view_focused: Rc<Cell<bool>>,
    open_orbit: Option<Rc<dyn Fn()>>,
) -> DesktopActions {
    let window = window.as_weak();
    DesktopActions {
        changed: Rc::new(move |change: Change| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match change {
                Change::Shown => refresh(&window, &controller),
                Change::CommandQueued => {
                    refresh(&window, &controller);
                    crate::engine::request_pump();
                }
                Change::Closed(outcome) => {
                    crate::tab_close::finish_closing_tab(&window, &controller.borrow(), outcome);
                    window.window().request_redraw();
                }
                Change::OpenedBlankTab => {
                    crate::shortcuts::unfocus_web_view(&controller, &web_view_focused);
                    refresh(&window, &controller);
                    window.invoke_focus_address_bar();
                }
            }
        }),
        open_orbit,
    }
}

fn refresh(window: &MainWindow, controller: &RefCell<BrowserController>) {
    sync_ui(window, &controller.borrow());
    window.window().request_redraw();
}

/// Carries out the ring's requests on the browser.
pub(crate) struct DesktopHost {
    controller: Rc<RefCell<BrowserController>>,
    actions: DesktopActions,
}

impl DesktopHost {
    pub(crate) fn new(controller: Rc<RefCell<BrowserController>>, actions: DesktopActions) -> Self {
        Self {
            controller,
            actions,
        }
    }

    /// Runs `change` on the controller. A request that arrives while the state is borrowed (it
    /// cannot, from a Slint callback, unless something re-entered) is dropped and logged rather
    /// than panicking.
    fn with_controller<R>(&self, change: impl FnOnce(&mut BrowserController) -> R) -> Option<R> {
        match self.controller.try_borrow_mut() {
            Ok(mut controller) => Some(change(&mut controller)),
            Err(_) => {
                log::warn!("hub ring: the browser state is busy; the request was dropped");
                None
            }
        }
    }

    fn report(&self, change: Change) {
        (self.actions.changed)(change);
    }
}

/// The ring's tab id as a `TabId`; `None` for the ring's "none" and any negative id.
fn tab_from_ring(tab: i32) -> Option<TabId> {
    crate::tab_strip::id_from_ui(tab)
}

/// Closing from the ring: an unpinned tab closes (the last one is `LastTab`, which quits, as
/// everywhere). A pinned tab is never closed by a shortcut-like request: the active one steps off
/// to the first unpinned tab, or to a new tab when all are pinned (`step_off_pinned_tab`, Cmd+W's
/// rule); a pinned tab that is not active is left alone, so the selection does not jump for a
/// tab that was not closed. The tab menu's "Close" remains the explicit way to close a pinned tab.
fn close_from_ring(controller: &mut BrowserController, tab: TabId) -> Option<Change> {
    let pinned = controller
        .tabs()
        .any(|candidate| candidate.id() == tab && candidate.is_pinned());
    if pinned {
        if controller.active_tab_id() != tab {
            return None;
        }
        return match controller.step_off_pinned_tab(tab) {
            PinnedCloseOutcome::NotPinned => None,
            PinnedCloseOutcome::SelectedUnpinned => Some(Change::Shown),
            PinnedCloseOutcome::OpenedTab => Some(Change::OpenedBlankTab),
        };
    }
    match controller.close_tab(tab) {
        CloseTabOutcome::NotFound => None,
        outcome => Some(Change::Closed(outcome)),
    }
}

/// Pause a playing tab, play a paused one, by script (`through_engine: false`, which works for
/// any tab: Servo's own media session belongs to one tab at a time). False when the tab has no
/// media to pause or play.
fn toggle_pause_of(controller: &mut BrowserController, tab: TabId) -> bool {
    let action = controller
        .tabs()
        .find(|candidate| candidate.id() == tab)
        .and_then(|candidate| match candidate.state().media().playback() {
            MediaPlaybackState::Playing => Some(MediaSessionAction::Pause),
            MediaPlaybackState::Paused => Some(MediaSessionAction::Play),
            MediaPlaybackState::None => None,
        });
    match action {
        Some(action) => controller.queue_command_for(
            tab,
            BrowserCommand::MediaSessionAction {
                action,
                through_engine: false,
            },
        ),
        None => false,
    }
}

/// A new foreground tab for the results page of `query` as the search engine of the settings
/// builds it (the address bar's builder: `BrowserController::search_url`, which searches the text
/// as it is, even when it looks like an address, as the suggestion list's search row does).
fn open_search_tab(controller: &mut BrowserController, query: &str) -> bool {
    let url = match controller.search_url(query) {
        Ok(url) => url,
        Err(error) => {
            log::warn!("hub ring: no web search: {error}");
            return false;
        }
    };
    controller
        .open_tab_with(Some(url.as_str()), TabActivation::Foreground)
        .is_ok()
}

impl HubRingHost for DesktopHost {
    fn activate_tab(&self, tab: i32) {
        let Some(tab) = tab_from_ring(tab) else {
            return;
        };
        if self.with_controller(|controller| controller.select_tab(tab)) == Some(true) {
            self.report(Change::Shown);
        }
    }

    fn close_tab(&self, tab: i32) {
        let Some(tab) = tab_from_ring(tab) else {
            return;
        };
        if let Some(Some(change)) =
            self.with_controller(|controller| close_from_ring(controller, tab))
        {
            self.report(change);
        }
    }

    fn toggle_mute(&self, tab: i32) {
        let Some(tab) = tab_from_ring(tab) else {
            return;
        };
        if self.with_controller(|controller| controller.toggle_audio_muted(tab)) == Some(true) {
            self.report(Change::Shown);
        }
    }

    fn toggle_pause(&self, tab: i32) {
        let Some(tab) = tab_from_ring(tab) else {
            return;
        };
        if self.with_controller(|controller| toggle_pause_of(controller, tab)) == Some(true) {
            self.report(Change::CommandQueued);
        }
    }

    fn move_tab(&self, tab: i32, group: i32, index: usize) {
        let (Some(tab), Some(target)) = (tab_from_ring(tab), MoveTarget::from_ring(group)) else {
            return;
        };
        let moved = self.with_controller(|controller| {
            let plan = plan_move(&strip_of(controller), tab, target, index)?;
            controller
                .move_tab_to_group(tab, plan.group, plan.index)
                .ok()
        });
        if moved.flatten().is_some() {
            self.report(Change::Shown);
        }
    }

    fn open_orbit(&self) {
        if let Some(open) = &self.actions.open_orbit {
            open();
        }
    }

    fn search_web(&self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        if self.with_controller(|controller| open_search_tab(controller, query)) == Some(true) {
            self.report(Change::Shown);
        }
    }
}

/// Installs the controller on a window that has the `HubRingBridge` global and returns it (keep it
/// alive as long as the window). `actions` shows the changes in the window: for the main window,
/// [`main_window_actions`]. The ring reads the window's live `ChromePalette`.
pub(crate) fn install<C>(
    window: &C,
    controller: Rc<RefCell<BrowserController>>,
    actions: DesktopActions,
) -> HubRing
where
    C: ComponentHandle + 'static,
    for<'a> HubRingBridge<'a>: Global<'a, C>,
    for<'a> ChromePalette<'a>: Global<'a, C>,
{
    HubRing::install(
        window,
        Rc::new(DesktopHost::new(controller, actions)),
        mercury_ui::ring_bands::metrics(),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use mercury_ui::RingPopoverPreview;
    use slint::platform::Key;
    use talaria_core::{BrowserEvent, LoadState, MediaProbe, SearchUrlBuilder, TabCommand};

    use super::*;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    impl RowStyle {
        /// Fixed colours of an ordinary light and dark chrome, for tests only: no window, and
        /// nothing in the adapter depends on them.
        fn for_tests(dark: bool) -> Self {
            let (popup_background, text) = if dark {
                (Colour::hex(0x0016_1e28), Colour::hex(0x00ed_f4fa))
            } else {
                (Colour::hex(0x00ff_ffff), Colour::hex(0x0014_1a21))
            };
            Self {
                dark,
                popup_background,
                text,
                ring: RingColours {
                    key: Color::from_rgb_u8(0xaf, 0x00, 0x62),
                    count: Color::from_rgb_u8(0x66, 0x66, 0x66),
                    ..RingColours::default()
                },
            }
        }
    }

    fn ring(tab: TabId) -> i32 {
        ring_id(tab).expect("a small id")
    }

    // --- pure helpers ---------------------------------------------------------------------

    #[test]
    fn ago_has_the_words_of_the_group_rows() {
        let now = at(1_000_000);
        let then = |seconds: u64| now - Duration::from_secs(seconds);
        for (seconds, words) in [
            (0, "now"),
            (59, "now"),
            (60, "1 min ago"),
            (120, "2 min ago"),
            (3_599, "59 min ago"),
            (3_600, "1 hour ago"),
            (7_199, "1 hour ago"),
            (7_200, "2 hours ago"),
            (86_399, "23 hours ago"),
            (86_400, "yesterday"),
            (172_799, "yesterday"),
            (172_800, "2 days ago"),
            (3 * 86_400 + 5, "3 days ago"),
            (400 * 86_400, "400 days ago"),
        ] {
            assert_eq!(ago(now, then(seconds)), words, "{seconds} s");
        }
        // A stamp from the future (the clock went back) is "now", never a panic.
        assert_eq!(ago(now, now + Duration::from_secs(500)), "now");
    }

    #[test]
    fn the_key_label_is_the_capital_letter_or_nothing() {
        let key = GroupKey::from_code_name("KeyA");
        assert_eq!(key_label(key), "A");
        assert_eq!(key_label(GroupKey::from_letter('s')), "S");
        assert_eq!(key_label(None), "");
    }

    #[test]
    fn a_tab_is_named_by_its_title_then_its_host_then_untitled() {
        assert_eq!(display_title("Servo", "servo.example"), "Servo");
        assert_eq!(display_title("  Servo  ", ""), "Servo");
        assert_eq!(display_title("", "servo.example"), "servo.example");
        assert_eq!(display_title("   ", "servo.example"), "servo.example");
        assert_eq!(display_title("", ""), "Untitled");
    }

    #[test]
    fn the_host_is_the_urls_host_and_empty_without_one() {
        assert_eq!(
            host_of("https://essays.example.net/a/b?c#d"),
            "essays.example.net"
        );
        assert_eq!(host_of("https://user@a.test:8443/"), "a.test");
        assert_eq!(host_of("about:blank"), "");
        assert_eq!(host_of("file:///tmp/a.html"), "");
        assert_eq!(host_of(""), "");
        assert_eq!(host_of("not a url"), "");
    }

    #[test]
    fn the_hold_time_is_the_setting_or_400() {
        assert_eq!(RowSettings::default().hold_ms, 400);
        let mut appearance = AppearanceSettings::default();
        assert_eq!(
            RowSettings::from_appearance(&appearance, false).hold_ms,
            400
        );
        appearance.tab_groups.keys.hold_ms = 750;
        let settings = RowSettings::from_appearance(&appearance, true);
        assert_eq!(settings.hold_ms, 750);
        assert!(settings.orbit_available);
        assert!(settings.previews);
        // Out of range: the default, as `mercury_appearance::resolve` repairs it.
        appearance.tab_groups.keys.hold_ms = u16::MAX;
        assert_eq!(
            RowSettings::from_appearance(&appearance, false).hold_ms,
            400
        );
    }

    #[test]
    fn media_state_says_what_can_be_done() {
        use MediaPlaybackState::{None as Nothing, Paused, Playing};
        let position = Some(MediaPosition::new(2880.0, 1.0, 760.4));

        let playing = media_for(7, Playing, false, position);
        assert_eq!(
            playing,
            TabMedia {
                tab: 7,
                playing: true,
                paused: false,
                can_pause: true,
                muted: false,
                position: Some((760, 2880)),
            }
        );
        let paused = media_for(7, Paused, false, None);
        assert!(paused.paused && !paused.playing && paused.can_pause);
        assert_eq!(paused.position, None);
        // Muted and not playing: only the mute is known, there is nothing to pause.
        let muted = media_for(7, Nothing, true, None);
        assert!(muted.muted && !muted.playing && !muted.paused && !muted.can_pause);
        // A live stream, a position past the end and nonsense give no position, or a clamped one.
        for (duration, seconds) in [
            (f64::INFINITY, 5.0),
            (f64::NAN, 5.0),
            (0.0, 0.0),
            (100.0, f64::NAN),
            (100.0, -1.0),
        ] {
            let position = Some(MediaPosition::new(duration, 1.0, seconds));
            assert_eq!(media_for(1, Playing, false, position).position, None);
        }
        let past_the_end = Some(MediaPosition::new(100.0, 1.0, 250.0));
        assert_eq!(
            media_for(1, Playing, false, past_the_end).position,
            Some((100, 100))
        );
    }

    #[test]
    fn load_progress_starts_unknown_then_follows_the_estimate() {
        let mut clock = LoadClock::default();
        let tab = TabId::new(5);
        let other = TabId::new(6);
        assert_eq!(clock.progress(tab, false, at(100)), 0.0);
        assert_eq!(clock.progress(tab, true, at(100)), UNKNOWN_PROGRESS);
        assert_eq!(clock.progress(tab, true, at(100)), 0.0);
        let later = clock.progress(tab, true, at(103));
        assert!((later - 0.78).abs() < 1e-6, "{later}");
        // A clock that runs backwards gives no negative progress.
        assert_eq!(clock.progress(tab, true, at(50)), 0.0);
        // Another tab has its own start; a finished load forgets its start.
        assert_eq!(clock.progress(other, true, at(110)), UNKNOWN_PROGRESS);
        assert_eq!(clock.progress(tab, false, at(111)), 0.0);
        assert_eq!(clock.progress(tab, true, at(112)), UNKNOWN_PROGRESS);
        // A closed tab is forgotten.
        clock.forget_closed(&HashSet::from([tab]));
        assert_eq!(clock.progress(other, true, at(120)), UNKNOWN_PROGRESS);
    }

    #[test]
    fn the_swatch_depends_on_the_scheme_and_the_colour_does_not() {
        let base = palette_colour(GroupColour::Preset(talaria_core::GroupPreset::Blue));
        let light = swatch_for(base, &RowStyle::for_tests(false));
        let dark = swatch_for(base, &RowStyle::for_tests(true));
        assert_eq!(light.colour, dark.colour);
        assert_eq!(light.colour, Colour::hex(0x002f_6fd6));
        assert_ne!(light.tint, dark.tint);
        assert_eq!(slate().base(), Colour::hex(0x007c_8088));
        let custom = palette_colour(GroupColour::Custom(talaria_core::Rgb {
            red: 1,
            green: 2,
            blue: 3,
        }));
        assert_eq!(custom.base(), Colour::rgb(1, 2, 3));
    }

    #[test]
    fn a_group_name_reads_on_whatever_popover_the_window_has() {
        let purple = RowStyle {
            dark: true,
            popup_background: Colour::hex(0x0024_1d42),
            text: Colour::hex(0x00f3_eef8),
            ..RowStyle::for_tests(true)
        };
        let cream = RowStyle {
            popup_background: Colour::hex(0x00fc_faf6),
            ..RowStyle::for_tests(false)
        };
        for style in [
            RowStyle::for_tests(false),
            RowStyle::for_tests(true),
            purple,
            cream,
        ] {
            for preset in &group_colour::PRESETS {
                let swatch = swatch_for(PaletteColour::Preset(preset), &style);
                let ratio = swatch.deep.contrast_ratio(style.popup_background);
                // The light `deep` is the appearance crate's (white reads on it at 4.5:1, so it
                // reads on a near-white popover at about that, a little less); the dark one is
                // searched against the live surface.
                let least = if style.dark { MIN_TEXT_CONTRAST } else { 4.0 };
                assert!(
                    ratio >= least,
                    "{} on {:?}: {ratio:.2}",
                    preset.id,
                    style.popup_background
                );
            }
        }
    }

    #[test]
    fn a_pinned_beads_white_initial_reads_on_its_mark_for_every_preset_and_scheme() {
        let white = Colour::WHITE;
        for dark in [false, true] {
            let style = RowStyle::for_tests(dark);
            let slate = mark_for(slate(), &style);
            assert!(
                white.contrast_ratio(slate) >= MIN_TEXT_CONTRAST,
                "slate, dark: {dark}"
            );
            for preset in &group_colour::PRESETS {
                let mark = mark_for(PaletteColour::Preset(preset), &style);
                let ratio = white.contrast_ratio(mark);
                assert!(
                    ratio >= MIN_TEXT_CONTRAST,
                    "{} dark: {dark}: {ratio:.2}",
                    preset.id
                );
            }
        }
    }

    // --- the move translation -------------------------------------------------------------

    /// Tab 1 pinned; group 1: tabs 2, 3, 4; group 2: 5, 6; loose 7; group 3: 8; loose 9.
    fn strip() -> Vec<StripTab> {
        let tab = |id: u64, pinned: bool, group: Option<u64>| StripTab {
            id: TabId::new(id),
            pinned,
            group: group.map(GroupId::new),
        };
        vec![
            tab(1, true, None),
            tab(2, false, Some(1)),
            tab(3, false, Some(1)),
            tab(4, false, Some(1)),
            tab(5, false, Some(2)),
            tab(6, false, Some(2)),
            tab(7, false, None),
            tab(8, false, Some(3)),
            tab(9, false, None),
        ]
    }

    fn plan(tab: u64, group: i32, slot: usize) -> Option<MovePlan> {
        plan_move(
            &strip(),
            TabId::new(tab),
            MoveTarget::from_ring(group).expect("a target"),
            slot,
        )
    }

    fn into_group(group: u64, index: usize) -> Option<MovePlan> {
        Some(MovePlan {
            group: Some(GroupId::new(group)),
            index,
        })
    }

    fn out_of_groups(index: usize) -> Option<MovePlan> {
        Some(MovePlan { group: None, index })
    }

    #[test]
    fn ring_group_ids_are_targets() {
        assert_eq!(
            MoveTarget::from_ring(LOOSE_GROUP_ID),
            Some(MoveTarget::Loose)
        );
        assert_eq!(
            MoveTarget::from_ring(4),
            Some(MoveTarget::Group(GroupId::new(4)))
        );
        assert_eq!(MoveTarget::from_ring(-1), None);
        assert_eq!(MoveTarget::from_ring(0), None);
        assert_eq!(MoveTarget::from_ring(-7), None);
    }

    #[test]
    fn a_slot_in_the_tabs_own_group_is_counted_with_the_tab_in_place() {
        // Group 1 shows [2, 3, 4]; slots 0..=3.
        assert_eq!(plan(2, 1, 3), into_group(1, 2), "to the end");
        assert_eq!(plan(2, 1, 2), into_group(1, 1), "before the last");
        assert_eq!(plan(4, 1, 0), into_group(1, 0), "to the front");
        assert_eq!(plan(4, 1, 1), into_group(1, 1), "one back");
        // The slots on either side of the tab are no move.
        assert_eq!(plan(3, 1, 1), None);
        assert_eq!(plan(3, 1, 2), None);
        assert_eq!(plan(2, 1, 0), None);
        assert_eq!(plan(2, 1, 1), None);
        assert_eq!(plan(4, 1, 2), None);
        assert_eq!(plan(4, 1, 3), None);
        // Past the end is the end.
        assert_eq!(plan(2, 1, 99), into_group(1, 2));
    }

    #[test]
    fn a_slot_in_another_group_is_the_final_index() {
        assert_eq!(plan(5, 1, 1), into_group(1, 1));
        assert_eq!(plan(5, 1, 0), into_group(1, 0));
        assert_eq!(plan(7, 2, 99), into_group(2, 2), "a loose tab appended");
        assert_eq!(plan(2, 3, 0), into_group(3, 0), "into a one-tab group");
    }

    #[test]
    fn a_slot_among_the_loose_tabs_is_a_place_in_the_strip() {
        // Loose tabs: 7 and 9. Without tab 2 the strip is [1, 3, 4, 5, 6, 7, 8, 9]: tab 7 is at 5
        // and tab 9 at 7.
        assert_eq!(plan(2, LOOSE_GROUP_ID, 0), out_of_groups(5), "before tab 7");
        assert_eq!(plan(2, LOOSE_GROUP_ID, 1), out_of_groups(7), "before tab 9");
        assert_eq!(plan(2, LOOSE_GROUP_ID, 2), out_of_groups(8), "after tab 9");
        assert_eq!(plan(2, LOOSE_GROUP_ID, 99), out_of_groups(8));
        // Among themselves: tab 9 to the front, tab 7 to the end; the slots next to a tab are
        // no move.
        assert_eq!(plan(9, LOOSE_GROUP_ID, 0), out_of_groups(6));
        assert_eq!(plan(9, LOOSE_GROUP_ID, 1), None);
        assert_eq!(plan(9, LOOSE_GROUP_ID, 2), None);
        assert_eq!(plan(7, LOOSE_GROUP_ID, 2), out_of_groups(8));
        assert_eq!(plan(7, LOOSE_GROUP_ID, 0), None);
        assert_eq!(plan(7, LOOSE_GROUP_ID, 1), None);
    }

    #[test]
    fn a_tab_out_of_its_group_with_no_loose_tab_goes_to_the_end() {
        let strip = [
            StripTab {
                id: TabId::new(1),
                pinned: true,
                group: None,
            },
            StripTab {
                id: TabId::new(2),
                pinned: false,
                group: Some(GroupId::new(1)),
            },
        ];
        assert_eq!(
            plan_move(&strip, TabId::new(2), MoveTarget::Loose, 0),
            out_of_groups(1)
        );
    }

    #[test]
    fn a_pinned_or_unknown_tab_or_group_is_no_move() {
        assert_eq!(plan(1, 1, 0), None, "a pinned tab");
        assert_eq!(plan(1, LOOSE_GROUP_ID, 0), None);
        assert_eq!(plan(99, 1, 0), None, "an unknown tab");
        assert_eq!(plan(2, 42, 0), None, "an unknown group");
    }

    // --- the browser ----------------------------------------------------------------------

    struct Fixture {
        controller: BrowserController,
        ids: Vec<TabId>,
        research: GroupId,
        lisbon: GroupId,
        reading: GroupId,
    }

    /// Eight tabs: Research (blue, key A) has tabs 0, 1 (loading) and 2 (plays, 12:40 of 48:00);
    /// Lisbon trip (amber, S) has 3 and 4 (muted); Reading (orchid, D) has 5 (unread); tab 6 is
    /// loose; tab 7 is pinned (so it is first in the strip). Tab 0 is the active tab. Strip order:
    /// 7, 0, 1, 2, 3, 4, 5, 6.
    fn fixture() -> Fixture {
        let mut controller = BrowserController::default();
        let mut ids = vec![controller.active_tab_id()];
        for _ in 1..8 {
            ids.push(
                controller
                    .open_tab_with(None, TabActivation::Background)
                    .expect("a blank tab opens"),
            );
        }
        let pages = [
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
        ];
        for (id, page) in ids.iter().zip(pages) {
            assert!(controller.handle_event(
                *id,
                BrowserEvent::AddressChanged {
                    url: format!("https://{page}.test/path"),
                },
            ));
            assert!(controller.handle_event(
                *id,
                BrowserEvent::PageTitleChanged {
                    title: Some(format!("Page {page}")),
                },
            ));
        }
        let research = controller
            .create_group(&ids[0..3], Some("Research"), None, at(1_000))
            .expect("a group of three");
        let lisbon = controller
            .create_group(&ids[3..5], Some("Lisbon trip"), None, at(1_000))
            .expect("a group of two");
        let reading = controller
            .create_group(&ids[5..6], Some("Reading"), None, at(1_000))
            .expect("a group of one");
        assert!(controller.set_tab_pinned(ids[7], true));
        // Showing every tab clears the "unread" that the titles above set in the background.
        for id in &ids {
            assert!(controller.select_tab(*id));
        }
        assert!(controller.select_tab(ids[0]));

        controller.handle_event(
            ids[1],
            BrowserEvent::LoadStateChanged {
                state: LoadState::Loading,
            },
        );
        for event in [
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Playing,
            },
            BrowserEvent::MediaProbed {
                probe: MediaProbe {
                    playing: 1,
                    audible: 1,
                    unseen: false,
                    fullscreen_video: false,
                },
            },
            BrowserEvent::MediaPositionChanged {
                position: MediaPosition::new(2880.0, 1.0, 760.0),
            },
        ] {
            controller.handle_event(ids[2], event);
        }
        assert!(controller.toggle_audio_muted(ids[4]));
        assert!(controller.handle_event(
            ids[5],
            BrowserEvent::PageTitleChanged {
                title: Some("Reading list (2)".to_owned()),
            },
        ));
        Fixture {
            controller,
            ids,
            research,
            lisbon,
            reading,
        }
    }

    fn tab_row(input: &HubRingInput, tab: TabId) -> &TabRow {
        input
            .tabs
            .iter()
            .find(|row| row.id == ring(tab))
            .expect("the tab has a row")
    }

    fn group_row(input: &HubRingInput, id: i32) -> &GroupRow {
        input
            .groups
            .iter()
            .find(|row| row.id == id)
            .expect("the group has a row")
    }

    #[test]
    fn the_groups_are_the_windows_in_order_with_the_loose_group_last() {
        let Fixture {
            controller,
            ids,
            research,
            lisbon,
            reading,
        } = fixture();
        // Research and Lisbon were stamped when they were left; the stamps are what "ago" shows.
        let mut controller = controller;
        controller.stamp_group_activity(at(10_000));
        assert!(controller.select_tab(ids[3]));
        controller.stamp_group_activity(at(10_300));
        assert!(controller.select_tab(ids[0]));

        let input = rows(&controller, at(10_420), &RowStyle::for_tests(false));

        let summary = input
            .groups
            .iter()
            .map(|row| {
                (
                    row.id,
                    row.name.to_string(),
                    row.key_label.to_string(),
                    row.count,
                    row.last_used.to_string(),
                    row.active,
                )
            })
            .collect::<Vec<_>>();
        let id = |group: GroupId| i32::try_from(group.value()).expect("a small id");
        assert_eq!(
            summary,
            [
                (
                    id(research),
                    "Research".to_owned(),
                    "A".to_owned(),
                    3,
                    "2 min ago".to_owned(),
                    true
                ),
                (
                    id(lisbon),
                    "Lisbon trip".to_owned(),
                    "S".to_owned(),
                    2,
                    "2 min ago".to_owned(),
                    false
                ),
                (
                    id(reading),
                    "Reading".to_owned(),
                    "D".to_owned(),
                    1,
                    "7 min ago".to_owned(),
                    false
                ),
                (
                    LOOSE_GROUP_ID,
                    "Loose".to_owned(),
                    String::new(),
                    1,
                    String::new(),
                    false
                ),
            ]
        );
        assert_eq!(LOOSE_GROUP_ID, -2);
    }

    #[test]
    fn the_groups_flags_come_from_their_tabs() {
        let Fixture {
            controller,
            research,
            lisbon,
            reading,
            ..
        } = fixture();
        let input = rows(&controller, at(2_000), &RowStyle::for_tests(false));
        let flags = |group: GroupId| {
            let row = group_row(&input, i32::try_from(group.value()).expect("a small id"));
            (row.any_loading, row.any_audible, row.any_unread)
        };
        // Research: a tab loads and one plays. Lisbon: a muted tab is not audible. Reading: unread.
        assert_eq!(flags(research), (true, true, false));
        assert_eq!(flags(lisbon), (false, false, false));
        assert_eq!(flags(reading), (false, false, true));
        let loose = group_row(&input, LOOSE_GROUP_ID);
        assert!(!loose.any_loading && !loose.any_audible && !loose.any_unread);
    }

    #[test]
    fn the_colours_are_the_groups_and_the_scheme_changes_tint_and_deep() {
        let controller = fixture().controller;
        let light = rows(&controller, at(2_000), &RowStyle::for_tests(false));
        let dark = rows(&controller, at(2_000), &RowStyle::for_tests(true));
        assert!(!light.dark && dark.dark);
        for (one, other) in light.groups.iter().zip(&dark.groups) {
            assert_eq!(one.id, other.id);
            assert_eq!(one.colour, other.colour, "{}", one.name);
            assert_ne!(one.tint, other.tint, "{}", one.name);
            assert_ne!(one.deep, other.deep, "{}", one.name);
        }
        // The first group is blue, the loose group slate.
        assert_eq!(light.groups[0].colour, Color::from_rgb_u8(0x2f, 0x6f, 0xd6));
        let loose = group_row(&light, LOOSE_GROUP_ID);
        assert_eq!(loose.colour, Color::from_rgb_u8(0x7c, 0x80, 0x88));
    }

    #[test]
    fn every_tab_has_a_row_with_its_group_and_index() {
        let Fixture {
            controller, ids, ..
        } = fixture();
        let input = rows(&controller, at(2_000), &RowStyle::for_tests(false));

        assert_eq!(input.tabs.len(), 8);
        let place = |tab: TabId| {
            let row = tab_row(&input, tab);
            (row.group, row.index_in_group, row.pinned)
        };
        // Window order: the pinned tab first.
        assert_eq!(
            input.tabs.iter().map(|row| row.id).collect::<Vec<_>>(),
            [7, 0, 1, 2, 3, 4, 5, 6].map(|n| ring(ids[n]))
        );
        assert_eq!(place(ids[7]), (LOOSE_GROUP_ID, 0, true));
        assert_eq!(place(ids[0]), (1, 0, false));
        assert_eq!(place(ids[1]), (1, 1, false));
        assert_eq!(place(ids[2]), (1, 2, false));
        assert_eq!(place(ids[3]), (2, 0, false));
        assert_eq!(place(ids[4]), (2, 1, false));
        assert_eq!(place(ids[5]), (3, 0, false));
        // The pinned tab does not count in the loose group.
        assert_eq!(place(ids[6]), (LOOSE_GROUP_ID, 0, false));
        // Every group's count is the number of its unpinned rows.
        for group in &input.groups {
            let own = input
                .tabs
                .iter()
                .filter(|row| row.group == group.id && !row.pinned)
                .count();
            assert_eq!(
                i32::try_from(own).expect("small"),
                group.count,
                "{}",
                group.name
            );
        }
    }

    #[test]
    fn a_tab_row_has_its_title_host_flags_and_shade() {
        let Fixture {
            controller, ids, ..
        } = fixture();
        let input = rows(&controller, at(2_000), &RowStyle::for_tests(false));

        let first = tab_row(&input, ids[0]);
        assert_eq!(first.title, "Page alpha");
        assert_eq!(first.host, "alpha.test");
        assert!(first.active);
        assert!(!first.loading && first.progress == 0.0);
        assert_eq!(
            first.shade,
            Color::from_rgb_u8(0x2f, 0x6f, 0xd6),
            "index 0 is the colour"
        );
        assert!(input.tabs.iter().filter(|row| row.active).count() == 1);

        let blue = Colour::hex(0x002f_6fd6);
        for (tab, index) in [(1, 1), (2, 2)] {
            let want = tab_shade(blue, index);
            assert_eq!(
                tab_row(&input, ids[tab]).shade,
                Color::from_rgb_u8(want.r, want.g, want.b),
                "tab {tab}"
            );
        }
        assert_ne!(tab_row(&input, ids[1]).shade, first.shade);

        let loading = tab_row(&input, ids[1]);
        assert!(loading.loading);
        assert_eq!(loading.progress, UNKNOWN_PROGRESS, "first sight");
        let playing = tab_row(&input, ids[2]);
        assert!(playing.audible && !playing.muted);
        let muted = tab_row(&input, ids[4]);
        assert!(muted.muted && !muted.audible);
        let unread = tab_row(&input, ids[5]);
        assert!(unread.unread);
        let pinned = tab_row(&input, ids[7]);
        assert!(pinned.pinned && !pinned.loading && !pinned.crashed && !pinned.capture);
        // The bead's mark is the slate swatch's `deep`: white reads on it.
        let mark = mark_for(slate(), &RowStyle::for_tests(false));
        assert_eq!(pinned.shade, Color::from_rgb_u8(mark.r, mark.g, mark.b));
        assert!(input.tabs.iter().all(|row| row.matches_query));
        // Blank favicons: no tab has an icon.
        assert!(input.tabs.iter().all(|row| row.favicon.size().width == 0));
    }

    #[test]
    fn the_progress_of_a_loading_tab_rises_with_the_time_it_has_been_seen_loading() {
        let Fixture {
            mut controller,
            ids,
            ..
        } = fixture();
        let progress = |controller: &BrowserController, seconds: u64| {
            let input = rows(controller, at(seconds), &RowStyle::for_tests(false));
            tab_row(&input, ids[1]).progress
        };
        assert_eq!(progress(&controller, 100), UNKNOWN_PROGRESS);
        let later = progress(&controller, 103);
        assert!((later - 0.78).abs() < 1e-6, "{later}");
        let much_later = progress(&controller, 200);
        assert!((0.8..=0.9).contains(&much_later), "{much_later}");
        controller.handle_event(
            ids[1],
            BrowserEvent::LoadStateChanged {
                state: LoadState::Loaded,
            },
        );
        assert_eq!(progress(&controller, 201), 0.0);
    }

    #[test]
    fn the_sound_of_audible_and_muted_tabs_is_in_the_media_rows() {
        let Fixture {
            controller, ids, ..
        } = fixture();
        let input = rows(&controller, at(2_000), &RowStyle::for_tests(false));

        assert_eq!(input.media.len(), 2);
        let media = |tab: TabId| {
            input
                .media
                .iter()
                .find(|entry| entry.tab == ring(tab))
                .copied()
                .expect("an entry")
        };
        assert_eq!(
            media(ids[2]),
            TabMedia {
                tab: ring(ids[2]),
                playing: true,
                paused: false,
                can_pause: true,
                muted: false,
                position: Some((760, 2880)),
            }
        );
        let muted = media(ids[4]);
        assert!(muted.muted && !muted.playing && !muted.can_pause);
    }

    #[test]
    fn a_paused_tab_keeps_its_media_entry_so_it_can_be_played_again() {
        let Fixture {
            mut controller,
            ids,
            ..
        } = fixture();
        // Tab 6 (loose, not muted) plays and is then paused by the user: it is no longer audible.
        for state in [MediaPlaybackState::Playing, MediaPlaybackState::Paused] {
            controller.handle_event(ids[6], BrowserEvent::MediaPlaybackChanged { state });
        }
        let input = rows(&controller, at(2_000), &RowStyle::for_tests(false));

        let paused = input
            .media
            .iter()
            .find(|entry| entry.tab == ring(ids[6]))
            .copied()
            .expect("a paused tab has an entry");
        assert_eq!(
            paused,
            TabMedia {
                tab: ring(ids[6]),
                playing: false,
                paused: true,
                can_pause: true,
                muted: false,
                position: None,
            }
        );
        assert!(!tab_row(&input, ids[6]).audible);
        // The playing tab and the muted one are still there.
        assert_eq!(input.media.len(), 3);
    }

    #[test]
    fn a_group_key_goes_to_the_tab_shown_last_in_its_group() {
        let Fixture {
            mut controller,
            ids,
            research,
            lisbon,
            reading,
            ..
        } = fixture();
        assert!(controller.select_tab(ids[4]));
        assert!(controller.select_tab(ids[0]));
        let input = rows(&controller, at(2_000), &RowStyle::for_tests(false));

        let id = |group: GroupId| i32::try_from(group.value()).expect("a small id");
        // Tab 0 was shown last of Research; tab 4 after tab 3 in Lisbon trip; Reading has one.
        assert_eq!(
            input.last_active,
            [
                (id(research), ring(ids[0])),
                (id(lisbon), ring(ids[4])),
                (id(reading), ring(ids[5])),
            ]
        );
    }

    #[test]
    fn the_settings_and_the_missing_snapshots_are_in_the_input() {
        let controller = fixture().controller;
        let style = RowStyle::for_tests(true);
        let input = rows(&controller, at(2_000), &style);
        assert_eq!(
            input.colours,
            Some(style.ring),
            "the ring's colours are the live ones"
        );
        assert_eq!(input.hold_ms, 400);
        assert!(!input.orbit_available && !input.reduce_motion);
        assert!(input.previews.is_empty(), "no snapshots in a test");

        let settings = RowSettings {
            hold_ms: 650,
            orbit_available: true,
            reduce_motion: true,
            previews: false,
        };
        let input = rows_with(
            &controller,
            at(2_000),
            &RowStyle::for_tests(false),
            &settings,
        );
        assert_eq!(input.hold_ms, 650);
        assert!(input.orbit_available && input.reduce_motion);
    }

    #[test]
    fn a_tab_whose_group_is_gone_is_loose() {
        let Fixture {
            mut controller,
            ids,
            reading,
            ..
        } = fixture();
        // Reading's only tab leaves it: the group is deleted with its last tab.
        controller
            .ungroup_tab(ids[5])
            .expect("a tab out of its group");
        assert!(controller.group(reading).is_none());

        let input = rows(&controller, at(2_000), &RowStyle::for_tests(false));

        assert_eq!(input.groups.len(), 3, "Research, Lisbon trip and Loose");
        assert_eq!(input.groups.last().map(|row| row.id), Some(LOOSE_GROUP_ID));
        assert_eq!(group_row(&input, LOOSE_GROUP_ID).count, 2);
        let place = |tab: TabId| {
            let row = tab_row(&input, tab);
            (row.group, row.index_in_group)
        };
        // The ungrouped tab stands right after Reading's old place, before the other loose tab.
        assert_eq!(place(ids[5]), (LOOSE_GROUP_ID, 0));
        assert_eq!(place(ids[6]), (LOOSE_GROUP_ID, 1));
        assert!(
            input
                .last_active
                .iter()
                .all(|(group, _)| *group != LOOSE_GROUP_ID)
        );
    }

    #[test]
    fn with_only_pinned_and_grouped_tabs_there_is_no_loose_group() {
        let Fixture {
            mut controller,
            ids,
            research,
            ..
        } = fixture();
        controller
            .move_tab_to_group(ids[6], Some(research), 3)
            .expect("the loose tab joins Research");
        let input = rows(&controller, at(2_000), &RowStyle::for_tests(false));
        assert!(input.groups.iter().all(|row| row.id != LOOSE_GROUP_ID));
        // The pinned tab keeps its row, in the loose id, and the ring draws it as a bead.
        let pinned = tab_row(&input, ids[7]);
        assert_eq!((pinned.group, pinned.pinned), (LOOSE_GROUP_ID, true));
        assert_eq!(group_row(&input, 1).count, 4);
    }

    // --- the host -------------------------------------------------------------------------

    struct Harness {
        host: DesktopHost,
        controller: Rc<RefCell<BrowserController>>,
        changes: Rc<RefCell<Vec<Change>>>,
        orbit_calls: Rc<Cell<u32>>,
        ids: Vec<TabId>,
        research: GroupId,
    }

    fn harness() -> Harness {
        let Fixture {
            controller,
            ids,
            research,
            ..
        } = fixture();
        harness_with(controller, ids, research)
    }

    fn harness_with(controller: BrowserController, ids: Vec<TabId>, research: GroupId) -> Harness {
        let controller = Rc::new(RefCell::new(controller));
        let changes = Rc::new(RefCell::new(Vec::new()));
        let orbit_calls = Rc::new(Cell::new(0));
        let recorded = Rc::clone(&changes);
        let counted = Rc::clone(&orbit_calls);
        let mut actions = DesktopActions::new(move |change| recorded.borrow_mut().push(change));
        actions.open_orbit = Some(Rc::new(move || counted.set(counted.get() + 1)));
        Harness {
            host: DesktopHost::new(Rc::clone(&controller), actions),
            controller,
            changes,
            orbit_calls,
            ids,
            research,
        }
    }

    impl Harness {
        fn changes(&self) -> Vec<Change> {
            self.changes.borrow().clone()
        }

        fn drain(&self) -> Vec<TabCommand> {
            let mut controller = self.controller.borrow_mut();
            std::iter::from_fn(|| controller.next_command()).collect()
        }
    }

    #[test]
    fn activating_selects_the_tab_and_reports_it() {
        let harness = harness();
        harness.host.activate_tab(ring(harness.ids[3]));
        assert_eq!(harness.controller.borrow().active_tab_id(), harness.ids[3]);
        assert_eq!(harness.changes(), [Change::Shown]);
    }

    #[test]
    fn activating_a_tab_that_does_not_exist_does_nothing() {
        let harness = harness();
        for tab in [9_999, 0, -1, LOOSE_GROUP_ID, i32::MAX] {
            harness.host.activate_tab(tab);
        }
        assert_eq!(harness.controller.borrow().active_tab_id(), harness.ids[0]);
        assert!(harness.changes().is_empty());
    }

    #[test]
    fn closing_an_unpinned_tab_closes_it_and_the_last_tab_says_so() {
        let harness = harness();
        harness.host.close_tab(ring(harness.ids[6]));
        assert_eq!(harness.controller.borrow().tabs().len(), 7);
        assert_eq!(harness.changes(), [Change::Closed(CloseTabOutcome::Closed)]);
        harness.host.close_tab(9_999);
        assert_eq!(harness.changes().len(), 1, "an unknown tab is nothing");

        let single = BrowserController::default();
        let only = single.active_tab_id();
        let harness = harness_with(single, vec![only], GroupId::new(1));
        harness.host.close_tab(ring(only));
        assert_eq!(harness.controller.borrow().tabs().len(), 1);
        assert_eq!(
            harness.changes(),
            [Change::Closed(CloseTabOutcome::LastTab)]
        );
    }

    #[test]
    fn a_pinned_tab_is_not_closed_and_the_active_one_steps_off() {
        let harness = harness();
        let pinned = harness.ids[7];
        // Not the active tab: nothing changes, not even the selection.
        harness.host.close_tab(ring(pinned));
        assert_eq!(harness.controller.borrow().tabs().len(), 8);
        assert_eq!(harness.controller.borrow().active_tab_id(), harness.ids[0]);
        assert!(harness.changes().is_empty());

        // The active pinned tab: the first unpinned tab is shown, like Cmd+W.
        assert!(harness.controller.borrow_mut().select_tab(pinned));
        harness.host.close_tab(ring(pinned));
        assert_eq!(harness.controller.borrow().tabs().len(), 8);
        assert_eq!(harness.controller.borrow().active_tab_id(), harness.ids[0]);
        assert_eq!(harness.changes(), [Change::Shown]);
    }

    #[test]
    fn closing_the_active_pinned_tab_of_an_all_pinned_window_opens_a_tab() {
        let mut controller = BrowserController::default();
        let only = controller.active_tab_id();
        assert!(controller.set_tab_pinned(only, true));
        let harness = harness_with(controller, vec![only], GroupId::new(1));

        harness.host.close_tab(ring(only));

        assert_eq!(harness.controller.borrow().tabs().len(), 2);
        assert_eq!(harness.changes(), [Change::OpenedBlankTab]);
    }

    #[test]
    fn muting_flips_the_tabs_mute_and_reports_it() {
        let harness = harness();
        let tab = harness.ids[4];
        let muted = |harness: &Harness| {
            harness
                .controller
                .borrow()
                .tabs()
                .find(|candidate| candidate.id() == tab)
                .map(|candidate| candidate.state().is_audio_muted())
        };
        assert_eq!(muted(&harness), Some(true));
        harness.host.toggle_mute(ring(tab));
        assert_eq!(muted(&harness), Some(false));
        harness.host.toggle_mute(ring(tab));
        assert_eq!(muted(&harness), Some(true));
        assert_eq!(harness.changes(), [Change::Shown, Change::Shown]);
        harness.host.toggle_mute(-1);
        assert_eq!(harness.changes().len(), 2);
    }

    #[test]
    fn pause_pauses_a_playing_tab_plays_a_paused_one_and_ignores_a_silent_one() {
        let harness = harness();
        let tab = harness.ids[2];
        harness.drain();

        harness.host.toggle_pause(ring(tab));
        assert_eq!(
            harness.drain(),
            [TabCommand::new(
                tab,
                BrowserCommand::MediaSessionAction {
                    action: MediaSessionAction::Pause,
                    through_engine: false,
                }
            )]
        );
        assert_eq!(harness.changes(), [Change::CommandQueued]);

        harness.controller.borrow_mut().handle_event(
            tab,
            BrowserEvent::MediaPlaybackChanged {
                state: MediaPlaybackState::Paused,
            },
        );
        harness.host.toggle_pause(ring(tab));
        assert_eq!(
            harness.drain(),
            [TabCommand::new(
                tab,
                BrowserCommand::MediaSessionAction {
                    action: MediaSessionAction::Play,
                    through_engine: false,
                }
            )]
        );

        // A tab without media has nothing to pause.
        harness.host.toggle_pause(ring(harness.ids[0]));
        assert!(harness.drain().is_empty());
        assert_eq!(harness.changes().len(), 2);
    }

    #[test]
    fn moving_into_a_group_at_a_slot_puts_the_tab_there() {
        let harness = harness();
        let ids = &harness.ids;
        // The loose tab joins Research between its first and second tab.
        harness.host.move_tab(ring(ids[6]), 1, 1);
        assert_eq!(
            harness.controller.borrow().group_tabs(harness.research),
            [ids[0], ids[6], ids[1], ids[2]]
        );
        assert_eq!(harness.changes(), [Change::Shown]);
        assert!(
            harness
                .controller
                .borrow()
                .group_invariant_violation()
                .is_none()
        );

        // Within the group: the first tab to the slot after the last is the end.
        harness.host.move_tab(ring(ids[0]), 1, 4);
        assert_eq!(
            harness.controller.borrow().group_tabs(harness.research),
            [ids[6], ids[1], ids[2], ids[0]]
        );
        // The slot just after a tab is no move, and reports nothing.
        harness.host.move_tab(ring(ids[1]), 1, 2);
        assert_eq!(harness.changes().len(), 2);
        assert!(
            harness
                .controller
                .borrow()
                .group_invariant_violation()
                .is_none()
        );
    }

    #[test]
    fn moving_to_the_loose_group_takes_the_tab_out_of_its_group() {
        let harness = harness();
        let ids = &harness.ids;
        // Loose tabs: tab 6 only. Tab 1 goes before it.
        harness.host.move_tab(ring(ids[1]), LOOSE_GROUP_ID, 0);
        let controller = harness.controller.borrow();
        assert_eq!(controller.loose_tabs(), [ids[1], ids[6]]);
        assert_eq!(controller.group_tabs(harness.research), [ids[0], ids[2]]);
        assert!(controller.group_invariant_violation().is_none());
        drop(controller);
        assert_eq!(harness.changes(), [Change::Shown]);
    }

    #[test]
    fn moves_the_controller_would_refuse_are_dropped() {
        let harness = harness();
        let ids = &harness.ids;
        harness.host.move_tab(ring(ids[7]), 1, 0); // pinned
        harness.host.move_tab(ring(ids[0]), 42, 0); // no such group
        harness.host.move_tab(ring(ids[0]), -1, 0); // the ring's "none"
        harness.host.move_tab(9_999, 1, 0); // no such tab
        assert!(harness.changes().is_empty());
        assert_eq!(
            harness.controller.borrow().group_tabs(harness.research),
            [ids[0], ids[1], ids[2]]
        );
    }

    #[test]
    fn orbit_is_opened_by_the_callback_and_is_nothing_without_one() {
        let harness = harness();
        harness.host.open_orbit();
        harness.host.open_orbit();
        assert_eq!(harness.orbit_calls.get(), 2);

        let host = DesktopHost::new(
            Rc::new(RefCell::new(BrowserController::default())),
            DesktopActions::new(|_| {}),
        );
        host.open_orbit();
    }

    #[test]
    fn a_web_search_opens_a_tab_on_the_engines_results_page() {
        let harness = harness();
        harness
            .controller
            .borrow_mut()
            .set_search_url_builder(SearchUrlBuilder::new(|text| {
                url::Url::parse(&format!("https://search.test/?q={text}"))
                    .map_err(|error| error.to_string())
            }));

        harness.host.search_web("  tram 28  ");

        let mut controller = harness.controller.borrow_mut();
        assert_eq!(controller.tabs().len(), 9);
        let opened = controller.active_tab_id();
        assert!(!harness.ids.contains(&opened), "the new tab is shown");
        assert_eq!(
            controller.take_initial_url(opened).as_deref(),
            Some("https://search.test/?q=tram%2028")
        );
        drop(controller);
        assert_eq!(harness.changes(), [Change::Shown]);

        // An address-like query is still searched for, never navigated to.
        harness.host.search_web("example.com");
        let mut controller = harness.controller.borrow_mut();
        let opened = controller.active_tab_id();
        assert_eq!(
            controller.take_initial_url(opened).as_deref(),
            Some("https://search.test/?q=example.com")
        );
    }

    #[test]
    fn a_web_search_for_nothing_or_without_an_engine_opens_nothing() {
        let harness = harness();
        harness.host.search_web("   ");
        // No search builder is set yet: nowhere to search.
        harness.host.search_web("tram");
        assert_eq!(harness.controller.borrow().tabs().len(), 8);
        assert!(harness.changes().is_empty());
    }

    #[test]
    fn a_request_while_the_state_is_borrowed_is_dropped_not_a_panic() {
        let harness = harness();
        let held = harness.controller.borrow();
        harness.host.activate_tab(ring(harness.ids[3]));
        harness.host.close_tab(ring(harness.ids[6]));
        harness.host.toggle_mute(ring(harness.ids[4]));
        harness.host.toggle_pause(ring(harness.ids[2]));
        harness.host.move_tab(ring(harness.ids[6]), 1, 0);
        harness.host.search_web("tram");
        assert_eq!(held.active_tab_id(), harness.ids[0]);
        drop(held);
        assert!(harness.changes().is_empty());
    }

    // --- install --------------------------------------------------------------------------

    /// The bridge's callbacks reach the host through the controller of `mercury_ui`: a tab chosen
    /// in the ring, and a key press (→ moves the focus on from the active tab, ⏎ shows it).
    #[test]
    fn install_connects_the_ring_to_the_host() {
        crate::settings::tests::init_slint();
        let window = RingPopoverPreview::new().expect("the ring preview can be created");
        let Fixture {
            controller, ids, ..
        } = fixture();
        let controller = Rc::new(RefCell::new(controller));
        let changes = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&changes);
        let ring_controller = install(
            &window,
            Rc::clone(&controller),
            DesktopActions::new(move |change| recorded.borrow_mut().push(change)),
        );
        let style = RowStyle::from_window(&window);
        ring_controller.set_input(rows(&controller.borrow(), at(2_000), &style));
        ring_controller.opened();
        let bridge = window.global::<HubRingBridge>();

        bridge.invoke_tab_activated(ring(ids[3]));
        assert_eq!(controller.borrow().active_tab_id(), ids[3]);
        assert_eq!(*changes.borrow(), [Change::Shown]);

        // The popover opens on the active tab; → is the next tab in the ring, ⏎ shows it.
        assert!(controller.borrow_mut().select_tab(ids[0]));
        ring_controller.opened();
        assert!(bridge.invoke_key_pressed(Key::RightArrow.into(), false, false, false, false));
        assert!(bridge.invoke_key_pressed(Key::Return.into(), false, false, false, false));
        assert_eq!(controller.borrow().active_tab_id(), ids[1]);
        assert_eq!(changes.borrow().len(), 2);
    }
}
