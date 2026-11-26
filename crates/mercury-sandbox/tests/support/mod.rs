// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Support for the process tests. A test starts a child that is this same test binary, run
//! with a job name as its argument; the child confines itself with the crate's own API and then
//! tries the forbidden thing. The test passes when the child exits as the test expects. No test
//! confines the test process, and every path lives in a temporary directory.

// Each test binary uses a different part of this module.
#![allow(dead_code)]

use std::fmt::Display;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use mercury_sandbox::{Confinement, Profile, ProfileInputs, confine, content_profile};

/// A test or a child job: a name and the function that runs it.
pub type Test = (&'static str, fn());

/// The exit code of a child whose check failed.
pub const EXIT_FAILED: i32 = 1;

/// The exit code of a child whose profile was refused.
pub const EXIT_REFUSED: i32 = 3;

/// The environment variable that lets a test pass when the runner is itself sandboxed.
pub const ENV_SKIP: &str = "MERCURY_SANDBOX_TEST_ALLOW_SKIP";

const WATCHDOG: Duration = Duration::from_secs(30);
const ENV_CHILD: &str = "MERCURY_TEST_CHILD";
const ENV_BUNDLE: &str = "MERCURY_TEST_BUNDLE";
const ENV_HOME: &str = "MERCURY_TEST_HOME";
const ENV_RUN: &str = "MERCURY_TEST_RUN";

/// Runs the tests of one binary, or, when the binary is started as a child with a job name, runs
/// that job. Exits 0 when every test passed.
pub fn run(tests: &[Test], jobs: &[Test]) {
    if let Some(job) = std::env::args().nth(1)
        && let Some((_, function)) = jobs.iter().find(|(name, _)| *name == job)
    {
        function();
        std::process::exit(0);
    }
    if std::env::var_os(ENV_CHILD).is_some() {
        eprintln!("unknown job for a child of this test binary");
        std::process::exit(2);
    }
    let mut failed = 0;
    for (name, function) in tests {
        match std::panic::catch_unwind(*function) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    println!(
        "test result: {} passed, {failed} failed",
        tests.len() - failed
    );
    if failed > 0 {
        std::process::exit(1);
    }
}

/// The files of one test: a fresh directory under the temporary directory, with a bundle
/// directory holding a readable file, a fake home directory holding a secret, and a run root.
pub struct Tree {
    pub root: PathBuf,
    pub bundle: PathBuf,
    pub home: PathBuf,
    pub run: PathBuf,
}

impl Tree {
    /// A fresh tree named after the test, with `home_name` as the fake home directory. Names
    /// are short: a Unix socket path must stay under the kernel's limit.
    pub fn new(test: &str, home_name: &str) -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).expect("the temporary directory exists");
        let root = base.join(format!("mercury-sbx-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let tree = Self {
            bundle: root.join("bundle"),
            home: root.join(home_name),
            run: root.join("run"),
            root,
        };
        for dir in [&tree.bundle, &tree.home, &tree.run] {
            fs::create_dir_all(dir).expect("create a test directory");
        }
        fs::write(tree.bundle.join("library.txt"), "BUNDLE-FILE\n").expect("write the bundle file");
        fs::write(tree.secret(), "SECRET\n").expect("write the secret");
        fs::create_dir_all(tree.home.join("Library/Keychains")).expect("create the keychains");
        tree
    }

    /// The secret in the fake home directory, which a confined child must not read.
    pub fn secret(&self) -> PathBuf {
        self.home.join("secret.txt")
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// What a child printed and how it ended.
pub struct Outcome {
    pub code: Option<i32>,
    pub stdout: String,
}

/// Starts a child that runs `job` from `tree`, with the paths of the tree and `extra` variables.
/// The child's output goes to a file in the tree, so a chatty child cannot block the test.
pub fn spawn_child(job: &str, tree: &Tree, extra: &[(&str, &str)]) -> (Child, PathBuf) {
    let output = tree.root.join(format!("{job}.out"));
    let stdout = File::create(&output).expect("create the child's output");
    let stderr = stdout.try_clone().expect("clone the child's output");
    let child = Command::new(std::env::current_exe().expect("the test binary's path"))
        .arg(job)
        .env_clear()
        .env(ENV_CHILD, "1")
        .env(ENV_BUNDLE, &tree.bundle)
        .env(ENV_HOME, &tree.home)
        .env(ENV_RUN, &tree.run)
        .envs(extra.iter().copied())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("start the child");
    (child, output)
}

/// Waits for `child` to end, for at most 30 seconds, and reads what it printed.
pub fn finish(mut child: Child, output: &Path) -> Outcome {
    let started = Instant::now();
    let code = loop {
        if let Some(status) = child.try_wait().expect("poll the child") {
            break status.code();
        }
        if started.elapsed() > WATCHDOG {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the child ran longer than {WATCHDOG:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Outcome {
        code,
        stdout: fs::read_to_string(output).unwrap_or_default(),
    }
}

/// Passes when the child exited 0. A child whose profile was refused because the runner is
/// already sandboxed is a SKIP, but only when [`ENV_SKIP`] is 1; otherwise it fails.
pub fn expect_pass(job: &str, outcome: &Outcome) {
    if outcome.code == Some(0) {
        return;
    }
    let runner_sandboxed = outcome.code == Some(EXIT_REFUSED) && outcome.stdout.contains("already");
    if runner_sandboxed && std::env::var(ENV_SKIP).is_ok_and(|value| value == "1") {
        eprintln!("SKIP {job} (runner is sandboxed)");
        return;
    }
    panic!("{job} exited {:?}:\n{}", outcome.code, outcome.stdout);
}

/// Runs `job` as a child from `tree` and passes when it exits 0.
pub fn verdict(job: &str, tree: &Tree, extra: &[(&str, &str)]) {
    let (child, output) = spawn_child(job, tree, extra);
    expect_pass(job, &finish(child, &output));
}

/// The value of a variable the parent set for this child.
pub fn child_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fail(format!("{name} is not set")))
}

/// The bundle directory of this child's tree.
pub fn child_bundle() -> PathBuf {
    PathBuf::from(child_env(ENV_BUNDLE))
}

/// The fake home directory of this child's tree.
pub fn child_home() -> PathBuf {
    PathBuf::from(child_env(ENV_HOME))
}

/// The run root of this child's tree (it may be a symlink).
pub fn child_run() -> PathBuf {
    PathBuf::from(child_env(ENV_RUN))
}

/// The secret in the fake home directory of this child's tree.
pub fn child_secret() -> PathBuf {
    child_home().join("secret.txt")
}

/// The profile inputs of this child, with every path resolved (the executable, the bundle,
/// the home directory and the run root).
pub fn child_inputs() -> ProfileInputs {
    let executable = std::env::current_exe().expect("the test binary's path");
    let mut inputs = ProfileInputs::new(
        canonical(child_bundle()),
        canonical(executable),
        canonical(child_home()),
    );
    inputs.run_root = Some(canonical(child_run()));
    inputs
}

/// The content-process profile for this child's tree.
pub fn child_profile() -> Profile {
    content_profile(&child_inputs()).unwrap_or_else(|error| fail(format!("profile: {error}")))
}

/// Confines this child with the content profile of its tree. A refusal ends the child with
/// [`EXIT_REFUSED`].
pub fn child_confine() -> Confinement {
    match confine(&child_profile()) {
        Ok(confinement) => confinement,
        Err(error) => {
            println!("REFUSED: {error}");
            std::process::exit(EXIT_REFUSED);
        }
    }
}

/// Ends the child with a failed check.
pub fn fail(message: impl Display) -> ! {
    println!("FAIL: {message}");
    std::process::exit(EXIT_FAILED)
}

/// Ends the child with a failed check unless `condition` holds.
pub fn check(condition: bool, message: &str) {
    if !condition {
        fail(message);
    }
}

/// Whether `error` is `EPERM`, the sandbox's answer to a denied operation.
pub fn is_eperm(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EPERM)
}

/// Ends the child unless `result` is an error with `EPERM`.
pub fn expect_eperm<T>(result: io::Result<T>, what: &str) {
    match result {
        Ok(_) => fail(format!("{what} succeeded")),
        Err(error) if is_eperm(&error) => {}
        Err(error) => fail(format!("{what} failed, but not with EPERM: {error}")),
    }
}

/// The value of `result`, or the child fails.
pub fn expect_ok<T, E: Display>(result: Result<T, E>, what: &str) -> T {
    result.unwrap_or_else(|error| fail(format!("{what} failed: {error}")))
}

fn canonical(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or_else(|error| fail(format!("{}: {error}", path.display())))
}
