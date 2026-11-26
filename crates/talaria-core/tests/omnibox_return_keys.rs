// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

#[path = "common/omnibox.rs"]
mod omnibox;

use omnibox::modes;
use talaria_core::{
    AddressResolutionError, OpenIn, Platform, ReturnKeys, ReturnResolution, ReturnTarget, Scope,
    return_plan,
};
use url::Url;

const NONE: ReturnKeys = ReturnKeys {
    shift: false,
    alt: false,
    ctrl: false,
    meta: false,
};

const SHIFT: ReturnKeys = ReturnKeys {
    shift: true,
    ..NONE
};
const ALT: ReturnKeys = ReturnKeys { alt: true, ..NONE };
const CTRL: ReturnKeys = ReturnKeys { ctrl: true, ..NONE };
const META: ReturnKeys = ReturnKeys { meta: true, ..NONE };

fn url(text: &str) -> Url {
    Url::parse(text).expect("the test URL should parse")
}

fn keys(shift: bool, alt: bool, ctrl: bool, meta: bool) -> ReturnKeys {
    ReturnKeys {
        shift,
        alt,
        ctrl,
        meta,
    }
}

fn resolve(text: &str, keys: ReturnKeys) -> Result<ReturnResolution, AddressResolutionError> {
    modes().resolve_return(text, keys, Platform::Mac)
}

fn new_tab(foreground: bool) -> OpenIn {
    OpenIn::NewTab { foreground }
}

#[test]
fn mac_table() {
    let cases = [
        (NONE, (OpenIn::CurrentTab, false)),
        (META, (new_tab(false), false)),
        (ALT, (new_tab(true), false)),
        (keys(true, false, false, true), (new_tab(true), false)),
        (SHIFT, (OpenIn::NewWindow, false)),
        (CTRL, (OpenIn::CurrentTab, true)),
        (keys(true, false, true, false), (OpenIn::NewWindow, false)),
        (keys(true, true, false, false), (OpenIn::CurrentTab, false)),
        (keys(false, true, false, true), (OpenIn::CurrentTab, false)),
        (keys(false, false, true, true), (OpenIn::CurrentTab, false)),
    ];

    for (keys, expected) in cases {
        assert_eq!(return_plan(keys, Platform::Mac), expected, "keys {keys:?}");
    }
}

#[test]
fn other_platform_table() {
    let cases = [
        (ALT, (new_tab(true), false)),
        (keys(true, true, false, false), (new_tab(false), false)),
        (SHIFT, (OpenIn::NewWindow, false)),
        (CTRL, (OpenIn::CurrentTab, true)),
        (keys(true, false, true, false), (OpenIn::NewWindow, false)),
        (META, (OpenIn::CurrentTab, false)),
        (keys(false, true, true, false), (OpenIn::CurrentTab, false)),
    ];

    for (keys, expected) in cases {
        assert_eq!(
            return_plan(keys, Platform::Other),
            expected,
            "keys {keys:?}"
        );
    }
}

#[test]
fn a_plain_search_goes_to_a_background_tab_on_command() {
    assert_eq!(
        resolve("rust book", META).unwrap(),
        ReturnResolution {
            target: ReturnTarget::Search {
                engine_id: "duckduckgo".to_owned(),
                query: "rust book".to_owned(),
            },
            open_in: new_tab(false),
        }
    );
}

#[test]
fn option_opens_an_address_in_a_foreground_tab() {
    assert_eq!(
        resolve("example.com", ALT).unwrap(),
        ReturnResolution {
            target: ReturnTarget::Address(url("https://example.com/")),
            open_in: new_tab(true),
        }
    );
}

#[test]
fn control_adds_www_and_com_to_a_bare_word() {
    let open = |text: &str| resolve(text, CTRL).unwrap();

    assert_eq!(
        open("mercury").target,
        ReturnTarget::Address(url("https://www.mercury.com/"))
    );
    assert_eq!(open("mercury").open_in, OpenIn::CurrentTab);
    assert_eq!(
        open("mercury/docs?x=1").target,
        ReturnTarget::Address(url("https://www.mercury.com/docs?x=1"))
    );
    assert_eq!(
        open("example.org").target,
        ReturnTarget::Address(url("https://example.org/"))
    );
    assert_eq!(
        open("localhost").target,
        ReturnTarget::Address(url("https://localhost/"))
    );
    assert_eq!(
        open("bücher").target,
        ReturnTarget::Address(url("https://www.xn--bcher-kva.com/"))
    );
}

#[test]
fn control_leaves_multi_word_and_dash_text_as_searches() {
    for text in ["rust book", "-x"] {
        let resolution = resolve(text, CTRL).unwrap();

        assert!(
            matches!(resolution.target, ReturnTarget::Search { .. }),
            "{text:?} should search, got {resolution:?}"
        );
    }
}

#[test]
fn shift_on_a_bare_keyword_opens_the_engine_home() {
    assert_eq!(
        resolve("@wikipedia", SHIFT).unwrap(),
        ReturnResolution {
            target: ReturnTarget::EngineHome {
                engine_id: "wikipedia".to_owned(),
                url: url("https://en.wikipedia.org/"),
            },
            open_in: OpenIn::CurrentTab,
        }
    );
}

#[test]
fn no_modifier_on_a_bare_keyword_is_an_ordinary_search() {
    assert_eq!(
        resolve("@wikipedia", NONE).unwrap(),
        ReturnResolution {
            target: ReturnTarget::Search {
                engine_id: "duckduckgo".to_owned(),
                query: "@wikipedia".to_owned(),
            },
            open_in: OpenIn::CurrentTab,
        }
    );
}

#[test]
fn shift_with_a_keyword_and_query_opens_a_new_window() {
    assert_eq!(
        resolve("@wikipedia rust", SHIFT).unwrap(),
        ReturnResolution {
            target: ReturnTarget::Search {
                engine_id: "wikipedia".to_owned(),
                query: "rust".to_owned(),
            },
            open_in: OpenIn::NewWindow,
        }
    );
}

#[test]
fn a_scope_word_is_returned_for_the_popup_to_narrow() {
    assert_eq!(
        resolve("@tabs x", META).unwrap(),
        ReturnResolution {
            target: ReturnTarget::Scope {
                scope: Scope::Tabs,
                query: "x".to_owned(),
            },
            open_in: new_tab(false),
        }
    );
}

#[test]
fn empty_input_is_refused_with_any_keys() {
    for keys in [NONE, SHIFT, ALT, CTRL, META] {
        assert!(matches!(
            resolve("", keys),
            Err(AddressResolutionError::EmptyInput)
        ));
    }
}
