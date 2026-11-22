/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: new file for ED5 (lane txt-1). Tests of the counter pass in `counters.rs`, which names
// no Stylo or Servo type and is therefore included by path: trees are built by hand, one
// `enter` and `leave` per element, the way `lists.rs` feeds the pass from the styled DOM.
//
// The expected numbers come from CSS Lists Level 3 (<https://drafts.csswg.org/css-lists-3/>):
// the nested counter example of "Nested Counters and Scope", the "list-item" counter section
// and the markup of its last example.

#[path = "../counters.rs"]
#[allow(dead_code)]
mod counters;

mod counters_tests {
    use std::time::Instant;

    use super::counters::{
        CounterKey, CounterOp, CounterPass, CounterTable, CounterUse, ElementInput, HtmlHint,
        NO_VALUE, PseudoKind, parse_html_integer,
    };

    // ---- a tiny document builder -------------------------------------------------------------

    struct Doc {
        pass: CounterPass,
        next: usize,
        open: usize,
    }

    impl Doc {
        fn new() -> Self {
            Self {
                pass: CounterPass::new(),
                next: 1,
                open: 0,
            }
        }

        /// Enters an element; its children follow, then `close`.
        fn open(&mut self, mut input: ElementInput<'static>) -> CounterKey {
            input.key = CounterKey::element(self.next);
            self.next += 1;
            self.pass.enter(&input);
            self.open += 1;
            input.key
        }

        fn close(&mut self) {
            self.pass.leave();
            self.open -= 1;
        }

        /// An element without children.
        fn leaf(&mut self, input: ElementInput<'static>) -> CounterKey {
            let key = self.open(input);
            self.close();
            key
        }

        /// A pseudo-element of `owner` (`::before` and so on), which is a child of it.
        fn pseudo(
            &mut self,
            owner: CounterKey,
            pseudo: PseudoKind,
            mut input: ElementInput<'static>,
        ) -> CounterKey {
            input.key = CounterKey::pseudo(owner.node, pseudo);
            self.pass.enter(&input);
            self.pass.leave();
            input.key
        }

        /// A list item the way `lists.rs` feeds it: the element and, as its first child, a marker
        /// that shows `counters(list-item, ...)`. The children follow, then `close`.
        fn open_item(&mut self, input: ElementInput<'static>) -> CounterKey {
            let key = self.open(input);
            self.pseudo(key, PseudoKind::Marker, uses_all("list-item"));
            key
        }

        /// An item without children.
        fn item(&mut self, input: ElementInput<'static>) -> CounterKey {
            let key = self.open_item(input);
            self.close();
            key
        }

        /// What `lists.rs` calls to start `<li>`s of `count` items in the current element.
        fn items(&mut self, count: usize) -> Vec<CounterKey> {
            (0..count).map(|_| self.item(li())).collect()
        }

        fn finish(self) -> CounterTable {
            assert_eq!(self.open, 0, "every element must be closed");
            self.pass.finish()
        }
    }

    fn uses_all(name: &'static str) -> ElementInput<'static> {
        ElementInput {
            uses: vec![CounterUse { name, all: true }],
            ..Default::default()
        }
    }

    fn uses(name: &'static str) -> ElementInput<'static> {
        ElementInput {
            uses: vec![CounterUse { name, all: false }],
            ..Default::default()
        }
    }

    fn plain() -> ElementInput<'static> {
        ElementInput::default()
    }

    fn ol() -> ElementInput<'static> {
        ElementInput {
            html: HtmlHint::OrderedList {
                start: None,
                reversed: false,
            },
            ..Default::default()
        }
    }

    fn ol_start(start: i32) -> ElementInput<'static> {
        ElementInput {
            html: HtmlHint::OrderedList {
                start: Some(start),
                reversed: false,
            },
            ..Default::default()
        }
    }

    fn ol_reversed() -> ElementInput<'static> {
        ElementInput {
            html: HtmlHint::OrderedList {
                start: None,
                reversed: true,
            },
            ..Default::default()
        }
    }

    fn ol_reversed_start(start: i32) -> ElementInput<'static> {
        ElementInput {
            html: HtmlHint::OrderedList {
                start: Some(start),
                reversed: true,
            },
            ..Default::default()
        }
    }

    fn ul() -> ElementInput<'static> {
        ElementInput {
            html: HtmlHint::UnorderedList,
            ..Default::default()
        }
    }

    /// `<li>`: `display: list-item`.
    fn li() -> ElementInput<'static> {
        ElementInput {
            is_list_item: true,
            observe_list_item: true,
            html: HtmlHint::ListItem { value: None },
            ..Default::default()
        }
    }

    fn li_value(value: i32) -> ElementInput<'static> {
        ElementInput {
            html: HtmlHint::ListItem { value: Some(value) },
            ..li()
        }
    }

    /// `<li style="display: block">`: not a list item.
    fn li_block() -> ElementInput<'static> {
        ElementInput {
            is_list_item: false,
            observe_list_item: false,
            ..li()
        }
    }

    /// `<div style="display: list-item">`.
    fn div_list_item() -> ElementInput<'static> {
        ElementInput {
            is_list_item: true,
            observe_list_item: true,
            ..Default::default()
        }
    }

    fn op(name: &'static str, value: i32) -> CounterOp<'static> {
        CounterOp {
            name,
            value,
            reversed: false,
        }
    }

    trait With: Sized {
        fn reset(self, name: &'static str, value: i32) -> Self;
        fn reset_reversed(self, name: &'static str, value: i32) -> Self;
        fn increment(self, name: &'static str, value: i32) -> Self;
        fn set(self, name: &'static str, value: i32) -> Self;
        fn uses(self, name: &'static str) -> Self;
    }

    impl With for ElementInput<'static> {
        fn reset(mut self, name: &'static str, value: i32) -> Self {
            self.reset.push(op(name, value));
            self
        }
        fn reset_reversed(mut self, name: &'static str, value: i32) -> Self {
            self.reset.push(CounterOp {
                reversed: true,
                ..op(name, value)
            });
            self
        }
        fn increment(mut self, name: &'static str, value: i32) -> Self {
            self.increment.push(op(name, value));
            self
        }
        fn set(mut self, name: &'static str, value: i32) -> Self {
            self.set.push(op(name, value));
            self
        }
        fn uses(mut self, name: &'static str) -> Self {
            self.uses.push(CounterUse { name, all: false });
            self
        }
    }

    // ---- reading the table -------------------------------------------------------------------

    fn numbers(table: &CounterTable, keys: &[CounterKey]) -> Vec<Option<i32>> {
        keys.iter().map(|key| table.list_item_value(*key)).collect()
    }

    fn some(values: &[i32]) -> Vec<Option<i32>> {
        values.iter().map(|value| Some(*value)).collect()
    }

    /// What `counter(name)` shows in the `::before` of each element.
    fn before_values(table: &CounterTable, owners: &[CounterKey], name: &str) -> Vec<i32> {
        owners
            .iter()
            .map(|owner| {
                table.counter_value(CounterKey::pseudo(owner.node, PseudoKind::Before), name)
            })
            .collect()
    }

    /// What the marker of the `<li>` shows for `counters(list-item, ".")`.
    fn marker_text(table: &CounterTable, item: CounterKey) -> String {
        let key = CounterKey::pseudo(item.node, PseudoKind::Marker);
        table
            .counters_values(key, "list-item")
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(".")
    }

    // ---- ordered lists -----------------------------------------------------------------------

    #[test]
    fn a_list_counts_from_one() {
        let mut doc = Doc::new();
        doc.open(ol());
        let items = doc.items(4);
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[1, 2, 3, 4]));
        assert!(table.has_observers());
    }

    #[test]
    fn two_lists_in_a_row_both_start_at_one() {
        let mut doc = Doc::new();
        let mut items = Vec::new();
        for _ in 0..2 {
            doc.open(ol());
            items.extend(doc.items(2));
            doc.close();
        }
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[1, 2, 1, 2]));
    }

    #[test]
    fn nested_lists_have_their_own_numbers_and_the_outer_one_goes_on() {
        let mut doc = Doc::new();
        doc.open(ol());
        let first = doc.item(li());
        let second = doc.open_item(li());
        doc.open(ol());
        let inner = doc.items(3);
        doc.close();
        doc.close();
        let third = doc.item(li());
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &[first, second, third]), some(&[1, 2, 3]));
        assert_eq!(numbers(&table, &inner), some(&[1, 2, 3]));
    }

    #[test]
    fn an_unordered_list_inside_an_item_has_its_own_counter() {
        let mut doc = Doc::new();
        doc.open(ol());
        let first = doc.open_item(li());
        doc.open(ul());
        doc.items(2);
        doc.close();
        doc.close();
        let second = doc.open_item(li());
        doc.open(ol());
        doc.item(li());
        let deeper = doc.open_item(li());
        doc.open(ol_start(10));
        let ten = doc.item(li());
        doc.close();
        doc.close();
        doc.close();
        doc.close();
        let third = doc.item(li());
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &[first, second, third]), some(&[1, 2, 3]));
        assert_eq!(numbers(&table, &[deeper, ten]), some(&[2, 10]));
    }

    #[test]
    fn start_attribute() {
        for (start, expected) in [
            (5, [5, 6, 7]),
            (0, [0, 1, 2]),
            (-2, [-2, -1, 0]),
            (100, [100, 101, 102]),
        ] {
            let mut doc = Doc::new();
            doc.open(ol_start(start));
            let items = doc.items(3);
            doc.close();
            let table = doc.finish();
            assert_eq!(numbers(&table, &items), some(&expected), "start={start}");
        }
    }

    #[test]
    fn reversed_without_start_counts_down_from_the_number_of_items() {
        let mut doc = Doc::new();
        doc.open(ol_reversed());
        let items = doc.items(4);
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[4, 3, 2, 1]));
    }

    #[test]
    fn reversed_with_start_counts_down_from_it() {
        let mut doc = Doc::new();
        doc.open(ol_reversed_start(10));
        let items = doc.items(3);
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[10, 9, 8]));

        let mut doc = Doc::new();
        doc.open(ol_reversed_start(0));
        let items = doc.items(3);
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[0, -1, -2]));
    }

    #[test]
    fn a_reversed_list_of_one_item_and_of_none() {
        let mut doc = Doc::new();
        doc.open(ol_reversed());
        let only = doc.item(li());
        doc.close();
        let table = doc.finish();
        assert_eq!(table.list_item_value(only), Some(1));

        let mut doc = Doc::new();
        doc.open(ol_reversed());
        doc.close();
        assert!(!doc.finish().has_observers());
    }

    #[test]
    fn a_reversed_list_inside_a_normal_one_does_not_change_the_outer_numbers() {
        let mut doc = Doc::new();
        doc.open(ol());
        let a = doc.open_item(li());
        doc.open(ol_reversed());
        let inner = doc.items(2);
        doc.close();
        doc.close();
        let b = doc.item(li());
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &[a, b]), some(&[1, 2]));
        assert_eq!(numbers(&table, &inner), some(&[2, 1]));
    }

    #[test]
    fn li_value_sets_the_number_and_the_items_after_it_go_on_from_there() {
        let mut doc = Doc::new();
        doc.open(ol());
        let a = doc.item(li());
        let b = doc.item(li_value(10));
        let c = doc.item(li());
        let d = doc.item(li_value(3));
        let e = doc.item(li());
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &[a, b, c, d, e]), some(&[1, 10, 11, 3, 4]));
    }

    #[test]
    fn li_value_in_a_reversed_list() {
        // The first item chooses 7: the list counts down from it.
        let mut doc = Doc::new();
        doc.open(ol_reversed());
        let items = [doc.item(li_value(7)), doc.item(li()), doc.item(li())];
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[7, 6, 5]));

        // The middle one chooses 10: the item before it is 11, the one after it 9.
        let mut doc = Doc::new();
        doc.open(ol_reversed());
        let items = [doc.item(li()), doc.item(li_value(10)), doc.item(li())];
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[11, 10, 9]));

        // Only the first one that chooses decides where the list starts; a later one just sets
        // the number from there on.
        let mut doc = Doc::new();
        doc.open(ol_reversed());
        let items = [
            doc.item(li()),
            doc.item(li_value(10)),
            doc.item(li_value(100)),
            doc.item(li()),
        ];
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[11, 10, 100, 99]));
    }

    #[test]
    fn li_value_wins_over_the_start_of_the_list() {
        let mut doc = Doc::new();
        doc.open(ol_start(5));
        let a = doc.item(li_value(2));
        let b = doc.item(li());
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &[a, b]), some(&[2, 3]));
    }

    #[test]
    fn an_li_that_is_display_block_is_not_counted() {
        let mut doc = Doc::new();
        doc.open(ol());
        let a = doc.item(li());
        // Not a list item: no increment, no marker, no number.
        let block = doc.leaf(li_block());
        let b = doc.item(li());
        doc.close();
        let table = doc.finish();
        assert_eq!(
            numbers(&table, &[a, block, b]),
            vec![Some(1), None, Some(2)]
        );
    }

    #[test]
    fn li_value_still_sets_the_counter_of_a_display_block_li() {
        // HTML maps `value` to `counter-set: list-item N`, whatever the display of the element.
        let mut doc = Doc::new();
        doc.open(ol());
        let a = doc.item(li());
        doc.leaf(ElementInput {
            html: HtmlHint::ListItem { value: Some(10) },
            ..li_block()
        });
        let b = doc.item(li());
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &[a, b]), some(&[1, 11]));
    }

    #[test]
    fn an_item_without_a_box_is_not_fed_and_does_not_count() {
        // A `display: none` item: `lists.rs` does not enter it (nor anything inside it), so the
        // numbering goes on without it, and in a reversed list it is not one of the items that
        // are counted either.
        let mut doc = Doc::new();
        doc.open(ol());
        let a = doc.item(li());
        // <li style="display: none"> is not fed.
        let c = doc.item(li());
        doc.close();
        assert_eq!(numbers(&doc.finish(), &[a, c]), some(&[1, 2]));

        let mut doc = Doc::new();
        doc.open(ol_reversed());
        let a = doc.item(li());
        // <li style="display: none"> is not fed.
        let c = doc.item(li());
        doc.close();
        assert_eq!(numbers(&doc.finish(), &[a, c]), some(&[2, 1]));
    }

    #[test]
    fn a_list_item_that_is_not_an_li_counts_too() {
        let mut doc = Doc::new();
        doc.open(ol());
        let a = doc.item(li());
        let b = doc.item(div_list_item());
        let c = doc.item(li());
        doc.close();
        assert_eq!(numbers(&doc.finish(), &[a, b, c]), some(&[1, 2, 3]));
    }

    #[test]
    fn items_inside_a_wrapper_belong_to_the_same_list() {
        let mut doc = Doc::new();
        doc.open(ol());
        let a = doc.item(li());
        doc.open(plain());
        let b = doc.item(li());
        doc.close();
        let c = doc.item(li());
        doc.close();
        assert_eq!(numbers(&doc.finish(), &[a, b, c]), some(&[1, 2, 3]));
    }

    #[test]
    fn items_outside_any_list_count_from_one() {
        let mut doc = Doc::new();
        doc.open(plain());
        let items = doc.items(2);
        doc.close();
        assert_eq!(numbers(&doc.finish(), &items), some(&[1, 2]));
    }

    #[test]
    fn counter_increment_of_list_item_replaces_the_implicit_one() {
        // `ol.evens > li { counter-increment: list-item 2 }`: 2, 4, 6.
        let mut doc = Doc::new();
        doc.open(ol());
        let items: Vec<_> = (0..3)
            .map(|_| doc.item(li().increment("list-item", 2)))
            .collect();
        doc.close();
        assert_eq!(numbers(&doc.finish(), &items), some(&[2, 4, 6]));

        // `counter-increment: list-item 0` turns the numbering off.
        let mut doc = Doc::new();
        doc.open(ol());
        let items: Vec<_> = (0..2)
            .map(|_| doc.item(li().increment("list-item", 0)))
            .collect();
        doc.close();
        assert_eq!(numbers(&doc.finish(), &items), some(&[0, 0]));

        // A counter-increment of another counter does not remove the implicit increment.
        let mut doc = Doc::new();
        doc.open(ol());
        let items: Vec<_> = (0..2)
            .map(|_| doc.item(li().increment("other", 1)))
            .collect();
        doc.close();
        assert_eq!(numbers(&doc.finish(), &items), some(&[1, 2]));
    }

    #[test]
    fn counter_reset_of_list_item_by_the_author_wins_over_the_list() {
        let mut doc = Doc::new();
        doc.open(ol_start(7).reset("list-item", 10));
        let items = doc.items(2);
        doc.close();
        assert_eq!(numbers(&doc.finish(), &items), some(&[11, 12]));

        // An unrelated counter-reset on the list does not stop it from numbering.
        let mut doc = Doc::new();
        doc.open(ol().reset("chapter", 4));
        let items = doc.items(2);
        doc.close();
        assert_eq!(numbers(&doc.finish(), &items), some(&[1, 2]));
    }

    #[test]
    fn the_last_example_of_the_list_item_counter_in_css_lists() {
        // ol > li::marker { content: counters(list-item, '.') '.' } on
        //   <ol> <li> <li value=5> <ol start=3> <li> <li> <ol reversed> 4 x <li> </ol> <li> </ol>
        //   <li> </ol>
        let mut doc = Doc::new();
        let mut items = Vec::new();
        doc.open(ol());
        items.push(doc.item(li()));
        items.push(doc.open_item(li_value(5)));
        doc.open(ol_start(3));
        items.push(doc.item(li()));
        items.push(doc.open_item(li()));
        doc.open(ol_reversed());
        items.extend(doc.items(4));
        doc.close();
        doc.close();
        items.push(doc.item(li()));
        doc.close();
        doc.close();
        items.push(doc.item(li()));
        doc.close();
        let table = doc.finish();
        let shown: Vec<String> = items
            .iter()
            .map(|item| marker_text(&table, *item))
            .collect();
        assert_eq!(
            shown,
            vec![
                "1", "5", "5.3", "5.4", "5.4.4", "5.4.3", "5.4.2", "5.4.1", "5.5", "6"
            ]
        );
    }

    // ---- named counters ----------------------------------------------------------------------

    #[test]
    fn counter_reset_and_increment_of_a_named_counter() {
        // h1 { counter-reset: section } h2 { counter-increment: section } h2::before { content: counter(section) }
        let mut doc = Doc::new();
        doc.open(plain().reset("section", 0));
        let mut headings = Vec::new();
        for _ in 0..3 {
            let heading = doc.open(plain().increment("section", 1));
            doc.pseudo(heading, PseudoKind::Before, uses("section"));
            doc.close();
            headings.push(heading);
        }
        doc.close();
        let table = doc.finish();
        assert_eq!(before_values(&table, &headings, "section"), vec![1, 2, 3]);
    }

    #[test]
    fn the_start_value_and_the_increment_of_a_named_counter() {
        let mut doc = Doc::new();
        doc.open(plain().reset("x", 10));
        let mut owners = Vec::new();
        for _ in 0..3 {
            let element = doc.open(plain().increment("x", 5));
            doc.pseudo(element, PseudoKind::Before, uses("x"));
            doc.close();
            owners.push(element);
        }
        doc.close();
        let table = doc.finish();
        assert_eq!(before_values(&table, &owners, "x"), vec![15, 20, 25]);
    }

    #[test]
    fn counter_set_sets_and_the_increments_go_on_from_it() {
        let mut doc = Doc::new();
        doc.open(plain().reset("x", 0));
        let mut owners = Vec::new();
        for element in [
            plain().increment("x", 1),
            plain().set("x", 40),
            plain().increment("x", 1),
        ] {
            let key = doc.open(element);
            doc.pseudo(key, PseudoKind::Before, uses("x"));
            doc.close();
            owners.push(key);
        }
        doc.close();
        let table = doc.finish();
        assert_eq!(before_values(&table, &owners, "x"), vec![1, 40, 41]);
    }

    #[test]
    fn several_changes_of_the_same_counter_in_one_declaration() {
        // counter-increment: x 2 x 3 -> both are applied; counter-reset: x 1 x 9 -> only the last.
        let mut doc = Doc::new();
        doc.open(plain().reset("x", 1).reset("x", 9));
        let element = doc.open(plain().increment("x", 2).increment("x", 3));
        doc.pseudo(element, PseudoKind::After, uses("x"));
        doc.close();
        doc.close();
        let table = doc.finish();
        assert_eq!(
            table.counter_value(CounterKey::pseudo(element.node, PseudoKind::After), "x"),
            14
        );
    }

    #[test]
    fn a_counter_that_is_not_there_is_created_by_increment() {
        // counter-increment of a counter nobody reset: created with 0, then incremented.
        let mut doc = Doc::new();
        doc.open(plain());
        let mut owners = Vec::new();
        for _ in 0..3 {
            let element = doc.open(plain().increment("n", 1));
            doc.pseudo(element, PseudoKind::Before, uses("n"));
            doc.close();
            owners.push(element);
        }
        doc.close();
        let table = doc.finish();
        assert_eq!(before_values(&table, &owners, "n"), vec![1, 2, 3]);
    }

    #[test]
    fn a_counter_used_before_it_is_reset_is_created_by_the_use() {
        // The use is on the element itself here (`content` on an element, or a pseudo-element
        // of a sibling's child): a counter that is not there is instantiated with 0 where it is
        // used, in the scope of the element and its following siblings.
        let mut doc = Doc::new();
        doc.open(plain());
        let first = doc.open(plain().uses("x"));
        doc.close();
        let second = doc.open(plain().increment("x", 1).uses("x"));
        doc.close();
        let third = doc.open(plain().reset("x", 7).uses("x"));
        doc.close();
        let fourth = doc.open(plain().uses("x"));
        doc.close();
        doc.close();
        let table = doc.finish();
        assert_eq!(table.counter_value(first, "x"), 0);
        assert_eq!(table.counter_value(second, "x"), 1);
        // A reset on a later sibling creates a new counter that replaces the old one.
        assert_eq!(table.counter_value(third, "x"), 7);
        assert_eq!(table.counter_value(fourth, "x"), 7);
    }

    #[test]
    fn a_counter_instantiated_by_a_pseudo_element_stays_in_the_scope_of_its_parent() {
        // `div::before { content: counter(x) }` with no counter x: the counter belongs to the
        // div's children, not to the div's siblings.
        let mut doc = Doc::new();
        doc.open(plain());
        let div = doc.open(plain());
        doc.pseudo(div, PseudoKind::Before, uses("x"));
        let child = doc.open(plain().increment("x", 1));
        doc.pseudo(child, PseudoKind::Before, uses("x"));
        doc.close();
        doc.close();
        let next = doc.open(plain().increment("x", 5));
        doc.pseudo(next, PseudoKind::Before, uses("x"));
        doc.close();
        doc.close();
        let table = doc.finish();
        assert_eq!(before_values(&table, &[div, child], "x"), vec![0, 1]);
        // For the next sibling of the div it is a new counter again (created by the increment).
        assert_eq!(before_values(&table, &[next], "x"), vec![5]);
    }

    #[test]
    fn counters_in_nested_scopes_from_the_specification() {
        // ol { counter-reset: item } li { display: block }
        // li::before { content: counters(item, "."); counter-increment: item }   ("Nested
        // Counters and Scope"): the n-th counter named item is item_n.
        let mut doc = Doc::new();
        let mut items = Vec::new();
        doc.open(plain()); // body
        doc.open(plain().reset("item", 0)); // <ol> creates item0
        items.push(counted(&mut doc)); //        <li> item0 = 1
        doc.close();
        items.push(counted(&mut doc)); //        <li> item0 = 2
        doc.open(plain().reset("item", 0)); //     <ol> creates item1, nested in item0
        items.push(counted(&mut doc)); //            <li> item1 = 1
        doc.close();
        items.push(counted(&mut doc)); //            <li> item1 = 2
        doc.close();
        items.push(counted(&mut doc)); //            <li> item1 = 3
        doc.open(plain().reset("item", 0)); //         <ol> creates item2, nested in item1
        items.push(counted(&mut doc)); //                <li> item2 = 1
        doc.close();
        doc.close(); //                                </ol>
        doc.close(); //                              </li>
        items.push(counted(&mut doc)); //            <li> item1 = 4
        doc.open(plain().reset("item", 0)); //         <ol> creates item3, nested in item1
        items.push(counted(&mut doc)); //                <li> item3 = 1
        doc.close();
        doc.close(); //                                </ol>
        doc.close(); //                              </li>
        items.push(counted(&mut doc)); //            <li> item1 = 5
        doc.close();
        doc.close(); //                            </ol>
        doc.close(); //                          </li>
        items.push(counted(&mut doc)); //        <li> item0 = 3
        doc.close();
        doc.close(); //                        </ol>
        doc.close(); //                      body
        let table = doc.finish();

        let chains: Vec<Vec<i32>> = items
            .iter()
            .map(|key| {
                table
                    .counters_values(CounterKey::pseudo(key.node, PseudoKind::Before), "item")
                    .to_vec()
            })
            .collect();
        assert_eq!(
            chains,
            vec![
                vec![1],
                vec![2],
                vec![2, 1],
                vec![2, 2],
                vec![2, 3],
                vec![2, 3, 1],
                vec![2, 4],
                vec![2, 4, 1],
                vec![2, 5],
                vec![3],
            ]
        );
    }

    /// `li { counter-increment: item } li::before { content: counters(item, ".") }`: opens the
    /// `li` (the caller closes it).
    fn counted(doc: &mut Doc) -> CounterKey {
        let key = doc.open(plain().increment("item", 1));
        doc.pseudo(
            key,
            PseudoKind::Before,
            ElementInput {
                uses: vec![CounterUse {
                    name: "item",
                    all: true,
                }],
                ..Default::default()
            },
        );
        key
    }

    #[test]
    fn a_reset_on_a_later_sibling_replaces_the_counter_of_an_earlier_one() {
        // Two siblings reset x: the second does not nest inside the first.
        let mut doc = Doc::new();
        doc.open(plain());
        let a = doc.open(plain().reset("x", 1));
        doc.pseudo(a, PseudoKind::Before, uses_all_of("x"));
        doc.close();
        let b = doc.open(plain().reset("x", 2));
        doc.pseudo(b, PseudoKind::Before, uses_all_of("x"));
        doc.close();
        doc.close();
        let table = doc.finish();
        let chain = |key: CounterKey| {
            table
                .counters_values(CounterKey::pseudo(key.node, PseudoKind::Before), "x")
                .to_vec()
        };
        assert_eq!(chain(a), vec![1]);
        assert_eq!(chain(b), vec![2]);
    }

    fn uses_all_of(name: &'static str) -> ElementInput<'static> {
        ElementInput {
            uses: vec![CounterUse { name, all: true }],
            ..Default::default()
        }
    }

    #[test]
    fn a_reset_in_a_descendant_nests_in_the_counter_of_an_ancestor() {
        let mut doc = Doc::new();
        doc.open(plain().reset("x", 1));
        let child = doc.open(plain().reset("x", 5));
        doc.pseudo(child, PseudoKind::Before, uses_all_of("x"));
        doc.close();
        // After the child, back in the scope of the outer counter only.
        let after = doc.open(plain().increment("x", 1));
        doc.pseudo(after, PseudoKind::Before, uses_all_of("x"));
        doc.close();
        doc.close();
        let table = doc.finish();
        let before_of = |key: CounterKey| CounterKey::pseudo(key.node, PseudoKind::Before);
        // The child's reset is the first at its level that has an outer counter in scope, so it
        // nests: both are in scope, and `counter()` shows the innermost.
        // (The child is a sibling of nothing that reset x, so the outer 1 stays.)
        assert_eq!(
            table.counters_values(before_of(child), "x").to_vec(),
            vec![1, 5]
        );
        assert_eq!(table.counter_value(before_of(child), "x"), 5);
        // The child's counter belongs to its parent's scope as a *sibling's* counter: the next
        // sibling sees it and increments it (6), the outer one stays 1.
        assert_eq!(
            table.counters_values(before_of(after), "x").to_vec(),
            vec![1, 6]
        );
    }

    #[test]
    fn a_counter_created_in_a_descendant_does_not_leak_to_the_next_sibling() {
        let mut doc = Doc::new();
        doc.open(plain());
        doc.open(plain());
        doc.open(plain().reset("y", 5)); // inside the first sibling
        doc.close();
        doc.close();
        // `y` is not in scope here: a new one is created by the use.
        let second = doc.open(plain().uses("y"));
        doc.close();
        doc.close();
        assert_eq!(doc.finish().counter_value(second, "y"), 0);
    }

    #[test]
    fn the_counters_of_a_pseudo_element_reach_the_children_and_the_next_siblings() {
        // div::before { counter-increment: c } counts before the children of the div see c, and
        // div::after after them.
        let mut doc = Doc::new();
        doc.open(plain().reset("c", 0));
        let div = doc.open(plain());
        doc.pseudo(div, PseudoKind::Before, plain().increment("c", 1));
        let child = doc.open(plain().uses("c"));
        doc.close();
        doc.pseudo(div, PseudoKind::After, plain().increment("c", 10));
        doc.close();
        let next = doc.open(plain().uses("c"));
        doc.close();
        doc.close();
        let table = doc.finish();
        assert_eq!(table.counter_value(child, "c"), 1);
        assert_eq!(table.counter_value(next, "c"), 11);
    }

    #[test]
    fn the_children_of_an_element_that_is_not_fed_belong_to_the_scope_around_it() {
        // `display: contents` elements are not entered, so what their children create belongs to
        // the nearest ancestor that has a box.
        let mut doc = Doc::new();
        doc.open(plain());
        // <span style="display: contents"> is not fed:
        doc.leaf(plain().reset("x", 4)); // <p style="counter-reset: x 4">
        let sibling = doc.open(plain().uses("x"));
        doc.close();
        doc.close();
        assert_eq!(doc.finish().counter_value(sibling, "x"), 4);
    }

    #[test]
    fn reversed_named_counter_without_a_start() {
        // counter-reset: reversed(c); each item counter-increment: c -1 -> counts down to 1.
        let mut doc = Doc::new();
        doc.open(plain().reset_reversed("c", NO_VALUE));
        let mut owners = Vec::new();
        for _ in 0..3 {
            let element = doc.open(plain().increment("c", -1));
            doc.pseudo(element, PseudoKind::Before, uses("c"));
            doc.close();
            owners.push(element);
        }
        doc.close();
        let table = doc.finish();
        assert_eq!(before_values(&table, &owners, "c"), vec![3, 2, 1]);
    }

    #[test]
    fn reversed_named_counter_with_a_start() {
        let mut doc = Doc::new();
        doc.open(plain().reset_reversed("c", 9));
        let element = doc.open(plain().increment("c", -1));
        doc.pseudo(element, PseudoKind::Before, uses("c"));
        doc.close();
        doc.close();
        let table = doc.finish();
        assert_eq!(before_values(&table, &[element], "c"), vec![8]);
    }

    #[test]
    fn deeply_nested_lists_do_not_overflow_the_stack() {
        let mut doc = Doc::new();
        let depth = 5_000;
        for _ in 0..depth {
            doc.open(ol());
        }
        let deepest = doc.item(li());
        for _ in 0..depth {
            doc.close();
        }
        assert_eq!(doc.finish().list_item_value(deepest), Some(1));
    }

    #[test]
    fn a_marker_that_is_a_list_item_of_its_own_pseudo_element() {
        // li::before { display: list-item } has a ::before::marker. The nested pseudo-element
        // has its own key.
        let mut doc = Doc::new();
        doc.open(ol());
        let item = doc.open(li());
        let before = CounterKey::pseudo(item.node, PseudoKind::Before);
        doc.pass.enter(&ElementInput {
            key: before,
            is_list_item: true,
            observe_list_item: true,
            ..Default::default()
        });
        let nested = before
            .with_pseudo(PseudoKind::Marker)
            .expect("a second level of pseudo-element");
        doc.pass.enter(&ElementInput {
            key: nested,
            uses: vec![CounterUse {
                name: "list-item",
                all: false,
            }],
            ..Default::default()
        });
        doc.pass.leave();
        doc.pass.leave();
        doc.close();
        doc.close();
        let table = doc.finish();
        assert_eq!(table.list_item_value(item), Some(1));
        // The pseudo-element is a list item inside the li: it is in the scope of the ol's
        // counter (its parent is the li), so it counts 2.
        assert_eq!(table.list_item_value(before), Some(2));
        assert_eq!(table.counter_value(nested, "list-item"), 2);
    }

    // ---- the table ---------------------------------------------------------------------------

    #[test]
    fn lookups_of_places_that_were_not_recorded_have_defaults() {
        let table = CounterTable::empty();
        let key = CounterKey::element(42);
        assert_eq!(table.list_item_value(key), None);
        assert_eq!(table.counter_value(key, "anything"), 0);
        assert_eq!(table.counters_values(key, "anything").to_vec(), vec![0]);
        assert!(!table.has_observers());
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn a_table_changed_from_another_when_a_shared_place_shows_something_else() {
        let list = |first: ElementInput<'static>, more: usize| {
            let mut doc = Doc::new();
            doc.open(ol());
            doc.item(first);
            doc.items(more);
            doc.close();
            doc.finish()
        };
        let three = list(li(), 2);
        let also_three = list(li(), 2);
        let four = list(li(), 3);
        // The same list with an item more at the end: the old items show what they showed.
        assert!(!four.changed_from(&three));
        assert!(!three.changed_from(&also_three));
        // An item less at the end: the places that are left show what they showed.
        assert!(!three.changed_from(&four));
        // The first item chooses its number: every place of the list moved.
        let moved = list(li_value(9), 2);
        assert!(moved.changed_from(&three));
        assert!(three.changed_from(&moved));
    }

    // ---- html microsyntax --------------------------------------------------------------------

    #[test]
    fn html_integers() {
        assert_eq!(parse_html_integer("5"), Some(5));
        assert_eq!(parse_html_integer("  5"), Some(5));
        assert_eq!(parse_html_integer("\n\t 12"), Some(12));
        assert_eq!(parse_html_integer("+7"), Some(7));
        assert_eq!(parse_html_integer("-7"), Some(-7));
        assert_eq!(parse_html_integer("-0"), Some(0));
        assert_eq!(parse_html_integer("5x"), Some(5));
        assert_eq!(parse_html_integer("5.9"), Some(5));
        assert_eq!(parse_html_integer("007"), Some(7));
        assert_eq!(parse_html_integer(""), None);
        assert_eq!(parse_html_integer("   "), None);
        assert_eq!(parse_html_integer("x5"), None);
        assert_eq!(parse_html_integer("-"), None);
        assert_eq!(parse_html_integer("+x"), None);
        assert_eq!(parse_html_integer("2147483647"), Some(i32::MAX));
        assert_eq!(parse_html_integer("-2147483648"), Some(i32::MIN));
        assert_eq!(parse_html_integer("2147483648"), None);
        assert_eq!(parse_html_integer("99999999999999999999999"), None);
    }

    #[test]
    fn extreme_values_saturate() {
        let mut doc = Doc::new();
        doc.open(ol_start(i32::MAX));
        let items = doc.items(2);
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[i32::MAX, i32::MAX]));

        let mut doc = Doc::new();
        doc.open(ol_start(i32::MIN));
        let items = doc.items(1);
        doc.close();
        let table = doc.finish();
        assert_eq!(numbers(&table, &items), some(&[i32::MIN + 1]));
    }

    // ---- size --------------------------------------------------------------------------------

    fn ten_thousand_items(list: ElementInput<'static>) -> (CounterTable, Vec<CounterKey>) {
        let mut doc = Doc::new();
        doc.open(list);
        let items = doc.items(10_000);
        doc.close();
        (doc.finish(), items)
    }

    #[test]
    fn ten_thousand_items_in_a_list() {
        let (table, items) = ten_thousand_items(ol());
        for (index, item) in items.iter().enumerate() {
            assert_eq!(table.list_item_value(*item), Some(index as i32 + 1));
            assert_eq!(marker_text(&table, *item), (index + 1).to_string());
        }
        // Each item and each marker is a place.
        assert_eq!(table.len(), 20_000);
    }

    #[test]
    fn ten_thousand_items_in_a_reversed_list() {
        let (table, items) = ten_thousand_items(ol_reversed());
        for (index, item) in items.iter().enumerate() {
            assert_eq!(table.list_item_value(*item), Some(10_000 - index as i32));
            assert_eq!(marker_text(&table, *item), (10_000 - index).to_string());
        }
    }

    /// Not run by default: `cargo test --release --test counters -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_the_pass_on_ten_thousand_items() {
        // The input is prepared before the clock starts: what is timed is the pass.
        let items: Vec<ElementInput<'static>> = (0..10_000)
            .map(|index| {
                let mut item = li();
                item.key = CounterKey::element(index + 2);
                item
            })
            .collect();
        let mut best = std::time::Duration::MAX;
        for _ in 0..20 {
            let start = Instant::now();
            let mut pass = CounterPass::new();
            let mut list = ol();
            list.key = CounterKey::element(1);
            pass.enter(&list);
            for item in &items {
                pass.enter(item);
                pass.leave();
            }
            pass.leave();
            let table = pass.finish();
            best = best.min(start.elapsed());
            assert_eq!(table.len(), 10_000);
        }
        eprintln!("counter pass, 10,000 list items (what lists.rs feeds): best of 20 = {best:?}");

        // The same with a marker that shows counters(list-item) in each item: the worst case.
        let markers: Vec<ElementInput<'static>> = (0..10_000)
            .map(|index| {
                let mut marker = uses_all_of("list-item");
                marker.key = CounterKey::pseudo(index + 2, PseudoKind::Marker);
                marker
            })
            .collect();
        let mut best = std::time::Duration::MAX;
        for _ in 0..20 {
            let start = Instant::now();
            let mut pass = CounterPass::new();
            let mut list = ol();
            list.key = CounterKey::element(1);
            pass.enter(&list);
            for (item, marker) in items.iter().zip(&markers) {
                pass.enter(item);
                pass.enter(marker);
                pass.leave();
                pass.leave();
            }
            pass.leave();
            let table = pass.finish();
            best = best.min(start.elapsed());
            assert_eq!(table.len(), 20_000);
        }
        eprintln!(
            "counter pass, 10,000 items with a counters() marker each: best of 20 = {best:?}"
        );

        // 100,000 elements that have no counters: an `enter` and a `leave` each.
        let mut best = std::time::Duration::MAX;
        for _ in 0..10 {
            let start = Instant::now();
            let mut pass = CounterPass::new();
            let plain_element = plain();
            for _ in 0..100_000 {
                pass.enter(&plain_element);
                pass.leave();
            }
            best = best.min(start.elapsed());
            let _ = pass.finish();
        }
        eprintln!("counter pass, 100,000 elements without counters: best of 10 = {best:?}");
    }
}
