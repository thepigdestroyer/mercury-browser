// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The decision the media player takes about a seek of a MediaSource element (lane med-1, review fix
//! T1). It is pure state so that it is tested here without GStreamer or Servo; the player glue
//! (`third_party/servo-media-gstreamer/player.rs`) executes what it says.
//!
//! The problem it solves: `GstPlay` drops a seek until it has media info, and `playbin3` refuses one
//! until its first buffer has reached `decodebin3`. A page that starts (or resumes) at 30 s seeks
//! before any data exists, so the seek has to be kept and sent once the pipeline can take it. Three
//! ways to get stuck are closed:
//!
//! * the first media info says the pipeline is not seekable: the seek is held until the pipeline
//!   settles (`GstPlay` reports Paused or Playing, by which time its media info is final), then sent if
//!   it is seekable and [`SeekDecision::Unseekable`] if it is not (the glue reports `SeekDone`, so the
//!   element never waits for ever);
//! * a seek that replaces a held one: only the latest is sent. The element has already abandoned the
//!   earlier seek (the HTML seek algorithm, step 3: "abort that other instance"), and its
//!   `playback_seek_done` ignores a `SeekDone` that is not for the current target, so no event is owed
//!   for it ([`SeekDecision::Held`] says which one was replaced);
//! * the seek is never sent from inside a `GstPlay` signal handler with the player's lock held: the
//!   decisions are returned to the caller, which acts on them after it released the lock.

/// What the player does with a seek (or with the end of a hold).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeekDecision {
    /// Give the seek to `GstPlay` now.
    Send(f64),
    /// The pipeline cannot take it yet; it is kept. `replaced` is an earlier held seek this one
    /// replaced (its time, for the log).
    Held { replaced: Option<f64> },
    /// The pipeline is settled and not seekable: the seek cannot be done. The glue reports `SeekDone`
    /// at this time so that the element finishes its seek.
    Unseekable(f64),
}

/// The seek state of one MediaSource player.
#[derive(Clone, Debug, Default)]
pub struct SeekHold {
    pending: Option<f64>,
    /// `GstPlay` has media info (the first metadata arrived).
    has_media_info: bool,
    /// The last media info said the stream is seekable.
    seekable: bool,
    /// The pipeline reported Paused or Playing since it was built.
    settled: bool,
}

impl SeekHold {
    pub fn new() -> Self {
        SeekHold::default()
    }

    /// The element asked for a seek to `time` (seconds).
    pub fn seek(&mut self, time: f64) -> SeekDecision {
        if self.has_media_info && self.seekable {
            // Whatever was held is replaced by this seek, which goes now.
            self.pending = None;
            return SeekDecision::Send(time);
        }
        if self.has_media_info && self.settled {
            self.pending = None;
            return SeekDecision::Unseekable(time);
        }
        // No media info yet, or media info that says "not seekable" before the pipeline settled (it
        // may say so only until its last stream is known): keep the latest seek.
        let replaced = self.pending.replace(time);
        SeekDecision::Held { replaced }
    }

    /// `GstPlay` published media info; `seekable` is what it says. Returns the held seek, if it can
    /// go now.
    pub fn metadata(&mut self, seekable: bool) -> Option<SeekDecision> {
        self.has_media_info = true;
        self.seekable = seekable;
        if seekable {
            return self.pending.take().map(SeekDecision::Send);
        }
        self.release_if_settled()
    }

    /// `GstPlay` reached Paused or Playing: its media info is final. The fallback that frees a seek
    /// held for a media info that never said "seekable".
    pub fn settled(&mut self) -> Option<SeekDecision> {
        self.settled = true;
        if !self.has_media_info {
            return None;
        }
        if self.seekable {
            return self.pending.take().map(SeekDecision::Send);
        }
        self.release_if_settled()
    }

    fn release_if_settled(&mut self) -> Option<SeekDecision> {
        if self.settled {
            return self.pending.take().map(SeekDecision::Unseekable);
        }
        None
    }

    /// The player stopped: nothing is held, nothing is known.
    pub fn reset(&mut self) {
        *self = SeekHold::default();
    }

    /// The held seek, if any.
    pub fn pending(&self) -> Option<f64> {
        self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seek_before_media_info_is_held_and_sent_with_the_first_seekable_metadata() {
        let mut hold = SeekHold::new();
        assert_eq!(hold.seek(30.0), SeekDecision::Held { replaced: None });
        assert_eq!(hold.pending(), Some(30.0));
        assert_eq!(hold.metadata(true), Some(SeekDecision::Send(30.0)));
        assert_eq!(hold.pending(), None);
        // Later seeks go straight through.
        assert_eq!(hold.seek(5.0), SeekDecision::Send(5.0));
    }

    #[test]
    fn a_later_seek_replaces_a_held_one_and_only_the_latest_is_sent() {
        let mut hold = SeekHold::new();
        assert_eq!(hold.seek(30.0), SeekDecision::Held { replaced: None });
        assert_eq!(
            hold.seek(10.0),
            SeekDecision::Held {
                replaced: Some(30.0)
            }
        );
        assert_eq!(hold.metadata(true), Some(SeekDecision::Send(10.0)));
        assert_eq!(hold.metadata(true), None, "nothing is sent twice");
    }

    #[test]
    fn first_metadata_that_is_not_seekable_does_not_stall_the_seek_for_ever() {
        let mut hold = SeekHold::new();
        hold.seek(30.0);
        // Not seekable yet: the seek stays held (GstPlay may say so before its last stream is known).
        assert_eq!(hold.metadata(false), None);
        assert_eq!(hold.pending(), Some(30.0));
        // The pipeline settles and still says not seekable: the fallback frees the seek, reported as
        // not possible so that the element's seek ends.
        assert_eq!(hold.settled(), Some(SeekDecision::Unseekable(30.0)));
        assert_eq!(hold.pending(), None);
        // After that a seek on the settled, unseekable player completes at once.
        assert_eq!(hold.seek(2.0), SeekDecision::Unseekable(2.0));
    }

    #[test]
    fn media_info_that_becomes_seekable_after_the_first_metadata_sends_the_held_seek() {
        let mut hold = SeekHold::new();
        hold.seek(30.0);
        assert_eq!(hold.metadata(false), None);
        assert_eq!(hold.metadata(true), Some(SeekDecision::Send(30.0)));
    }

    #[test]
    fn settling_before_metadata_changes_nothing_until_the_metadata_arrives() {
        let mut hold = SeekHold::new();
        hold.seek(7.0);
        assert_eq!(hold.settled(), None);
        assert_eq!(hold.pending(), Some(7.0));
        assert_eq!(hold.metadata(true), Some(SeekDecision::Send(7.0)));
        // The other order: metadata not seekable, after settling, releases at once.
        let mut hold = SeekHold::new();
        hold.seek(7.0);
        hold.settled();
        assert_eq!(hold.metadata(false), Some(SeekDecision::Unseekable(7.0)));
    }

    #[test]
    fn a_seek_after_metadata_that_is_not_seekable_is_freed_when_the_pipeline_settles() {
        let mut hold = SeekHold::new();
        hold.metadata(true);
        // Cannot be held once seekable: it goes.
        assert_eq!(hold.seek(1.0), SeekDecision::Send(1.0));
        let mut hold = SeekHold::new();
        hold.metadata(false);
        hold.seek(4.0);
        assert_eq!(hold.settled(), Some(SeekDecision::Unseekable(4.0)));
    }

    /// A fake `GstPlay`, as the player glue drives it: it drops a seek until it has media info (what the
    /// real one does), counts what reaches it, and notes the `SeekDone` events the glue reports itself.
    #[derive(Default)]
    struct FakePlay {
        media_info: bool,
        sent: Vec<f64>,
        dropped: Vec<f64>,
        seek_done: Vec<f64>,
    }

    impl FakePlay {
        fn apply(&mut self, decision: SeekDecision) {
            match decision {
                SeekDecision::Send(t) if self.media_info => self.sent.push(t),
                SeekDecision::Send(t) => self.dropped.push(t),
                SeekDecision::Unseekable(t) => self.seek_done.push(t),
                SeekDecision::Held { .. } => {}
            }
        }
    }

    #[test]
    fn a_fake_player_never_sees_a_seek_it_would_drop_and_the_element_always_hears_back() {
        // A page that starts at 30 s, changes its mind to 12 s, on a pipeline whose first metadata is
        // not seekable and that settles seekable-less: the element hears SeekDone(12) exactly once and
        // the player is never given a seek it has to drop.
        let mut hold = SeekHold::new();
        let mut play = FakePlay::default();
        play.apply(hold.seek(30.0));
        play.apply(hold.seek(12.0));
        play.media_info = true;
        if let Some(d) = hold.metadata(false) {
            play.apply(d);
        }
        if let Some(d) = hold.settled() {
            play.apply(d);
        }
        assert!(play.sent.is_empty() && play.dropped.is_empty());
        assert_eq!(play.seek_done, vec![12.0]);

        // The same page on a pipeline that does become seekable: one seek, to the latest target.
        let mut hold = SeekHold::new();
        let mut play = FakePlay::default();
        play.apply(hold.seek(30.0));
        play.apply(hold.seek(12.0));
        play.media_info = true;
        if let Some(d) = hold.metadata(true) {
            play.apply(d);
        }
        assert_eq!(play.sent, vec![12.0]);
        assert!(play.dropped.is_empty() && play.seek_done.is_empty());
    }

    #[test]
    fn reset_forgets_everything() {
        let mut hold = SeekHold::new();
        hold.seek(3.0);
        hold.metadata(true);
        hold.reset();
        assert_eq!(hold.seek(3.0), SeekDecision::Held { replaced: None });
    }
}
