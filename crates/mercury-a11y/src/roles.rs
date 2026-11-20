// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Roles: the ARIA role list and the HTML-AAM element roles as AccessKit roles.
//!
//! Owner: item 2 (`core-names`). Written from the W3C tables (WAI-ARIA 1.2 section 5.4 and the
//! DPub and Graphics modules, HTML-AAM section 4) and checked against the list of Chromium's
//! `ax::mojom::Role` that AccessKit's `Role` follows; no code is copied.
//!
//! # How a role is decided
//!
//! 1. A text node is a [`Role::TextRun`].
//! 2. The `role` attribute is a token list: the first token that names a role (ARIA 1.2 concrete
//!    roles, DPub `doc-*`, Graphics `graphics-*` and the ARIA 1.3 alias `image`) wins. Abstract roles
//!    (`widget`, `landmark`, ...) and unknown tokens are skipped, as the specification says.
//!    `none` and `presentation` lose to the **presentational roles conflict resolution**: on an
//!    element that is focusable or carries a global ARIA attribute the role is ignored and the
//!    implicit role applies (the same rule `accname::role` applies, so names and roles agree).
//! 3. Otherwise the implicit role of the element: `accname::implicit` (HTML-AAM with its context
//!    rules: `header` and `footer` scope, `img alt=""`, `a` without `href`, `select` size, `th`
//!    scope) with Mercury's corrections below. `input` is decided here, from `type`.
//! 4. The ARIA name is mapped to an AccessKit role through [`ARIA_ROLES`], and three roles are
//!    refined with what only the host knows: `textbox` (multi-line), `combobox` (editable) and
//!    `option` (a pop-up menu item or a list box option). A `div` or other element with no role
//!    that is an editing host (`contenteditable`) is a text input.
//!
//! # Where Mercury differs from `accname::implicit`
//!
//! * `input` is read from its `type` here: a `password` is [`Role::PasswordInput`] (`accname`
//!   answers no role; the value of a password never leaves the page, see `HostState::control_value`),
//!   `email`, `tel` and `url` are their own text input roles, `color`, `date`, `datetime-local`,
//!   `month`, `week` and `time` are [`Role::ColorWell`] and the date and time inputs, `file` is a
//!   button (as in Chromium), an unknown `type` is a text input (as in HTML), and `hidden` is
//!   not exposed.
//! * `form` and `section` are landmarks only when they have an accessible name from `aria-label`,
//!   `aria-labelledby` or `title` that is not blank (VoiceOver lists every landmark; an unnamed one
//!   is noise). `accname` tests only that the attribute exists. `aria-labelledby` is not resolved
//!   here (the signature has no id index), so a `labelledby` that names nothing still makes a region.
//! * `header` and `footer` are `banner` and `contentinfo` only outside `article`, `aside`, `main`,
//!   `nav` and `section` **and outside elements with the role `article`, `complementary`, `main`,
//!   `navigation` or `region`** (`accname` knows the elements only, and not `main`).
//! * `html` is not exposed as a `document` (HTML-AAM says so; browsers expose the document itself as
//!   the root web area, and a second `document` node inside it would be announced twice). The layout
//!   glue sets [`Role::RootWebArea`] where the tree's root is; today Servo does that for `body`.
//! * A child of a list or table whose role is `none` or `presentation` is presentational as well
//!   (`li`, `dt`, `dd`, `tr`, `td`, `th`, `thead`, `tbody`, `tfoot`; ARIA 1.2 section 5.4 "presentational
//!   roles inheritance").
//! * `td` is a `gridcell` inside a table whose role is `grid` or `treegrid`.
//! * `img alt=""` is presentational only when it has no `title`, no global ARIA attribute and no
//!   `tabindex` (HTML-AAM); otherwise an image.
//! * `summary` is a disclosure triangle when it is the first `summary` of a `details`, `details` a
//!   [`Role::Details`] (both as in Chromium); `dl` a description list; `figcaption` a figure
//!   caption; `legend` a legend; `address` and `hgroup` groups; `audio`, `video`, `canvas`, `iframe`,
//!   `embed`/`object`, `svg` and `math` their media or embedded roles.
//!
//! # Choices where AccessKit has no role or its macOS mapping would be noise
//!
//! * `subscript`, `superscript`, `none`, `presentation` and `generic` are [`Role::GenericContainer`]:
//!   AccessKit has no role for the first two (Chromium keeps the position as a text attribute, a
//!   follow-up with the text work), and the consumer prunes a generic container from the tree it
//!   hands to VoiceOver while keeping its children.
//! * `directory` (deprecated in ARIA 1.2) is a [`Role::List`], its nearest role.
//! * Nothing is mapped to a role that macOS shows as `AXUnknown` (`Pane`, `Keyboard`, `ImeCandidate`,
//!   `RubyAnnotation`, `ListGrid`...): such a node would appear in VoiceOver as an unknown element.
//!   `kbd`, `ruby`, `rt`, `pre`, `br`, `abbr`, `q` and the other elements with no role in HTML-AAM
//!   are generic containers. `label` is generic as well: [`Role::Label`] makes AccessKit read the
//!   text from the node's *value* (which a `<label>` element does not have), so a labelled control
//!   gets its name from `names` instead.
//! * `Role::Header` and `Role::Footer` are never used: macOS shows them as banner and content-info
//!   landmarks, which is what a `header` outside sectioning content already is.
//!
//! # Known gaps
//!
//! Role tokens are matched case-sensitively, as `accname` does (`role=Button` is ignored, a
//! difference from Chromium). `comment` and `suggestion` (ARIA 1.3 draft) are not roles. The
//! required context of ARIA roles (a `listitem` outside a `list`) is not enforced: the role is
//! exposed as written, as browsers do. The layout-table heuristic (`LayoutTable`) and the `th` scope
//! guess for tables without `scope` belong to the table work (EE9).

use a11y_dom::{Node, NodeKind, Rendering, ancestors};
use accesskit::Role;

use crate::host::HostState;

/// One source of truth for the ARIA role names: the table [`ARIA_ROLES`] (in order: ARIA 1.2, the
/// alias `image`, DPub, Graphics) and the lookup [`role_for_aria`] (a `match`, so a lookup is a
/// string comparison and no scan).
macro_rules! aria_roles {
    ($($name:literal => $role:expr),* $(,)?) => {
        /// Every ARIA role name Mercury exposes, with its AccessKit role. The first entry of an
        /// AccessKit role is its canonical name (see [`aria_name_of`]).
        pub const ARIA_ROLES: &[(&str, Role)] = &[$(($name, $role)),*];

        /// The ARIA role name (or the entry of the table) with its static name, for a token.
        fn lookup(token: &str) -> Option<(&'static str, Role)> {
            match token {
                $($name => Some(($name, $role)),)*
                _ => None,
            }
        }
    };
}

aria_roles! {
    // WAI-ARIA 1.2, concrete roles.
    "alert" => Role::Alert,
    "alertdialog" => Role::AlertDialog,
    "application" => Role::Application,
    "article" => Role::Article,
    "banner" => Role::Banner,
    "blockquote" => Role::Blockquote,
    "button" => Role::Button,
    "caption" => Role::Caption,
    "cell" => Role::Cell,
    "checkbox" => Role::CheckBox,
    "code" => Role::Code,
    "columnheader" => Role::ColumnHeader,
    "combobox" => Role::ComboBox,
    "complementary" => Role::Complementary,
    "contentinfo" => Role::ContentInfo,
    "definition" => Role::Definition,
    "deletion" => Role::ContentDeletion,
    "dialog" => Role::Dialog,
    "document" => Role::Document,
    "emphasis" => Role::Emphasis,
    "feed" => Role::Feed,
    "figure" => Role::Figure,
    "form" => Role::Form,
    "generic" => Role::GenericContainer,
    "grid" => Role::Grid,
    "gridcell" => Role::GridCell,
    "group" => Role::Group,
    "heading" => Role::Heading,
    "img" => Role::Image,
    "insertion" => Role::ContentInsertion,
    "link" => Role::Link,
    "list" => Role::List,
    "listbox" => Role::ListBox,
    "listitem" => Role::ListItem,
    "log" => Role::Log,
    "main" => Role::Main,
    "mark" => Role::Mark,
    "marquee" => Role::Marquee,
    "math" => Role::Math,
    "menu" => Role::Menu,
    "menubar" => Role::MenuBar,
    "menuitem" => Role::MenuItem,
    "menuitemcheckbox" => Role::MenuItemCheckBox,
    "menuitemradio" => Role::MenuItemRadio,
    "meter" => Role::Meter,
    "navigation" => Role::Navigation,
    "none" => Role::GenericContainer,
    "note" => Role::Note,
    "option" => Role::ListBoxOption,
    "paragraph" => Role::Paragraph,
    "presentation" => Role::GenericContainer,
    "progressbar" => Role::ProgressIndicator,
    "radio" => Role::RadioButton,
    "radiogroup" => Role::RadioGroup,
    "region" => Role::Region,
    "row" => Role::Row,
    "rowgroup" => Role::RowGroup,
    "rowheader" => Role::RowHeader,
    "scrollbar" => Role::ScrollBar,
    "search" => Role::Search,
    "searchbox" => Role::SearchInput,
    "separator" => Role::Splitter,
    "slider" => Role::Slider,
    "spinbutton" => Role::SpinButton,
    "status" => Role::Status,
    "strong" => Role::Strong,
    "subscript" => Role::GenericContainer,
    "superscript" => Role::GenericContainer,
    "switch" => Role::Switch,
    "tab" => Role::Tab,
    "table" => Role::Table,
    "tablist" => Role::TabList,
    "tabpanel" => Role::TabPanel,
    "term" => Role::Term,
    "textbox" => Role::TextInput,
    "time" => Role::Time,
    "timer" => Role::Timer,
    "toolbar" => Role::Toolbar,
    "tooltip" => Role::Tooltip,
    "tree" => Role::Tree,
    "treegrid" => Role::TreeGrid,
    "treeitem" => Role::TreeItem,
    // `directory` is deprecated in ARIA 1.2; its nearest role is a list.
    "directory" => Role::List,
    // ARIA 1.3 draft: a synonym of `img` that browsers already accept.
    "image" => Role::Image,
    // Digital Publishing WAI-ARIA Module 1.1.
    "doc-abstract" => Role::DocAbstract,
    "doc-acknowledgments" => Role::DocAcknowledgements,
    "doc-afterword" => Role::DocAfterword,
    "doc-appendix" => Role::DocAppendix,
    "doc-backlink" => Role::DocBackLink,
    "doc-biblioentry" => Role::DocBiblioEntry,
    "doc-bibliography" => Role::DocBibliography,
    "doc-biblioref" => Role::DocBiblioRef,
    "doc-chapter" => Role::DocChapter,
    "doc-colophon" => Role::DocColophon,
    "doc-conclusion" => Role::DocConclusion,
    "doc-cover" => Role::DocCover,
    "doc-credit" => Role::DocCredit,
    "doc-credits" => Role::DocCredits,
    "doc-dedication" => Role::DocDedication,
    "doc-endnote" => Role::DocEndnote,
    "doc-endnotes" => Role::DocEndnotes,
    "doc-epigraph" => Role::DocEpigraph,
    "doc-epilogue" => Role::DocEpilogue,
    "doc-errata" => Role::DocErrata,
    "doc-example" => Role::DocExample,
    "doc-footnote" => Role::DocFootnote,
    "doc-foreword" => Role::DocForeword,
    "doc-glossary" => Role::DocGlossary,
    "doc-glossref" => Role::DocGlossRef,
    "doc-index" => Role::DocIndex,
    "doc-introduction" => Role::DocIntroduction,
    "doc-noteref" => Role::DocNoteRef,
    "doc-notice" => Role::DocNotice,
    "doc-pagebreak" => Role::DocPageBreak,
    "doc-pagefooter" => Role::DocPageFooter,
    "doc-pageheader" => Role::DocPageHeader,
    "doc-pagelist" => Role::DocPageList,
    "doc-part" => Role::DocPart,
    "doc-preface" => Role::DocPreface,
    "doc-prologue" => Role::DocPrologue,
    "doc-pullquote" => Role::DocPullquote,
    "doc-qna" => Role::DocQna,
    "doc-subtitle" => Role::DocSubtitle,
    "doc-tip" => Role::DocTip,
    "doc-toc" => Role::DocToc,
    // Graphics Module 1.0.
    "graphics-document" => Role::GraphicsDocument,
    "graphics-object" => Role::GraphicsObject,
    "graphics-symbol" => Role::GraphicsSymbol,
}

/// The AccessKit role for an ARIA role name, with no refinement by element (`textbox` is
/// [`Role::TextInput`], `option` a [`Role::ListBoxOption`]). `None` for a name that is not a role
/// authors may use (an abstract role, or a misspelling).
pub fn role_for_aria(name: &str) -> Option<Role> {
    lookup(name).map(|(_, role)| role)
}

/// The canonical ARIA role name of an AccessKit role: the first name in [`ARIA_ROLES`] that maps to
/// it (`GenericContainer` is `generic`, `Image` is `img`, `List` is `list`). `None` for roles that
/// have no ARIA name (`TextRun`, `Label`, `PasswordInput`, `DateInput`, ...).
pub fn aria_name_of(role: Role) -> Option<&'static str> {
    ARIA_ROLES
        .iter()
        .find(|(_, candidate)| *candidate == role)
        .map(|(name, _)| *name)
}

/// Whether an accessible name is prohibited for this role (ARIA 1.2 and `accname`'s list: `generic`,
/// `paragraph`, `presentation`, `caption`, `code`, `emphasis`, `strong`, `insertion`, `deletion`,
/// `subscript`, `superscript`, `term`, `time`). Derived from [`ARIA_ROLES`] and
/// `accname::name_is_prohibited`, so it can never disagree with the names `accname` refuses to
/// compute.
pub fn name_prohibited(role: Role) -> bool {
    ARIA_ROLES
        .iter()
        .any(|(name, candidate)| *candidate == role && accname::name_is_prohibited(name))
}

/// What the rules decided, before the refinement by the host's knowledge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Computed {
    /// An ARIA role name from [`ARIA_ROLES`].
    Aria(&'static str),
    /// An AccessKit role with no ARIA name (or a more specific one than the name gives).
    Native(Role),
}

/// The AccessKit role of an element or text node.
pub fn role<'n, D>(doc: &'n D, node: D::N<'n>) -> Role
where
    D: Rendering + HostState,
{
    if node.kind() == NodeKind::Text {
        return Role::TextRun;
    }
    refine(doc, node, computed(doc, node))
}

/// The ARIA role name of an element, or `None` for a node that has none (a text node, or an element
/// whose role has no ARIA name, such as an `input type=password` or an `iframe`). A generic
/// container answers `generic`. Used by `names` and `states` to decide what applies to a role.
pub fn aria_role<'n, D>(doc: &'n D, node: D::N<'n>) -> Option<&'static str>
where
    D: Rendering + HostState,
{
    if node.kind() == NodeKind::Text {
        return None;
    }
    match computed(doc, node) {
        Computed::Aria(name) => Some(name),
        Computed::Native(Role::GenericContainer) => Some("generic"),
        Computed::Native(_) => None,
    }
}

fn computed<'n, D>(doc: &'n D, node: D::N<'n>) -> Computed
where
    D: Rendering + HostState,
{
    if let Some((name, _)) = explicit_token(node) {
        let conflict = matches!(name, "none" | "presentation") && ignores_presentation(doc, node);
        if !conflict {
            return Computed::Aria(name);
        }
    }
    implicit(doc, node)
}

/// The first token of the `role` attribute that names a role.
fn explicit_token<'a, N: Node<'a>>(node: N) -> Option<(&'static str, Role)> {
    node.attr("role")?.split_whitespace().find_map(lookup)
}

/// Global states and properties of WAI-ARIA 1.2 (the deprecated ones included, `aria-hidden`
/// excluded) and `aria-description`: any of them on an element makes the user agent ignore a
/// presentational role. The mere presence counts, with an empty value too.
const GLOBAL_ARIA: &[&str] = &[
    "aria-atomic",
    "aria-busy",
    "aria-controls",
    "aria-current",
    "aria-describedby",
    "aria-description",
    "aria-details",
    "aria-disabled",
    "aria-dropeffect",
    "aria-errormessage",
    "aria-flowto",
    "aria-grabbed",
    "aria-haspopup",
    "aria-invalid",
    "aria-keyshortcuts",
    "aria-label",
    "aria-labelledby",
    "aria-live",
    "aria-owns",
    "aria-relevant",
    "aria-roledescription",
];

fn has_global_aria<'a, N: Node<'a>>(node: N) -> bool {
    GLOBAL_ARIA.iter().any(|name| node.has_attr(name))
}

/// Whether the element is focusable by what the markup shows (a control that is not disabled, a link
/// with `href`, a parsable `tabindex`, `contenteditable`), or by what the host says.
fn is_focusable<'n, D>(doc: &'n D, node: D::N<'n>) -> bool
where
    D: HostState,
{
    if doc.native_flags(node).focusable {
        return true;
    }
    let tag = node.local_name();
    let control = matches!(tag, "button" | "input" | "select" | "textarea");
    if control
        && (node.has_attr("disabled")
            || ancestors(node).any(|a| a.is_element("fieldset") && a.has_attr("disabled")))
    {
        return false;
    }
    (control && input_type(node) != "hidden")
        || (matches!(tag, "a" | "area") && node.has_attr("href"))
        || node
            .attr("tabindex")
            .is_some_and(|t| t.trim().parse::<i32>().is_ok())
        || node
            .attr("contenteditable")
            .is_some_and(|v| !v.trim().eq_ignore_ascii_case("false"))
}

/// The presentational roles conflict resolution: a focusable element or one with a global ARIA
/// attribute ignores `none` and `presentation`.
fn ignores_presentation<'n, D>(doc: &'n D, node: D::N<'n>) -> bool
where
    D: HostState,
{
    is_focusable(doc, node) || has_global_aria(node)
}

/// Whether the element has an effective `none` or `presentation` role of its own (the author wrote
/// it and no conflict cancels it).
fn is_presentational<'n, D>(doc: &'n D, node: D::N<'n>) -> bool
where
    D: HostState,
{
    explicit_token(node).is_some_and(|(name, _)| matches!(name, "none" | "presentation"))
        && !ignores_presentation(doc, node)
}

fn non_blank(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

/// Whether the element has an accessible name that the markup alone shows: a non-blank `aria-label`,
/// `aria-labelledby` or `title`.
fn has_markup_name<'a, N: Node<'a>>(node: N) -> bool {
    non_blank(node.attr("aria-label"))
        || non_blank(node.attr("aria-labelledby"))
        || non_blank(node.attr("title"))
}

fn input_type<'a, N: Node<'a>>(node: N) -> String {
    node.attr("type")
        .map(|t| t.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// `header` or `footer` is scoped to the page (a landmark) only outside sectioning content.
fn in_sectioning_content<'a, N: Node<'a>>(node: N) -> bool {
    ancestors(node).any(|a| {
        matches!(
            a.local_name(),
            "article" | "aside" | "main" | "nav" | "section"
        ) || explicit_token(a).is_some_and(|(name, _)| {
            matches!(
                name,
                "article" | "complementary" | "main" | "navigation" | "region"
            )
        })
    })
}

/// The implicit role (HTML-AAM) with Mercury's corrections (module documentation).
fn implicit<'n, D>(doc: &'n D, node: D::N<'n>) -> Computed
where
    D: Rendering + HostState,
{
    use Computed::{Aria, Native};
    let tag = node.local_name();
    let from_html = || match accname::implicit(node) {
        Some(name) => Aria(name),
        None => Native(Role::GenericContainer),
    };
    let base = match tag {
        "input" => return input_role(node),
        "html" => Native(Role::GenericContainer),
        "form" => {
            if has_markup_name(node) {
                Aria("form")
            } else {
                Aria("generic")
            }
        }
        "section" => {
            if has_markup_name(node) {
                Aria("region")
            } else {
                Aria("generic")
            }
        }
        "header" | "footer" => {
            if in_sectioning_content(node) {
                Aria("generic")
            } else if tag == "header" {
                Aria("banner")
            } else {
                Aria("contentinfo")
            }
        }
        "img" => {
            let decorative = node.attr("alt") == Some("")
                && !non_blank(node.attr("title"))
                && !has_global_aria(node)
                && !node.has_attr("tabindex");
            if decorative {
                Aria("presentation")
            } else {
                Aria("img")
            }
        }
        "td" => match from_html() {
            Aria("cell") if in_grid(doc, node) => Aria("gridcell"),
            other => other,
        },
        "address" | "hgroup" => Aria("group"),
        "dl" => Native(Role::DescriptionList),
        "details" => Native(Role::Details),
        "summary" => {
            if is_first_summary(node) {
                Native(Role::DisclosureTriangle)
            } else {
                Aria("generic")
            }
        }
        "figcaption" => Native(Role::FigureCaption),
        "legend" => Native(Role::Legend),
        "audio" => Native(Role::Audio),
        "video" => Native(Role::Video),
        "canvas" => Native(Role::Canvas),
        "iframe" => Native(Role::Iframe),
        "embed" | "object" => Native(Role::EmbeddedObject),
        "svg" => Native(Role::SvgRoot),
        "math" => Aria("math"),
        _ => from_html(),
    };
    if inherits_presentational(doc, node) {
        return Aria("presentation");
    }
    base
}

fn is_first_summary<'a, N: Node<'a>>(node: N) -> bool {
    node.parent().is_some_and(|details| {
        details.is_element("details")
            && details.children().find(|c| c.is_element("summary")) == Some(node)
    })
}

/// A `td` is a grid cell inside the nearest `table` whose role is `grid` or `treegrid`.
fn in_grid<'n, D>(doc: &'n D, node: D::N<'n>) -> bool
where
    D: Rendering + HostState,
{
    ancestors(node)
        .find(|a| a.is_element("table"))
        .is_some_and(|table| {
            matches!(
                explicit_token(table).map(|(name, _)| name),
                Some("grid" | "treegrid")
            ) && !is_presentational(doc, table)
        })
}

/// ARIA 1.2 "presentational roles inheritance": the structural children of a list or table whose role
/// is `none` or `presentation` are presentational as well.
fn inherits_presentational<'n, D>(doc: &'n D, node: D::N<'n>) -> bool
where
    D: HostState,
{
    let (parents, climb): (&[&str], bool) = match node.local_name() {
        "li" => (&["ul", "ol", "menu"], false),
        "dt" | "dd" => (&["dl"], false),
        "tr" | "thead" | "tbody" | "tfoot" => (&["table", "thead", "tbody", "tfoot"], true),
        "td" | "th" => (&["tr", "thead", "tbody", "tfoot", "table"], true),
        _ => return false,
    };
    for ancestor in ancestors(node) {
        if !parents.contains(&ancestor.local_name()) {
            return false;
        }
        if is_presentational(doc, ancestor) {
            return true;
        }
        if !climb {
            return false;
        }
    }
    false
}

/// The role of an `input`, from its `type` (module documentation).
fn input_role<'a, N: Node<'a>>(node: N) -> Computed {
    use Computed::{Aria, Native};
    let combobox_if_list = |plain: Computed| {
        if node.has_attr("list") {
            Aria("combobox")
        } else {
            plain
        }
    };
    match input_type(node).as_str() {
        "button" | "submit" | "reset" | "image" | "file" => Aria("button"),
        "checkbox" => Aria("checkbox"),
        "radio" => Aria("radio"),
        "range" => Aria("slider"),
        "number" => Aria("spinbutton"),
        "color" => Native(Role::ColorWell),
        "date" => Native(Role::DateInput),
        "datetime-local" => Native(Role::DateTimeInput),
        "month" => Native(Role::MonthInput),
        "week" => Native(Role::WeekInput),
        "time" => Native(Role::TimeInput),
        "password" => Native(Role::PasswordInput),
        "hidden" => Native(Role::GenericContainer),
        "search" => combobox_if_list(Aria("searchbox")),
        "email" => combobox_if_list(Native(Role::EmailInput)),
        "tel" => combobox_if_list(Native(Role::PhoneNumberInput)),
        "url" => combobox_if_list(Native(Role::UrlInput)),
        // `text` and every value HTML does not know (its invalid value default is `text`).
        _ => combobox_if_list(Aria("textbox")),
    }
}

/// The AccessKit role for what was decided, refined with what the host knows.
fn refine<'n, D>(doc: &'n D, node: D::N<'n>, computed: Computed) -> Role
where
    D: Rendering + HostState,
{
    let name = match computed {
        Computed::Native(role) => return role,
        Computed::Aria(name) => name,
    };
    let tag = node.local_name();
    let table = role_for_aria(name).unwrap_or(Role::GenericContainer);
    match name {
        "textbox" => {
            let multiline = match tag {
                "input" => false,
                "textarea" => true,
                _ => {
                    node.attr("aria-multiline") == Some("true") || doc.native_flags(node).multiline
                }
            };
            if multiline {
                Role::MultilineTextInput
            } else {
                Role::TextInput
            }
        }
        "combobox" => {
            // `select` is a pop-up button on macOS; a text field with a list (and a contenteditable
            // host with the role) is the editable combo box.
            let editable = match tag {
                "select" => false,
                "input" | "textarea" => true,
                _ => doc.native_flags(node).editable,
            };
            if editable {
                Role::EditableComboBox
            } else {
                Role::ComboBox
            }
        }
        "option" => {
            // An `option` of a drop-down `select` is an item of a pop-up menu; elsewhere a list box option.
            let in_drop_down = node.is_element("option")
                && ancestors(node)
                    .find(|a| a.is_element("select"))
                    .is_some_and(|select| {
                        !select.has_attr("multiple")
                            && !select
                                .attr("size")
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .is_some_and(|s| s > 1)
                    });
            if in_drop_down {
                Role::MenuListOption
            } else {
                Role::ListBoxOption
            }
        }
        "generic" => {
            // An editing host with no role of its own is a text field (Chromium and Safari both).
            let flags = doc.native_flags(node);
            if flags.editable && !matches!(tag, "input" | "textarea") {
                if flags.multiline {
                    Role::MultilineTextInput
                } else {
                    Role::TextInput
                }
            } else {
                table
            }
        }
        _ => table,
    }
}
