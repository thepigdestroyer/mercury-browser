// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Helpers shared by the `package_*` tests: hand-written zip files (so a test can declare any size, name,
//! flag or method), ordinary zip files, a deterministic random source, and the test PKI that signs
//! XPIs.
#![allow(dead_code)]

pub mod pki;

use std::io::{Cursor, Write};

/// CRC-32 (IEEE), bit by bit: only used for small test inputs.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A deterministic xorshift generator (no `rand`).
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n.max(1) as u64)).unwrap_or(0)
    }
}

/// One entry of a hand-written zip. Every field of both headers can be set to a lie.
#[derive(Clone)]
pub struct RawEntry {
    pub name: Vec<u8>,
    /// The bytes written after the local header (already compressed, if `method` says so).
    pub data: Vec<u8>,
    pub method: u16,
    pub crc: u32,
    pub declared_compressed: u32,
    pub declared_size: u32,
    pub flags: u16,
    pub external_attributes: u32,
    pub version_made_by: u16,
    pub dos_date: u16,
}

impl RawEntry {
    /// A stored, regular entry with honest headers.
    pub fn stored(name: &str, data: &[u8]) -> RawEntry {
        let len = u32::try_from(data.len()).unwrap();
        RawEntry {
            name: name.as_bytes().to_vec(),
            data: data.to_vec(),
            method: 0,
            crc: crc32(data),
            declared_compressed: len,
            declared_size: len,
            flags: 0,
            external_attributes: 0o100_644 << 16,
            version_made_by: 0x031E,
            dos_date: 0x0021,
        }
    }

    /// A deflated entry with honest headers.
    pub fn deflated(name: &str, data: &[u8]) -> RawEntry {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        let compressed = encoder.finish().unwrap();
        RawEntry {
            method: 8,
            declared_compressed: u32::try_from(compressed.len()).unwrap(),
            data: compressed,
            ..RawEntry::stored(name, data)
        }
    }

    pub fn raw_name(mut self, name: &[u8]) -> RawEntry {
        self.name = name.to_vec();
        self
    }

    pub fn directory(name: &str) -> RawEntry {
        RawEntry {
            external_attributes: (0o040_755 << 16) | 0x10,
            ..RawEntry::stored(name, b"")
        }
    }

    pub fn symlink(name: &str, target: &str) -> RawEntry {
        RawEntry {
            external_attributes: 0o120_777 << 16,
            ..RawEntry::stored(name, target.as_bytes())
        }
    }

    pub fn mode(mut self, mode: u32) -> RawEntry {
        self.external_attributes = mode << 16;
        self
    }
}

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Local headers, central directory and end record, written exactly as the entries say.
pub fn raw_zip(entries: &[RawEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for e in entries {
        offsets.push(u32::try_from(out.len()).unwrap());
        le32(&mut out, 0x0403_4B50);
        le16(&mut out, 20);
        le16(&mut out, e.flags);
        le16(&mut out, e.method);
        le16(&mut out, 0);
        le16(&mut out, e.dos_date);
        le32(&mut out, e.crc);
        le32(&mut out, e.declared_compressed);
        le32(&mut out, e.declared_size);
        le16(&mut out, u16::try_from(e.name.len()).unwrap());
        le16(&mut out, 0);
        out.extend_from_slice(&e.name);
        out.extend_from_slice(&e.data);
    }
    let directory_start = u32::try_from(out.len()).unwrap();
    for (e, offset) in entries.iter().zip(&offsets) {
        le32(&mut out, 0x0201_4B50);
        le16(&mut out, e.version_made_by);
        le16(&mut out, 20);
        le16(&mut out, e.flags);
        le16(&mut out, e.method);
        le16(&mut out, 0);
        le16(&mut out, e.dos_date);
        le32(&mut out, e.crc);
        le32(&mut out, e.declared_compressed);
        le32(&mut out, e.declared_size);
        le16(&mut out, u16::try_from(e.name.len()).unwrap());
        le16(&mut out, 0);
        le16(&mut out, 0);
        le16(&mut out, 0);
        le16(&mut out, 0);
        le32(&mut out, e.external_attributes);
        le32(&mut out, *offset);
        out.extend_from_slice(&e.name);
    }
    let directory_len = u32::try_from(out.len()).unwrap() - directory_start;
    le32(&mut out, 0x0605_4B50);
    le16(&mut out, 0);
    le16(&mut out, 0);
    let count = u16::try_from(entries.len()).unwrap();
    le16(&mut out, count);
    le16(&mut out, count);
    le32(&mut out, directory_len);
    le32(&mut out, directory_start);
    le16(&mut out, 0);
    out
}

/// An ordinary deflated zip of files, written by the `zip` crate.
pub fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let owned: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(n, d)| ((*n).to_string(), d.to_vec()))
        .collect();
    zip_files(&owned)
}

pub fn zip_files(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (name, data) in files {
        writer.start_file(name.as_str(), options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// A temporary directory that is removed when dropped.
pub fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
