// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `mercury-release`: writes, signs and checks the update manifest of a release.
//!
//! ```text
//! mercury-release update-manifest --version 0.3.0 --published 2026-12-01 \
//!     --platform macos-arm64=NAME:SHA256:SIZE --platform linux-x86_64=NAME:SHA256:SIZE \
//!     --out update.json
//! mercury-release sign --manifest update.json --version 0.3.0 --out update.json.minisig
//!     (the key in $MINISIGN_SECRET_KEY, its password in $MINISIGN_PASSWORD)
//! mercury-release verify --manifest update.json --signature update.json.minisig \
//!     --public-key apps/mercury-desktop/resources/update-key.pub --version 0.3.0
//! mercury-release keygen --public-key update-key.pub --secret-key update-key.key
//!     (asks for the password twice)
//! ```

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use mercury_release::{Asset, Release};

const USAGE: &str = "usage:
  mercury-release update-manifest --version V --published YYYY-MM-DD --platform NAME=FILE:SHA256:SIZE... --out FILE
  mercury-release sign --manifest FILE --version V --out FILE   (key in $MINISIGN_SECRET_KEY, password in $MINISIGN_PASSWORD)
  mercury-release verify --manifest FILE --signature FILE --public-key FILE --version V
  mercury-release keygen --public-key FILE --secret-key FILE";

/// Command-line options: `--name value` pairs, `--platform` may repeat.
struct Options {
    values: HashMap<String, String>,
    platforms: Vec<(String, String)>,
}

impl Options {
    fn parse(args: &[String], allowed: &[&str]) -> Result<Self, String> {
        let mut values = HashMap::new();
        let mut platforms = Vec::new();
        let mut rest = args.iter();
        while let Some(flag) = rest.next() {
            let name = flag
                .strip_prefix("--")
                .filter(|name| allowed.contains(name))
                .ok_or_else(|| format!("unknown option '{flag}'"))?;
            let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
            if name == "platform" {
                let (platform, spec) = value
                    .split_once('=')
                    .ok_or_else(|| format!("--platform '{value}' is not NAME=FILE:SHA256:SIZE"))?;
                platforms.push((platform.to_owned(), spec.to_owned()));
            } else if values.insert(name.to_owned(), value.clone()).is_some() {
                return Err(format!("{flag} given twice"));
            }
        }
        Ok(Self { values, platforms })
    }

    fn get(&self, name: &str) -> Result<&str, String> {
        self.values
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("--{name} is required"))
    }
}

fn read(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))
}

/// Writes a new file; refuses to replace one, so a key or a manifest is never overwritten.
fn write_new(path: &str, contents: &str, private: bool) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if private { 0o600 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options
        .open(Path::new(path))
        .map_err(|error| format!("cannot create {path}: {error}"))?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot write {path}: {error}"))
}

fn update_manifest(args: &[String]) -> Result<(), String> {
    let options = Options::parse(args, &["version", "published", "platform", "out"])?;
    let platforms = options
        .platforms
        .iter()
        .map(|(platform, spec)| Ok((platform.clone(), Asset::parse(spec)?)))
        .collect::<Result<Vec<_>, mercury_release::Error>>()
        .map_err(|error| error.to_string())?;
    let release = Release {
        version: options.get("version")?.to_owned(),
        published: options.get("published")?.to_owned(),
        platforms,
    };
    let json = mercury_release::manifest_json(&release).map_err(|error| error.to_string())?;
    write_new(options.get("out")?, &json, false)
}

fn sign(args: &[String]) -> Result<(), String> {
    let options = Options::parse(args, &["manifest", "version", "out"])?;
    let secret_key = std::env::var("MINISIGN_SECRET_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .ok_or("MINISIGN_SECRET_KEY is not set")?;
    let password = std::env::var("MINISIGN_PASSWORD")
        .ok()
        .filter(|password| !password.is_empty())
        .ok_or("MINISIGN_PASSWORD is not set; the release key must be password-protected")?;
    let manifest = read(options.get("manifest")?)?;
    let signature = mercury_release::sign_manifest(
        manifest.as_bytes(),
        options.get("version")?,
        &secret_key,
        Some(password),
    )
    .map_err(|error| error.to_string())?;
    write_new(options.get("out")?, &signature, false)
}

fn verify(args: &[String]) -> Result<(), String> {
    let options = Options::parse(args, &["manifest", "signature", "public-key", "version"])?;
    let manifest = read(options.get("manifest")?)?;
    let signature = read(options.get("signature")?)?;
    let public_key = read(options.get("public-key")?)?;
    let version = options.get("version")?;
    mercury_release::verify_manifest(manifest.as_bytes(), &signature, &public_key, version)
        .map_err(|error| error.to_string())?;
    println!("update manifest for {version}: signature good");
    Ok(())
}

fn keygen(args: &[String]) -> Result<(), String> {
    let options = Options::parse(args, &["public-key", "secret-key"])?;
    let public_path = options.get("public-key")?;
    let secret_path = options.get("secret-key")?;
    for path in [public_path, secret_path] {
        if Path::new(path).exists() {
            return Err(format!("{path} already exists; not replacing a key"));
        }
    }
    let password = rpassword::prompt_password("Password for the new secret key: ")
        .map_err(|error| format!("cannot read the password: {error}"))?;
    if password.is_empty() {
        return Err("the secret key needs a password".into());
    }
    let again = rpassword::prompt_password("The same password again: ")
        .map_err(|error| format!("cannot read the password: {error}"))?;
    if again != password {
        return Err("the passwords differ".into());
    }
    eprintln!("deriving the key from the password (a few seconds, about 1 GB of memory)...");
    let pair = minisign::KeyPair::generate_encrypted_keypair(Some(password))
        .map_err(|error| error.to_string())?;
    let public = pair
        .pk
        .to_box()
        .map_err(|error| error.to_string())?
        .into_string();
    let secret = pair
        .sk
        .to_box(Some("Mercury Browser update signing key"))
        .map_err(|error| error.to_string())?
        .into_string();
    write_new(secret_path, &secret, true)?;
    write_new(public_path, &public, false)?;
    println!(
        "public key: {public_path}\nsecret key: {secret_path} (keep it offline; never commit it)"
    );
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.split_first() {
        Some((command, rest)) => match command.as_str() {
            "update-manifest" => update_manifest(rest),
            "sign" => sign(rest),
            "verify" => verify(rest),
            "keygen" => keygen(rest),
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => Err(format!("unknown command '{other}'\n{USAGE}")),
        },
        None => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("mercury-release: {message}");
            ExitCode::from(2)
        }
    }
}
