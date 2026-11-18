// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `.crate` archives: checksums and unpacking into memory.

use crate::error::{Context, Result};
use crate::fail;
use crate::tree::{Tree, check_relative_path};
use sha2::{Digest, Sha256};
use std::io::Read;

/// The lowercase hexadecimal SHA-256 of `bytes`, as `Cargo.lock` writes it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Fails unless `bytes` has the SHA-256 `expected`.
pub fn verify(bytes: &[u8], expected: &str, what: &str) -> Result<()> {
    let actual = sha256_hex(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(fail!(
            "{what}: SHA-256 is {actual}, expected {expected}; the archive is not the published crate"
        ))
    }
}

/// Unpacks a `.crate` archive (a gzipped tar whose entries all sit under
/// `<name>-<version>/`) into memory, without the prefix. Only regular files are
/// accepted: a link, a device or a path that leaves the prefix is an error.
pub fn unpack(archive: &[u8], name: &str, version: &str) -> Result<Tree> {
    let what = || format!("{name} {version}: reading the .crate archive");
    let prefix = format!("{name}-{version}/");
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut tree = Tree::default();
    for entry in tar.entries().context(what)? {
        let mut entry = entry.context(what)?;
        let kind = entry.header().entry_type();
        let raw = String::from_utf8(entry.path_bytes().into_owned())
            .map_err(|_| fail!("{}: an entry's path is not UTF-8", what()))?;
        if kind.is_pax_global_extensions() {
            continue;
        }
        let Some(path) = raw.strip_prefix(&prefix) else {
            return Err(fail!("{}: entry {raw:?} is outside {prefix}", what()));
        };
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(fail!(
                "{}: entry {raw:?} is not a regular file ({kind:?})",
                what()
            ));
        }
        let path = path.trim_end_matches('/');
        check_relative_path(path).map_err(|error| fail!("{}: {error}", what()))?;
        let executable = entry.header().mode().context(what)? & 0o111 != 0;
        let mut data = Vec::new();
        entry.read_to_end(&mut data).context(what)?;
        if tree.files.insert(path.to_owned(), data).is_some() {
            return Err(fail!("{}: {path} appears twice", what()));
        }
        if executable {
            tree.executable.insert(path.to_owned());
        }
    }
    if tree.files.is_empty() {
        return Err(fail!("{}: the archive is empty", what()));
    }
    Ok(tree)
}

#[cfg(test)]
pub mod fixture {
    //! Small `.crate` archives built in memory for tests.
    use flate2::Compression;
    use flate2::write::GzEncoder;

    /// A `.crate` archive of `files` under `<name>-<version>/`.
    pub fn pack(name: &str, version: &str, files: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for (path, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(if path.ends_with(".sh") { 0o755 } else { 0o644 });
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    format!("{name}-{version}/{path}"),
                    contents.as_bytes(),
                )
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// A tar with one raw entry, for the cases `tar::Builder` refuses to write.
    pub fn pack_raw(path: &str, kind: tar::EntryType) -> Vec<u8> {
        let mut header = tar::Header::new_old();
        header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
        header.set_entry_type(kind);
        header.set_size(0);
        header.set_mode(0o644);
        header.set_cksum();
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend_from_slice(&[0; 1024]);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, &bytes).unwrap();
        encoder.finish().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{pack, pack_raw};
    use super::*;

    #[test]
    fn checksum_is_lowercase_hex_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_rejects_a_different_archive() {
        let error = verify(b"abc", &"0".repeat(64), "demo 1.0.0").unwrap_err();
        assert!(error.0.contains("not the published crate"), "{error}");
        verify(b"abc", &sha256_hex(b"abc"), "demo 1.0.0").unwrap();
    }

    #[test]
    fn unpack_strips_the_prefix_and_keeps_bytes_and_modes() {
        let archive = pack(
            "demo",
            "1.0.0",
            &[("src/lib.rs", "pub fn f() {}\n"), ("run.sh", "#!/bin/sh\n")],
        );
        let tree = unpack(&archive, "demo", "1.0.0").unwrap();
        assert_eq!(tree.files.len(), 2);
        assert_eq!(tree.files["src/lib.rs"], b"pub fn f() {}\n");
        assert!(tree.executable.contains("run.sh"));
        assert!(!tree.executable.contains("src/lib.rs"));
    }

    #[test]
    fn unpack_rejects_entries_outside_the_prefix() {
        let archive = pack("other", "1.0.0", &[("src/lib.rs", "")]);
        let error = unpack(&archive, "demo", "1.0.0").unwrap_err();
        assert!(error.0.contains("outside demo-1.0.0/"), "{error}");
    }

    #[test]
    fn unpack_rejects_links_and_parent_directories() {
        let link = pack_raw("demo-1.0.0/link", tar::EntryType::Symlink);
        assert!(
            unpack(&link, "demo", "1.0.0")
                .unwrap_err()
                .0
                .contains("not a regular file")
        );
        let escape = pack_raw("demo-1.0.0/../escape", tar::EntryType::Regular);
        assert!(
            unpack(&escape, "demo", "1.0.0")
                .unwrap_err()
                .0
                .contains("..")
        );
    }
}
