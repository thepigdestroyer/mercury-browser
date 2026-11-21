// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The committed fixtures as the real parser (item 1) turns them into an initialization segment and one
//! batch of coded frames per media segment, for the tests of item 2 that use real frames.

use mercury_mse::{CodedFrame, Container, InitSegment, ParseEvent, new_parser};
use mercury_mse_testkit::Container as KitContainer;

/// A fixture parsed: the initialization segment and the frames of each media segment.
pub struct Real {
    pub init: InitSegment,
    pub segments: Vec<Vec<CodedFrame>>,
}

impl Real {
    /// Every frame of every segment, in order.
    pub fn all_frames(&self) -> Vec<CodedFrame> {
        self.segments.iter().flatten().cloned().collect()
    }

    /// The frames of segments `range`, in order.
    pub fn frames(&self, range: std::ops::Range<usize>) -> Vec<CodedFrame> {
        self.segments[range].iter().flatten().cloned().collect()
    }
}

/// Parses a fixture of `mercury-mse-testkit` (`h264-low`, `opus`, ...) whole, one segment per append.
pub fn load(name: &str) -> Real {
    let stream = mercury_mse_testkit::load(name).unwrap_or_else(|e| panic!("fixture {name}: {e}"));
    let container = match stream.container {
        KitContainer::Mp4 => Container::Mp4,
        KitContainer::WebM => Container::WebM,
    };
    let mut parser = new_parser(container);
    let mut init = None;
    for ev in parser.push(&stream.init.bytes).expect("init segment") {
        if let ParseEvent::Init(i) = ev {
            init = Some(i);
        }
    }
    let init = init.expect("the parser found no initialization segment");
    let mut segments = Vec::new();
    for seg in &stream.media {
        let mut frames = Vec::new();
        for ev in parser.push(&seg.bytes).expect("media segment") {
            if let ParseEvent::Frames(fs) = ev {
                frames.extend(fs);
            }
        }
        segments.push(frames);
    }
    Real { init, segments }
}
