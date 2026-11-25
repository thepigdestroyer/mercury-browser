// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Axes, values and lifetimes.

use std::collections::HashSet;

use mercury_site_settings::{
    AutoplayPolicy, Axis, DataRule, HOUR_MS, Lifetime, Setting, Value, ValueKind,
};

fn every_value_of(kind: ValueKind) -> Vec<Value> {
    match kind {
        ValueKind::Permission => [Setting::Allow, Setting::Block, Setting::Ask]
            .map(Value::Permission)
            .to_vec(),
        ValueKind::Toggle => [Setting::Allow, Setting::Block]
            .map(Value::Permission)
            .to_vec(),
        ValueKind::Autoplay => [
            AutoplayPolicy::Allow,
            AutoplayPolicy::BlockAudible,
            AutoplayPolicy::Block,
        ]
        .map(Value::Autoplay)
        .to_vec(),
        ValueKind::Zoom => [30, 100, 125, 500].map(Value::Zoom).to_vec(),
        ValueKind::Data => [
            DataRule::FollowGlobal,
            DataRule::Keep,
            DataRule::ClearOnExit,
            DataRule::ForgetOnClose,
            DataRule::Block,
        ]
        .map(Value::Data)
        .to_vec(),
    }
}

#[test]
fn every_axis_has_a_unique_lower_snake_case_id_that_parses_back() {
    let mut seen = HashSet::new();
    for axis in Axis::ALL {
        let id = axis.id();
        assert!(!id.is_empty() && id.len() <= 32, "{id}");
        assert!(
            id.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{id}"
        );
        assert!(seen.insert(id), "duplicate id {id}");
        assert_eq!(Axis::parse(id), Some(*axis));
    }
    assert_eq!(Axis::ALL.len(), 27);
    assert_eq!(Axis::parse("teleport"), None);
    assert_eq!(Axis::parse("Camera"), None);
    assert_eq!(Axis::parse(""), None);
}

#[test]
fn the_ids_are_the_stored_spelling_and_do_not_change() {
    let table = [
        (Axis::Camera, "camera"),
        (Axis::Microphone, "microphone"),
        (Axis::Location, "location"),
        (Axis::Notifications, "notifications"),
        (Axis::ClipboardRead, "clipboard_read"),
        (Axis::PersistentStorage, "persistent_storage"),
        (Axis::AutomaticDownloads, "automatic_downloads"),
        (Axis::Popups, "popups"),
        (Axis::JavaScript, "javascript"),
        (Axis::Images, "images"),
        (Axis::Sound, "sound"),
        (Axis::Autoplay, "autoplay"),
        (Axis::Zoom, "zoom"),
        (Axis::SiteData, "site_data"),
        (Axis::ExternalLinks, "external_links"),
        (Axis::ProtocolHandlers, "protocol_handlers"),
        (Axis::SavePasswords, "save_passwords"),
    ];
    for (axis, id) in table {
        assert_eq!(axis.id(), id);
    }
}

#[test]
fn builtin_defaults_are_the_documented_ones() {
    let ask = Value::Permission(Setting::Ask);
    let allow = Value::Permission(Setting::Allow);
    for axis in [
        Axis::Camera,
        Axis::Microphone,
        Axis::Location,
        Axis::Notifications,
        Axis::ClipboardRead,
        Axis::Midi,
        Axis::Bluetooth,
        Axis::Usb,
        Axis::Serial,
        Axis::Hid,
        Axis::Sensors,
        Axis::BackgroundSync,
        Axis::PersistentStorage,
        Axis::ScreenWakeLock,
        Axis::Gamepad,
        Axis::AutomaticFullscreen,
        Axis::AutomaticDownloads,
        Axis::ExternalLinks,
        Axis::ProtocolHandlers,
    ] {
        assert_eq!(axis.builtin_default(), ask, "{}", axis.id());
    }
    assert_eq!(
        Axis::Popups.builtin_default(),
        Value::Permission(Setting::Block)
    );
    for axis in [
        Axis::JavaScript,
        Axis::Images,
        Axis::Sound,
        Axis::SavePasswords,
    ] {
        assert_eq!(axis.builtin_default(), allow, "{}", axis.id());
    }
    assert_eq!(
        Axis::Autoplay.builtin_default(),
        Value::Autoplay(AutoplayPolicy::BlockAudible)
    );
    assert_eq!(Axis::Zoom.builtin_default(), Value::Zoom(100));
    assert_eq!(
        Axis::SiteData.builtin_default(),
        Value::Data(DataRule::FollowGlobal)
    );
}

#[test]
fn every_builtin_default_is_a_value_its_axis_accepts() {
    for axis in Axis::ALL {
        assert!(axis.builtin_default().allowed_for(*axis), "{}", axis.id());
    }
}

#[test]
fn values_round_trip_through_their_stored_text_on_every_axis_that_allows_them() {
    for axis in Axis::ALL {
        for value in every_value_of(axis.kind()) {
            assert!(value.allowed_for(*axis));
            let text = value.encode();
            assert_eq!(
                Value::decode(*axis, &text),
                Some(value),
                "{} {text}",
                axis.id()
            );
        }
    }
}

#[test]
fn the_stored_text_of_values_is_fixed() {
    let table = [
        (Value::Permission(Setting::Allow), "allow"),
        (Value::Permission(Setting::Block), "block"),
        (Value::Permission(Setting::Ask), "ask"),
        (Value::Autoplay(AutoplayPolicy::Allow), "allow"),
        (
            Value::Autoplay(AutoplayPolicy::BlockAudible),
            "block_audible",
        ),
        (Value::Autoplay(AutoplayPolicy::Block), "block"),
        (Value::Zoom(125), "zoom:125"),
        (Value::Data(DataRule::FollowGlobal), "follow_global"),
        (Value::Data(DataRule::Keep), "keep"),
        (Value::Data(DataRule::ClearOnExit), "clear_on_exit"),
        (Value::Data(DataRule::ForgetOnClose), "forget_on_close"),
        (Value::Data(DataRule::Block), "block"),
    ];
    for (value, text) in table {
        assert_eq!(value.encode(), text);
    }
}

#[test]
fn text_a_axis_cannot_hold_does_not_decode() {
    let table = [
        (Axis::Zoom, "zoom:29"),
        (Axis::Zoom, "zoom:501"),
        (Axis::Zoom, "zoom:abc"),
        (Axis::Zoom, "zoom:0125"),
        (Axis::Zoom, "zoom:+125"),
        (Axis::Zoom, "zoom:"),
        (Axis::Zoom, "allow"),
        (Axis::Camera, "block_audible"),
        (Axis::Camera, "Allow"),
        (Axis::Camera, " allow"),
        (Axis::Camera, ""),
        (Axis::Camera, "zoom:100"),
        (Axis::Sound, "ask"),
        (Axis::Popups, "ask"),
        (Axis::JavaScript, "ask"),
        (Axis::Autoplay, "ask"),
        (Axis::SiteData, "allow"),
    ];
    for (axis, text) in table {
        assert_eq!(Value::decode(axis, text), None, "{} {text:?}", axis.id());
    }
}

#[test]
fn a_value_of_the_wrong_kind_is_not_allowed_for_the_axis() {
    assert!(!Value::Zoom(125).allowed_for(Axis::Camera));
    assert!(!Value::Permission(Setting::Allow).allowed_for(Axis::Zoom));
    assert!(!Value::Permission(Setting::Ask).allowed_for(Axis::Popups));
    assert!(Value::Permission(Setting::Ask).allowed_for(Axis::AutomaticDownloads));
    assert!(!Value::Zoom(29).allowed_for(Axis::Zoom));
    assert!(!Value::Zoom(501).allowed_for(Axis::Zoom));
    assert!(Value::Zoom(30).allowed_for(Axis::Zoom));
    assert!(Value::Zoom(500).allowed_for(Axis::Zoom));
    assert!(!Value::Autoplay(AutoplayPolicy::Block).allowed_for(Axis::Sound));
    assert!(!Value::Data(DataRule::Keep).allowed_for(Axis::JavaScript));
}

#[test]
fn only_external_links_take_a_qualifier() {
    for axis in Axis::ALL {
        assert_eq!(
            axis.takes_qualifier(),
            *axis == Axis::ExternalLinks,
            "{}",
            axis.id()
        );
    }
}

#[test]
fn exactly_the_sensitive_permission_axes_are_auto_revocable() {
    let revocable: HashSet<&str> = Axis::ALL
        .iter()
        .filter(|axis| axis.auto_revocable())
        .map(|axis| axis.id())
        .collect();
    let expected: HashSet<&str> = [
        "camera",
        "microphone",
        "location",
        "notifications",
        "clipboard_read",
        "midi",
        "bluetooth",
        "usb",
        "serial",
        "hid",
        "sensors",
        "background_sync",
        "screen_wake_lock",
        "gamepad",
    ]
    .into_iter()
    .collect();
    assert_eq!(revocable, expected);
}

#[test]
fn lifetimes_know_whether_they_reach_the_disk() {
    assert_eq!(
        Lifetime::for_duration(1_000, HOUR_MS),
        Lifetime::Until(1_000 + HOUR_MS)
    );
    assert_eq!(
        Lifetime::for_duration(i64::MAX - 1, HOUR_MS),
        Lifetime::Until(i64::MAX)
    );
    assert!(Lifetime::Permanent.is_persistent());
    assert!(Lifetime::Until(5).is_persistent());
    assert!(!Lifetime::Session.is_persistent());
    assert!(!Lifetime::ThisVisit.is_persistent());
}
