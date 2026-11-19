/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file is new (K0, ED2). It is part of the build script (`build.rs`: `mod
// mercury_bridge;`) and writes `set_bool` and `get_bool` into the generated code. Upstream's
// `set_pref!` takes a literal name written in the source, so an embedder cannot forward a list of
// names it reads at run time; these two functions take the name as a string.

/// The start of the generated lookup, up to the first match arm.
const INDEX_START: &str = r#"/// Mercury: the position in `BOOLS` of the boolean preference called `name`.
fn mercury_bool_index(name: &str) -> Option<usize> {
    match name {"#;

/// The generated lookup after its last match arm, and the two accessors built on it.
const INDEX_END_AND_ACCESSORS: &str = r#"        _ => None,
    }
}

/// Mercury: sets the boolean preference called `name` (a Gecko name such as
/// `layout.css.nth-child-of.enabled`). Returns `false`, and sets nothing, when there is no
/// boolean preference of that name.
pub fn set_bool(name: &str, value: bool) -> bool {
    match mercury_bool_index(name) {
        Some(index) => {
            BOOLS[index].store(value, std::sync::atomic::Ordering::Relaxed);
            true
        },
        None => false,
    }
}

/// Mercury: the current value of the boolean preference called `name`, or `None` when there is
/// no boolean preference of that name.
pub fn get_bool(name: &str) -> Option<bool> {
    mercury_bool_index(name).map(|index| BOOLS[index].load(std::sync::atomic::Ordering::Relaxed))
}"#;

/// Appends `set_bool` and `get_bool` to the generated code in `output`.
///
/// `boolean_names` must be the boolean preferences in the order of the generated `BOOLS` array.
/// The generated functions find a preference by matching its name, so a name that is not a
/// boolean preference (an integer one, or none at all) is refused.
pub fn append_bridge<'a>(
    output: &mut Vec<String>,
    boolean_names: impl IntoIterator<Item = &'a str>,
) {
    println!("cargo::rerun-if-changed=mercury_bridge.rs");
    output.push(INDEX_START.to_owned());
    for (index, name) in boolean_names.into_iter().enumerate() {
        output.push(format!("        {name:?} => Some({index}),"));
    }
    output.push(INDEX_END_AND_ACCESSORS.to_owned());
}
