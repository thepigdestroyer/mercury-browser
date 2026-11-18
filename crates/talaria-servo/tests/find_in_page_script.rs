// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The find script's tests (`src/find_in_page.js`), run in boa_engine, a JavaScript engine written in
//! Rust, so no Node.js is needed. Each case (`find_in_page/cases.js`) runs in a fresh `Context`, a
//! realm of its own, with the fake DOM of `find_in_page/fake_dom.js`; the script is called exactly
//! as `find_in_page.rs` calls it in the browser.

use boa_engine::{Context, Source};

const SCRIPT: &str = include_str!("../src/find_in_page.js");
const FAKE_DOM: &str = include_str!("find_in_page/fake_dom.js");
const CASES: &str = include_str!("find_in_page/cases.js");

fn context() -> Context {
    let mut context = Context::default();
    let script = serde_json::to_string(SCRIPT).expect("the script is a string");
    for (name, source) in [
        ("the script", format!("globalThis.__findSource = {script};")),
        ("the fake DOM", FAKE_DOM.to_owned()),
        ("the cases", CASES.to_owned()),
    ] {
        if let Err(error) = context.eval(Source::from_bytes(&source)) {
            panic!("{name} did not load: {error}");
        }
    }
    context
}

fn run_case(name: &str) {
    let mut context = context();
    let call = format!(
        "runCase({})",
        serde_json::to_string(name).expect("a case name is a string")
    );
    if let Err(error) = context.eval(Source::from_bytes(&call)) {
        panic!("{name}: {error}");
    }
}

macro_rules! cases {
    ($($test:ident => $name:literal,)*) => {
        $(
            #[test]
            fn $test() {
                run_case($name);
            }
        )*

        /// Every case in `cases.js` has a test here, and every test a case.
        #[test]
        fn every_case_runs() {
            let mut context = context();
            let names = context
                .eval(Source::from_bytes("caseNames()"))
                .expect("the case names")
                .to_string(&mut context)
                .expect("a string")
                .to_std_string_escaped();
            let defined = names.lines().collect::<Vec<_>>();
            assert_eq!(defined, vec![$($name),*]);
        }
    };
}

cases! {
    a_page_without_the_text_has_no_matches => "a page without the text has no matches",
    one_match_is_selected_and_scrolled_into_view => "one match is selected and scrolled into view",
    a_match_already_in_view_is_not_scrolled => "a match already in view is not scrolled",
    matching_ignores_case_unless_asked => "matching ignores case unless asked",
    case_folding_covers_non_ascii_letters => "case folding covers non-ASCII letters",
    text_split_across_elements_is_found_and_selected_across_them => "text split across elements is found and selected across them",
    matches_do_not_run_from_one_block_into_the_next => "matches do not run from one block into the next",
    runs_of_white_space_match_any_white_space => "runs of white space match any white space",
    scripts_styles_templates_hidden_and_unrendered_elements_are_skipped => "scripts, styles, templates, hidden and unrendered elements are skipped",
    special_characters_in_the_query_are_literal => "special characters in the query are literal",
    next_and_previous_move_from_the_selected_match_and_wrap => "next and previous move from the selected match and wrap",
    a_selection_made_by_the_user_is_where_the_search_starts => "a selection made by the user is where the search starts",
    without_a_selection_the_reported_match_is_the_starting_point => "without a selection the reported match is the starting point",
    a_new_search_with_no_selection_starts_at_the_first_match_in_view => "a new search with no selection starts at the first match in view",
    open_shadow_roots_and_slotted_content_are_searched_once => "open shadow roots and slotted content are searched once",
    past_the_positions_it_keeps_the_search_still_counts_and_knows_where_it_is => "past the positions it keeps, the search still counts and knows where it is",
    a_page_that_breaks_selecting_still_gets_a_count => "a page that breaks selecting still gets a count",
    a_page_that_removes_the_selection_api_makes_find_fail_not_throw => "a page that removes the selection API makes find fail, not throw",
    globals_the_script_does_not_use_cannot_change_the_answer => "globals the script does not use cannot change the answer",
    a_page_that_replaces_regexp_prototype_exec_gets_a_wrong_answer_not_an_exception => "a page that replaces RegExp.prototype.exec gets a wrong answer, not an exception",
    opening_reads_a_short_selection_and_where_it_is => "opening reads a short selection and where it is",
    opening_ignores_long_or_multi_line_selections_and_reports_none => "opening ignores long or multi-line selections and reports none",
    closing_puts_the_page_s_selection_back_only_while_a_match_is_selected => "closing puts the page's selection back only while a match is selected",
    closing_rejects_a_saved_position_that_no_longer_exists => "closing rejects a saved position that no longer exists",
    several_presses_at_once_move_that_many_matches => "several presses at once move that many matches",
    previous_with_no_selection_goes_to_the_match_just_above_the_view => "previous with no selection goes to the match just above the view",
    a_match_on_screen_inside_an_element_taller_than_the_view_is_not_scrolled_to => "a match on screen inside an element taller than the view is not scrolled to",
    the_time_limit_also_stops_a_run_of_matches_that_are_not_rendered => "the time limit also stops a run of matches that are not rendered",
    a_long_text_node_that_is_not_rendered_is_skipped_without_being_read => "a long text node that is not rendered is skipped without being read",
    the_limit_cuts_the_side_of_the_page_farthest_from_the_selection => "the limit cuts the side of the page farthest from the selection",
    a_huge_text_node_is_read_in_part_around_the_selection => "a huge text node is read in part around the selection",
    text_directly_inside_display_contents_counts_as_rendered => "text directly inside display: contents counts as rendered",
    text_in_the_shadow_root_of_a_hidden_host_is_not_counted => "text in the shadow root of a hidden host is not counted",
    an_explicit_search_cut_short_with_nothing_found_continues_from_where_it_stopped_next_time => "an explicit search cut short with nothing found continues from where it stopped next time",
    setting_up_the_walk_does_not_run_through_every_sibling_of_the_start => "setting up the walk does not run through every sibling of the start",
    a_match_that_runs_across_the_start_point_is_found => "a match that runs across the start point is found",
    only_an_explicit_search_leaves_a_caret_where_it_stopped => "only an explicit search leaves a caret where it stopped",
    closing_after_a_caret_puts_the_page_s_own_selection_back => "closing after a caret puts the page's own selection back",
    in_a_tall_element_the_place_is_estimated_from_the_offset_in_the_whole_element => "in a tall element the place is estimated from the offset in the whole element",
    a_page_that_wraps_regexp_and_reflect_apply_sees_the_query_as_the_header_says => "a page that wraps RegExp and Reflect.apply sees the query, as the header says",
    rounds_make_progress_inside_one_long_block_of_inline_content => "rounds make progress inside one long block of inline content",
}
