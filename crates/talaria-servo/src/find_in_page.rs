// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Find in page. Servo 0.8.0 has no find API, so the search is `find_in_page.js`, run through
//! `WebView::evaluate_javascript` in the top-level document's own world. The page can observe and
//! disturb it (see the comment at the top of the script); its answer is a short string, parsed
//! strictly here, so the worst a page can do is make find fail or report wrong counts for itself.
//! Iframes are not searched: evaluation always targets the top-level browsing context.

use servo::{JSValue, JavaScriptEvaluationError};
use talaria_core::{
    EndFindRequest, FindMatches, FindOutcome, FindRequest, FindSelection, FindStep,
};

const SCRIPT: &str = include_str!("find_in_page.js");
/// `MAX_COUNT` in the script: the most matches it counts.
const MAX_COUNT: usize = 1_000_000;
/// `MAX_SELECTION_TEXT` in the script, in UTF-16 units; a `char` is at least one.
const MAX_SELECTION_CHARS: usize = 256;
const MAX_SAVED_SELECTION_LENGTH: usize = 4096;

pub(crate) fn begin_script() -> String {
    call(&serde_json::json!({
        "mode": "begin",
        "query": "",
        "matchCase": false,
        "step": "here",
        "repeat": 1,
        "thorough": false,
        "current": -1,
        "restore": null,
        "caretLeft": false,
    }))
}

pub(crate) fn find_script(request: &FindRequest) -> String {
    call(&serde_json::json!({
        "mode": "find",
        "query": request.query,
        "matchCase": request.match_case,
        "step": match request.step {
            FindStep::Here => "here",
            FindStep::Next => "next",
            FindStep::Previous => "previous",
        },
        "repeat": request.repeat,
        "thorough": request.thorough,
        "current": request.current.map_or(-1, |current| current as i64),
        "restore": null,
        "caretLeft": false,
    }))
}

pub(crate) fn end_script(request: &EndFindRequest) -> String {
    call(&serde_json::json!({
        "mode": "end",
        "query": request.query,
        "matchCase": request.match_case,
        "step": "here",
        "repeat": 1,
        "thorough": false,
        "current": -1,
        "restore": request.restore_selection,
        "caretLeft": request.caret_left,
    }))
}

/// The script is one function expression; the request is a JSON object literal, which is also a
/// JavaScript literal, so nothing in the query can become code.
fn call(request: &serde_json::Value) -> String {
    format!("({SCRIPT})({request})")
}

pub(crate) fn find_outcome(result: Result<JSValue, JavaScriptEvaluationError>) -> FindOutcome {
    match result {
        Ok(JSValue::String(answer)) => parse_find_answer(&answer),
        Ok(_) => FindOutcome::Failed,
        Err(error) => {
            log::debug!(
                "the find script did not run: {}",
                evaluation_error_kind(&error)
            );
            FindOutcome::Failed
        }
    }
}

/// The page's selection text is never logged.
pub(crate) fn selection_read(
    result: Result<JSValue, JavaScriptEvaluationError>,
) -> Option<FindSelection> {
    match result {
        Ok(JSValue::String(answer)) => parse_selection_answer(&answer),
        Ok(_) => None,
        Err(error) => {
            log::debug!(
                "the find script could not read the selection: {}",
                evaluation_error_kind(&error)
            );
            None
        }
    }
}

fn evaluation_error_kind(error: &JavaScriptEvaluationError) -> &'static str {
    match error {
        JavaScriptEvaluationError::DocumentNotFound => "no document",
        JavaScriptEvaluationError::CompilationFailure => "compilation failed",
        JavaScriptEvaluationError::EvaluationFailure(_) => "evaluation failed",
        JavaScriptEvaluationError::InternalError => "internal error",
        JavaScriptEvaluationError::WebViewNotReady => "web view not ready",
        JavaScriptEvaluationError::SerializationError(_) => "result not serialisable",
    }
}

/// `ok;<count>;<current or -1>;<truncated 0|1>;<selected 0|1>`. The current match's position is
/// -1 when there are no matches, or when the search was cut short (truncated).
fn parse_find_answer(answer: &str) -> FindOutcome {
    parse_matches(answer).map_or(FindOutcome::Failed, FindOutcome::Matches)
}

fn parse_matches(answer: &str) -> Option<FindMatches> {
    let mut fields = answer.split(';');
    if fields.next()? != "ok" {
        return None;
    }
    let count = fields.next()?.parse::<usize>().ok()?;
    let current = fields.next()?;
    let truncated = parse_flag(fields.next()?)?;
    let _selected = parse_flag(fields.next()?)?;
    if fields.next().is_some() || count > MAX_COUNT {
        return None;
    }
    let current = match (count, current) {
        (0, "-1") => None,
        (0, _) => return None,
        (_, "-1") if truncated => None,
        (_, _) if truncated => return None,
        (_, current) => Some(
            current
                .parse::<usize>()
                .ok()
                .filter(|&index| index < count)?,
        ),
    };
    Some(FindMatches {
        count,
        current,
        truncated,
    })
}

fn parse_flag(field: &str) -> Option<bool> {
    match field {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// `sel;<saved position or empty>;<text or empty>`.
fn parse_selection_answer(answer: &str) -> Option<FindSelection> {
    let mut fields = answer.splitn(3, ';');
    if fields.next()? != "sel" {
        return None;
    }
    let saved = fields.next()?;
    let text = fields.next()?;
    let saved = (!saved.is_empty()
        && saved.len() <= MAX_SAVED_SELECTION_LENGTH
        && saved
            .chars()
            .all(|character| character.is_ascii_digit() || matches!(character, '.' | ':' | '|')))
    .then(|| saved.to_owned());
    let text = if text.chars().count() <= MAX_SELECTION_CHARS && !text.chars().any(char::is_control)
    {
        text.to_owned()
    } else {
        String::new()
    };
    if saved.is_none() && text.is_empty() {
        return None;
    }
    Some(FindSelection { text, saved })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(count: usize, current: Option<usize>, truncated: bool) -> FindOutcome {
        FindOutcome::Matches(FindMatches {
            count,
            current,
            truncated,
        })
    }

    #[test]
    fn answers_with_matches_are_parsed() {
        assert_eq!(
            parse_find_answer("ok;1500;1499;0;1"),
            matches(1500, Some(1499), false),
            "counts past the kept positions keep their place"
        );
        assert_eq!(parse_find_answer("ok;7;2;0;1"), matches(7, Some(2), false));
        assert_eq!(
            parse_find_answer("ok;1000;-1;1;0"),
            matches(1000, None, true)
        );
        assert_eq!(parse_find_answer("ok;0;-1;0;0"), matches(0, None, false));
        assert_eq!(parse_find_answer("ok;0;-1;1;0"), matches(0, None, true));
        assert_eq!(
            parse_find_answer("ok;12;-1;1;1"),
            matches(12, None, true),
            "a search cut short does not know the match's position"
        );
    }

    #[test]
    fn answers_that_make_no_sense_are_failures() {
        for answer in [
            "fail",
            "",
            "ok",
            "ok;3;3;0;1",
            "ok;3;-1;0;1",
            "ok;3;5;1;1",
            "ok;0;0;0;0",
            "ok;1000001;0;1;1",
            "ok;2;1;2;1",
            "ok;2;1;0;1;extra",
            "ok;-2;0;0;0",
            "ok; 2;0;0;0",
            "sel;;needle",
        ] {
            assert_eq!(parse_find_answer(answer), FindOutcome::Failed, "{answer}");
        }
    }

    #[test]
    fn only_string_answers_count() {
        assert_eq!(find_outcome(Ok(JSValue::Number(3.0))), FindOutcome::Failed);
        assert_eq!(
            find_outcome(Err(JavaScriptEvaluationError::WebViewNotReady)),
            FindOutcome::Failed
        );
        assert_eq!(
            find_outcome(Ok(JSValue::String("ok;1;0;0;1".to_owned()))),
            matches(1, Some(0), false)
        );
        assert_eq!(selection_read(Ok(JSValue::Null)), None);
    }

    #[test]
    fn a_selection_answer_gives_its_text_and_position() {
        assert_eq!(
            parse_selection_answer("sel;1.0.0:9|1.0.0:15;the; needle"),
            Some(FindSelection {
                text: "the; needle".to_owned(),
                saved: Some("1.0.0:9|1.0.0:15".to_owned()),
            })
        );
        assert_eq!(
            parse_selection_answer("sel;1.0:0|1.0:0;"),
            Some(FindSelection {
                text: String::new(),
                saved: Some("1.0:0|1.0:0".to_owned()),
            })
        );
        assert_eq!(parse_selection_answer("sel;;"), None);
    }

    #[test]
    fn a_selection_answer_is_bounded() {
        assert_eq!(
            parse_selection_answer(&format!("sel;;{}", "x".repeat(257))),
            None
        );
        assert_eq!(parse_selection_answer("sel;;two\nlines"), None);
        assert_eq!(
            parse_selection_answer("sel;1.0:0|<script>;x"),
            Some(FindSelection {
                text: "x".to_owned(),
                saved: None,
            })
        );
        assert_eq!(
            parse_selection_answer(&format!("sel;{};", "1".repeat(4097))),
            None
        );
        assert_eq!(parse_selection_answer("ok;1;0;0;1"), None);
    }

    #[test]
    fn the_query_is_passed_as_a_literal() {
        let script = find_script(&FindRequest {
            id: 1,
            query: "\"); alert(1); (\"\u{2028}".to_owned(),
            match_case: true,
            step: FindStep::Previous,
            repeat: 3,
            thorough: true,
            current: Some(4),
        });

        let argument = script
            .strip_prefix(&format!("({SCRIPT})("))
            .and_then(|rest| rest.strip_suffix(')'))
            .expect("the script is called with one argument");
        let request: serde_json::Value =
            serde_json::from_str(argument).expect("the argument is one JSON value");
        assert_eq!(
            request,
            serde_json::json!({
                "mode": "find",
                "query": "\"); alert(1); (\"\u{2028}",
                "matchCase": true,
                "step": "previous",
                "repeat": 3,
                "thorough": true,
                "current": 4,
                "restore": null,
                "caretLeft": false,
            })
        );
    }

    #[test]
    fn opening_and_closing_scripts_carry_their_mode() {
        assert!(begin_script().contains(r#""mode":"begin""#));
        let end = end_script(&EndFindRequest {
            query: "x".to_owned(),
            match_case: false,
            restore_selection: Some("1:0|1:2".to_owned()),
            caret_left: true,
        });
        assert!(end.contains(r#""mode":"end""#));
        assert!(end.contains(r#""restore":"1:0|1:2""#));
        assert!(end.contains(r#""caretLeft":true"#));
    }

    #[test]
    fn the_limits_here_match_the_script() {
        assert!(SCRIPT.contains(&format!("var MAX_COUNT = {MAX_COUNT};")));
        assert!(SCRIPT.contains(&format!("var MAX_SELECTION_TEXT = {MAX_SELECTION_CHARS};")));
    }
}
