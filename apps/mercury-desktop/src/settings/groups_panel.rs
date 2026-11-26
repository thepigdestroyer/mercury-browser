// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Appearance and Tab groups sections of the settings window, and the browser behind them
//! (lane `settings-ui`, brief item 4).
//!
//! What each control shows and what it does to the settings is decided by the pure models in
//! `mercury_appearance::panels`; this file is the link to the rest of the browser: it reads the
//! window's groups from the controller, applies the panel's [`Change`]s to them with
//! `set_group_key` and `swap_group_key`, and routes the sections' actions into
//! `SettingsHandle::update`, so that every edit is validated, saved after the quiet period and
//! seen by listeners like any other setting.
//!
//! One browser window and one controller exist today (brief D13); the panel takes plain input
//! (`GroupInput`), so nothing here assumes it.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use mercury_appearance::keymap::LetterKey;
use mercury_appearance::panels::key_panel::{
    Change, GroupInput, KeyPanel, KeyboardFacts, PanelAction, PanelView, set_capture, set_hold,
};
use mercury_appearance::panels::layout_panel::{
    AppearanceAction, layout_panel, set_choice, set_switch,
};
use mercury_settings::{GroupKeySettings, Settings};
use mercury_ui::SettingsWindow;
use mercury_ui::settings_sections::{TAB_GROUP_INTERFACE_LIVE, appearance, tab_groups};
use talaria_core::{BrowserController, GroupError, GroupId, GroupKey};

use super::{SettingsHandle, window};

/// The core's key for a panel letter; both name the physical key `"KeyA"` to `"KeyZ"`.
pub(super) fn to_group_key(key: LetterKey) -> Option<GroupKey> {
    GroupKey::from_code_name(key.code_name())
}

/// The panel's letter for a core key.
pub(super) fn to_letter_key(key: GroupKey) -> Option<LetterKey> {
    LetterKey::from_code_name(key.code_name())
}

/// The window's groups in ring order as the panel takes them, and how many tabs are in none.
pub(super) fn inputs_from_controller(controller: &BrowserController) -> (Vec<GroupInput>, usize) {
    let groups = controller
        .groups()
        .iter()
        .map(|group| GroupInput {
            id: group.id.value(),
            name: group.name.clone(),
            colour: group.colour.to_name(),
            tabs: controller.group_tabs(group.id).len(),
            key: group.key.and_then(to_letter_key),
        })
        .collect();
    (groups, controller.loose_tabs().len())
}

/// Applies one change to the controller. A group that has gone, or a letter that is not a key,
/// is refused (nothing changes); the caller refreshes the table either way.
pub(super) fn apply_change(
    controller: &mut BrowserController,
    change: Change,
) -> Result<(), GroupError> {
    match change {
        Change::SetKey { group, key } => {
            let key = match key {
                Some(letter) => Some(to_group_key(letter).ok_or(GroupError::NotFound)?),
                None => None,
            };
            controller.set_group_key(GroupId::new(group), key)
        }
        Change::Swap { group, other } => {
            let key = controller
                .group(GroupId::new(other))
                .and_then(|holder| holder.key)
                .ok_or(GroupError::NotFound)?;
            controller.swap_group_key(GroupId::new(group), key)
        }
    }
}

/// Applies the changes one after the other, stopping at the first one refused.
pub(super) fn apply_changes(
    controller: &mut BrowserController,
    changes: impl IntoIterator<Item = Change>,
) -> Result<(), GroupError> {
    changes
        .into_iter()
        .try_for_each(|change| apply_change(controller, change))
}

/// Runs the actions of the table that touch groups: typing a key, Swap, Choose another and
/// Reset. The settings-only actions are `apply_key_setting`'s. The error is the first refusal of
/// the controller; the caller logs it and refreshes.
pub(super) fn run_panel_action(
    panel: &mut KeyPanel,
    controller: &mut BrowserController,
    action: &PanelAction,
) -> Result<(), GroupError> {
    match action {
        PanelAction::KeyTyped { group, text } => panel
            .key_typed(*group, text)
            .map_or(Ok(()), |change| apply_change(controller, change)),
        PanelAction::Swap { group } => panel
            .swap(*group)
            .map_or(Ok(()), |change| apply_change(controller, change)),
        PanelAction::ChooseAnother { group } => {
            panel.choose_another(*group);
            Ok(())
        }
        PanelAction::Reset => apply_changes(controller, panel.reset_keys()),
        PanelAction::Opened
        | PanelAction::ShowBesideNames(_)
        | PanelAction::Capture(_)
        | PanelAction::Hold(_) => Ok(()),
    }
}

/// What one switch or list of the Appearance section does to the settings. False for an index
/// outside the list (nothing changes).
pub(super) fn apply_appearance_action(settings: &mut Settings, action: AppearanceAction) -> bool {
    match action {
        AppearanceAction::Switch(control, on) => {
            set_switch(&mut settings.appearance, control, on);
            true
        }
        AppearanceAction::Choice(choice, index) => {
            set_choice(&mut settings.appearance, choice, index)
        }
    }
}

/// What the settings-only actions of the Tab groups section do to the key settings. False for an
/// action that is not one of them, or an index outside its list.
pub(super) fn apply_key_setting(keys: &mut GroupKeySettings, action: &PanelAction) -> bool {
    match action {
        PanelAction::ShowBesideNames(on) => {
            keys.show_beside_names = *on;
            true
        }
        PanelAction::Capture(index) => set_capture(keys, *index),
        PanelAction::Hold(index) => set_hold(keys, *index),
        _ => false,
    }
}

/// The letters new groups take, from the stored default sequence.
pub(super) fn key_sequence(keys: &GroupKeySettings) -> Vec<GroupKey> {
    GroupKey::parse_sequence(&keys.default_sequence)
}

/// The key panel, the controller it reads and the facts about the keyboard. One per settings
/// handle.
pub(super) struct GroupsPanel {
    panel: RefCell<KeyPanel>,
    controller: RefCell<Weak<RefCell<BrowserController>>>,
    facts: KeyboardFacts,
}

impl Default for GroupsPanel {
    fn default() -> Self {
        Self {
            panel: RefCell::new(KeyPanel::new()),
            controller: RefCell::new(Weak::new()),
            // The macOS U.S. layout until the key-switcher lanes learn the real one.
            facts: KeyboardFacts::us(),
        }
    }
}

impl GroupsPanel {
    /// Remembers the browser's controller (the handle never keeps it alive).
    pub(super) fn set_controller(&self, controller: &Rc<RefCell<BrowserController>>) {
        *self.controller.borrow_mut() = Rc::downgrade(controller);
    }

    /// Takes the groups as they are now and the key settings. Without a controller (the window
    /// was opened before the browser, or after it closed) the table is empty. A controller that
    /// is borrowed at this moment is left for the next refresh.
    pub(super) fn refresh(&self, settings: &Settings) {
        let keys = &settings.appearance.tab_groups.keys;
        let controller = self.controller.borrow().upgrade();
        let Ok(mut panel) = self.panel.try_borrow_mut() else {
            return;
        };
        match controller {
            Some(controller) => {
                let Ok(controller) = controller.try_borrow() else {
                    return;
                };
                let (groups, loose) = inputs_from_controller(&controller);
                panel.refresh(&groups, loose, keys, &self.facts, TAB_GROUP_INTERFACE_LIVE);
            }
            None => panel.refresh(&[], 0, keys, &self.facts, TAB_GROUP_INTERFACE_LIVE),
        }
    }

    /// What the table shows now.
    pub(super) fn view(&self) -> PanelView {
        self.panel.borrow().view()
    }

    /// A fresh panel: the section was opened again, so a conflict left over from the last visit
    /// is gone (D14).
    pub(super) fn reopen(&self) {
        *self.panel.borrow_mut() = KeyPanel::new();
    }

    /// Runs an action of the table that touches groups, then the caller refreshes.
    fn run(&self, action: &PanelAction) {
        let Some(controller) = self.controller.borrow().upgrade() else {
            return;
        };
        let Ok(mut controller) = controller.try_borrow_mut() else {
            log::warn!("tab groups settings: the browser was busy; the edit was dropped");
            return;
        };
        let Ok(mut panel) = self.panel.try_borrow_mut() else {
            return;
        };
        if let Err(error) = run_panel_action(&mut panel, &mut controller, action) {
            // A group that closed since the table was drawn: the refresh shows what is there.
            log::info!("tab groups settings: {action:?} was refused: {error}");
        }
    }
}

/// Fills the Tab groups section from the key panel, after refreshing the panel. Called for every
/// refresh of the window.
pub(super) fn fill_tab_groups(handle: &SettingsHandle, window: &SettingsWindow) {
    handle.groups.refresh(&handle.settings());
    tab_groups::fill(window, &handle.groups.view());
}

/// Fills the Appearance section from the settings.
pub(super) fn fill_appearance(window: &SettingsWindow, settings: &Settings) {
    appearance::fill(
        window,
        &layout_panel(&settings.appearance, TAB_GROUP_INTERFACE_LIVE),
    );
}

/// Connects both sections of a settings window to the handle. Call it once per window.
pub(super) fn connect(window: &SettingsWindow, handle: Weak<SettingsHandle>) {
    let appearance_handle = handle.clone();
    appearance::connect(window, move |action| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(handle) = appearance_handle.upgrade() {
            handle.update(|settings| {
                if !apply_appearance_action(settings, action) {
                    log::warn!("settings window: no such appearance choice {action:?}");
                }
            });
        }
    });
    tab_groups::connect(window, move |action| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let Some(handle) = handle.upgrade() else {
            return;
        };
        match action {
            PanelAction::Opened => handle.groups.reopen(),
            PanelAction::ShowBesideNames(_) | PanelAction::Capture(_) | PanelAction::Hold(_) => {
                handle.update(|settings| {
                    if !apply_key_setting(&mut settings.appearance.tab_groups.keys, &action) {
                        log::warn!("settings window: no such key setting {action:?}");
                    }
                });
            }
            _ => handle.groups.run(&action),
        }
        // A key edit changes the controller and not the settings, so nothing else would redraw;
        // and the table is a snapshot taken now (D14).
        window::refresh(&handle);
    });
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use mercury_appearance::panels::key_panel::RowState;
    use mercury_appearance::panels::layout_panel::{Choice, Control};
    use talaria_core::{GroupColour, GroupPreset, TabId};

    use super::*;

    fn at(seconds: u64) -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn letter(character: char) -> LetterKey {
        LetterKey::from_char(character).expect("a letter")
    }

    /// A controller with `count` groups of one tab each (named "Group 1", ...), keyed A, S, D, ...
    /// by the controller's own sequence, and `loose` tabs in no group.
    fn controller_with(count: usize, loose: usize) -> (BrowserController, Vec<GroupId>) {
        let mut controller = BrowserController::default();
        controller.set_group_key_sequence(GroupKey::parse_sequence("ASDFGHJKL"));
        let first = controller.active_tab_id();
        let mut tabs: Vec<TabId> = vec![first];
        while tabs.len() < count + loose {
            tabs.push(controller.open_tab());
        }
        let groups = tabs
            .iter()
            .take(count)
            .enumerate()
            .map(|(index, tab)| {
                controller
                    .create_group(
                        &[*tab],
                        Some(&format!("Group {}", index + 1)),
                        None,
                        at(index as u64),
                    )
                    .expect("a group")
            })
            .collect();
        (controller, groups)
    }

    fn keys_of(controller: &BrowserController) -> Vec<Option<char>> {
        controller
            .groups()
            .iter()
            .map(|group| group.key.map(GroupKey::letter))
            .collect()
    }

    #[test]
    fn the_26_letter_keys_map_to_the_26_group_keys_and_back() {
        let mut seen = Vec::new();
        for index in 0..26_u8 {
            let character = char::from(b'A' + index);
            let key = letter(character);
            let group_key = to_group_key(key).expect("a group key");
            assert_eq!(group_key.letter(), character);
            assert_eq!(to_letter_key(group_key), Some(key));
            assert!(!seen.contains(&group_key));
            seen.push(group_key);
        }
        assert_eq!(seen.len(), 26);
    }

    #[test]
    fn the_rows_come_from_the_controllers_groups_in_ring_order_with_their_keys_and_colours() {
        let (mut controller, groups) = controller_with(3, 2);
        controller
            .recolour_group(groups[0], GroupColour::Preset(GroupPreset::Mint))
            .expect("a preset");
        controller
            .recolour_group(groups[1], GroupColour::parse("#a1b2c3").expect("a colour"))
            .expect("a custom colour");

        let (inputs, loose) = inputs_from_controller(&controller);

        assert_eq!(loose, 2);
        assert_eq!(
            inputs.iter().map(|group| group.id).collect::<Vec<_>>(),
            groups.iter().map(|group| group.value()).collect::<Vec<_>>()
        );
        assert_eq!(inputs[0].name, "Group 1");
        assert_eq!(inputs[0].colour, "mint");
        assert_eq!(inputs[1].colour, "#a1b2c3");
        assert_eq!(inputs[0].tabs, 1);
        assert_eq!(
            inputs.iter().map(|group| group.key).collect::<Vec<_>>(),
            [Some(letter('A')), Some(letter('S')), Some(letter('D'))]
        );
    }

    #[test]
    fn the_panel_built_from_the_controller_shows_the_groups_and_the_loose_count() {
        let (controller, _) = controller_with(2, 3);
        let (inputs, loose) = inputs_from_controller(&controller);
        let mut panel = KeyPanel::new();
        panel.refresh(
            &inputs,
            loose,
            &GroupKeySettings::default(),
            &KeyboardFacts::us(),
            true,
        );

        let view = panel.view();

        assert_eq!(view.rows.len(), 2);
        assert_eq!(view.loose_tabs, 3);
        assert_eq!(view.rows[0].letter, "A");
        assert_eq!(view.rows[1].letter, "S");
        assert!(view.rows.iter().all(|row| row.state != RowState::Conflict));
    }

    #[test]
    fn setting_a_key_and_clearing_it_reach_the_controller() {
        let (mut controller, groups) = controller_with(2, 0);

        apply_change(
            &mut controller,
            Change::SetKey {
                group: groups[0].value(),
                key: Some(letter('Q')),
            },
        )
        .expect("a free letter");
        assert_eq!(keys_of(&controller), [Some('Q'), Some('S')]);

        apply_change(
            &mut controller,
            Change::SetKey {
                group: groups[1].value(),
                key: None,
            },
        )
        .expect("clearing");
        assert_eq!(keys_of(&controller), [Some('Q'), None]);
    }

    #[test]
    fn a_swap_gives_each_group_the_others_letter_and_a_keyless_group_leaves_none_behind() {
        let (mut controller, groups) = controller_with(3, 0);

        apply_change(
            &mut controller,
            Change::Swap {
                group: groups[0].value(),
                other: groups[1].value(),
            },
        )
        .expect("a swap");
        assert_eq!(keys_of(&controller), [Some('S'), Some('A'), Some('D')]);

        controller
            .set_group_key(groups[2], None)
            .expect("a group with no key");
        apply_change(
            &mut controller,
            Change::Swap {
                group: groups[2].value(),
                other: groups[0].value(),
            },
        )
        .expect("a swap with a keyless group");
        assert_eq!(keys_of(&controller), [None, Some('A'), Some('S')]);
    }

    #[test]
    fn a_stale_group_is_refused_without_a_panic_and_changes_nothing() {
        let (mut controller, groups) = controller_with(2, 0);
        let before = keys_of(&controller);

        for change in [
            Change::SetKey {
                group: 9_999,
                key: Some(letter('Z')),
            },
            Change::Swap {
                group: groups[0].value(),
                other: 9_999,
            },
            Change::Swap {
                group: 9_999,
                other: groups[0].value(),
            },
        ] {
            assert_eq!(
                apply_change(&mut controller, change),
                Err(GroupError::NotFound),
                "{change:?}"
            );
        }
        assert_eq!(keys_of(&controller), before);
    }

    #[test]
    fn typing_a_held_letter_leaves_a_conflict_and_swap_resolves_it_through_the_controller() {
        let (mut controller, groups) = controller_with(2, 0);
        let mut panel = KeyPanel::new();
        let refresh = |panel: &mut KeyPanel, controller: &BrowserController| {
            let (inputs, loose) = inputs_from_controller(controller);
            panel.refresh(
                &inputs,
                loose,
                &GroupKeySettings::default(),
                &KeyboardFacts::us(),
                true,
            );
        };
        refresh(&mut panel, &controller);

        // Group 2 is asked to take A, which Group 1 holds: nothing changes, the row is in conflict.
        let typed = PanelAction::KeyTyped {
            group: groups[1].value(),
            text: "a".to_owned(),
        };
        run_panel_action(&mut panel, &mut controller, &typed).expect("no refusal");
        assert_eq!(keys_of(&controller), [Some('A'), Some('S')]);
        refresh(&mut panel, &controller);
        assert_eq!(panel.view().rows[1].state, RowState::Conflict);

        let swap = PanelAction::Swap {
            group: groups[1].value(),
        };
        run_panel_action(&mut panel, &mut controller, &swap).expect("the swap");
        assert_eq!(keys_of(&controller), [Some('S'), Some('A')]);
        refresh(&mut panel, &controller);
        assert!(
            panel
                .view()
                .rows
                .iter()
                .all(|row| row.state == RowState::Plain)
        );
    }

    #[test]
    fn typing_into_a_group_that_has_closed_is_dropped_by_the_panel() {
        let (mut controller, _) = controller_with(1, 0);
        let mut panel = KeyPanel::new();
        let (inputs, loose) = inputs_from_controller(&controller);
        panel.refresh(
            &inputs,
            loose,
            &GroupKeySettings::default(),
            &KeyboardFacts::us(),
            true,
        );
        let action = PanelAction::KeyTyped {
            group: 4_242,
            text: "q".to_owned(),
        };

        assert_eq!(
            run_panel_action(&mut panel, &mut controller, &action),
            Ok(())
        );
        assert_eq!(keys_of(&controller), [Some('A')]);
    }

    #[test]
    fn a_group_closed_after_the_table_was_drawn_is_refused_and_the_refresh_drops_it() {
        let (mut controller, groups) = controller_with(2, 0);
        let mut panel = KeyPanel::new();
        let (inputs, loose) = inputs_from_controller(&controller);
        panel.refresh(
            &inputs,
            loose,
            &GroupKeySettings::default(),
            &KeyboardFacts::us(),
            true,
        );
        controller.close_group(groups[1]).expect("closing");

        let action = PanelAction::KeyTyped {
            group: groups[1].value(),
            text: "q".to_owned(),
        };
        let result = run_panel_action(&mut panel, &mut controller, &action);

        assert_eq!(result, Err(GroupError::NotFound));
        let (inputs, loose) = inputs_from_controller(&controller);
        panel.refresh(
            &inputs,
            loose,
            &GroupKeySettings::default(),
            &KeyboardFacts::us(),
            true,
        );
        assert_eq!(panel.view().rows.len(), 1);
    }

    #[test]
    fn reset_gives_the_groups_the_stored_sequence_in_ring_order() {
        let (mut controller, groups) = controller_with(3, 0);
        controller
            .set_group_key(groups[0], Some(letter_key('Z')))
            .expect("free");
        let keys = GroupKeySettings {
            default_sequence: "QWE".to_owned(),
            ..GroupKeySettings::default()
        };
        let mut panel = KeyPanel::new();
        let (inputs, loose) = inputs_from_controller(&controller);
        panel.refresh(&inputs, loose, &keys, &KeyboardFacts::us(), true);

        run_panel_action(&mut panel, &mut controller, &PanelAction::Reset).expect("reset");

        assert_eq!(keys_of(&controller), [Some('Q'), Some('W'), Some('E')]);
    }

    fn letter_key(character: char) -> GroupKey {
        GroupKey::from_letter(character).expect("a letter")
    }

    #[test]
    fn each_appearance_action_changes_exactly_one_field_of_the_settings() {
        let actions = [
            AppearanceAction::Switch(Control::Dock, false),
            AppearanceAction::Switch(Control::Ring, false),
            AppearanceAction::Switch(Control::Orbit, false),
            AppearanceAction::Switch(Control::OrbitPinch, false),
            AppearanceAction::Switch(Control::Keys, false),
            AppearanceAction::Switch(Control::Dial, false),
            AppearanceAction::Choice(Choice::Layout, 1),
            AppearanceAction::Choice(Choice::DockStarts, 1),
            AppearanceAction::Choice(Choice::DialCorner, 0),
            AppearanceAction::Choice(Choice::SnapshotCap, 0),
        ];
        for action in actions {
            let before = Settings::default();
            let mut after = before.clone();
            assert!(apply_appearance_action(&mut after, action), "{action:?}");
            let (validated, repairs) = after.clone().validated();
            assert!(repairs.is_empty(), "{action:?}: {repairs:?}");
            assert_eq!(validated, after);
            assert_ne!(after, before, "{action:?} changed something");
            let (a, b) = (&before.appearance.tab_groups, &after.appearance.tab_groups);
            let changed = [
                before.appearance.layout != after.appearance.layout,
                a.dock != b.dock,
                a.dock_in_classic != b.dock_in_classic,
                a.dock_starts != b.dock_starts,
                a.ring != b.ring,
                a.orbit != b.orbit,
                a.orbit_pinch != b.orbit_pinch,
                a.keys.enabled != b.keys.enabled,
                a.dial != b.dial,
                a.dial_corner != b.dial_corner,
                a.snapshot_cap_mb != b.snapshot_cap_mb,
            ]
            .into_iter()
            .filter(|changed| *changed)
            .count();
            assert_eq!(changed, 1, "{action:?}");
        }
    }

    #[test]
    fn an_appearance_choice_outside_its_list_changes_nothing() {
        let mut settings = Settings::default();
        assert!(!apply_appearance_action(
            &mut settings,
            AppearanceAction::Choice(Choice::Layout, 7)
        ));
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn the_settings_only_actions_of_the_table_edit_the_key_settings() {
        let mut keys = GroupKeySettings::default();

        assert!(apply_key_setting(
            &mut keys,
            &PanelAction::ShowBesideNames(false)
        ));
        assert!(!keys.show_beside_names);
        assert!(apply_key_setting(&mut keys, &PanelAction::Capture(2)));
        assert_eq!(keys.capture, mercury_settings::KeyCapture::Always);
        assert!(apply_key_setting(&mut keys, &PanelAction::Hold(0)));
        assert_eq!(keys.hold_ms, 250);

        let before = keys.clone();
        assert!(!apply_key_setting(&mut keys, &PanelAction::Capture(3)));
        assert!(!apply_key_setting(&mut keys, &PanelAction::Hold(99)));
        assert!(!apply_key_setting(&mut keys, &PanelAction::Reset));
        assert_eq!(keys, before);
    }

    #[test]
    fn the_key_sequence_of_the_settings_is_what_the_controller_is_given() {
        let keys = GroupKeySettings {
            default_sequence: "QWERT".to_owned(),
            ..GroupKeySettings::default()
        };
        let sequence = key_sequence(&keys);
        assert_eq!(
            sequence.iter().map(|key| key.letter()).collect::<String>(),
            "QWERT"
        );
        let none = GroupKeySettings {
            default_sequence: String::new(),
            ..GroupKeySettings::default()
        };
        assert!(key_sequence(&none).is_empty());
    }

    #[test]
    fn without_a_controller_the_table_is_empty_and_a_borrowed_controller_keeps_the_last_table() {
        let panel = GroupsPanel::default();
        panel.refresh(&Settings::default());
        assert!(panel.view().empty);

        let (controller, _) = controller_with(2, 0);
        let controller = Rc::new(RefCell::new(controller));
        panel.set_controller(&controller);
        panel.refresh(&Settings::default());
        assert_eq!(panel.view().rows.len(), 2);

        let busy = controller.borrow_mut();
        panel.refresh(&Settings::default());
        drop(busy);
        assert_eq!(
            panel.view().rows.len(),
            2,
            "nothing was lost while it was busy"
        );
    }

    #[test]
    fn opening_the_section_again_drops_a_conflict_left_from_the_last_visit() {
        let (controller, groups) = controller_with(2, 0);
        let controller = Rc::new(RefCell::new(controller));
        let panel = GroupsPanel::default();
        panel.set_controller(&controller);
        panel.refresh(&Settings::default());
        panel.run(&PanelAction::KeyTyped {
            group: groups[1].value(),
            text: "a".to_owned(),
        });
        panel.refresh(&Settings::default());
        assert_eq!(panel.view().rows[1].state, RowState::Conflict);

        panel.reopen();
        panel.refresh(&Settings::default());

        assert_eq!(panel.view().rows[1].state, RowState::Plain);
    }
}
