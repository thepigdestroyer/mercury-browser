// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `SearchModes::interpret` reads engine keywords the same way `SearchSettings::match_keyword`
//! does, for every built-in engine and a custom one.

use mercury_settings::{SearchEngine, SearchSettings, builtin_engines};
use talaria_core::{EngineEntry, ScopeNames, SearchModes, Typed};

fn entry(engine: &SearchEngine) -> EngineEntry {
    EngineEntry {
        id: engine.id.clone(),
        name: engine.name.clone(),
        keyword: engine.keyword.clone(),
        url_template: engine.url_template.clone(),
    }
}

const INPUTS: [&str; 8] = [
    "@wikipedia rust",
    "@WIKIPEDIA rust",
    "w rust",
    "w",
    "w.org",
    "@wikipedia",
    "rust",
    "@bing  two  words ",
];

#[test]
fn interpret_agrees_with_match_keyword_for_every_engine() {
    let mut settings = SearchSettings::default();
    settings.custom_engines.push(SearchEngine {
        id: "custom".to_owned(),
        name: "Custom".to_owned(),
        keyword: "w".to_owned(),
        url_template: "https://w.test/?q={searchTerms}".to_owned(),
        suggest_template: None,
    });

    let engines: Vec<SearchEngine> = builtin_engines()
        .into_iter()
        .chain(
            settings
                .engines()
                .into_iter()
                .filter(|engine| engine.id == "custom"),
        )
        .collect();
    let entries: Vec<EngineEntry> = engines.iter().map(entry).collect();
    let modes = SearchModes::new(entries, "duckduckgo", ScopeNames::default());

    for input in INPUTS {
        let expected = settings
            .match_keyword(input)
            .map(|(engine, query)| (engine.id, query.to_owned()));

        match modes.interpret(input) {
            Ok(Typed::Search {
                engine_id,
                query,
                by_keyword: true,
            }) => assert_eq!(Some((engine_id, query)), expected, "input {input:?}"),
            Ok(_) => assert_eq!(None, expected, "input {input:?} should not be a keyword"),
            Err(error) => panic!("input {input:?} should resolve, got {error:?}"),
        }
    }
}
