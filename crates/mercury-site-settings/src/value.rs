// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What an axis can be set to, and how a value is written in the profile database.

use crate::{MAX_ZOOM_PERCENT, MIN_ZOOM_PERCENT};

/// The three answers to "may this site?".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Setting {
    /// The site may, without asking.
    Allow,
    /// The site may not, without asking.
    Block,
    /// Ask the user when the site wants to.
    Ask,
}

/// What a site may play automatically (ME07).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutoplayPolicy {
    /// Everything may start by itself.
    Allow,
    /// Media with sound needs a click; muted media may start (Mercury's default).
    BlockAudible,
    /// Nothing starts by itself.
    Block,
}

/// What happens to a site's cookies and storage (MH42).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataRule {
    /// No rule for the site: the global "clear on exit" settings apply. This is the built-in
    /// answer; it cannot be stored as a rule.
    FollowGlobal,
    /// Keep the data, even when "clear on exit" is on (a clear-on-exit exception).
    Keep,
    /// Clear the data when the browser quits, even when "clear on exit" is off.
    ClearOnExit,
    /// Clear the data when the site's last tab closes.
    ForgetOnClose,
    /// The site cannot store data.
    Block,
}

/// The kind of value an axis takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// Allow, block or ask.
    Permission,
    /// Allow or block (asking makes no sense).
    Toggle,
    /// An [`AutoplayPolicy`].
    Autoplay,
    /// A zoom percentage.
    Zoom,
    /// A [`DataRule`].
    Data,
}

/// A value of an axis. Which values an axis takes is [`Axis::kind`](crate::Axis::kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    /// For a [`ValueKind::Permission`] or [`ValueKind::Toggle`] axis.
    Permission(Setting),
    /// For the autoplay axis.
    Autoplay(AutoplayPolicy),
    /// For the zoom axis, in percent.
    Zoom(u16),
    /// For the site-data axis.
    Data(DataRule),
}

impl Value {
    /// Whether `axis` takes this value.
    pub fn allowed_for(&self, axis: crate::Axis) -> bool {
        match (axis.kind(), self) {
            (ValueKind::Permission, Self::Permission(_)) => true,
            (ValueKind::Toggle, Self::Permission(setting)) => *setting != Setting::Ask,
            (ValueKind::Autoplay, Self::Autoplay(_)) | (ValueKind::Data, Self::Data(_)) => true,
            (ValueKind::Zoom, Self::Zoom(percent)) => {
                (MIN_ZOOM_PERCENT..=MAX_ZOOM_PERCENT).contains(percent)
            }
            _ => false,
        }
    }

    /// The text this value is stored as (`allow`, `block_audible`, `zoom:125`, ...). Which kind
    /// it is comes from the axis, so `block` is both a permission and a data rule.
    pub fn encode(&self) -> String {
        match self {
            Self::Permission(Setting::Allow) | Self::Autoplay(AutoplayPolicy::Allow) => {
                "allow".into()
            }
            Self::Permission(Setting::Block)
            | Self::Autoplay(AutoplayPolicy::Block)
            | Self::Data(DataRule::Block) => "block".into(),
            Self::Permission(Setting::Ask) => "ask".into(),
            Self::Autoplay(AutoplayPolicy::BlockAudible) => "block_audible".into(),
            Self::Zoom(percent) => format!("zoom:{percent}"),
            Self::Data(DataRule::FollowGlobal) => "follow_global".into(),
            Self::Data(DataRule::Keep) => "keep".into(),
            Self::Data(DataRule::ClearOnExit) => "clear_on_exit".into(),
            Self::Data(DataRule::ForgetOnClose) => "forget_on_close".into(),
        }
    }

    /// Reads stored text for `axis`; `None` when the text is not one this axis can hold.
    pub fn decode(axis: crate::Axis, text: &str) -> Option<Self> {
        let value = match axis.kind() {
            ValueKind::Permission | ValueKind::Toggle => Self::Permission(match text {
                "allow" => Setting::Allow,
                "block" => Setting::Block,
                "ask" => Setting::Ask,
                _ => return None,
            }),
            ValueKind::Autoplay => Self::Autoplay(match text {
                "allow" => AutoplayPolicy::Allow,
                "block_audible" => AutoplayPolicy::BlockAudible,
                "block" => AutoplayPolicy::Block,
                _ => return None,
            }),
            ValueKind::Zoom => {
                let percent: u16 = text.strip_prefix("zoom:")?.parse().ok()?;
                Self::Zoom(percent)
            }
            ValueKind::Data => Self::Data(match text {
                "follow_global" => DataRule::FollowGlobal,
                "keep" => DataRule::Keep,
                "clear_on_exit" => DataRule::ClearOnExit,
                "forget_on_close" => DataRule::ForgetOnClose,
                "block" => DataRule::Block,
                _ => return None,
            }),
        };
        // One spelling per value: "zoom:0125" and "zoom:+125" are not ours.
        (value.allowed_for(axis) && value.encode() == text).then_some(value)
    }
}
