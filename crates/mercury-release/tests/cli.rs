// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Drives the real `mercury-release` binary, the way the release workflow does.
//!
//! Keys here are made by the `minisign` crate for each test and are never the release key.
//! `keygen` is run only where it must refuse (it reads its password from the terminal); the
//! password-protected key of the `sign` tests is made with the same library call `keygen`
//! uses, at minisign's default cost (about 0.1 s and 32 MiB).

use std::fs::{self, File};
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_mercury-release");
const DIGEST_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const DIGEST_B: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const PASSWORD: &str = "correct horse battery staple";

/// A directory of its own under the system temporary directory, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mercury-release-cli-{}-{nanos}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What a run of the binary did.
#[derive(Debug)]
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn succeeded(&self) {
        assert_eq!(self.code, Some(0), "{self:?}");
    }

    /// Refused cleanly: status 2 and a message, not a crash (a panic exits with 101).
    fn refused(&self) {
        assert_eq!(self.code, Some(2), "{self:?}");
        assert!(self.stderr.starts_with("mercury-release: "), "{self:?}");
        assert!(!self.stdout.contains("signature good"), "{self:?}");
    }

    fn says_nothing_of(&self, secrets: &[&str]) {
        for secret in secrets {
            assert!(
                !self.stdout.contains(secret) && !self.stderr.contains(secret),
                "output holds {secret:?}: {self:?}"
            );
        }
    }
}

/// Runs the binary with no `MINISIGN_*` variables except `env`, no stdin, a time limit, and
/// its output captured in files (so a full pipe can never stall it).
fn run(scratch: &Scratch, args: &[&str], env: &[(&str, &str)]) -> Run {
    static CALL: AtomicU32 = AtomicU32::new(0);
    let call = CALL.fetch_add(1, Ordering::Relaxed);
    let stdout_path = scratch.path(&format!("run-{call}.stdout"));
    let stderr_path = scratch.path(&format!("run-{call}.stderr"));
    let mut child = Command::new(BIN)
        .args(args)
        .env_remove("MINISIGN_SECRET_KEY")
        .env_remove("MINISIGN_PASSWORD")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(File::create(&stdout_path).unwrap())
        .stderr(File::create(&stderr_path).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("mercury-release {args:?} did not finish in 60 seconds");
        }
        thread::sleep(Duration::from_millis(10));
    };
    Run {
        code: status.code(),
        stdout: fs::read_to_string(&stdout_path).unwrap(),
        stderr: fs::read_to_string(&stderr_path).unwrap(),
    }
}

/// A key pair (public key text, secret key text) stored without a password.
fn unprotected_key_pair() -> (String, String) {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    (
        pair.pk.to_box().unwrap().into_string(),
        pair.sk.to_box(None).unwrap().into_string(),
    )
}

/// One password-protected key pair, shared by the tests that need one.
fn protected_key_pair() -> &'static (String, String) {
    static PAIR: OnceLock<(String, String)> = OnceLock::new();
    PAIR.get_or_init(|| {
        let pair =
            minisign::KeyPair::generate_encrypted_keypair(Some(PASSWORD.to_owned())).unwrap();
        (
            pair.pk.to_box().unwrap().into_string(),
            pair.sk.to_box(None).unwrap().into_string(),
        )
    })
}

/// Signs `manifest` straight through minisign, with any trusted comment.
fn sign_with(unprotected_secret: &str, manifest: &[u8], comment: &str) -> String {
    let secret = minisign::SecretKeyBox::from_string(unprotected_secret)
        .unwrap()
        .into_unencrypted_secret_key()
        .unwrap();
    minisign::sign(
        None,
        &secret,
        Cursor::new(manifest),
        Some(comment),
        Some("signature made by a test"),
    )
    .unwrap()
    .into_string()
}

/// Writes `update.json` for `version` with the real `update-manifest` command.
fn write_manifest(scratch: &Scratch, version: &str) -> String {
    let out = scratch.path(&format!("update-{version}.json"));
    run(
        scratch,
        &[
            "update-manifest",
            "--version",
            version,
            "--published",
            "2026-12-01",
            "--platform",
            &format!("macos-arm64=Mercury-Browser-{version}-macos-arm64.dmg:{DIGEST_A}:123456789"),
            "--platform",
            &format!("linux-x86_64=mercury-browser-linux-x86_64.tar.gz:{DIGEST_B}:42"),
            "--out",
            &out,
        ],
        &[],
    )
    .succeeded();
    out
}

/// A manifest of 0.3.0, signed inside the test with a fresh unprotected key, and the files
/// `verify` reads.
struct Fixture {
    scratch: Scratch,
    manifest: String,
    signature: String,
    public_key: String,
    secret_key: String,
}

impl Fixture {
    fn new() -> Self {
        let scratch = Scratch::new();
        let manifest = write_manifest(&scratch, "0.3.0");
        let (public, secret) = unprotected_key_pair();
        let bytes = fs::read(&manifest).unwrap();
        let signature = scratch.path("update.json.minisig");
        fs::write(
            &signature,
            sign_with(&secret, &bytes, "mercury-browser update 0.3.0"),
        )
        .unwrap();
        let public_key = scratch.path("update-key.pub");
        fs::write(&public_key, public).unwrap();
        Self {
            scratch,
            manifest,
            signature,
            public_key,
            secret_key: secret,
        }
    }

    /// `verify` with these files and `version`.
    fn verify(&self, manifest: &str, signature: &str, public_key: &str, version: &str) -> Run {
        run(
            &self.scratch,
            &[
                "verify",
                "--manifest",
                manifest,
                "--signature",
                signature,
                "--public-key",
                public_key,
                "--version",
                version,
            ],
            &[],
        )
    }

    /// A copy of the signature file with `edit` applied to its text.
    fn edited_signature(&self, name: &str, edit: impl FnOnce(&str) -> String) -> String {
        let path = self.scratch.path(name);
        fs::write(&path, edit(&fs::read_to_string(&self.signature).unwrap())).unwrap();
        path
    }
}

#[test]
fn update_manifest_writes_a_file_and_refuses_to_overwrite_it() {
    let scratch = Scratch::new();
    let out = write_manifest(&scratch, "0.3.0");
    let written = fs::read_to_string(&out).unwrap();
    assert!(
        written.starts_with(
            "{\n  \"product\": \"mercury-browser\",\n  \"version\": \"0.3.0\",\n  \"published\": \"2026-12-01\",\n"
        ),
        "{written}"
    );
    assert!(written.contains(&format!("\"sha256\": \"{DIGEST_A}\"")));
    assert!(written.contains("\"size\": 123456789"));
    assert!(written.ends_with("}\n"));

    let again = run(
        &scratch,
        &[
            "update-manifest",
            "--version",
            "0.3.0",
            "--published",
            "2026-12-02",
            "--platform",
            &format!("linux-x86_64=a.tar.gz:{DIGEST_A}:1"),
            "--out",
            &out,
        ],
        &[],
    );
    again.refused();
    assert!(again.stderr.contains("cannot create"), "{again:?}");
    assert_eq!(fs::read_to_string(&out).unwrap(), written);
}

#[test]
fn update_manifest_refuses_bad_input_and_writes_nothing() {
    let scratch = Scratch::new();
    let good_platform = format!("linux-x86_64=a.tar.gz:{DIGEST_A}:1");
    let cases: Vec<(&str, Vec<String>)> = vec![
        (
            "an impossible date",
            vec![
                "--version".into(),
                "0.3.0".into(),
                "--published".into(),
                "2026-02-31".into(),
                "--platform".into(),
                good_platform.clone(),
            ],
        ),
        (
            "a pre-release",
            vec![
                "--version".into(),
                "0.3.0-rc.1".into(),
                "--published".into(),
                "2026-12-01".into(),
                "--platform".into(),
                good_platform.clone(),
            ],
        ),
        (
            "an upper-case digest",
            vec![
                "--version".into(),
                "0.3.0".into(),
                "--published".into(),
                "2026-12-01".into(),
                "--platform".into(),
                format!("linux-x86_64=a.tar.gz:{}:1", DIGEST_A.to_uppercase()),
            ],
        ),
        (
            "a name that needs JSON escaping",
            vec![
                "--version".into(),
                "0.3.0".into(),
                "--published".into(),
                "2026-12-01".into(),
                "--platform".into(),
                format!("linux-x86_64=a\"b.tar.gz:{DIGEST_A}:1"),
            ],
        ),
        (
            "a platform listed twice",
            vec![
                "--version".into(),
                "0.3.0".into(),
                "--published".into(),
                "2026-12-01".into(),
                "--platform".into(),
                good_platform.clone(),
                "--platform".into(),
                good_platform.clone(),
            ],
        ),
        (
            "no platform",
            vec![
                "--version".into(),
                "0.3.0".into(),
                "--published".into(),
                "2026-12-01".into(),
            ],
        ),
        (
            "an unknown option",
            vec![
                "--version".into(),
                "0.3.0".into(),
                "--published".into(),
                "2026-12-01".into(),
                "--platform".into(),
                good_platform.clone(),
                "--force".into(),
                "yes".into(),
            ],
        ),
    ];
    for (index, (what, mut args)) in cases.into_iter().enumerate() {
        let out = scratch.path(&format!("never-{index}.json"));
        args.insert(0, "update-manifest".into());
        args.extend(["--out".into(), out.clone()]);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        run(&scratch, &args, &[]).refused();
        assert!(!Path::new(&out).exists(), "{what} still wrote a file");
    }
    // The missing --out is refused too.
    run(
        &scratch,
        &[
            "update-manifest",
            "--version",
            "0.3.0",
            "--published",
            "2026-12-01",
            "--platform",
            &good_platform,
        ],
        &[],
    )
    .refused();
}

#[test]
fn verify_accepts_a_manifest_signed_inside_the_test() {
    let fixture = Fixture::new();
    let good = fixture.verify(
        &fixture.manifest,
        &fixture.signature,
        &fixture.public_key,
        "0.3.0",
    );
    good.succeeded();
    assert_eq!(
        good.stdout.trim_end(),
        "update manifest for 0.3.0: signature good"
    );
    assert_eq!(good.stderr, "");
}

#[test]
fn verify_refuses_a_tampered_manifest() {
    let fixture = Fixture::new();
    let tampered = fixture.scratch.path("tampered.json");
    let original = fs::read_to_string(&fixture.manifest).unwrap();
    fs::write(&tampered, original.replace("123456789", "123456788")).unwrap();
    fixture
        .verify(&tampered, &fixture.signature, &fixture.public_key, "0.3.0")
        .refused();

    // A manifest with one more byte, and an empty one.
    let longer = fixture.scratch.path("longer.json");
    fs::write(&longer, format!("{original} ")).unwrap();
    fixture
        .verify(&longer, &fixture.signature, &fixture.public_key, "0.3.0")
        .refused();
    let empty = fixture.scratch.path("empty.json");
    fs::write(&empty, "").unwrap();
    fixture
        .verify(&empty, &fixture.signature, &fixture.public_key, "0.3.0")
        .refused();
}

#[test]
fn verify_refuses_a_wrong_version() {
    let fixture = Fixture::new();
    for version in ["0.4.0", "0.2.0", "0.3", "0.3.0-rc.1", "v0.3.0", ""] {
        fixture
            .verify(
                &fixture.manifest,
                &fixture.signature,
                &fixture.public_key,
                version,
            )
            .refused();
    }
    // A manifest of 0.2.0 correctly signed for 0.2.0 is not accepted as 0.3.0 either.
    let old_manifest = write_manifest(&fixture.scratch, "0.2.0");
    let old_signature = fixture.scratch.path("old.minisig");
    fs::write(
        &old_signature,
        sign_with(
            &fixture.secret_key,
            &fs::read(&old_manifest).unwrap(),
            "mercury-browser update 0.2.0",
        ),
    )
    .unwrap();
    fixture
        .verify(&old_manifest, &old_signature, &fixture.public_key, "0.2.0")
        .succeeded();
    fixture
        .verify(&old_manifest, &old_signature, &fixture.public_key, "0.3.0")
        .refused();
}

#[test]
fn verify_refuses_a_missing_file() {
    let fixture = Fixture::new();
    let missing = fixture.scratch.path("does-not-exist");
    fixture
        .verify(&missing, &fixture.signature, &fixture.public_key, "0.3.0")
        .refused();
    fixture
        .verify(&fixture.manifest, &missing, &fixture.public_key, "0.3.0")
        .refused();
    fixture
        .verify(&fixture.manifest, &fixture.signature, &missing, "0.3.0")
        .refused();
    // And a missing option.
    run(
        &fixture.scratch,
        &[
            "verify",
            "--manifest",
            &fixture.manifest,
            "--signature",
            &fixture.signature,
            "--version",
            "0.3.0",
        ],
        &[],
    )
    .refused();
}

#[test]
fn verify_refuses_a_garbage_or_damaged_signature() {
    let fixture = Fixture::new();
    let check = |signature: &str| {
        fixture
            .verify(&fixture.manifest, signature, &fixture.public_key, "0.3.0")
            .refused();
    };
    for (name, text) in [
        ("garbage.minisig", "garbage"),
        ("empty.minisig", ""),
        ("blank-lines.minisig", "\n\n\n\n"),
        (
            "lookalike.minisig",
            "untrusted comment: x\nAAAA\ntrusted comment: mercury-browser update 0.3.0\nAAAA\n",
        ),
    ] {
        let path = fixture.scratch.path(name);
        fs::write(&path, text).unwrap();
        check(&path);
    }
    check(&fixture.edited_signature("truncated.minisig", |text| {
        text[..text.len() / 2].to_owned()
    }));
    check(&fixture.edited_signature("no-global.minisig", |text| {
        text.lines().take(3).collect::<Vec<_>>().join("\n")
    }));
    check(&fixture.edited_signature("other-comment.minisig", |text| {
        text.replace("update 0.3.0", "update 0.9.9")
    }));
    // A legacy (non-prehashed) signature is refused even before its value is checked.
    check(&fixture.edited_signature("legacy.minisig", |text| text.replacen("\nRU", "\nRW", 1)));
    // A directory is not a signature.
    check(fixture.scratch.0.to_str().unwrap());
}

#[test]
fn verify_refuses_another_key_and_a_foreign_trusted_comment() {
    let fixture = Fixture::new();
    let other_public = fixture.scratch.path("other.pub");
    fs::write(&other_public, unprotected_key_pair().0).unwrap();
    fixture
        .verify(
            &fixture.manifest,
            &fixture.signature,
            &other_public,
            "0.3.0",
        )
        .refused();

    // The right key, a valid signature, but a comment that is not the release comment.
    let bytes = fs::read(&fixture.manifest).unwrap();
    for (index, comment) in [
        "timestamp:1700000000\tfile:update.json",
        "mercury-browser update 0.3.0-rc.1",
        "mercury-browser update 0.4.0",
        "mercury-browser update",
    ]
    .into_iter()
    .enumerate()
    {
        let signature = fixture.scratch.path(&format!("comment-{index}.minisig"));
        fs::write(&signature, sign_with(&fixture.secret_key, &bytes, comment)).unwrap();
        fixture
            .verify(&fixture.manifest, &signature, &fixture.public_key, "0.3.0")
            .refused();
    }
}

#[test]
fn sign_needs_the_key_and_a_password_in_the_environment() {
    let scratch = Scratch::new();
    let manifest = write_manifest(&scratch, "0.3.0");
    let (_, secret) = unprotected_key_pair();
    let key_body = secret.lines().nth(1).unwrap();
    const KEY_WORD: &str = "SENTINEL-KEY-4d2a91";
    const PASSWORD_WORD: &str = "SENTINEL-PASSWORD-7be310";
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("nothing set", vec![]),
        ("only the key", vec![("MINISIGN_SECRET_KEY", KEY_WORD)]),
        (
            "only the password",
            vec![("MINISIGN_PASSWORD", PASSWORD_WORD)],
        ),
        (
            "an empty key",
            vec![
                ("MINISIGN_SECRET_KEY", ""),
                ("MINISIGN_PASSWORD", PASSWORD_WORD),
            ],
        ),
        (
            "a key of white space",
            vec![
                ("MINISIGN_SECRET_KEY", " \n\t "),
                ("MINISIGN_PASSWORD", PASSWORD_WORD),
            ],
        ),
        (
            "an empty password",
            vec![("MINISIGN_SECRET_KEY", KEY_WORD), ("MINISIGN_PASSWORD", "")],
        ),
        (
            "a real key and no password",
            vec![("MINISIGN_SECRET_KEY", secret.as_str())],
        ),
        (
            "a real key and an empty password",
            vec![
                ("MINISIGN_SECRET_KEY", secret.as_str()),
                ("MINISIGN_PASSWORD", ""),
            ],
        ),
    ];
    for (index, (what, env)) in cases.into_iter().enumerate() {
        let out = scratch.path(&format!("never-{index}.minisig"));
        let result = run(
            &scratch,
            &[
                "sign",
                "--manifest",
                &manifest,
                "--version",
                "0.3.0",
                "--out",
                &out,
            ],
            &env,
        );
        result.refused();
        result.says_nothing_of(&[KEY_WORD, PASSWORD_WORD, key_body]);
        assert!(!Path::new(&out).exists(), "{what} still wrote a signature");
    }
}

#[test]
fn sign_with_a_bad_key_or_password_refuses_without_echoing_either() {
    let scratch = Scratch::new();
    let manifest = write_manifest(&scratch, "0.3.0");
    let (_, unprotected) = unprotected_key_pair();
    let (_, protected) = protected_key_pair();
    const WORD: &str = "SENTINEL-WORD-91c5e0";
    let secrets = [
        WORD,
        PASSWORD,
        unprotected.lines().nth(1).unwrap(),
        protected.lines().nth(1).unwrap(),
    ];
    let cases = [
        ("garbage as the key", WORD.to_owned(), WORD),
        (
            "a key-shaped line of garbage",
            format!("untrusted comment: x\n{WORD}\n"),
            WORD,
        ),
        (
            "a key without a password protecting it",
            unprotected.clone(),
            WORD,
        ),
        ("the wrong password", protected.clone(), WORD),
        (
            "a near miss of the password",
            protected.clone(),
            "correct horse battery stapl",
        ),
    ];
    for (index, (what, key, password)) in cases.into_iter().enumerate() {
        let out = scratch.path(&format!("never-{index}.minisig"));
        let result = run(
            &scratch,
            &[
                "sign",
                "--manifest",
                &manifest,
                "--version",
                "0.3.0",
                "--out",
                &out,
            ],
            &[
                ("MINISIGN_SECRET_KEY", &key),
                ("MINISIGN_PASSWORD", password),
            ],
        );
        result.refused();
        result.says_nothing_of(&secrets);
        assert!(!Path::new(&out).exists(), "{what} still wrote a signature");
    }
}

#[test]
fn sign_refuses_a_manifest_it_did_not_write_or_for_another_version() {
    let scratch = Scratch::new();
    let manifest = write_manifest(&scratch, "0.3.0");
    let (_, protected) = protected_key_pair();
    let env = [
        ("MINISIGN_SECRET_KEY", protected.as_str()),
        ("MINISIGN_PASSWORD", PASSWORD),
    ];

    let out = scratch.path("wrong-version.minisig");
    run(
        &scratch,
        &[
            "sign",
            "--manifest",
            &manifest,
            "--version",
            "0.4.0",
            "--out",
            &out,
        ],
        &env,
    )
    .refused();
    assert!(!Path::new(&out).exists());

    // The head says 0.3.0, but a second "version" key follows it.
    let doctored = scratch.path("doctored.json");
    let original = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &doctored,
        original.replace(
            "  \"platforms\"",
            "  \"version\": \"0.1.0\",\n  \"platforms\"",
        ),
    )
    .unwrap();
    let out = scratch.path("doctored.minisig");
    run(
        &scratch,
        &[
            "sign",
            "--manifest",
            &doctored,
            "--version",
            "0.3.0",
            "--out",
            &out,
        ],
        &env,
    )
    .refused();
    assert!(!Path::new(&out).exists());

    let out = scratch.path("missing.minisig");
    run(
        &scratch,
        &[
            "sign",
            "--manifest",
            &scratch.path("no-such-manifest.json"),
            "--version",
            "0.3.0",
            "--out",
            &out,
        ],
        &env,
    )
    .refused();
    assert!(!Path::new(&out).exists());
}

#[test]
fn sign_with_a_protected_key_makes_a_signature_that_verify_accepts() {
    let scratch = Scratch::new();
    let manifest = write_manifest(&scratch, "0.3.0");
    let (public, protected) = protected_key_pair();
    let public_key = scratch.path("update-key.pub");
    fs::write(&public_key, public).unwrap();
    let signature = scratch.path("update.json.minisig");
    let env = [
        ("MINISIGN_SECRET_KEY", protected.as_str()),
        ("MINISIGN_PASSWORD", PASSWORD),
    ];
    let args = [
        "sign",
        "--manifest",
        manifest.as_str(),
        "--version",
        "0.3.0",
        "--out",
        signature.as_str(),
    ];
    let signed = run(&scratch, &args, &env);
    signed.succeeded();
    assert_eq!((signed.stdout.as_str(), signed.stderr.as_str()), ("", ""));
    signed.says_nothing_of(&[PASSWORD, protected.lines().nth(1).unwrap()]);

    let text = fs::read_to_string(&signature).unwrap();
    assert!(
        text.contains("\ntrusted comment: mercury-browser update 0.3.0\n"),
        "{text}"
    );
    let verified = run(
        &scratch,
        &[
            "verify",
            "--manifest",
            &manifest,
            "--signature",
            &signature,
            "--public-key",
            &public_key,
            "--version",
            "0.3.0",
        ],
        &[],
    );
    verified.succeeded();
    assert!(verified.stdout.contains("signature good"), "{verified:?}");

    // A second signature never replaces the first.
    let again = run(&scratch, &args, &env);
    again.refused();
    assert!(again.stderr.contains("cannot create"), "{again:?}");
    assert_eq!(fs::read_to_string(&signature).unwrap(), text);
}

#[test]
fn keygen_never_replaces_a_key_file() {
    let scratch = Scratch::new();
    let public = scratch.path("update-key.pub");
    let secret = scratch.path("update-key.key");
    fs::write(&public, "the committed public key").unwrap();
    let result = run(
        &scratch,
        &["keygen", "--public-key", &public, "--secret-key", &secret],
        &[],
    );
    result.refused();
    assert!(result.stderr.contains("already exists"), "{result:?}");
    assert_eq!(
        fs::read_to_string(&public).unwrap(),
        "the committed public key"
    );
    assert!(!Path::new(&secret).exists());

    fs::remove_file(&public).unwrap();
    fs::write(&secret, "the old secret key").unwrap();
    run(
        &scratch,
        &["keygen", "--public-key", &public, "--secret-key", &secret],
        &[],
    )
    .refused();
    assert_eq!(fs::read_to_string(&secret).unwrap(), "the old secret key");
    assert!(!Path::new(&public).exists());

    // One file for both keys is refused up front, before any password is asked for.
    let same = scratch.path("both");
    run(
        &scratch,
        &["keygen", "--public-key", &same, "--secret-key", &same],
        &[],
    )
    .refused();
    assert!(!Path::new(&same).exists());
}

#[test]
fn usage_errors_and_help() {
    let scratch = Scratch::new();
    let none = run(&scratch, &[], &[]);
    none.refused();
    assert!(none.stderr.contains("usage:"), "{none:?}");

    let unknown = run(&scratch, &["frobnicate"], &[]);
    unknown.refused();
    assert!(unknown.stderr.contains("unknown command"), "{unknown:?}");

    for flag in ["--help", "-h"] {
        let help = run(&scratch, &[flag], &[]);
        help.succeeded();
        assert!(help.stdout.contains("usage:"), "{help:?}");
        assert_eq!(help.stderr, "");
    }
}
