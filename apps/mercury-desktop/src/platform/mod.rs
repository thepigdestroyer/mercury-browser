// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! macOS integrations: the share sheet, Services, the Dock menu, System Settings links, Look Up
//! and the login item.
//!
//! Each OS service sits behind a trait with a fake (`Recording*`, `FakeLoginItem`). The logic is
//! plain Rust in `<name>.rs`; the `objc2` glue is in `<name>_macos.rs`. A process that never
//! calls [`attach`] (every unit test) gets the fakes from [`services`], and `attach` installs the
//! real objects only when the run is not automated, so a test or an automated run never registers
//! a login item or a Service, never opens System Settings and never causes a privacy prompt.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use mercury_ui::MainWindow;
use talaria_core::BrowserController;

pub(crate) mod dock_menu;
#[cfg(target_os = "macos")]
mod dock_menu_macos;
pub(crate) mod login_item;
#[cfg(target_os = "macos")]
mod login_item_macos;
pub(crate) mod look_up;
#[cfg(target_os = "macos")]
mod look_up_macos;
pub(crate) mod relaunch;
pub(crate) mod services;
#[cfg(target_os = "macos")]
mod services_macos;
pub(crate) mod share;
#[cfg(target_os = "macos")]
mod share_macos;
pub(crate) mod system_settings;
#[cfg(target_os = "macos")]
mod system_settings_macos;

use login_item::{FakeLoginItem, LoginItemService, RefusingLoginItem};
use look_up::{Dictionary, RecordingDictionary};
use relaunch::{RecordingRelaunch, RelaunchOnLogin};
use share::{RecordingSharer, Sharer};
use system_settings::{RecordingSettingsOpener, SettingsOpener};

/// What a menu item of the platform module asks for (`MenuCommand::Platform`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlatformCommand {
    /// File ▸ Share…
    SharePage,
    /// Edit ▸ Look Up Selection
    LookUp,
}

/// The OS services of this run.
pub(crate) struct PlatformServices {
    pub(crate) sharer: Rc<dyn Sharer>,
    pub(crate) dictionary: Rc<dyn Dictionary>,
    pub(crate) settings_opener: Rc<dyn SettingsOpener>,
    pub(crate) login_item: Rc<dyn LoginItemService>,
    pub(crate) relaunch: Rc<dyn RelaunchOnLogin>,
}

/// The fakes behind a [`PlatformServices::fakes`], for tests to read back.
pub(crate) struct Fakes {
    pub(crate) sharer: Rc<RecordingSharer>,
    pub(crate) dictionary: Rc<RecordingDictionary>,
    pub(crate) settings_opener: Rc<RecordingSettingsOpener>,
    pub(crate) login_item: Rc<FakeLoginItem>,
    pub(crate) relaunch: Rc<RecordingRelaunch>,
}

impl PlatformServices {
    /// The real services for an interactive run on macOS; recording or refusing ones otherwise.
    pub(crate) fn for_run(automated: bool) -> Self {
        #[cfg(target_os = "macos")]
        if !automated {
            return Self {
                sharer: Rc::new(share_macos::NsSharingPicker),
                dictionary: Rc::new(look_up_macos::NsDictionary),
                settings_opener: Rc::new(system_settings_macos::WorkspaceOpener),
                login_item: Rc::new(login_item_macos::SmAppService),
                relaunch: Rc::new(login_item_macos::NsRelaunch),
            };
        }
        let _ = automated;
        let (services, _) = Self::fakes();
        Self {
            login_item: Rc::new(RefusingLoginItem::new("automated run")),
            ..services
        }
    }

    /// Fakes that record their calls.
    pub(crate) fn fakes() -> (Self, Fakes) {
        let fakes = Fakes {
            sharer: Rc::new(RecordingSharer::default()),
            dictionary: Rc::new(RecordingDictionary::default()),
            settings_opener: Rc::new(RecordingSettingsOpener::default()),
            login_item: Rc::new(FakeLoginItem::new(
                login_item::LoginItemStatus::NotRegistered,
            )),
            relaunch: Rc::new(RecordingRelaunch::default()),
        };
        let services = Self {
            sharer: fakes.sharer.clone(),
            dictionary: fakes.dictionary.clone(),
            settings_opener: fakes.settings_opener.clone(),
            login_item: fakes.login_item.clone(),
            relaunch: fakes.relaunch.clone(),
        };
        (services, fakes)
    }
}

thread_local! {
    static SERVICES: RefCell<Option<Rc<PlatformServices>>> = const { RefCell::new(None) };
    static CONTROLLER: RefCell<Weak<RefCell<BrowserController>>> = RefCell::new(Weak::new());
}

/// The services of this run: the ones `attach` installed, or fakes if it was never called.
pub(crate) fn services() -> Rc<PlatformServices> {
    SERVICES.with(|services| {
        services
            .borrow_mut()
            .get_or_insert_with(|| Rc::new(PlatformServices::fakes().0))
            .clone()
    })
}

/// Installs `services` for this thread and tells macOS to relaunch Mercury after a restart.
/// Nothing registers anything: the login item only changes when the Settings check box is clicked.
pub(crate) fn attach_services(installed: Rc<PlatformServices>) {
    system_settings::set_opener(installed.settings_opener.clone());
    installed.relaunch.enable();
    SERVICES.with(|services| *services.borrow_mut() = Some(installed));
}

/// Before the event loop runs (on macOS, once winit's application delegate exists): adds the
/// Services and Dock-menu methods to the delegate. Registers nothing with the system.
pub(crate) fn install(automated: bool) {
    #[cfg(target_os = "macos")]
    {
        services_macos::install(automated);
        dock_menu_macos::install();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = automated;
}

/// Once the window and the controller exist.
pub(crate) fn attach(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    automated: bool,
) {
    attach_services(Rc::new(PlatformServices::for_run(automated)));
    CONTROLLER.with(|stored| *stored.borrow_mut() = Rc::downgrade(&controller));
    #[cfg(target_os = "macos")]
    dock_menu_macos::attach(window, controller);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, controller);
}

/// The controller `attach` was given, while it lives.
pub(crate) fn controller() -> Option<Rc<RefCell<BrowserController>>> {
    CONTROLLER.with(|stored| stored.borrow().upgrade())
}

/// Runs a File or Edit menu item of this module.
pub(crate) fn run_menu_command(
    command: PlatformCommand,
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
) {
    match command {
        PlatformCommand::SharePage => {
            share::share_active_page(&controller.borrow(), &*services().sharer);
        }
        PlatformCommand::LookUp => {
            if let Some(controller) = self::controller() {
                look_up::look_up_selection(window, controller, services().dictionary.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_that_never_attaches_gets_fakes() {
        let services = services();
        assert!(services.login_item.register().is_ok());
    }

    #[test]
    fn automated_runs_refuse_the_login_item_and_open_nothing() {
        let services = PlatformServices::for_run(true);
        assert!(services.login_item.register().is_err());
        assert!(services.login_item.unregister().is_err());
        assert!(
            !services
                .settings_opener
                .open(system_settings::SettingsPane::Camera)
        );
        assert!(!services.dictionary.show("word"));
    }

    #[test]
    fn attaching_enables_relaunch_once_and_registers_nothing() {
        let (installed, fakes) = PlatformServices::fakes();
        attach_services(Rc::new(installed));
        assert_eq!(fakes.relaunch.enabled.get(), 1);
        assert!(fakes.login_item.calls().is_empty());
        assert!(fakes.settings_opener.opened.borrow().is_empty());
    }

    // The OS calls that change the user's system live in one place each.
    #[test]
    fn only_the_login_item_glue_registers_anything() {
        let sources = [
            ("dock_menu_macos", include_str!("dock_menu_macos.rs")),
            ("look_up_macos", include_str!("look_up_macos.rs")),
            ("services_macos", include_str!("services_macos.rs")),
            ("share_macos", include_str!("share_macos.rs")),
            (
                "system_settings_macos",
                include_str!("system_settings_macos.rs"),
            ),
            ("dock_menu", include_str!("dock_menu.rs")),
            ("login_item", include_str!("login_item.rs")),
            ("look_up", include_str!("look_up.rs")),
            ("relaunch", include_str!("relaunch.rs")),
            ("services", include_str!("services.rs")),
            ("share", include_str!("share.rs")),
            ("system_settings", include_str!("system_settings.rs")),
        ];
        let forbidden = [
            concat!("register", "AndReturnError"),
            concat!("pb", "s -"),
            concat!("ls", "register"),
            concat!("NSUpdate", "DynamicServices"),
            concat!("sfl", "tool"),
            concat!("setDefault", "Application"),
        ];
        for (name, source) in sources {
            for needle in forbidden {
                assert!(!source.contains(needle), "{name} mentions {needle}");
            }
        }
    }
}
