// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The stable text form of an AccessKit tree, for tests (EE12).
//!
//! Owner: item 3 (`core-states`). [`dump`] reads an [`accesskit_consumer::Tree`] (the consumer
//! AccessKit's own adapters use, so what is dumped is what a platform adapter would see, grafts
//! included) and prints one line per node, indented by depth:
//!
//! ```text
//! root-web-area
//!   heading "Welcome" [level 1]
//!   text-input "Name" [required, invalid] value="Ada" desc="Your full name" placeholder="Name"
//!   checkbox "Remember me" [unchecked]
//!   link "Home" url="https://example.com/"
//!   graft
//!     root-web-area
//! ```
//!
//! * A line is `role`, then the label in quotes, then `[states]`, then `value=`, `desc=`, `url=` and
//!   `placeholder=`, then (when asked for) `actions=` and `bounds=`. Absent parts are left out.
//! * The role is AccessKit's [`Role`] in lower-case words (`list-box-option`, `text-input`); a **graft
//!   node** (a node with a tree id) prints `graft` instead, never the id, which is random, and the
//!   root of the grafted tree is its only child.
//! * Strings are quoted and escaped (`\"`, `\\`, `\n`, `\r`, `\t`, other control characters as
//!   `\u{..}`), so a line is always one line.
//! * States are in a fixed order: `hidden`, `disabled`, `read-only`, `required`, `busy`, `modal`,
//!   `multiselectable`, `checked`/`unchecked`/`mixed` (`pressed`/`unpressed`/`mixed` on a button),
//!   `expanded`/`collapsed`, `selected`/`unselected`, `invalid`, `has-popup=`, `current`,
//!   `orientation`, `autocomplete=`, `level N`, `item P of N`, and the range (`now`, `min`, `max`,
//!   `step`).
//! * No node ids, tree ids or timestamps appear; geometry only with [`DumpOptions::bounds`], rounded
//!   to a pixel. A node flagged hidden and its subtree are left out unless [`DumpOptions::hidden`]
//!   (a screen reader never sees them).
//!
//! The dump reads the node's own properties (`label`, `value`, `description`), not the ones a
//! platform adapter derives through relations, so it never follows a relation.

use std::fmt::Write as _;

use accesskit::{Action, AriaCurrent, AutoComplete, HasPopup, Invalid, Orientation, Role, Toggled};
use accesskit_consumer::{Node, Tree};

/// What `dump` prints beyond role, name, states and value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DumpOptions {
    /// Print each node's bounds, rounded to one pixel. Off by default: geometry makes a snapshot brittle.
    pub bounds: bool,
    /// Print the actions each node supports (`actions=click,focus`).
    pub actions: bool,
    /// Print the nodes flagged hidden, and their subtrees, with the `hidden` state (default: leave
    /// them out, as a screen reader does).
    pub hidden: bool,
}

impl DumpOptions {
    pub fn with_bounds(mut self) -> Self {
        self.bounds = true;
        self
    }

    pub fn with_actions(mut self) -> Self {
        self.actions = true;
        self
    }

    pub fn with_hidden(mut self) -> Self {
        self.hidden = true;
        self
    }
}

/// The tree as stable text, one line per node, indented by depth.
pub fn dump(tree: &Tree, options: &DumpOptions) -> String {
    dump_from(&tree.state().root(), options)
}

/// The subtree under `root` (the node included, at depth 0) as stable text: what a test prints to
/// show one page without the window and the web view above it.
pub fn dump_from(root: &Node<'_>, options: &DumpOptions) -> String {
    let mut out = String::new();
    // An explicit stack: a page can be nested far deeper than the thread's call stack allows.
    let mut stack = vec![(*root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if !options.hidden && node.data().is_hidden() {
            continue;
        }
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(&describe(&node, options));
        out.push('\n');
        let children: Vec<Node<'_>> = node.children().collect();
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    out
}

/// One node as one line, without indentation: the unit of [`dump`], also used to list candidates in
/// a failure message.
pub fn describe(node: &Node<'_>, options: &DumpOptions) -> String {
    let data = node.data();
    let mut line = if node.is_graft() {
        "graft".to_string()
    } else {
        role_name(data.role())
    };
    if let Some(label) = data.label() {
        line.push(' ');
        line.push_str(&quote(label));
    }
    let states = states(data);
    if !states.is_empty() {
        let _ = write!(line, " [{}]", states.join(", "));
    }
    if let Some(value) = data.value() {
        let _ = write!(line, " value={}", quote(value));
    }
    if let Some(description) = data.description() {
        let _ = write!(line, " desc={}", quote(description));
    }
    if let Some(url) = data.url() {
        let _ = write!(line, " url={}", quote(url));
    }
    if let Some(placeholder) = data.placeholder() {
        let _ = write!(line, " placeholder={}", quote(placeholder));
    }
    if options.actions {
        let actions: Vec<String> = ALL_ACTIONS
            .iter()
            .filter(|action| data.supports_action(**action))
            .map(|action| kebab(&format!("{action:?}")))
            .collect();
        if !actions.is_empty() {
            let _ = write!(line, " actions={}", actions.join(","));
        }
    }
    if options.bounds
        && let Some(rect) = node.bounding_box()
    {
        let _ = write!(
            line,
            " bounds={},{},{},{}",
            pixel(rect.x0),
            pixel(rect.y0),
            pixel(rect.x1),
            pixel(rect.y1)
        );
    }
    line
}

/// The role in lower-case words: `ListBoxOption` is `list-box-option`.
pub fn role_name(role: Role) -> String {
    kebab(&format!("{role:?}"))
}

/// Every action AccessKit defines, in its own order (the list `dump` prints from).
const ALL_ACTIONS: &[Action] = &[
    Action::Click,
    Action::Focus,
    Action::Blur,
    Action::Collapse,
    Action::Expand,
    Action::CustomAction,
    Action::Decrement,
    Action::Increment,
    Action::HideTooltip,
    Action::ShowTooltip,
    Action::ReplaceSelectedText,
    Action::ScrollDown,
    Action::ScrollLeft,
    Action::ScrollRight,
    Action::ScrollUp,
    Action::ScrollIntoView,
    Action::ScrollToPoint,
    Action::SetScrollOffset,
    Action::SetTextSelection,
    Action::SetSequentialFocusNavigationStartingPoint,
    Action::SetValue,
    Action::ShowContextMenu,
];

fn kebab(camel: &str) -> String {
    let mut out = String::with_capacity(camel.len() + 4);
    for (index, c) in camel.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A coordinate rounded to a whole pixel (NaN and infinities print as 0 and the extremes).
fn pixel(value: f64) -> i64 {
    value.round() as i64
}

fn states(data: &accesskit::Node) -> Vec<String> {
    let mut states: Vec<String> = Vec::new();
    let mut flag = |on: bool, text: &str| {
        if on {
            states.push(text.to_string());
        }
    };
    flag(data.is_hidden(), "hidden");
    flag(data.is_disabled(), "disabled");
    flag(data.is_read_only(), "read-only");
    flag(data.is_required(), "required");
    flag(data.is_busy(), "busy");
    flag(data.is_modal(), "modal");
    flag(data.is_multiselectable(), "multiselectable");
    if let Some(toggled) = data.toggled() {
        let button = matches!(data.role(), Role::Button | Role::DefaultButton);
        states.push(
            match (toggled, button) {
                (Toggled::True, false) => "checked",
                (Toggled::False, false) => "unchecked",
                (Toggled::True, true) => "pressed",
                (Toggled::False, true) => "unpressed",
                (Toggled::Mixed, _) => "mixed",
            }
            .to_string(),
        );
    }
    match data.is_expanded() {
        Some(true) => states.push("expanded".to_string()),
        Some(false) => states.push("collapsed".to_string()),
        None => {}
    }
    match data.is_selected() {
        Some(true) => states.push("selected".to_string()),
        Some(false) => states.push("unselected".to_string()),
        None => {}
    }
    if let Some(invalid) = data.invalid() {
        states.push(
            match invalid {
                Invalid::True => "invalid",
                Invalid::Grammar => "invalid=grammar",
                Invalid::Spelling => "invalid=spelling",
            }
            .to_string(),
        );
    }
    if let Some(popup) = data.has_popup() {
        let name = match popup {
            HasPopup::Menu => "menu",
            HasPopup::Listbox => "listbox",
            HasPopup::Tree => "tree",
            HasPopup::Grid => "grid",
            HasPopup::Dialog => "dialog",
        };
        states.push(format!("has-popup={name}"));
    }
    if let Some(current) = data.aria_current() {
        states.push(match current {
            AriaCurrent::True => "current".to_string(),
            other => format!("current={}", kebab(&format!("{other:?}"))),
        });
    }
    if let Some(orientation) = data.orientation() {
        states.push(
            match orientation {
                Orientation::Horizontal => "horizontal",
                Orientation::Vertical => "vertical",
            }
            .to_string(),
        );
    }
    if let Some(complete) = data.auto_complete() {
        states.push(
            match complete {
                AutoComplete::Inline => "autocomplete=inline",
                AutoComplete::List => "autocomplete=list",
                AutoComplete::Both => "autocomplete=both",
            }
            .to_string(),
        );
    }
    if let Some(level) = data.level() {
        states.push(format!("level {level}"));
    }
    match (data.position_in_set(), data.size_of_set()) {
        (Some(position), Some(size)) => states.push(format!("item {position} of {size}")),
        (Some(position), None) => states.push(format!("item {position}")),
        (None, Some(size)) => states.push(format!("set of {size}")),
        (None, None) => {}
    }
    if let Some(now) = data.numeric_value() {
        states.push(format!("now {now}"));
    }
    if let Some(min) = data.min_numeric_value() {
        states.push(format!("min {min}"));
    }
    if let Some(max) = data.max_numeric_value() {
        states.push(format!("max {max}"));
    }
    if let Some(step) = data.numeric_value_step() {
        states.push(format!("step {step}"));
    }
    states
}
