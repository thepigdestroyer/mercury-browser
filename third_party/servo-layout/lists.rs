/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use layout_api::{LayoutDamage, LayoutElement, LayoutElementType, LayoutNode, LayoutNodeType};
use script::layout_dom::{ServoLayoutElement, ServoLayoutNode};
use style::counter_style::{CounterStyle, Symbol, SymbolsType};
use style::dom::{NodeInfo, TElement};
use style::properties::ComputedValues;
use style::properties::longhands::list_style_type::computed_value::T as ListStyleType;
use style::selector_parser::PseudoElement;
use style::values::computed::Image;
use style::values::generics::counters::{Content, ContentItem, CounterPair};
use web_atoms::{local_name, ns};

use crate::context::LayoutContext;
// Mercury: ED5 (lane txt-1). Counter values and the text of counter styles come from the two
// new files `counters.rs` and `counter_styles.rs`.
use crate::counter_styles::{CounterSystem, SymbolsKind, anonymous, generate, predefined};
use crate::counters::{
    CounterKey, CounterOp, CounterPass, CounterTable, CounterUse, ElementInput, HtmlHint,
    PseudoKind, parse_html_integer,
};
use crate::dom_traversal::{
    NodeAndStyleInfo, PseudoElementContentItem, generate_pseudo_element_content,
};
use crate::replaced::ReplacedContents;
use crate::style_ext::Display;

/// <https://drafts.csswg.org/css-lists/#content-property>
pub(crate) fn make_marker<'dom>(
    context: &LayoutContext,
    info: &NodeAndStyleInfo<'dom>,
) -> Option<(NodeAndStyleInfo<'dom>, Vec<PseudoElementContentItem>)> {
    let marker_info =
        info.with_pseudo_element(context, style::selector_parser::PseudoElement::Marker)?;
    let style = &marker_info.style;
    let list_style = style.get_list();

    // https://drafts.csswg.org/css-lists/#marker-image
    let marker_image = || match &list_style.list_style_image {
        Image::Url(url) => Some(vec![
            PseudoElementContentItem::Replaced(ReplacedContents::from_image_url(
                marker_info.node,
                context,
                url,
            )?),
            PseudoElementContentItem::Text(" ".into()),
        ]),
        // XXX: Non-None image types unimplemented.
        Image::ImageSet(..) |
        Image::Gradient(..) |
        Image::Image(..) |
        Image::CrossFade(..) |
        Image::PaintWorklet(..) |
        Image::None => None,
        Image::LightDark(..) => unreachable!("light-dark() should be disabled"),
    };

    let content = match &marker_info.style.get_counters().content {
        Content::Items(_) => generate_pseudo_element_content(&marker_info, context),
        Content::None => return None,
        Content::Normal => marker_image().or_else(|| {
            Some(vec![PseudoElementContentItem::Text(marker_string(
                &list_style.list_style_type,
                // Mercury: ED5. The number of the list item (was: a value of 0 for every item).
                list_item_number(context, info),
            )?)])
        })?,
    };

    Some((marker_info, content))
}

fn symbol_to_string(symbol: &Symbol) -> &str {
    match symbol {
        Symbol::String(string) => string,
        Symbol::Ident(ident) => &ident.0,
    }
}

/// Mercury: ED5. Runs `f` on the counter style that `counter_style` names. A name that is not a
/// predefined style (`@counter-style` is not supported) is written in `decimal`, as the
/// specification asks for a style that does not exist.
/// <https://drafts.csswg.org/css-counter-styles-3/#generate-a-counter>
fn with_counter_system<R>(
    counter_style: &CounterStyle,
    f: impl FnOnce(&CounterSystem<'_>) -> R,
) -> R {
    let decimal = || predefined("decimal").expect("decimal is predefined");
    match counter_style {
        CounterStyle::Name(name) => f(&predefined(&name.0).unwrap_or_else(decimal)),
        CounterStyle::Symbols { ty, symbols } => {
            let symbols: Vec<&str> = symbols.0.iter().map(symbol_to_string).collect();
            let kind = match ty {
                SymbolsType::Cyclic => SymbolsKind::Cyclic,
                SymbolsType::Numeric => SymbolsKind::Numeric,
                SymbolsType::Alphabetic => SymbolsKind::Alphabetic,
                SymbolsType::Symbolic => SymbolsKind::Symbolic,
                SymbolsType::Fixed => SymbolsKind::Fixed,
            };
            f(&anonymous(kind, &symbols))
        },
        CounterStyle::None | CounterStyle::String(_) => f(&decimal()),
    }
}

/// <https://drafts.csswg.org/css-counter-styles-3/#generate-a-counter>
///
/// Mercury: ED5. Takes the counter value and writes it with every predefined style and with
/// `symbols()` (`counter_styles.rs`); it used to assume a value of 0.
pub(crate) fn generate_counter_representation(counter_style: &CounterStyle, value: i32) -> String {
    match counter_style {
        // `counter()` and `counters()` do not accept `none` or a string, so this is not reached.
        CounterStyle::None | CounterStyle::String(_) => String::new(),
        style => with_counter_system(style, |system| generate(system, value)),
    }
}

/// <https://drafts.csswg.org/css-lists/#marker-string>
///
/// Mercury: ED5. Takes the value of the list item, and the prefix and the suffix come from the
/// counter style (`counter_styles.rs`) instead of a table in this function. The suffix of a
/// style is the one of the style that was named, not of the fallback that wrote the number.
pub(crate) fn marker_string(list_style_type: &ListStyleType, value: i32) -> Option<String> {
    match &list_style_type.0 {
        CounterStyle::None => None,
        CounterStyle::String(string) => Some(string.to_string()),
        style => Some(with_counter_system(style, |system| {
            format!(
                "{}{}{}",
                system.prefix,
                generate(system, value),
                system.suffix
            )
        })),
    }
}

// =================================================================================================
// Mercury: ED5 (lane txt-1). The counter pass over the styled DOM.
//
// `counters.rs` computes every counter value from plain input (it names no Stylo or Servo type);
// this part reads the styled DOM, feeds it, keeps the table between reflows and decides when it
// has to be computed again. See `counters.rs` for the rules and `MERCURY.md` for the hook in
// `layout_impl.rs`.
// =================================================================================================

/// What the layout thread keeps between reflows: the table of the last counter pass.
pub(crate) struct CounterState {
    table: RefCell<Arc<CounterTable>>,
    /// Whether the pass has run at least once. The first time there are no boxes yet that could be
    /// out of date.
    computed: Cell<bool>,
}

impl Default for CounterState {
    fn default() -> Self {
        Self {
            table: RefCell::new(Arc::new(CounterTable::empty())),
            computed: Cell::new(false),
        }
    }
}

impl CounterState {
    pub(crate) fn table(&self) -> Arc<CounterTable> {
        self.table.borrow().clone()
    }
}

fn pseudo_kind(pseudo: PseudoElement) -> Option<PseudoKind> {
    match pseudo {
        PseudoElement::Marker => Some(PseudoKind::Marker),
        PseudoElement::Before => Some(PseudoKind::Before),
        PseudoElement::After => Some(PseudoKind::After),
        _ => None,
    }
}

/// `Some(None)` for "no pseudo-element at this level", `Some(Some(kind))` for one that has
/// counters, `None` for one that does not (`::first-letter`, ...).
fn chain_kind(pseudo: Option<PseudoElement>) -> Option<Option<PseudoKind>> {
    match pseudo {
        None => Some(None),
        Some(pseudo) => pseudo_kind(pseudo).map(Some),
    }
}

/// Where the pass records what an element or pseudo-element shows. `None` for a pseudo-element
/// that has no counters (`::first-letter`, ...): nothing is recorded for it.
pub(crate) fn counter_key(info: &NodeAndStyleInfo<'_>) -> Option<CounterKey> {
    let chain = info.pseudo_element_chain();
    Some(CounterKey {
        node: info.node.opaque().0,
        primary: chain_kind(chain.primary)?,
        secondary: chain_kind(chain.secondary)?,
    })
}

/// The number that the marker of this list item shows: what the pass recorded for it, or 1 for a
/// list item that it did not walk.
fn list_item_number(context: &LayoutContext, info: &NodeAndStyleInfo<'_>) -> i32 {
    counter_key(info)
        .and_then(|key| context.counter_table().list_item_value(key))
        .unwrap_or(1)
}

/// `counter(name)`: what the pass recorded for this `content` (0 for a counter that is not there).
pub(crate) fn counter_value(
    context: &LayoutContext,
    info: &NodeAndStyleInfo<'_>,
    name: &str,
) -> i32 {
    counter_key(info).map_or(0, |key| context.counter_table().counter_value(key, name))
}

/// `counters(name, ...)`: every value of the counters of that name in scope, outermost first.
pub(crate) fn counters_values(
    context: &LayoutContext,
    info: &NodeAndStyleInfo<'_>,
    name: &str,
) -> Vec<i32> {
    match counter_key(info) {
        Some(key) => context.counter_table().counters_values(key, name).to_vec(),
        None => vec![0],
    }
}

/// Whether a list item whose `list-style-type` is this one writes its number. A bullet does not,
/// so a list of bullets never has to be numbered.
fn marker_shows_value(list_style_type: &ListStyleType) -> bool {
    match &list_style_type.0 {
        CounterStyle::None | CounterStyle::String(_) => false,
        style => with_counter_system(style, |system| system.depends_on_value()),
    }
}

fn shows_number(style: &ComputedValues) -> bool {
    style.get_box().display.is_list_item() && marker_shows_value(&style.get_list().list_style_type)
}

fn counter_ops(pairs: &[CounterPair<i32>]) -> Vec<CounterOp<'_>> {
    pairs
        .iter()
        .map(|pair| CounterOp {
            name: &pair.name.0,
            // `reversed(name)` without a number is `i32::MIN` in Stylo, as in `counters.rs`.
            value: pair.value,
            reversed: pair.is_reversed,
        })
        .collect()
}

/// The `counter()` and `counters()` items of a `content` property.
fn content_uses(content: &Content<Image>) -> Vec<CounterUse<'_>> {
    let Content::Items(items) = content else {
        return Vec::new();
    };
    items
        .items
        .iter()
        .filter_map(|item| match item {
            ContentItem::Counter(name, _) => Some(CounterUse {
                name: &name.0,
                all: false,
            }),
            ContentItem::Counters(name, ..) => Some(CounterUse {
                name: &name.0,
                all: true,
            }),
            _ => None,
        })
        .collect()
}

/// What HTML adds to the counters of `<ol>`, `<ul>` and `<li>`: their `start`, `reversed` and
/// `value` attributes. `type` is already mapped to `list-style-type` by the user agent style
/// sheet.
fn html_hint(node: ServoLayoutNode<'_>) -> HtmlHint {
    let Some(element) = node.as_html_element() else {
        return HtmlHint::None;
    };
    let name = element.local_name();
    if *name == local_name!("ol") {
        HtmlHint::OrderedList {
            start: element
                .attribute_as_str(&ns!(), &local_name!("start"))
                .and_then(parse_html_integer),
            reversed: element
                .attribute(&ns!(), &local_name!("reversed"))
                .is_some(),
        }
    } else if *name == local_name!("ul") ||
        *name == local_name!("menu") ||
        *name == local_name!("dir")
    {
        HtmlHint::UnorderedList
    } else if *name == local_name!("li") {
        HtmlHint::ListItem {
            value: element
                .attribute_as_str(&ns!(), &local_name!("value"))
                .and_then(parse_html_integer),
        }
    } else {
        HtmlHint::None
    }
}

/// Whether the box tree is built from the children of this element. Replaced elements and the
/// internals of form controls have no list items or counters of the page's.
fn has_children_with_boxes(node: ServoLayoutNode<'_>) -> bool {
    !matches!(
        node.type_id(),
        Some(LayoutNodeType::Element(
            LayoutElementType::HTMLCanvasElement |
                LayoutElementType::HTMLIFrameElement |
                LayoutElementType::HTMLImageElement |
                LayoutElementType::HTMLMediaElement |
                LayoutElementType::SVGImageElement |
                LayoutElementType::SVGSVGElement |
                LayoutElementType::HTMLInputElement |
                LayoutElementType::HTMLTextAreaElement |
                LayoutElementType::HTMLSelectElement |
                LayoutElementType::HTMLMeterElement |
                LayoutElementType::HTMLProgressElement
        ))
    )
}

/// The walk of `counters.rs` over the styled DOM, in the order boxes are built (see
/// `dom_traversal.rs`): an element, its `::marker` (a list item has one first), `::before`, the
/// children of the flat tree, `::after`. Elements and pseudo-elements without a box
/// (`display: none`, a pseudo-element without `content`) are left out with everything inside
/// them, and `display: contents` elements are not fed but their children are.
struct CounterWalker<'a, 'ctx> {
    context: &'a LayoutContext<'ctx>,
    pass: CounterPass,
}

impl<'dom> CounterWalker<'_, '_> {
    fn walk_element(&mut self, node: ServoLayoutNode<'dom>) {
        let style = node.style(&self.context.style_context);
        match Display::from(style.get_box().display) {
            Display::None => {},
            Display::Contents => self.walk_children(node),
            Display::GeneratingBox(_) => self.walk_box(node, &style),
        }
    }

    fn walk_box(&mut self, node: ServoLayoutNode<'dom>, style: &ComputedValues) {
        let key = CounterKey::element(node.opaque().0);
        let numbered = shows_number(style);
        let counters = style.get_counters();
        self.pass.enter(&ElementInput {
            key,
            is_list_item: style.get_box().display.is_list_item(),
            reset: counter_ops(&counters.counter_reset),
            increment: counter_ops(&counters.counter_increment),
            set: Vec::new(),
            html: html_hint(node),
            uses: Vec::new(),
            observe_list_item: numbered,
        });
        if numbered && let Some(element) = node.as_element() {
            self.walk_marker(element, key);
        }
        if has_children_with_boxes(node) {
            self.walk_children(node);
        }
        self.pass.leave();
    }

    fn walk_children(&mut self, node: ServoLayoutNode<'dom>) {
        self.walk_generated(node, PseudoElement::Before, PseudoKind::Before);
        for child in node.flat_tree_children() {
            if child.is_element() {
                self.walk_element(child);
            }
        }
        self.walk_generated(node, PseudoElement::After, PseudoKind::After);
    }

    /// `::before` and `::after`.
    fn walk_generated(
        &mut self,
        node: ServoLayoutNode<'dom>,
        pseudo: PseudoElement,
        kind: PseudoKind,
    ) {
        let Some(element) = node.as_element().and_then(|e| e.with_pseudo(pseudo)) else {
            return;
        };
        let style = element.style(&self.context.style_context);
        if style.ineffective_content_property() ||
            !matches!(
                Display::from(style.get_box().display),
                Display::GeneratingBox(_)
            )
        {
            return;
        }
        let key = CounterKey::pseudo(node.opaque().0, kind);
        let numbered = shows_number(&style);
        let counters = style.get_counters();
        self.pass.enter(&ElementInput {
            key,
            is_list_item: style.get_box().display.is_list_item(),
            reset: counter_ops(&counters.counter_reset),
            increment: counter_ops(&counters.counter_increment),
            set: Vec::new(),
            html: HtmlHint::None,
            uses: content_uses(&counters.content),
            observe_list_item: numbered,
        });
        if numbered {
            self.walk_marker(element, key);
        }
        self.pass.leave();
    }

    /// The `::marker` of a list item. Its counters matter when it has a `content` of its own that
    /// shows one (`li::marker { content: counters(list-item, ".") }`) or changes one. Only a list
    /// item that writes its number is walked: a bullet does not need the pass, and the style of
    /// the marker is computed on demand, not stored.
    fn walk_marker(&mut self, owner: ServoLayoutElement<'dom>, owner_key: CounterKey) {
        let Some(key) = owner_key.with_pseudo(PseudoKind::Marker) else {
            return;
        };
        let Some(marker) = owner.with_pseudo(PseudoElement::Marker) else {
            return;
        };
        let style = marker.style(&self.context.style_context);
        let counters = style.get_counters();
        if matches!(counters.content, Content::None) {
            return;
        }
        let uses = content_uses(&counters.content);
        if uses.is_empty() &&
            counters.counter_reset.is_empty() &&
            counters.counter_increment.is_empty()
        {
            return;
        }
        self.pass.enter(&ElementInput {
            key,
            reset: counter_ops(&counters.counter_reset),
            increment: counter_ops(&counters.counter_increment),
            uses,
            ..Default::default()
        });
        self.pass.leave();
    }
}

/// The counter table of the whole document, from the styles of the last restyle.
fn build_counter_table(context: &LayoutContext, root: ServoLayoutNode<'_>) -> CounterTable {
    let mut walker = CounterWalker {
        context,
        pass: CounterPass::new(),
    };
    if root.is_element() {
        walker.walk_element(root);
    }
    walker.pass.finish()
}

/// What the region of the document that the restyle touched looks like to the counter pass.
#[derive(Default)]
struct DirtyScan {
    /// Some element there has damage that can move a number: its box is rebuilt (it appeared,
    /// disappeared, changed `display`, gained or lost a child), or it is laid out again, which
    /// is all that Servo's style damage says about a change of `counter-reset` or
    /// `counter-increment` (`compute_layout_damage` in the script crate has no case for them).
    changed: bool,
    /// Some element with such damage has to do with counters: a numbered list item, a counter
    /// property, or a `content` that shows a counter.
    relevant: bool,
}

/// Looks at the elements that the restyle marked, as the damage traversal that follows it does
/// (`traversal.rs`): an element is looked into when it has dirty descendants or must be rebuilt
/// with everything below it. Nothing is changed.
#[expect(unsafe_code)]
fn scan_dirty_region(dirty_root: ServoLayoutNode<'_>) -> DirtyScan {
    let mut scan = DirtyScan::default();
    let mut pending = vec![dirty_root];
    while let Some(node) = pending.pop() {
        let Some(element) = node.as_element() else {
            continue;
        };
        let damage;
        {
            let data = element.element_data();
            damage = LayoutDamage::from(data.damage);
            if damage.contains(LayoutDamage::Relayout) ||
                damage.intersects(LayoutDamage::DescendantHasBoxDamage)
            {
                scan.changed = true;
                if let Some(style) = data.styles.get_primary() {
                    let counters = style.get_counters();
                    scan.relevant |= shows_number(style) ||
                        !counters.counter_reset.is_empty() ||
                        !counters.counter_increment.is_empty() ||
                        [PseudoElement::Before, PseudoElement::After]
                            .iter()
                            .filter_map(|pseudo| data.styles.pseudos.get(pseudo))
                            .any(|style| {
                                !content_uses(&style.get_counters().content).is_empty()
                            });
                }
            }
        }
        // SAFETY: the same call as the damage traversal makes, on the same thread, before it.
        let has_dirty_descendants =
            unsafe { element.dangerous_style_element().has_dirty_descendants() };
        if has_dirty_descendants || damage.contains(LayoutDamage::BoxDamage) {
            pending.extend(node.flat_tree_children().filter(|child| child.is_element()));
        }
    }
    scan
}

/// Runs between styling and the construction of boxes (`layout_impl.rs`). The counter pass is
/// computed again when the document has just been laid out for the first time, or when a restyle
/// left damage that can move a number somewhere and either an earlier pass found something that
/// shows a counter or the damaged elements have to do with counters. Returns whether the boxes that already exist
/// show a number that is not the one the new pass found: then every box is rebuilt.
pub(crate) fn update_counters(
    state: &CounterState,
    context: &LayoutContext,
    root: ServoLayoutNode<'_>,
    dirty_root: ServoLayoutNode<'_>,
    device_changed: bool,
) -> bool {
    let old = state.table();
    let first = !state.computed.get();
    if !first {
        let scan = scan_dirty_region(dirty_root);
        let needed = (scan.changed || device_changed) && (old.has_observers() || scan.relevant);
        if !needed {
            return false;
        }
    }

    let table = Arc::new(build_counter_table(context, root));
    let changed = !first && table.changed_from(&old);
    *context.counter_table.write() = table.clone();
    *state.table.borrow_mut() = table;
    state.computed.set(true);
    changed
}
