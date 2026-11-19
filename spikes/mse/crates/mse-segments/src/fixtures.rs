// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The committed fixtures (`spikes/mse/fixtures/`, made by `gen.sh`, listed with their SHA-256 in
//! `SHA256SUMS` and described in `README.md`).

use std::path::PathBuf;

use crate::{Container, Fixture, MediaKind, SegmentedStream, SplitError, split};

/// Every committed fixture. The `mime` of each is what `split` derives from the file, and `duration_ms`
/// is first start to last end of its media segments; a test keeps both equal to the files.
pub const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "h264-low",
        path: "fmp4/video-h264-low.mp4",
        container: Container::Mp4,
        kind: MediaKind::Video,
        mime: "video/mp4; codecs=\"avc1.42C00C\"",
        width: 320,
        height: 180,
        duration_ms: 8000,
    },
    Fixture {
        name: "h264-high",
        path: "fmp4/video-h264-high.mp4",
        container: Container::Mp4,
        kind: MediaKind::Video,
        mime: "video/mp4; codecs=\"avc1.42C01E\"",
        width: 640,
        height: 360,
        duration_ms: 8000,
    },
    Fixture {
        name: "aac",
        path: "fmp4/audio-aac.mp4",
        container: Container::Mp4,
        kind: MediaKind::Audio,
        mime: "audio/mp4; codecs=\"mp4a.40.2\"",
        width: 0,
        height: 0,
        duration_ms: 8000,
    },
    Fixture {
        name: "h264-aac",
        path: "fmp4/muxed-h264-aac.mp4",
        container: Container::Mp4,
        kind: MediaKind::Muxed,
        mime: "video/mp4; codecs=\"avc1.42C00C, mp4a.40.2\"",
        width: 320,
        height: 180,
        duration_ms: 8000,
    },
    Fixture {
        name: "h264-bframes",
        path: "fmp4/video-h264-bframes.mp4",
        container: Container::Mp4,
        kind: MediaKind::Video,
        mime: "video/mp4; codecs=\"avc1.64001E\"",
        width: 640,
        height: 360,
        duration_ms: 8000,
    },
    Fixture {
        name: "vp9-low",
        path: "webm/video-vp9-low.webm",
        container: Container::WebM,
        kind: MediaKind::Video,
        mime: "video/webm; codecs=\"vp09.00.11.08\"",
        width: 320,
        height: 180,
        duration_ms: 8000,
    },
    Fixture {
        name: "vp9-high",
        path: "webm/video-vp9-high.webm",
        container: Container::WebM,
        kind: MediaKind::Video,
        mime: "video/webm; codecs=\"vp09.00.21.08\"",
        width: 640,
        height: 360,
        duration_ms: 8000,
    },
    Fixture {
        name: "opus",
        path: "webm/audio-opus.webm",
        container: Container::WebM,
        kind: MediaKind::Audio,
        mime: "audio/webm; codecs=\"opus\"",
        width: 0,
        height: 0,
        duration_ms: 8000,
    },
    Fixture {
        name: "av1-low",
        path: "webm/video-av1-low.webm",
        container: Container::WebM,
        kind: MediaKind::Video,
        mime: "video/webm; codecs=\"av01.0.00M.08\"",
        width: 320,
        height: 180,
        duration_ms: 8000,
    },
];

/// `spikes/mse/fixtures/`, found from `CARGO_MANIFEST_DIR` at compile time.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// The fixture called `name`.
pub fn fixture(name: &str) -> Option<&'static Fixture> {
    FIXTURES.iter().find(|f| f.name == name)
}

/// The bytes of the committed fixture called `name`, as they are on disk.
pub fn read_fixture(name: &str) -> Result<Vec<u8>, SplitError> {
    let f = fixture(name).ok_or_else(|| SplitError::UnknownFixture(name.to_string()))?;
    let path = fixtures_dir().join(f.path);
    std::fs::read(&path).map_err(|e| SplitError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })
}

/// Reads and splits the committed fixture called `name`. The stream's `mime` is derived from the
/// file, not copied from `FIXTURES`.
pub fn load(name: &str) -> Result<SegmentedStream, SplitError> {
    let f = fixture(name).ok_or_else(|| SplitError::UnknownFixture(name.to_string()))?;
    split(f.container, &read_fixture(name)?, "")
}
