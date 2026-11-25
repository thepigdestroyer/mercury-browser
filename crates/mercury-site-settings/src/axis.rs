// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The things a site can be allowed, blocked or configured for.

use crate::value::{AutoplayPolicy, DataRule, Setting, Value, ValueKind};

/// One per-site decision. The ids ([`id`](Self::id)) are how rules are stored: they never change,
/// and a row whose id this build does not know is kept and ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Axis {
    /// `navigator.mediaDevices` video (MS03, ME12).
    Camera,
    /// `navigator.mediaDevices` audio.
    Microphone,
    /// Geolocation.
    Location,
    /// Web notifications (MS07).
    Notifications,
    /// Reading the clipboard from a script.
    ClipboardRead,
    /// Web MIDI (MS12).
    Midi,
    /// Web Bluetooth (MS14).
    Bluetooth,
    /// WebUSB (MS14).
    Usb,
    /// Web Serial (MS14).
    Serial,
    /// WebHID (MS14).
    Hid,
    /// Motion and other sensors (MS15).
    Sensors,
    /// Background sync (and push).
    BackgroundSync,
    /// Keeping the screen on.
    ScreenWakeLock,
    /// Gamepads (MS13).
    Gamepad,
    /// `navigator.storage.persist()`.
    PersistentStorage,
    /// Going full screen without a click (MS16).
    AutomaticFullscreen,
    /// More than one download without a click (MF08).
    AutomaticDownloads,
    /// Pop-up windows (MA06). Built-in default: blocked.
    Popups,
    /// JavaScript (MS05; the engine switch is EB22).
    JavaScript,
    /// Images (MS05).
    Images,
    /// Sound; `Block` mutes the site (ME08).
    Sound,
    /// Automatic playback (ME07).
    Autoplay,
    /// Page zoom in percent (MD04).
    Zoom,
    /// Cookies and storage rules (MH42).
    SiteData,
    /// Opening links of one URL scheme in another app without asking (MN13); the qualifier is
    /// the scheme.
    ExternalLinks,
    /// Registering protocol handlers (MS11); the handlers themselves are a table.
    ProtocolHandlers,
    /// Offering to save passwords (the "never save" list).
    SavePasswords,
}

impl Axis {
    /// Every axis, in the order lists show them.
    pub const ALL: &'static [Axis] = &[
        Self::Camera,
        Self::Microphone,
        Self::Location,
        Self::Notifications,
        Self::ClipboardRead,
        Self::Midi,
        Self::Bluetooth,
        Self::Usb,
        Self::Serial,
        Self::Hid,
        Self::Sensors,
        Self::BackgroundSync,
        Self::ScreenWakeLock,
        Self::Gamepad,
        Self::PersistentStorage,
        Self::AutomaticFullscreen,
        Self::AutomaticDownloads,
        Self::Popups,
        Self::JavaScript,
        Self::Images,
        Self::Sound,
        Self::Autoplay,
        Self::Zoom,
        Self::SiteData,
        Self::ExternalLinks,
        Self::ProtocolHandlers,
        Self::SavePasswords,
    ];

    /// The stored id: lower-case ASCII and underscores, fixed for ever.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Microphone => "microphone",
            Self::Location => "location",
            Self::Notifications => "notifications",
            Self::ClipboardRead => "clipboard_read",
            Self::Midi => "midi",
            Self::Bluetooth => "bluetooth",
            Self::Usb => "usb",
            Self::Serial => "serial",
            Self::Hid => "hid",
            Self::Sensors => "sensors",
            Self::BackgroundSync => "background_sync",
            Self::ScreenWakeLock => "screen_wake_lock",
            Self::Gamepad => "gamepad",
            Self::PersistentStorage => "persistent_storage",
            Self::AutomaticFullscreen => "automatic_fullscreen",
            Self::AutomaticDownloads => "automatic_downloads",
            Self::Popups => "popups",
            Self::JavaScript => "javascript",
            Self::Images => "images",
            Self::Sound => "sound",
            Self::Autoplay => "autoplay",
            Self::Zoom => "zoom",
            Self::SiteData => "site_data",
            Self::ExternalLinks => "external_links",
            Self::ProtocolHandlers => "protocol_handlers",
            Self::SavePasswords => "save_passwords",
        }
    }

    /// The axis with this stored id, if this build has one.
    pub fn parse(id: &str) -> Option<Axis> {
        Self::ALL.iter().copied().find(|axis| axis.id() == id)
    }

    /// Which values the axis takes.
    pub const fn kind(self) -> ValueKind {
        match self {
            Self::Popups | Self::JavaScript | Self::Images | Self::Sound | Self::SavePasswords => {
                ValueKind::Toggle
            }
            Self::Autoplay => ValueKind::Autoplay,
            Self::Zoom => ValueKind::Zoom,
            Self::SiteData => ValueKind::Data,
            _ => ValueKind::Permission,
        }
    }

    /// What applies when no rule matches and the user has stored no default.
    pub const fn builtin_default(self) -> Value {
        match self {
            Self::Popups => Value::Permission(Setting::Block),
            Self::JavaScript | Self::Images | Self::Sound | Self::SavePasswords => {
                Value::Permission(Setting::Allow)
            }
            Self::Autoplay => Value::Autoplay(AutoplayPolicy::BlockAudible),
            Self::Zoom => Value::Zoom(100),
            Self::SiteData => Value::Data(DataRule::FollowGlobal),
            _ => Value::Permission(Setting::Ask),
        }
    }

    /// Whether a rule of this axis carries a qualifier (the URL scheme of `ExternalLinks`).
    pub const fn takes_qualifier(self) -> bool {
        matches!(self, Self::ExternalLinks)
    }

    /// Whether a permanent "allow" of this axis is reset to "ask" when unused for
    /// `AUTO_REVOKE_AFTER_MS` (MH59): the sensitive permissions.
    pub const fn auto_revocable(self) -> bool {
        matches!(
            self,
            Self::Camera
                | Self::Microphone
                | Self::Location
                | Self::Notifications
                | Self::ClipboardRead
                | Self::Midi
                | Self::Bluetooth
                | Self::Usb
                | Self::Serial
                | Self::Hid
                | Self::Sensors
                | Self::BackgroundSync
                | Self::ScreenWakeLock
                | Self::Gamepad
        )
    }
}
