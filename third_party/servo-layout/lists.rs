/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use style::counter_style::{CounterStyle, Symbol, SymbolsType};
use style::properties::longhands::list_style_type::computed_value::T as ListStyleType;
use style::values::computed::Image;
use style::values::generics::counters::Content;

use crate::context::LayoutContext;
// Mercury: ED5 (lane txt-1). Counter values and the text of counter styles come from the new
// files `counters.rs`, `counter_walk.rs` and `counter_styles.rs`.
use crate::counter_styles::{CounterSystem, SymbolsKind, anonymous, generate, predefined};
use crate::counter_walk::list_item_number;
use crate::dom_traversal::{
    NodeAndStyleInfo, PseudoElementContentItem, generate_pseudo_element_content,
};
use crate::replaced::ReplacedContents;

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
pub(crate) fn with_counter_system<R>(
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

