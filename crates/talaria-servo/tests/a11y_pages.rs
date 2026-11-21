// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The pages of `tests/pages/a11y/` in a headless Servo, read the way an assistive technology reads
//! them (a11y-1, EE12): roles, names, descriptions, states and values of named nodes; the password
//! text nowhere in the tree; a hidden subtree absent; an attribute change reflected in the next
//! update; headings, links, paragraphs and text (R14 section 6.3 milestone 1).
//!
//! **Properties of named nodes, not goldens** (decision D12): a golden of a Servo-run tree cannot be
//! blessed without running Servo. `MERCURY_A11Y_BLESS=1` and `A11yPage::snapshot` exist for the first
//! run on a machine that renders.
//!
//! **Every test here starts a headless Servo, so every test is `#[ignore]`d** and named
//! `headless_servo_<what>`: a software rendering context on macOS is a GL context through surfman, and
//! a machine whose GPU cannot be used (or a locked screen) hangs on it. They compile everywhere and run
//! with `make test-a11y-pages` (`cargo test --release -p talaria-servo --test a11y_pages -- --ignored
//! --test-threads=1`: Servo's tests run serially on one thread).

mod common;

use accesskit::{Action, AriaCurrent, HasPopup, Invalid, Orientation, Role, Toggled};
use accesskit_consumer::Node as TreeNode;
use common::a11y_harness::A11yPage;
use mercury_a11y::actions::SUPPORTED_ACTIONS;
use mercury_a11y::snapshot::{DumpOptions, describe};

/// The node with this role and exact name (exactly one, or the test fails with the candidates).
#[track_caller]
fn node<'a>(page: &'a A11yPage, role: Role, name: &str) -> TreeNode<'a> {
    page.node(page.find(role, name))
}

/// Whether the page has a visible node with this role and name.
fn has(page: &A11yPage, role: Role, name: &str) -> bool {
    page.nodes()
        .iter()
        .any(|node| node.role() == role && node.data().label() == Some(name))
}

/// The text of every paragraph of the page, in order.
fn paragraphs(page: &A11yPage) -> Vec<String> {
    page.nodes()
        .iter()
        .filter(|node| node.role() == Role::Paragraph)
        .map(|node| page.text_of(node))
        .collect()
}

/// Every text run of the page that is visible, as text.
fn visible_text(page: &A11yPage) -> Vec<String> {
    page.nodes()
        .iter()
        .filter(|node| node.role() == Role::TextRun)
        .filter_map(|node| node.data().value().map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// milestone 1: headings, links, paragraphs and text
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_headings_links_and_paragraphs_are_read() {
    let page = A11yPage::load_page("headings-links-article.html");

    for (name, level) in [
        ("Welcome to the test page", 1),
        ("Second section", 2),
        ("A third-level heading", 3),
    ] {
        let heading = node(&page, Role::Heading, name);
        assert_eq!(heading.data().level(), Some(level), "level of {name:?}");
    }
    let docs = node(&page, Role::Link, "Read the documentation");
    assert_eq!(docs.data().url(), Some("https://example.com/docs"));
    let back = node(&page, Role::Link, "Back to top");
    assert!(
        back.data().url().is_some_and(|url| url.ends_with("#intro")),
        "the URL of a fragment link ends with its fragment: {:?}",
        back.data().url()
    );

    // Headings and links come in the order of the page.
    let sequence: Vec<(Role, String)> = page
        .nodes()
        .iter()
        .filter(|node| matches!(node.role(), Role::Heading | Role::Link))
        .map(|node| {
            (
                node.role(),
                node.data().label().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let expected: Vec<(Role, String)> = [
        (Role::Heading, "Welcome to the test page"),
        (Role::Link, "Read the documentation"),
        (Role::Heading, "Second section"),
        (Role::Heading, "A third-level heading"),
        (Role::Link, "Back to top"),
    ]
    .into_iter()
    .map(|(role, name)| (role, name.to_string()))
    .collect();
    assert_eq!(sequence, expected);

    // Paragraphs carry their text, inline markup included.
    let texts = paragraphs(&page);
    assert!(
        texts
            .iter()
            .any(|text| text
                == "Plain text, emphasised words, strong words and code in one paragraph."),
        "paragraphs: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("A screen reader reads, in order"))
    );

    // The list: three items, each its own position in the set.
    let items: Vec<TreeNode<'_>> = page
        .nodes()
        .into_iter()
        .filter(|node| node.role() == Role::ListItem)
        .collect();
    assert_eq!(items.len(), 3);
    for (index, (item, text)) in items
        .iter()
        .zip(["First item", "Second item", "Third item"])
        .enumerate()
    {
        assert_eq!(page.text_of(item), text);
        assert_eq!(item.data().position_in_set(), Some(index + 1));
        assert_eq!(item.data().size_of_set(), Some(3));
    }
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_landmarks_are_roles_with_names() {
    let page = A11yPage::load_page("headings-links-landmarks.html");
    let count = |role: Role| {
        page.nodes()
            .iter()
            .filter(|node| node.role() == role)
            .count()
    };
    assert_eq!(count(Role::Banner), 1, "the page header is one banner");
    assert_eq!(count(Role::Main), 1);
    assert_eq!(count(Role::ContentInfo), 1, "the page footer");
    assert_eq!(count(Role::Article), 1);

    let navigation = node(&page, Role::Navigation, "Main menu");
    let links: Vec<String> = descendants(&navigation)
        .into_iter()
        .filter(|node| node.role() == Role::Link)
        .map(|node| node.data().label().unwrap_or_default().to_string())
        .collect();
    assert_eq!(links, ["Home", "Products", "Contact"]);
    node(&page, Role::Complementary, "Related");
    node(&page, Role::Search, "Search");
    node(&page, Role::SearchInput, "Search the site");
    assert_eq!(
        node(&page, Role::Heading, "Article title").data().level(),
        Some(2)
    );
}

/// The nodes under `root`, in order, `root` excluded.
fn descendants<'a>(root: &TreeNode<'a>) -> Vec<TreeNode<'a>> {
    let mut found = Vec::new();
    let mut stack: Vec<TreeNode<'a>> = root.children().collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        if node.data().is_hidden() {
            continue;
        }
        found.push(node);
        let mut children: Vec<TreeNode<'a>> = node.children().collect();
        children.reverse();
        stack.extend(children);
    }
    found
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_text_is_exactly_what_the_page_wrote() {
    let mut page = A11yPage::load_page("text-paragraphs.html");
    let texts = paragraphs(&page);
    assert!(
        texts
            .iter()
            .any(|text| text == "First paragraph with a link and bold text."),
        "paragraphs: {texts:?}"
    );
    assert!(texts.iter().any(|text| text == "Second paragraph."));
    let broken = texts
        .iter()
        .find(|text| text.contains("Line one"))
        .expect("the paragraph with a line break");
    assert!(broken.contains("Line two"), "{broken:?}");
    let all = visible_text(&page).join("|");
    assert!(all.contains("Quoted text"));
    assert!(all.contains("spaced"));

    // A text change reaches the next update.
    page.eval("changeText('Changed paragraph.')");
    page.spin_until(|page| {
        paragraphs(page)
            .iter()
            .any(|text| text == "Changed paragraph.")
    });
    assert!(
        !paragraphs(&page)
            .iter()
            .any(|text| text == "Second paragraph.")
    );
}

// ---------------------------------------------------------------------------------------------
// forms and the password rule
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_form_controls_have_roles_names_values_and_states() {
    let page = A11yPage::load_page("forms-login.html");
    node(&page, Role::Form, "Sign in form");

    let name = node(&page, Role::TextInput, "Name");
    assert!(name.data().is_required());
    assert_eq!(name.data().value(), Some("Ada Lovelace"));

    let email = node(&page, Role::EmailInput, "Email address");
    assert_eq!(email.data().placeholder(), Some("you@example.com"));
    assert!(email.data().value().unwrap_or_default().is_empty());
    assert!(!email.data().is_required());

    let remember = node(&page, Role::CheckBox, "Remember me");
    assert_eq!(remember.data().toggled(), Some(Toggled::False));
    node(&page, Role::Group, "Plan");
    for (label, checked, position) in [("Free", true, 1), ("Pro", false, 2), ("Team", false, 3)] {
        let radio = node(&page, Role::RadioButton, label);
        assert_eq!(
            radio.data().toggled(),
            Some(Toggled::from(checked)),
            "{label}"
        );
        assert_eq!(radio.data().position_in_set(), Some(position), "{label}");
        assert_eq!(radio.data().size_of_set(), Some(3), "{label}");
    }

    let country = node(&page, Role::ComboBox, "Country");
    assert_eq!(country.data().value(), Some("Brazil"));
    assert_eq!(country.data().is_expanded(), Some(false));

    let notes = node(&page, Role::MultilineTextInput, "Notes");
    assert!(notes.data().is_read_only());
    assert_eq!(notes.data().value(), Some("Typed notes"));

    assert!(
        node(&page, Role::TextInput, "Locked field")
            .data()
            .is_disabled()
    );
    let submit = node(&page, Role::Button, "Sign in");
    assert!(submit.data().supports_action(Action::Click));
    assert!(submit.data().supports_action(Action::Focus));
}

/// Decision D7: a password field has no value, and the text typed into it is nowhere in the tree.
#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_password_text_is_nowhere_in_the_tree() {
    let mut page = A11yPage::load_page("forms-login.html");
    let from_script = "correct horse battery staple";
    let from_markup = "attr-secret-4321";

    for label in ["Password", "Password confirmation"] {
        let field = node(&page, Role::PasswordInput, label);
        assert_eq!(field.data().value(), None, "{label}");
        assert_eq!(field.data().numeric_value(), None, "{label}");
    }
    assert_eq!(
        node(&page, Role::PasswordInput, "Password")
            .data()
            .placeholder(),
        Some("Password")
    );
    // The check is not vacuous: a text field's value is in the tree.
    assert!(page.property_containing("Ada Lovelace").is_some());
    for secret in [from_script, from_markup] {
        assert_eq!(
            page.property_containing(secret),
            None,
            "{secret:?} is in the tree"
        );
    }

    // Typing more does not put it there either.
    let typed = "typed-later-5150";
    page.eval(&format!("document.getElementById('pw').value = '{typed}'"));
    page.eval("document.getElementById('name').value = 'Grace Hopper'");
    page.spin_until(|page| page.property_containing("Grace Hopper").is_some());
    page.settle();
    for secret in [from_script, from_markup, typed] {
        assert_eq!(
            page.property_containing(secret),
            None,
            "{secret:?} is in the tree"
        );
    }
    // The text form of the tree (hidden nodes too) holds none of them either.
    let snapshot = page.snapshot_with(&DumpOptions::default().with_hidden());
    for secret in [from_script, from_markup, typed] {
        assert!(!snapshot.contains(secret), "{secret:?} is in the snapshot");
    }
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_form_errors_are_invalid_described_and_linked() {
    let page = A11yPage::load_page("forms-errors.html");
    let card = node(&page, Role::TextInput, "Card number");
    assert_eq!(card.data().invalid(), Some(Invalid::True));
    assert!(card.data().is_required());
    assert_eq!(card.data().description(), Some("Digits only."));
    let message = card
        .data()
        .error_message()
        .expect("an error message relation");
    let target = page
        .tree()
        .state()
        .node_by_tree_local_id(message, card.locate().1)
        .expect("the message node is in the card's tree");
    assert_eq!(page.text_of(&target), "Enter 16 digits");

    let expiry = node(&page, Role::TextInput, "Expiry date");
    assert!(expiry.data().is_required());
    assert_eq!(expiry.data().invalid(), None);
    let holder = node(&page, Role::TextInput, "Cardholder");
    assert_eq!(holder.data().invalid(), None);
}

/// The labelling rule for a page: every control on the sign-in form has a name.
#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_every_control_of_the_form_has_a_name() {
    let page = A11yPage::load_page("forms-login.html");
    let controls = [
        Role::Button,
        Role::CheckBox,
        Role::RadioButton,
        Role::TextInput,
        Role::EmailInput,
        Role::PasswordInput,
        Role::MultilineTextInput,
        Role::ComboBox,
    ];
    let unnamed: Vec<String> = page
        .nodes()
        .iter()
        .filter(|node| controls.contains(&node.role()))
        .filter(|node| {
            node.data()
                .label()
                .is_none_or(|label| label.trim().is_empty())
        })
        .map(|node| describe(node, &DumpOptions::default()))
        .collect();
    assert!(unnamed.is_empty(), "controls with no name: {unnamed:?}");
}

// ---------------------------------------------------------------------------------------------
// ARIA widgets
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_aria_widgets_expose_their_states() {
    let page = A11yPage::load_page("aria-widgets-toolbar.html");

    let tablist = node(&page, Role::TabList, "Sections");
    assert_eq!(tablist.data().orientation(), Some(Orientation::Horizontal));
    for (label, selected, position) in [
        ("Overview", true, 1),
        ("Details", false, 2),
        ("History", false, 3),
    ] {
        let tab = node(&page, Role::Tab, label);
        assert_eq!(tab.data().is_selected(), Some(selected), "{label}");
        assert_eq!(tab.data().position_in_set(), Some(position), "{label}");
        assert_eq!(tab.data().size_of_set(), Some(3), "{label}");
    }

    let options = node(&page, Role::Button, "Options");
    assert_eq!(options.data().is_expanded(), Some(false));
    assert_eq!(options.data().has_popup(), Some(HasPopup::Menu));
    assert_eq!(
        options.data().controls().len(),
        1,
        "aria-controls names the menu"
    );
    assert!(!has(&page, Role::MenuItem, "Rename"), "the menu is hidden");

    let volume = node(&page, Role::Slider, "Volume");
    assert_eq!(volume.data().numeric_value(), Some(30.0));
    assert_eq!(volume.data().min_numeric_value(), Some(0.0));
    assert_eq!(volume.data().max_numeric_value(), Some(100.0));
    assert!(volume.data().supports_action(Action::Increment));
    assert!(volume.data().supports_action(Action::Decrement));

    let upload = node(&page, Role::ProgressIndicator, "Upload");
    assert_eq!(upload.data().numeric_value(), Some(40.0));
    assert_eq!(
        node(&page, Role::Switch, "Notifications").data().toggled(),
        Some(Toggled::True)
    );

    let dialog = node(&page, Role::Dialog, "Confirm");
    assert!(dialog.data().is_modal());
    assert!(
        descendants(&dialog)
            .iter()
            .any(|node| node.role() == Role::Button && node.data().label() == Some("Cancel"))
    );

    node(&page, Role::ListBox, "Fruit");
    assert_eq!(
        node(&page, Role::ListBoxOption, "Apple")
            .data()
            .is_selected(),
        Some(true)
    );
    assert_eq!(
        node(&page, Role::ListBoxOption, "Banana")
            .data()
            .is_selected(),
        Some(false)
    );
    assert_eq!(
        node(&page, Role::Link, "Home").data().aria_current(),
        Some(AriaCurrent::Page)
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_aria_changes_reach_the_next_update() {
    let mut page = A11yPage::load_page("aria-widgets-toolbar.html");
    page.eval("setVolume(65)");
    page.spin_until(|page| {
        page.nodes()
            .iter()
            .any(|node| node.role() == Role::Slider && node.data().numeric_value() == Some(65.0))
    });

    assert!(!has(&page, Role::MenuItem, "Rename"));
    page.eval("openOptions()");
    page.spin_until(|page| has(page, Role::MenuItem, "Rename"));
    assert_eq!(
        node(&page, Role::Button, "Options").data().is_expanded(),
        Some(true)
    );
}

// ---------------------------------------------------------------------------------------------
// names
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_names_come_from_labels_and_contents() {
    let page = A11yPage::load_page("names-labels.html");
    let expected = [
        (Role::TextInput, "Email address"),
        (Role::TextInput, "Full name"),
        (Role::TextInput, "Search the site"),
        (Role::Button, "Close dialog"),
        (Role::TextInput, "Company"),
        (Role::TextInput, "Company (optional)"),
        (Role::TextInput, "Title name"),
        (Role::TextInput, "Placeholder name"),
        (Role::Button, "Save"),
        (Role::Group, "Delivery"),
        (Role::TextInput, "Street"),
        (Role::Table, "Sales"),
        (Role::Figure, "Cat figure"),
        (Role::Link, "Home page"),
    ];
    let missing: Vec<String> = expected
        .iter()
        .filter(|(role, name)| !has(&page, *role, name))
        .map(|(role, name)| format!("{role:?} {name:?}"))
        .collect();
    assert!(
        missing.is_empty(),
        "no node for: {missing:?}\n{}",
        page.snapshot()
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_a_label_change_reaches_the_next_update() {
    let mut page = A11yPage::load_page("names-labels.html");
    assert!(has(&page, Role::Button, "Save"));
    page.eval("renameSave('Save now')");
    page.spin_until(|page| has(page, Role::Button, "Save now"));
    assert!(!has(&page, Role::Button, "Save"));

    // The text of a <label> is the name of the control it labels: the control is damaged, not the label.
    assert!(has(&page, Role::TextInput, "Email address"));
    page.eval("relabelEmail('Work email')");
    page.spin_until(|page| has(page, Role::TextInput, "Work email"));
    assert!(!has(&page, Role::TextInput, "Email address"));
}

// ---------------------------------------------------------------------------------------------
// states and values
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_states_and_values_of_controls() {
    let page = A11yPage::load_page("states-controls.html");

    assert_eq!(
        node(&page, Role::CheckBox, "Subscribe").data().toggled(),
        Some(Toggled::True)
    );
    assert_eq!(
        node(&page, Role::CheckBox, "Partial").data().toggled(),
        Some(Toggled::Mixed)
    );
    assert_eq!(
        node(&page, Role::Button, "Bold").data().toggled(),
        Some(Toggled::False)
    );
    assert_eq!(
        node(&page, Role::DisclosureTriangle, "More details")
            .data()
            .is_expanded(),
        Some(false)
    );

    let volume = node(&page, Role::Slider, "Volume");
    assert_eq!(volume.data().numeric_value(), Some(3.0));
    assert_eq!(volume.data().min_numeric_value(), Some(0.0));
    assert_eq!(volume.data().max_numeric_value(), Some(10.0));
    assert_eq!(volume.data().numeric_value_step(), Some(1.0));
    let quantity = node(&page, Role::SpinButton, "Quantity");
    assert_eq!(quantity.data().numeric_value(), Some(2.0));
    assert_eq!(quantity.data().min_numeric_value(), Some(1.0));
    assert_eq!(quantity.data().max_numeric_value(), Some(9.0));
    let loading = node(&page, Role::ProgressIndicator, "Loading");
    assert_eq!(loading.data().numeric_value(), Some(30.0));
    assert_eq!(loading.data().max_numeric_value(), Some(100.0));

    let read_only = node(&page, Role::TextInput, "Read-only field");
    assert!(read_only.data().is_read_only());
    assert_eq!(read_only.data().value(), Some("fixed"));
    assert!(
        node(&page, Role::TextInput, "Disabled field")
            .data()
            .is_disabled()
    );
    assert_eq!(
        node(&page, Role::ComboBox, "Size").data().value(),
        Some("Small")
    );
    assert_eq!(
        node(&page, Role::Heading, "Level four").data().level(),
        Some(4)
    );
    assert_eq!(
        node(&page, Role::Link, "Current page")
            .data()
            .aria_current(),
        Some(AriaCurrent::Page)
    );
    let items = page
        .nodes()
        .iter()
        .filter(|node| node.role() == Role::ListItem)
        .count();
    assert_eq!(items, 3);
}

/// Every state a script can change without an attribute reaches the tree: the sites the engine must
/// damage by hand (a checkbox's `checked`, `indeterminate`, a `details` that opens, the value of a
/// range, a number, a text area and a `select`, `readOnly` and `disabled`).
#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_state_changes_reach_the_tree() {
    let mut page = A11yPage::load_page("states-controls.html");

    page.eval("document.getElementById('subscribe').checked = false");
    page.spin_until(|page| {
        page.nodes().iter().any(|node| {
            node.role() == Role::CheckBox
                && node.data().label() == Some("Subscribe")
                && node.data().toggled() == Some(Toggled::False)
        })
    });

    page.eval(
        "var p = document.getElementById('partial'); p.indeterminate = false; p.checked = true",
    );
    page.spin_until(|page| {
        page.nodes().iter().any(|node| {
            node.role() == Role::CheckBox
                && node.data().label() == Some("Partial")
                && node.data().toggled() == Some(Toggled::True)
        })
    });

    page.eval("document.getElementById('bold').setAttribute('aria-pressed', 'true')");
    page.spin_until(|page| {
        page.nodes().iter().any(|node| {
            node.role() == Role::Button
                && node.data().label() == Some("Bold")
                && node.data().toggled() == Some(Toggled::True)
        })
    });

    page.eval("document.getElementById('more').open = true");
    page.spin_until(|page| {
        page.nodes().iter().any(|node| {
            node.role() == Role::DisclosureTriangle && node.data().is_expanded() == Some(true)
        })
    });

    page.eval("document.getElementById('volume').value = 7");
    page.spin_until(|page| {
        page.nodes()
            .iter()
            .any(|node| node.role() == Role::Slider && node.data().numeric_value() == Some(7.0))
    });

    page.eval("document.getElementById('qty').value = 5");
    page.spin_until(|page| {
        page.nodes()
            .iter()
            .any(|node| node.role() == Role::SpinButton && node.data().numeric_value() == Some(5.0))
    });

    page.eval("document.getElementById('loading').value = 80");
    page.spin_until(|page| {
        page.nodes().iter().any(|node| {
            node.role() == Role::ProgressIndicator && node.data().numeric_value() == Some(80.0)
        })
    });

    page.eval("document.getElementById('size').selectedIndex = 2");
    page.spin_until(|page| {
        page.nodes()
            .iter()
            .any(|node| node.role() == Role::ComboBox && node.data().value() == Some("Large"))
    });

    page.eval("document.getElementById('msg').value = 'hello there'");
    page.spin_until(|page| {
        page.nodes().iter().any(|node| {
            node.role() == Role::MultilineTextInput && node.data().value() == Some("hello there")
        })
    });

    page.eval("document.getElementById('ro').readOnly = false");
    page.spin_until(|page| {
        page.nodes().iter().any(|node| {
            node.role() == Role::TextInput
                && node.data().label() == Some("Read-only field")
                && !node.data().is_read_only()
        })
    });

    page.eval("document.getElementById('dis').disabled = false");
    page.spin_until(|page| {
        page.nodes().iter().any(|node| {
            node.role() == Role::TextInput
                && node.data().label() == Some("Disabled field")
                && !node.data().is_disabled()
        })
    });
}

// ---------------------------------------------------------------------------------------------
// hidden content
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_hidden_subtrees_are_absent() {
    let mut page = A11yPage::load_page("hidden-content.html");
    let text = visible_text(&page);
    assert!(
        text.iter().all(|run| !run.contains("MARKER-")),
        "a hidden block is in the tree: {text:?}"
    );
    assert!(text.iter().any(|run| run == "Visible paragraph"));
    assert!(has(&page, Role::Button, "Visible button"));
    assert!(!has(&page, Role::Button, "Hidden button"));
    assert!(!text.iter().any(|run| run == "Revealed paragraph"));

    // What the page reveals appears.
    page.eval("reveal()");
    page.spin_until(|page| {
        visible_text(page)
            .iter()
            .any(|run| run == "Revealed paragraph")
    });

    // aria-hidden set by script removes a visible paragraph; removing it brings it back.
    assert!(
        visible_text(&page)
            .iter()
            .any(|run| run == "Toggled paragraph")
    );
    page.eval("hideToggled()");
    page.spin_until(|page| {
        !visible_text(page)
            .iter()
            .any(|run| run == "Toggled paragraph")
    });
    page.eval("showToggled()");
    page.spin_until(|page| {
        visible_text(page)
            .iter()
            .any(|run| run == "Toggled paragraph")
    });
}

// ---------------------------------------------------------------------------------------------
// actions and the snapshot text
// ---------------------------------------------------------------------------------------------

/// Every action AccessKit defines (the list to ask a node about).
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

/// A node advertises only actions the engine performs (`mercury_a11y::actions::SUPPORTED_ACTIONS`).
/// Whether each does something is `a11y_actions.rs`'s test.
#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_advertised_actions_are_supported_actions() {
    for name in [
        "forms-login.html",
        "aria-widgets-toolbar.html",
        "states-controls.html",
    ] {
        let page = A11yPage::load_page(name);
        let mut seen: Vec<Action> = Vec::new();
        for node in page.all_nodes() {
            for action in ALL_ACTIONS {
                if node.data().supports_action(*action) {
                    assert!(
                        SUPPORTED_ACTIONS.contains(action),
                        "{name}: {action:?} is advertised by {}",
                        describe(&node, &DumpOptions::default())
                    );
                    if !seen.contains(action) {
                        seen.push(*action);
                    }
                }
            }
        }
        // The check is not vacuous.
        for action in [Action::Click, Action::Focus, Action::ScrollIntoView] {
            assert!(
                seen.contains(&action),
                "{name}: nothing advertises {action:?}"
            );
        }
    }
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_snapshot_is_stable_text_of_the_page() {
    let mut page = A11yPage::load_page("headings-links-article.html");
    let first = page.snapshot();
    page.settle();
    assert_eq!(
        first,
        page.snapshot(),
        "an idle page's snapshot does not change"
    );
    assert!(
        first.starts_with("root-web-area"),
        "the snapshot starts at the page: {first}"
    );
    assert!(first.ends_with('\n'));
    let lines: Vec<&str> = first.lines().map(str::trim_start).collect();
    for expected in [
        r#"heading "Welcome to the test page" [level 1]"#,
        r#"heading "Second section" [level 2]"#,
        r#"link "Read the documentation" url="https://example.com/docs""#,
    ] {
        assert!(
            lines.contains(&expected),
            "missing {expected:?} in:\n{first}"
        );
    }
    // No identifier that changes from run to run.
    assert!(!first.contains("NodeId"));
    assert!(!first.contains("Uuid"));
}
