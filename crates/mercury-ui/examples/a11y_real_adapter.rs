// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Reads Mercury's windows through the real macOS accessibility connection: the AccessKit
//! adapter that Slint creates for each window, answering NSAccessibility queries as it does for
//! VoiceOver. The `tests/a11y_*.rs` tests use Slint's testing platform, which has no adapter, so
//! this is what shows that the adapter builds the tree, names the controls and keeps a typed
//! password to itself.
//!
//! It opens, on Slint's winit backend with the *software* renderer (no GPU is used, so a wedged
//! graphics driver does not matter), the real `MainWindow` with the HTTP sign-in sheet and the
//! real `SettingsWindow` on the Passwords section with its edit form, plus a small window with a
//! std `LineEdit` for a control. Then, in this process, it asks each window's view for its
//! `accessibilityChildren` (the lazy start of the tree: what VoiceOver's first query does), walks
//! the tree, and checks:
//!
//! - the toolbar buttons, the address field, the page node and the sheet's controls are there by
//!   role and name;
//! - the sign-in sheet's password field and the settings window's "New password" field read an
//!   empty `AXValue` after a password was set through `setAccessibilityValue:` (what a screen
//!   reader's dictation does), and the password did reach the browser's property;
//! - the control, a std `LineEdit` with a password type, does read its text: that is the leak
//!   `SecretField` and `RequestField` exist to avoid.
//!
//! The tree is printed. Exit status 0 when every check holds. It needs a login session on the
//! Mac (a window server) but no accessibility permission, because it asks its own process. The
//! windows stay on screen for about a second and take no focus.
//!
//! `cargo run --release -p mercury-ui --example a11y_real_adapter`

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("a11y_real_adapter reads the macOS accessibility API: it runs on macOS only");
}

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(macos::run());
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use mercury_ui::settings_sections::section;
    use mercury_ui::{
        LoginListRow, MainWindow, PageRequestStyle, PageRequestView, PasswordManagerUi,
        SettingsWindow,
    };
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{class, msg_send, sel};
    use objc2_foundation::NSString;
    use slint::winit_030::winit::event_loop::EventLoop;
    use slint::winit_030::winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

    slint::slint! {
        import { LineEdit } from "std-widgets.slint";

        // The control: Slint's own password field, as the settings window used before.
        export component StdPasswordControl inherits Window {
            in-out property <string> secret;
            title: "std LineEdit control";
            width: 320px;
            height: 80px;
            LineEdit {
                accessible-label: "Control password";
                input-type: password;
                text <=> root.secret;
            }
        }
    }

    /// One node of the accessibility tree, as `NSAccessibility` answers for it.
    struct Item {
        object: Retained<AnyObject>,
        depth: usize,
        role: String,
        subrole: String,
        title: String,
        label: String,
        value: String,
        help: String,
    }

    impl Item {
        /// The name VoiceOver would speak.
        fn name(&self) -> &str {
            if !self.label.is_empty() {
                &self.label
            } else {
                &self.title
            }
        }
    }

    /// Calls a selector that takes no argument and returns an object, if the object has it.
    fn ask(object: &AnyObject, selector: Sel) -> Option<Retained<AnyObject>> {
        // SAFETY: `respondsToSelector:` and `performSelector:` exist on every NSObject, and the
        // selectors asked for here (the NSAccessibility getters) take no argument and return an
        // object or nil.
        unsafe {
            let responds: bool = msg_send![object, respondsToSelector: selector];
            if !responds {
                return None;
            }
            msg_send![object, performSelector: selector]
        }
    }

    fn text_of(object: &AnyObject) -> String {
        // SAFETY: `description` exists on every NSObject and returns an NSString.
        let description: Option<Retained<NSString>> = unsafe { msg_send![object, description] };
        description.map(|text| text.to_string()).unwrap_or_default()
    }

    fn string_of(object: &AnyObject, selector: Sel) -> String {
        ask(object, selector)
            .map(|value| text_of(&value))
            .unwrap_or_default()
    }

    fn array_items(array: &AnyObject) -> Vec<Retained<AnyObject>> {
        // SAFETY: the object is an NSArray, and the indices are below its count.
        unsafe {
            let count: usize = msg_send![array, count];
            (0..count)
                .filter_map(|index| msg_send![array, objectAtIndex: index])
                .collect()
        }
    }

    fn children_of(object: &AnyObject) -> Vec<Retained<AnyObject>> {
        ask(object, sel!(accessibilityChildren))
            .map(|array| array_items(&array))
            .unwrap_or_default()
    }

    fn walk(object: Retained<AnyObject>, depth: usize, found: &mut Vec<Item>) {
        if depth > 40 || found.len() > 5_000 {
            return;
        }
        found.push(Item {
            role: string_of(&object, sel!(accessibilityRole)),
            subrole: string_of(&object, sel!(accessibilitySubrole)),
            title: string_of(&object, sel!(accessibilityTitle)),
            label: string_of(&object, sel!(accessibilityLabel)),
            value: string_of(&object, sel!(accessibilityValue)),
            help: string_of(&object, sel!(accessibilityHelp)),
            object: Retained::clone(&object),
            depth,
        });
        for child in children_of(&object) {
            walk(child, depth + 1, found);
        }
    }

    /// The windows of this process, with their titles.
    fn windows() -> Vec<(String, Retained<AnyObject>)> {
        // SAFETY: `NSApplication` is linked by winit; `sharedApplication`, `windows`, `title` and
        // `contentView` are plain getters.
        unsafe {
            let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
            let windows: Option<Retained<AnyObject>> = msg_send![&*app, windows];
            windows
                .map(|windows| array_items(&windows))
                .unwrap_or_default()
                .into_iter()
                .filter_map(|window| {
                    let title: Option<Retained<NSString>> = msg_send![&*window, title];
                    let view: Option<Retained<AnyObject>> = msg_send![&*window, contentView];
                    Some((title.map(|t| t.to_string()).unwrap_or_default(), view?))
                })
                .collect()
        }
    }

    /// The accessibility tree of the window whose title contains `title`.
    fn tree_of(title: &str) -> Vec<Item> {
        let mut found = Vec::new();
        for (window_title, view) in windows() {
            if window_title.contains(title) {
                walk(view, 0, &mut found);
            }
        }
        found
    }

    fn find<'a>(tree: &'a [Item], role: &str, name: &str) -> Option<&'a Item> {
        tree.iter()
            .find(|item| item.role == role && item.name() == name)
    }

    fn set_value(item: &Item, value: &str) {
        let value = NSString::from_str(value);
        // SAFETY: `setAccessibilityValue:` takes an object and returns nothing.
        unsafe {
            let _: () = msg_send![&*item.object, setAccessibilityValue: &*value];
        }
    }

    fn press(item: &Item) -> bool {
        // SAFETY: `accessibilityPerformPress` takes no argument and returns a BOOL.
        unsafe { msg_send![&*item.object, accessibilityPerformPress] }
    }

    fn print_tree(title: &str, tree: &[Item]) {
        println!("--- {title}: {} nodes", tree.len());
        for item in tree {
            let mut line = format!(
                "{}{} {}",
                "  ".repeat(item.depth),
                item.role.trim_start_matches("AX"),
                item.name()
            );
            if !item.subrole.is_empty() {
                line.push_str(&format!(" [{}]", item.subrole.trim_start_matches("AX")));
            }
            if !item.value.is_empty() {
                line.push_str(&format!(" value={:?}", item.value));
            }
            if !item.help.is_empty() {
                line.push_str(&format!(" help={:?}", item.help));
            }
            println!("{line}");
        }
    }

    /// Check results, in the order they ran.
    #[derive(Default)]
    struct Report {
        lines: Vec<String>,
        failed: bool,
    }

    impl Report {
        fn check(&mut self, ok: bool, what: &str) {
            self.failed |= !ok;
            self.lines
                .push(format!("{} {what}", if ok { "PASS" } else { "FAIL" }));
        }

        fn note(&mut self, what: &str) {
            self.lines.push(format!("NOTE {what}"));
        }
    }

    fn login(site: &str, username: &str) -> LoginListRow {
        LoginListRow {
            site: site.into(),
            username: username.into(),
            kind: "".into(),
        }
    }

    pub fn run() -> i32 {
        // As `--background` of the browser: no Dock icon, no activation, no keyboard focus. The
        // software renderer needs no GPU.
        let mut builder = EventLoop::with_user_event();
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false);
        let selected = slint::BackendSelector::new()
            .backend_name("winit".to_owned())
            .renderer_name("software".to_owned())
            .with_winit_window_attributes_hook(|attributes| attributes.with_active(false))
            .with_winit_event_loop_builder(builder)
            .select();
        if let Err(error) = selected {
            eprintln!("no winit backend with the software renderer: {error}");
            return 2;
        }

        // The browser window, with the HTTP sign-in sheet holding a saved password.
        let main = MainWindow::new().expect("the main window");
        main.set_page_request(PageRequestView {
            id: 11,
            style: PageRequestStyle::SignIn,
            heading: "Sign in to example.com".into(),
            message: "example.com needs a user name and password.".into(),
            value: "ann".into(),
            secret: "fake-saved-secret".into(),
            accept_label: "Sign in".into(),
            decline_label: "Cancel".into(),
            ..PageRequestView::default()
        });
        let answers = Rc::new(RefCell::new(Vec::<String>::new()));
        main.on_page_request_answered({
            let answers = Rc::clone(&answers);
            move |id, accepted, text, password, _| {
                answers
                    .borrow_mut()
                    .push(format!("{id} {accepted} {text} {password}"));
            }
        });
        main.show().expect("shows");

        // The settings window on the Passwords section, editing the first login.
        let settings = SettingsWindow::new().expect("the settings window");
        settings.set_section(section::PASSWORDS);
        let ui = settings.global::<PasswordManagerUi>();
        ui.set_storage_note("Saved in your login keychain.".into());
        ui.set_rows(ModelRc::new(VecModel::from(vec![
            login("a.test", "alice"),
            login("b.test", "bob"),
        ])));
        ui.set_total(2);
        ui.set_selected(0);
        ui.set_detail_site("a.test".into());
        ui.set_detail_username("alice".into());
        ui.set_editing(true);
        ui.set_edit_username("alice".into());
        settings.show().expect("shows");

        // The control.
        let control = StdPasswordControl::new().expect("the control window");
        control.show().expect("shows");

        let report = Rc::new(RefCell::new(Report::default()));
        let timers = Rc::new(RefCell::new(Vec::<Timer>::new()));
        let hold = |timer: Timer| timers.borrow_mut().push(timer);

        // 1. The first query: the lazy adapter requests the initial tree, which Slint builds on
        // its next turn of the event loop.
        let first = Timer::default();
        first.start(TimerMode::SingleShot, Duration::from_millis(400), || {
            for title in ["Mercury Browser", "Settings", "std LineEdit control"] {
                let _ = tree_of(title);
            }
        });
        hold(first);

        // 2. The tree is there: check it, then act through the accessibility API.
        let second = Timer::default();
        second.start(TimerMode::SingleShot, Duration::from_millis(1200), {
            let report = Rc::clone(&report);
            move || {
                let mut report = report.borrow_mut();
                let main = tree_of("Mercury Browser");
                let settings = tree_of("Settings");
                let control = tree_of("std LineEdit control");
                print_tree("main window", &main);
                print_tree("settings window", &settings);
                print_tree("std LineEdit control", &control);

                report.check(main.len() > 15, "the main window's tree has nodes");
                for button in ["Back", "Forward", "Reload page", "New tab"] {
                    report.check(
                        find(&main, "AXButton", button).is_some(),
                        &format!("main: button {button:?} is in the tree"),
                    );
                }
                report.check(
                    main.iter().any(|item| item.name() == "Address and search"),
                    "main: the address field is in the tree",
                );
                report.check(
                    main.iter().any(|item| item.name() == "Page content"),
                    "main: the page node is in the tree",
                );
                for (name, tree, label) in [
                    (
                        "main: the sign-in sheet's password field",
                        &main,
                        "Password",
                    ),
                    ("settings: New password", &settings, "New password"),
                ] {
                    let field = find(tree, "AXTextField", label);
                    report.check(field.is_some(), &format!("{name} is in the tree"));
                    if let Some(field) = field {
                        report.check(
                            field.subrole == "AXSecureTextField",
                            &format!("{name} is a secure text field ({})", field.subrole),
                        );
                        report.check(
                            field.value.is_empty(),
                            &format!("{name} reads an empty value before anything is typed"),
                        );
                    }
                }
                if let Some(field) = find(&main, "AXTextField", "Password") {
                    set_value(field, "fake-typed-secret-A");
                }
                if let Some(field) = find(&settings, "AXTextField", "New password") {
                    set_value(field, "fake-typed-secret-B");
                }
                if let Some(field) = find(&control, "AXTextField", "Control password") {
                    set_value(field, "fake-typed-secret-C");
                }
            }
        });
        hold(second);

        // 3. The set-value actions have run: read back, press, and read again.
        let third = Timer::default();
        third.start(TimerMode::SingleShot, Duration::from_millis(2000), {
            let report = Rc::clone(&report);
            let settings = settings.as_weak();
            move || {
                let mut report = report.borrow_mut();
                let main_tree = tree_of("Mercury Browser");
                let settings_tree = tree_of("Settings");
                let control_tree = tree_of("std LineEdit control");

                // The sheet's password field: empty to the assistive technology, filled for the
                // browser (Sign in sends what the field holds).
                let sheet = find(&main_tree, "AXTextField", "Password");
                report.check(
                    sheet.is_some_and(|field| field.value.is_empty()),
                    "main: the sign-in password reads empty after a set-value",
                );
                if let Some(button) = find(&main_tree, "AXButton", "Sign in") {
                    report.check(press(button), "main: Sign in can be pressed");
                }

                // The settings window's New password.
                let field = find(&settings_tree, "AXTextField", "New password");
                report.check(
                    field.is_some_and(|field| field.value.is_empty()),
                    "settings: New password reads empty after a set-value",
                );
                let reached = settings
                    .upgrade()
                    .map(|window| window.global::<PasswordManagerUi>().get_edit_password());
                report.check(
                    reached.as_deref() == Some("fake-typed-secret-B"),
                    &format!(
                        "settings: the set-value reached the browser's property ({reached:?})"
                    ),
                );
                // A window builds the elements under an `if` (the reveal button, which exists
                // while the field has text) when it is drawn. A visible window is redrawn by
                // the system after a change; these windows are not drawn at all if the system
                // thinks them hidden, so draw it here, as the redraw would.
                if let Some(window) = settings.upgrade() {
                    let _ = window.window().take_snapshot();
                }

                // The control: the std field hands its text on.
                let leaked = find(&control_tree, "AXTextField", "Control password")
                    .map(|field| field.value.clone());
                match leaked.as_deref() {
                    Some("fake-typed-secret-C") => report.note(
                        "control: a std LineEdit with a password type reads its text to the \
                         accessibility API (the leak the two safe fields avoid)",
                    ),
                    other => report.note(&format!("control: the std field read {other:?}")),
                }
            }
        });
        hold(third);

        // 4. The field has text now, so its reveal button exists (the tree was updated by the
        // event loop): press it.
        let fourth = Timer::default();
        fourth.start(TimerMode::SingleShot, Duration::from_millis(2800), {
            let report = Rc::clone(&report);
            move || {
                let mut report = report.borrow_mut();
                let tree = tree_of("Settings");
                print_tree("settings window, a password typed", &tree);
                let show = find(&tree, "AXButton", "Show the new password");
                report.check(show.is_some(), "settings: the reveal button is in the tree");
                if let Some(show) = show {
                    report.check(press(show), "settings: the reveal button can be pressed");
                }
            }
        });
        hold(fourth);

        // 5. The presses have run: Sign in sent the typed password, the reveal button says Hide,
        // and the field still reads empty while its characters are drawn.
        let fifth = Timer::default();
        fifth.start(TimerMode::SingleShot, Duration::from_millis(3600), {
            let report = Rc::clone(&report);
            let answers = Rc::clone(&answers);
            move || {
                let mut report = report.borrow_mut();
                report.check(
                    answers.borrow().len() == 1
                        && answers.borrow()[0] == "11 true ann fake-typed-secret-A",
                    &format!(
                        "main: Sign in sent the typed password ({:?})",
                        answers.borrow()
                    ),
                );
                let tree = tree_of("Settings");
                report.check(
                    find(&tree, "AXButton", "Hide the new password").is_some(),
                    "settings: after the press the button says Hide",
                );
                report.check(
                    find(&tree, "AXTextField", "New password")
                        .is_some_and(|field| field.value.is_empty()),
                    "settings: New password still reads empty while its characters are drawn",
                );
                let _ = slint::quit_event_loop();
            }
        });
        hold(fifth);

        if let Err(error) = slint::run_event_loop_until_quit() {
            eprintln!("the event loop failed: {error}");
            return 2;
        }
        let report = report.borrow();
        println!("--- checks");
        for line in &report.lines {
            println!("{line}");
        }
        i32::from(report.failed)
    }
}
