// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Reads the interface the way an assistive technology does, through Slint's testing backend:
//! every element with an accessible role, in the order a screen reader walks them, with its
//! label, description and state. Tests find a control by role and label, press its default
//! action, check the labelling rule, the bounds VoiceOver is given, and compare the reading
//! order with a golden file in `tests/a11y-snapshots/`.
//!
//! The rules the checks enforce are written out in `docs/developer-guide/testing.md`
//! ("Accessibility of the interface"). Only Slint's debug info lets `ElementHandle` see the
//! elements, so every test file using this one is gated on the `test-support` feature.
#![allow(dead_code)]

use std::cell::Cell;
use std::fmt::Write as _;
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use slint::{ComponentHandle, LogicalSize};

/// The command that rewrites the golden files, named in the header of each one and in failures.
pub const BLESS_COMMAND: &str = "MERCURY_A11Y_BLESS=1 cargo test --release -p mercury-ui --features test-support --test 'a11y_*'";

/// How far two lengths may differ and still count as equal (the layout works in `f32`).
pub const TOLERANCE: f32 = 0.01;

thread_local! {
    static PLATFORM_READY: Cell<bool> = const { Cell::new(false) };
}

/// Installs Slint's testing platform on this test thread, once. Call it first in every test: a
/// test binary can use only one platform, and each test runs on its own thread.
pub fn init() {
    PLATFORM_READY.with(|ready| {
        if !ready.replace(true) {
            i_slint_backend_testing::init_no_event_loop();
        }
    });
}

/// Sets the window's size in logical pixels and shows it, so the layout is computed.
pub fn open_window(component: &impl ComponentHandle, width: f32, height: f32) {
    component.window().set_size(LogicalSize::new(width, height));
    component
        .show()
        .expect("the testing platform shows any window");
}

/// A rectangle in window coordinates, in logical pixels: what an assistive technology is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Whether `inner` lies inside this rectangle (touching the edge counts).
    pub fn contains(&self, inner: &Rect) -> bool {
        inner.x >= self.x - TOLERANCE
            && inner.y >= self.y - TOLERANCE
            && inner.right() <= self.right() + TOLERANCE
            && inner.bottom() <= self.bottom() + TOLERANCE
    }

    /// Whether the two rectangles are the same, up to [`TOLERANCE`].
    pub fn same_as(&self, other: &Rect) -> bool {
        (self.x - other.x).abs() <= TOLERANCE
            && (self.y - other.y).abs() <= TOLERANCE
            && (self.width - other.width).abs() <= TOLERANCE
            && (self.height - other.height).abs() <= TOLERANCE
    }
}

/// The rectangle Slint hands to the assistive technology for an element: the item's origin
/// mapped to the window (`item.map_to_window(item.geometry().origin)`) and the item's size.
pub fn rect_of(handle: &ElementHandle) -> Rect {
    let position = handle.absolute_position();
    let size = handle.size();
    Rect::new(position.x, position.y, size.width, size.height)
}

/// One element an assistive technology can see.
#[derive(Clone)]
pub struct Node {
    pub role: AccessibleRole,
    pub label: String,
    pub description: String,
    pub value: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    /// `accessible-item-index` and `accessible-item-count`, as the properties hold them.
    pub item: Option<(usize, usize)>,
    pub handle: ElementHandle,
}

impl Node {
    /// The role in the words of `.slint` (`button`, `text-input`, ...).
    pub fn role_name(&self) -> &'static str {
        role_name(self.role)
    }

    /// The rectangle an assistive technology is given for this element.
    pub fn rect(&self) -> Rect {
        rect_of(&self.handle)
    }

    /// The `accessible-placeholder-text` (empty when the element has none).
    pub fn placeholder(&self) -> String {
        self.handle
            .accessible_placeholder_text()
            .map(String::from)
            .unwrap_or_default()
    }

    /// What an assistive technology's "press" does.
    pub fn press(&self) {
        self.handle.invoke_accessible_default_action();
    }
}

/// The role in the words of `.slint`.
pub fn role_name(role: AccessibleRole) -> &'static str {
    use AccessibleRole as R;
    match role {
        R::None => "none",
        R::Button => "button",
        R::Checkbox => "checkbox",
        R::Combobox => "combobox",
        R::Groupbox => "groupbox",
        R::Image => "image",
        R::List => "list",
        R::Slider => "slider",
        R::Spinbox => "spinbox",
        R::Tab => "tab",
        R::TabList => "tab-list",
        R::TabPanel => "tab-panel",
        R::Text => "text",
        R::Table => "table",
        R::Tree => "tree",
        R::ProgressIndicator => "progress-indicator",
        R::TextInput => "text-input",
        R::Switch => "switch",
        R::ListItem => "list-item",
        R::RadioButton => "radio-button",
        R::RadioGroup => "radio-group",
        R::Banner => "banner",
        R::Complementary => "complementary",
        R::ContentInfo => "content-info",
        R::Form => "form",
        R::Main => "main",
        R::Navigation => "navigation",
        R::Region => "region",
        R::Search => "search",
        _ => "unknown-role",
    }
}

/// What tells two handles of one item apart from two items: an element that shares its item
/// with an earlier one has the earlier one's role and rectangle and answers no property.
struct Fingerprint {
    role: AccessibleRole,
    rect: Rect,
}

fn node_of_handle(handle: &ElementHandle) -> Option<Node> {
    let role = handle.accessible_role()?;
    if role == AccessibleRole::None {
        return None;
    }
    let item = match (
        handle.accessible_item_index(),
        handle.accessible_item_count(),
    ) {
        (Some(index), Some(count)) => Some((index, count)),
        _ => None,
    };
    let string = |value: Option<slint::SharedString>| value.map(String::from).unwrap_or_default();
    Some(Node {
        role,
        label: string(handle.accessible_label()),
        description: string(handle.accessible_description()),
        value: string(handle.accessible_value()),
        enabled: handle.accessible_enabled().unwrap_or(true),
        checked: handle
            .accessible_checkable()
            .filter(|checkable| *checkable)
            .and_then(|_| handle.accessible_checked()),
        selected: handle
            .accessible_item_selectable()
            .filter(|selectable| *selectable)
            .and_then(|_| handle.accessible_item_selected()),
        item,
        handle: handle.clone(),
    })
}

/// Whether `handle` is another element of the item the previous handle belonged to. With debug
/// info on, which these tests need, Slint keeps every element in an item of its own, so this
/// is a guard rather than something that happens; element index 0 is the one that answers
/// the accessible properties, the others answer none of them.
fn is_extra_element(handle: &ElementHandle, previous: Option<&Fingerprint>) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    let answers_nothing = handle.accessible_label().is_none()
        && handle.accessible_enabled().is_none()
        && handle.accessible_description().is_none()
        && handle.accessible_value().is_none();
    answers_nothing
        && handle.accessible_role() == Some(previous.role)
        && rect_of(handle).same_as(&previous.rect)
}

fn collect(start: &ElementHandle, include_start: bool) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut previous: Option<Fingerprint> = None;
    let mut visit = |handle: ElementHandle| -> ControlFlow<()> {
        let extra = is_extra_element(&handle, previous.as_ref());
        if let Some(role) = handle.accessible_role() {
            previous = Some(Fingerprint {
                role,
                rect: rect_of(&handle),
            });
        }
        if !extra && let Some(node) = node_of_handle(&handle) {
            nodes.push(node);
        }
        ControlFlow::Continue(())
    };
    if include_start {
        let _ = visit(start.clone());
    }
    start.visit_descendants(&mut visit);
    nodes
}

/// Every visible element of the window with a role other than `none`, in item-tree order (the
/// order a screen reader walks them). Elements clipped away or hidden are left out. The window
/// must be shown.
pub fn reading_order(root: &impl ElementRoot) -> Vec<Node> {
    collect(&root.root_element(), false)
}

/// The element itself, then its accessible descendants: for a popup or one component.
pub fn subtree(handle: &ElementHandle) -> Vec<Node> {
    collect(handle, true)
}

/// The node with this role and label among `nodes`; panics unless there is exactly one.
pub fn node_in<'a>(nodes: &'a [Node], role: AccessibleRole, label: &str) -> &'a Node {
    let found: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.role == role && n.label == label)
        .collect();
    match found.as_slice() {
        [one] => one,
        [] => panic!(
            "no {} labelled {label:?}; candidates (same label or same role):\n{}",
            role_name(role),
            candidates(nodes, role, label)
        ),
        many => panic!(
            "{} {} labelled {label:?}, expected exactly one:\n{}",
            many.len(),
            role_name(role),
            dump_refs(many)
        ),
    }
}

/// The element with this role and label among `nodes`; panics unless there is exactly one.
pub fn control_in(nodes: &[Node], role: AccessibleRole, label: &str) -> ElementHandle {
    node_in(nodes, role, label).handle.clone()
}

/// The element of the window with this role and label; panics unless there is exactly one,
/// listing the candidates. The window must be shown.
pub fn control(root: &impl ElementRoot, role: AccessibleRole, label: &str) -> ElementHandle {
    control_in(&reading_order(root), role, label)
}

fn candidates(nodes: &[Node], role: AccessibleRole, label: &str) -> String {
    let near: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.label == label || n.role == role)
        .collect();
    let shown = if near.is_empty() {
        nodes.iter().collect()
    } else {
        near
    };
    let mut text = dump_refs(&shown.iter().take(40).copied().collect::<Vec<_>>());
    if shown.len() > 40 {
        let _ = writeln!(text, "... and {} more", shown.len() - 40);
    }
    text
}

fn dump_refs(nodes: &[&Node]) -> String {
    let mut text = String::new();
    for node in nodes {
        let _ = writeln!(text, "  {}", dump_line(node));
    }
    text
}

fn spaced(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!(" {}", text.replace('\n', "\\n"))
    }
}

fn flags(node: &Node) -> Vec<String> {
    let mut flags = Vec::new();
    if !node.enabled {
        flags.push("disabled".to_string());
    }
    match node.checked {
        Some(true) => flags.push("checked".to_string()),
        Some(false) => flags.push("unchecked".to_string()),
        None => {}
    }
    match node.selected {
        Some(true) => flags.push("selected".to_string()),
        Some(false) => flags.push("unselected".to_string()),
        None => {}
    }
    if let Some((index, count)) = node.item {
        flags.push(format!("item {index} of {count}"));
    }
    if node.handle.accessible_expandable() == Some(true) {
        let expanded = node.handle.accessible_expanded() == Some(true);
        flags.push(if expanded { "expanded" } else { "collapsed" }.to_string());
    }
    if node.handle.accessible_read_only() == Some(true) {
        flags.push("read-only".to_string());
    }
    let placeholder = node.placeholder();
    if !placeholder.is_empty() {
        flags.push(format!("placeholder \"{placeholder}\""));
    }
    if !node.value.is_empty() {
        flags.push(format!("value \"{}\"", node.value));
    }
    flags
}

fn dump_line(node: &Node) -> String {
    format!(
        "{} |{} |{} |{}",
        node.role_name(),
        spaced(&node.label),
        spaced(&node.description),
        spaced(&flags(node).join(", "))
    )
}

/// One line per node: `role | label | description | flags`, a trailing newline after each. No
/// geometry, ids or anything else that changes from run to run. A field that is empty is left
/// empty, so a button without a description reads `button | Back | |`.
pub fn dump(nodes: &[Node]) -> String {
    let mut text = String::new();
    for node in nodes {
        let _ = writeln!(text, "{}", dump_line(node));
    }
    text
}

/// A node the labelling rule lets through, with the reason (never an exemption without one).
pub struct Exempt {
    pub role: AccessibleRole,
    pub label: &'static str,
    pub reason: &'static str,
}

pub const NEEDS_LABEL: &[AccessibleRole] = &[
    AccessibleRole::Button,
    AccessibleRole::Checkbox,
    AccessibleRole::Combobox,
    AccessibleRole::Slider,
    AccessibleRole::Spinbox,
    AccessibleRole::Tab,
    AccessibleRole::ListItem,
    AccessibleRole::RadioButton,
    AccessibleRole::Switch,
    AccessibleRole::List,
    AccessibleRole::TabList,
    AccessibleRole::Groupbox,
    AccessibleRole::Search,
    AccessibleRole::Navigation,
    AccessibleRole::Main,
    AccessibleRole::Region,
];

/// The rule broken by each node, as one line each (empty when the nodes are fine).
fn labelling_violations(nodes: &[Node], exemptions: &[Exempt]) -> Vec<String> {
    for exemption in exemptions {
        assert!(!exemption.reason.is_empty(), "an exemption needs a reason");
    }
    let mut used = vec![false; exemptions.len()];
    let mut problems = Vec::new();
    let mut seen: Vec<(AccessibleRole, &str)> = Vec::new();
    for node in nodes {
        if let Some(position) = exemptions
            .iter()
            .position(|e| e.role == node.role && e.label == node.label)
        {
            used[position] = true;
            continue;
        }
        let line = dump_line(node);
        if NEEDS_LABEL.contains(&node.role) && node.label.is_empty() {
            problems.push(format!("has no label: {line}"));
        }
        if node.role == AccessibleRole::TextInput
            && node.label.is_empty()
            && node.placeholder().is_empty()
        {
            problems.push(format!(
                "a text field with neither label nor placeholder: {line}"
            ));
        }
        if node.role == AccessibleRole::Image {
            problems.push(format!(
                "an image is exposed (hide a decorative one with `accessible-role: none`): {line}"
            ));
        }
        if !node.label.is_empty() && !node.label.chars().any(char::is_alphanumeric) {
            problems.push(format!("the label is only a glyph or punctuation: {line}"));
        }
        // Static text may repeat; controls and landmarks may not (rows and tabs have an index).
        if node.item.is_none() && node.role != AccessibleRole::Text && !node.label.is_empty() {
            if seen.contains(&(node.role, node.label.as_str())) {
                problems.push(format!(
                    "another {} has the same label: {line}",
                    node.role_name()
                ));
            } else {
                seen.push((node.role, node.label.as_str()));
            }
        }
    }
    for (exemption, used) in exemptions.iter().zip(used) {
        if !used {
            problems.push(format!(
                "the exemption for {} {:?} ({}) matches nothing: remove it",
                role_name(exemption.role),
                exemption.label,
                exemption.reason
            ));
        }
    }
    problems
}

/// Fails unless the nodes follow the labelling rule: a control or landmark has a label (a text
/// field a label or a placeholder), no meaningful image is exposed, no label is only a glyph, and
/// no two controls without an item index share role and label. Static text (`text`) may repeat.
/// `what` names the state under test in the failure.
pub fn assert_labelled(nodes: &[Node], what: &str) {
    assert_labelled_except(nodes, what, &[]);
}

/// [`assert_labelled`] with a short list of nodes let through, each with its reason. An
/// exemption that matches no node fails, so the list cannot go stale.
pub fn assert_labelled_except(nodes: &[Node], what: &str, exemptions: &[Exempt]) {
    let problems = labelling_violations(nodes, exemptions);
    assert!(
        problems.is_empty(),
        "{what}: the labelling rule is broken:\n  {}\nreading order:\n{}",
        problems.join("\n  "),
        dump(nodes)
    );
}

/// The lines every golden file starts with: the two SPDX lines and the bless command.
fn snapshot_header() -> String {
    format!(
        "# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0\n\
         # SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors\n\
         # Reading order of the interface, one element per line: role | label | description | flags.\n\
         # Rewrite with: {BLESS_COMMAND}\n"
    )
}

/// The golden file's text without its leading `# ` comment lines.
fn without_header(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut lines = text.lines().peekable();
    while lines
        .peek()
        .is_some_and(|line| *line == "#" || line.starts_with("# "))
    {
        lines.next();
    }
    let mut body = String::new();
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    body
}

/// The folder of the golden files, `tests/a11y-snapshots/` of `mercury-ui`.
pub fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("a11y-snapshots")
}

/// A line diff of two texts: `-` a line only in the golden file, `+` one only in the dump.
fn line_diff(expected: &str, actual: &str) -> String {
    let old: Vec<&str> = expected.lines().collect();
    let new: Vec<&str> = actual.lines().collect();
    // longest common subsequence of the lines, from the end
    let mut common = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut diff = String::new();
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            let _ = writeln!(diff, "  {}", old[i]);
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || common[i][j + 1] >= common[i + 1][j]) {
            let _ = writeln!(diff, "+ {}", new[j]);
            j += 1;
        } else {
            let _ = writeln!(diff, "- {}", old[i]);
            i += 1;
        }
    }
    diff
}

/// Compares `actual` with `dir/<name>.txt` (or, with `bless`, rewrites that file). The leading
/// `# ` lines of the file are ignored when comparing and written when blessing. The test
/// helpers call this through [`assert_snapshot`]; it takes the folder and the bless switch so the
/// helper's own test can run in a temporary folder.
pub fn check_snapshot(dir: &Path, name: &str, actual: &str, bless: bool) {
    assert!(
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "a snapshot name is lower-case words joined by hyphens, got {name:?}"
    );
    let path = dir.join(format!("{name}.txt"));
    if bless {
        fs::create_dir_all(dir).expect("create the snapshot folder");
        fs::write(&path, format!("{}{actual}", snapshot_header())).expect("write the golden file");
        eprintln!("blessed {}", path.display());
        return;
    }
    let Ok(text) = fs::read_to_string(&path) else {
        panic!(
            "no golden file {}.\nCreate it with: {BLESS_COMMAND}\nand read it as a VoiceOver user would. It would hold:\n{actual}",
            path.display()
        );
    };
    let expected = without_header(&text);
    assert!(
        expected == actual,
        "the reading order of {name} changed (- golden file, + now), {}.\nIf the change is wanted, read the new order as a VoiceOver user would, then bless it with: {BLESS_COMMAND}\n{}",
        path.display(),
        line_diff(&expected, actual)
    );
}

/// Compares the reading order with `tests/a11y-snapshots/<name>.txt`. With `MERCURY_A11Y_BLESS=1`
/// it rewrites the file instead. A missing file fails and says how to create it.
pub fn assert_snapshot(name: &str, nodes: &[Node]) {
    let bless = std::env::var("MERCURY_A11Y_BLESS").is_ok_and(|value| value == "1");
    check_snapshot(&snapshot_dir(), name, &dump(nodes), bless);
}

/// Bounds problems, one line each: a node with no area, or one outside the window.
fn bounds_violations(nodes: &[Node], window_size: (f32, f32)) -> Vec<String> {
    let window = Rect::new(0.0, 0.0, window_size.0, window_size.1);
    let mut problems = Vec::new();
    for node in nodes {
        let rect = node.rect();
        if rect.width <= 0.0 || rect.height <= 0.0 {
            problems.push(format!(
                "no area ({} x {}): {}",
                rect.width,
                rect.height,
                dump_line(node)
            ));
        } else if !window.contains(&rect) {
            problems.push(format!(
                "outside the {} x {} window (x {}, y {}, {} x {}): {}",
                window_size.0,
                window_size.1,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                dump_line(node)
            ));
        }
    }
    problems
}

/// Fails unless every node has a positive size and lies inside the window: the rectangle
/// VoiceOver draws its cursor in and hit-tests with. Filter out a node that may legitimately
/// have none before calling.
pub fn assert_bounds(nodes: &[Node], window_size: (f32, f32)) {
    let problems = bounds_violations(nodes, window_size);
    assert!(
        problems.is_empty(),
        "the bounds given to assistive technology are wrong:\n  {}",
        problems.join("\n  ")
    );
}

/// The window's size in logical pixels, for [`assert_bounds`].
pub fn window_size(component: &impl ComponentHandle) -> (f32, f32) {
    let size = component
        .window()
        .size()
        .to_logical(component.window().scale_factor());
    (size.width, size.height)
}

/// Fails unless `inner` lies inside `outer` (a control inside its landmark or overlay).
pub fn assert_inside(inner: &Node, outer: &Node) {
    assert!(
        outer.rect().contains(&inner.rect()),
        "{} is not inside {}: {:?} is not inside {:?}",
        dump_line(inner),
        dump_line(outer),
        inner.rect(),
        outer.rect()
    );
}

/// Fails unless the node's rectangle is exactly `expected` (up to [`TOLERANCE`]).
pub fn assert_rect(node: &Node, expected: Rect) {
    assert!(
        node.rect().same_as(&expected),
        "{} is at {:?}, expected {:?}",
        dump_line(node),
        node.rect(),
        expected
    );
}
