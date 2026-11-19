// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Helpers shared by the PDFium tests: finding the library (and skipping without it), a lock
//! that keeps the tests from using PDFium at the same time, scratch files, pixel counting, and a
//! generator for a password-protected PDF.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use mercury_pdf::backend::PdfBackend;
use mercury_pdf::types::{DocumentInfo, Rect, Rotation, Tile, TileRequest};
use mercury_pdfium::{PdfiumBackend, locate_library};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The PDFium library, or `None` after saying the test is skipped. With
/// `MERCURY_REQUIRE_PDFIUM=1` a missing library fails the test instead.
pub fn library() -> Option<PathBuf> {
    match locate_library() {
        Some(library) => Some(library),
        None => {
            if std::env::var_os("MERCURY_REQUIRE_PDFIUM").is_some_and(|value| value == "1") {
                panic!("MERCURY_REQUIRE_PDFIUM=1 and no PDFium library was found");
            }
            eprintln!("skipped: no PDFium library");
            None
        }
    }
}

/// What a PDFium test holds while it runs: the lock (PDFium is used by one test at a time, as it
/// is by one thread in the helper) and the backend.
pub struct Session {
    pub backend: PdfiumBackend,
    _lock: MutexGuard<'static, ()>,
}

/// A backend with the library loaded, or `None` (skipped) without the library.
pub fn session() -> Option<Session> {
    let library = library()?;
    let lock = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let backend = PdfiumBackend::load(&library).expect("the PDFium library loads");
    Some(Session {
        backend,
        _lock: lock,
    })
}

/// `tests/pages/pdf/<name>` of the repository.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/pages/pdf")
        .join(name)
}

/// A scratch directory removed when it is dropped.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("mercury-pdfium-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Self(dir)
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("a scratch file");
        path
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opens a document and returns what the backend says about it.
pub fn open(backend: &mut PdfiumBackend, path: &Path) -> DocumentInfo {
    backend
        .open(path, None)
        .unwrap_or_else(|error| panic!("{} did not open: {error:?}", path.display()))
}

/// The tile for the rectangle `rect` (page space, in points) of a page at `scale`, with a margin
/// of `margin` pixels, and the pixel position of its top-left corner on the whole rendered page.
pub fn tile_around(
    backend: &mut PdfiumBackend,
    info: &DocumentInfo,
    page: u32,
    rect: &Rect,
    scale: f32,
    rotation: Rotation,
    margin: u32,
) -> (Tile, (u32, u32)) {
    let (page_width, page_height) = info.pages[page as usize].pixel_size(scale, rotation);
    let clamp = |value: f32, limit: u32| (value.max(0.0) as u32).min(limit);
    let x = clamp((rect.left * scale).floor(), page_width).saturating_sub(margin);
    let y = clamp((rect.top * scale).floor(), page_height).saturating_sub(margin);
    let right = (clamp((rect.right * scale).ceil(), page_width) + margin).min(page_width);
    let bottom = (clamp((rect.bottom * scale).ceil(), page_height) + margin).min(page_height);
    let request = TileRequest {
        page,
        scale,
        rotation,
        x,
        y,
        width: (right - x).max(1),
        height: (bottom - y).max(1),
    };
    let tile = backend.render(&request).expect("the tile renders");
    (tile, (x, y))
}

/// The whole page as one tile (only for pages small enough for the tile limits).
pub fn whole_page(
    backend: &mut PdfiumBackend,
    info: &DocumentInfo,
    page: u32,
    scale: f32,
    rotation: Rotation,
) -> Tile {
    let (width, height) = info.pages[page as usize].pixel_size(scale, rotation);
    backend
        .render(&TileRequest {
            page,
            scale,
            rotation,
            x: 0,
            y: 0,
            width,
            height,
        })
        .expect("the page renders")
}

pub fn pixel(tile: &Tile, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * tile.width + x) * 4) as usize;
    [
        tile.rgba[at],
        tile.rgba[at + 1],
        tile.rgba[at + 2],
        tile.rgba[at + 3],
    ]
}

/// The number of dark pixels (average of the three colour channels below 128) in the part of the
/// tile that lies inside `region` (`[left, top, right, bottom]` in tile pixels).
pub fn dark_pixels(tile: &Tile, region: [u32; 4]) -> usize {
    let mut count = 0;
    for y in region[1]..region[3].min(tile.height) {
        for x in region[0]..region[2].min(tile.width) {
            let [r, g, b, _] = pixel(tile, x, y);
            if (u32::from(r) + u32::from(g) + u32::from(b)) / 3 < 128 {
                count += 1;
            }
        }
    }
    count
}

/// The pixels of `rect` (page space, points) of a tile whose top-left is the page pixel `origin`
/// at `scale`, as a pixel region of the tile.
pub fn region_of(rect: &Rect, origin: (u32, u32), scale: f32) -> [u32; 4] {
    let to_tile =
        |value: f32, offset: u32| ((value * scale) as i64 - i64::from(offset)).max(0) as u32;
    [
        to_tile(rect.left, origin.0),
        to_tile(rect.top, origin.1),
        to_tile(rect.right, origin.0) + 1,
        to_tile(rect.bottom, origin.1) + 1,
    ]
}

// ---------------------------------------------------------------------------------------------
// A password-protected PDF: the standard security handler, revision 2 (RC4, 40-bit key).
// ---------------------------------------------------------------------------------------------

const PASSWORD_PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

fn pad_password(password: &str) -> [u8; 32] {
    let mut padded = [0_u8; 32];
    let bytes = password.as_bytes();
    let used = bytes.len().min(32);
    padded[..used].copy_from_slice(&bytes[..used]);
    padded[used..].copy_from_slice(&PASSWORD_PADDING[..32 - used]);
    padded
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut state: Vec<u8> = (0..=255).collect();
    let mut j = 0_usize;
    for i in 0..256 {
        j = (j + usize::from(state[i]) + usize::from(key[i % key.len()])) % 256;
        state.swap(i, j);
    }
    let (mut i, mut j) = (0_usize, 0_usize);
    data.iter()
        .map(|byte| {
            i = (i + 1) % 256;
            j = (j + usize::from(state[i])) % 256;
            state.swap(i, j);
            byte ^ state[(usize::from(state[i]) + usize::from(state[j])) % 256]
        })
        .collect()
}

/// MD5 (RFC 1321), for the key derivation of the old security handlers.
fn md5(message: &[u8]) -> [u8; 16] {
    const SHIFTS: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let constants: Vec<u32> = (0..64)
        .map(|index| ((f64::from(index + 1)).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut data = message.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&((message.len() as u64) * 8).to_le_bytes());
    let (mut a0, mut b0, mut c0, mut d0) = (
        0x6745_2301_u32,
        0xEFCD_AB89_u32,
        0x98BA_DCFE_u32,
        0x1032_5476_u32,
    );
    for chunk in data.as_chunks::<64>().0 {
        let words: Vec<u32> = chunk
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for index in 0..64 {
            let (mixed, word) = match index / 16 {
                0 => ((b & c) | (!b & d), index),
                1 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                2 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let sum = a
                .wrapping_add(mixed)
                .wrapping_add(constants[index])
                .wrapping_add(words[word]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(sum.rotate_left(SHIFTS[index]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut digest = [0_u8; 16];
    for (slot, word) in digest
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip([a0, b0, c0, d0])
    {
        slot.copy_from_slice(&word.to_le_bytes());
    }
    digest
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// A one-page PDF whose content stream is RC4-encrypted; opening it needs `password` (the user
/// password; the owner password is a different word).
pub fn encrypted_pdf(password: &str) -> Vec<u8> {
    let identifier = [0x11_u8; 16];
    let owner_key = &md5(&pad_password("the owner's word"))[..5];
    let owner_entry = rc4(owner_key, &pad_password(password));
    let permissions: i32 = -4;
    let mut key_input = pad_password(password).to_vec();
    key_input.extend_from_slice(&owner_entry);
    key_input.extend_from_slice(&permissions.to_le_bytes());
    key_input.extend_from_slice(&identifier);
    let key = md5(&key_input)[..5].to_vec();
    let user_entry = rc4(&key, &PASSWORD_PADDING);

    let content_number = 4_u32;
    let mut object_key_input = key.clone();
    object_key_input.extend_from_slice(&content_number.to_le_bytes()[..3]);
    object_key_input.extend_from_slice(&[0, 0]);
    let object_key = md5(&object_key_input)[..10].to_vec();
    let plain = b"BT /F1 24 Tf 20 40 Td (Secret page) Tj ET\n";
    let encrypted = rc4(&object_key, plain);

    let mut objects: Vec<(u32, Vec<u8>)> = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
                .to_vec(),
        ),
    ];
    let mut stream = format!("<< /Length {} >>\nstream\n", encrypted.len()).into_bytes();
    stream.extend_from_slice(&encrypted);
    stream.extend_from_slice(b"\nendstream");
    objects.push((4, stream));
    objects.push((
        5,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ));
    objects.push((
        6,
        format!(
            "<< /Filter /Standard /V 1 /R 2 /O <{}> /U <{}> /P {permissions} >>",
            hex(&owner_entry),
            hex(&user_entry)
        )
        .into_bytes(),
    ));
    let mut file = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (number, body) in &objects {
        offsets.push(file.len());
        file.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        file.extend_from_slice(body);
        file.extend_from_slice(b"\nendobj\n");
    }
    let xref = file.len();
    file.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in &offsets {
        file.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    file.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Encrypt 6 0 R /ID [<{id}> <{id}>] >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1,
            id = hex(&identifier)
        )
        .as_bytes(),
    );
    file
}

#[cfg(test)]
mod self_tests {
    use super::*;

    #[test]
    fn md5_matches_the_published_test_vectors() {
        assert_eq!(hex(&md5(b"")), "D41D8CD98F00B204E9800998ECF8427E");
        assert_eq!(hex(&md5(b"abc")), "900150983CD24FB0D6963F7D28E17F72");
        assert_eq!(
            hex(&md5(b"The quick brown fox jumps over the lazy dog")),
            "9E107D9D372BB6826BD81D3542A419D6"
        );
    }

    #[test]
    fn rc4_matches_a_published_test_vector() {
        assert_eq!(hex(&rc4(b"Key", b"Plaintext")), "BBF316E8D940AF0AD3");
    }
}
