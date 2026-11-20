// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! VideoToolbox's own answer to "can this machine decode this codec in hardware?"
//! (`VTIsHardwareDecodeSupported`), so that a decoder that fails in T07 can be compared with what the
//! operating system says about the chip it runs on, instead of with what the chip is assumed to be.
//!
//! One plain C function that takes a four-character code by value; no pointers, no Objective-C. The
//! spike only runs on macOS (the bundled GStreamer is a macOS framework); elsewhere the answer is `None`.

use serde_json::{Map, Value, json};

/// A four-character code as the `u32` CoreMedia uses (`CMVideoCodecType`).
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

/// The codecs T07 cares about, with their CoreMedia four-character codes.
pub const CODECS: &[(&str, u32)] = &[
    ("avc1 (H.264)", fourcc(b"avc1")),
    ("hvc1 (HEVC)", fourcc(b"hvc1")),
    ("vp09 (VP9)", fourcc(b"vp09")),
    ("av01 (AV1)", fourcc(b"av01")),
];

#[cfg(target_os = "macos")]
#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    /// `Boolean VTIsHardwareDecodeSupported(CMVideoCodecType codecType)`; `Boolean` is an `unsigned char`.
    fn VTIsHardwareDecodeSupported(codec_type: u32) -> u8;
}

/// Whether VideoToolbox reports a hardware decoder for `codec` on this machine; `None` off macOS.
#[cfg(target_os = "macos")]
pub fn hardware_decode_supported(codec: u32) -> Option<bool> {
    // SAFETY: a plain C function of the system's VideoToolbox framework; it takes the code by value.
    Some(unsafe { VTIsHardwareDecodeSupported(codec) } != 0)
}

#[cfg(not(target_os = "macos"))]
pub fn hardware_decode_supported(_codec: u32) -> Option<bool> {
    None
}

/// `{ "avc1 (H.264)": true, ... }` for [`CODECS`] (`null` where the answer is not available).
pub fn support_table() -> Value {
    let mut table = Map::new();
    for (name, code) in CODECS {
        table.insert((*name).to_owned(), json!(hardware_decode_supported(*code)));
    }
    Value::Object(table)
}

/// One line for notes: `avc1 (H.264) yes, hvc1 (HEVC) yes, vp09 (VP9) no, av01 (AV1) no`.
pub fn support_line() -> String {
    CODECS
        .iter()
        .map(|(name, code)| {
            let answer = match hardware_decode_supported(*code) {
                Some(true) => "yes",
                Some(false) => "no",
                None => "unknown",
            };
            format!("{name} {answer}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_character_codes_are_big_endian_ascii() {
        assert_eq!(fourcc(b"av01"), 0x6176_3031);
        assert_eq!(fourcc(b"avc1"), 0x6176_6331);
        assert_eq!(fourcc(b"hvc1"), 0x6876_6331);
        assert_eq!(fourcc(b"vp09"), 0x7670_3039);
    }

    #[test]
    fn the_table_names_every_codec_once() {
        let table = support_table();
        let object = table.as_object().unwrap();
        assert_eq!(object.len(), CODECS.len());
        assert!(object.contains_key("av01 (AV1)"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_answers_for_every_codec() {
        // The answers differ per machine (a virtual machine may say yes where the chip has no engine);
        // what must hold is that the call returns a yes or a no and the line says so.
        for (_, code) in CODECS {
            assert!(hardware_decode_supported(*code).is_some());
        }
        let line = support_line();
        assert!(line.contains("av01 (AV1) yes") || line.contains("av01 (AV1) no"));
    }
}
