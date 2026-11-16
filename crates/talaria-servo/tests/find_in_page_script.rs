//! Runs the find script's JavaScript tests (`find_in_page.test.mjs`) with Node.js. Node.js is not
//! a build requirement: without it the test reports that it was skipped and passes, unless
//! `MERCURY_REQUIRE_NODE=1` is set.

use std::path::Path;
use std::process::Command;

#[test]
fn find_script_passes_its_javascript_tests() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/find_in_page.test.mjs");
    let output = match Command::new("node").arg("--test").arg(&tests).output() {
        Ok(output) => output,
        Err(error) => {
            assert!(
                std::env::var_os("MERCURY_REQUIRE_NODE").is_none(),
                "Node.js is required but could not be run: {error}"
            );
            eprintln!("skipped: Node.js is not available ({error})");
            return;
        }
    };

    assert!(
        output.status.success(),
        "the find script's JavaScript tests failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
