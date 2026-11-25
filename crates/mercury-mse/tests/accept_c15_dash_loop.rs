// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C15 of the GstMSE spike at core level (lane med-1, item 3): a dash.js-shaped loop on real threads.
//!
//! The "player" lives in the event closures: when an append or a remove ends, the closure (running on
//! the worker thread) appends the next segment while less than 6 s is buffered ahead of the playback
//! position, removes more than 4 s behind it in 2 s steps, switches the video rendition at a segment
//! boundary (a new init segment of another configuration), and ends the stream after the last append.
//! The "pipeline" is the test thread: it pulls the feed as fast as data comes, sleeps on the feed's wake
//! closure when the feed says `WaitingForData` (a lost wake shows as a timeout while data is buffered),
//! reports the playback position to the backend, and, in the first test, seeks inside the buffer.
//!
//! The feed never stalls: whenever the data for the next frame is buffered, the next pull gives it.

mod model_support;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use mercury_mse::MseBackend;
use mercury_mse::api::{
    FeedItem, FeedState, MediaSourceBackend, MediaSourceReadyState, MseEvent, SourceBufferId,
    TrackKind,
};
use mercury_mse_testkit::{SegmentedStream, tile};
use model_support::*;

const AHEAD_S: f64 = 6.0;
const BEHIND_S: f64 = 4.0;
const STEP_S: f64 = 2.0;

struct Seg {
    init: Option<Vec<u8>>,
    media: Vec<u8>,
    /// Where the data ends once this segment is in, seconds.
    end_s: f64,
}

struct Plan {
    id: SourceBufferId,
    segs: Vec<Seg>,
    next: usize,
    pending: Option<Vec<u8>>,
    end_s: f64,
    removed_to: f64,
}

struct State {
    plans: Vec<Plan>,
    eos: bool,
    appends: usize,
    removes: usize,
    errors: Vec<String>,
    /// (position, buffered ranges of the first buffer) at every completion.
    snapshots: Vec<(f64, Vec<(f64, f64)>)>,
}

struct Dash {
    backend: Arc<MseBackend>,
    pos_ms: AtomicI64,
    st: Mutex<State>,
}

impl Dash {
    fn position(&self) -> f64 {
        self.pos_ms.load(Ordering::SeqCst) as f64 / 1000.0
    }

    fn set_position(&self, secs: f64) {
        self.pos_ms.store((secs * 1000.0) as i64, Ordering::SeqCst);
        self.backend.set_position(secs);
    }

    /// One turn of the player: at most one operation per buffer that is not updating. Removing media
    /// behind the position comes before appending more (a buffer pruner runs on its own schedule), and a
    /// stream that was reopened (a remove or a seek in the ended state) is ended again, as a real
    /// player does in its `sourceopen` handler.
    fn pump(&self) {
        let mut st = self.st.lock().unwrap();
        if st.eos && self.backend.ready_state() == MediaSourceReadyState::Open {
            st.eos = false;
        }
        let pos = self.position();
        let mut appends = 0;
        let mut removes = 0;
        let mut errors = Vec::new();
        for plan in st.plans.iter_mut() {
            if self.backend.updating(plan.id) {
                continue;
            }
            if let Some(media) = plan.pending.take() {
                match self.backend.append(plan.id, media) {
                    Ok(()) => appends += 1,
                    Err(e) => errors.push(format!("append: {e}")),
                }
            } else if plan.removed_to + STEP_S <= pos - BEHIND_S && plan.removed_to < plan.end_s {
                let (from, to) = (plan.removed_to, plan.removed_to + STEP_S);
                plan.removed_to = to;
                match self.backend.remove(plan.id, from, to) {
                    Ok(()) => removes += 1,
                    Err(e) => errors.push(format!("remove: {e}")),
                }
            } else if plan.next < plan.segs.len() && plan.end_s - pos < AHEAD_S {
                let seg = &plan.segs[plan.next];
                plan.next += 1;
                plan.end_s = seg.end_s;
                let first = match &seg.init {
                    Some(init) => {
                        plan.pending = Some(seg.media.clone());
                        init.clone()
                    }
                    None => seg.media.clone(),
                };
                match self.backend.append(plan.id, first) {
                    Ok(()) => appends += 1,
                    Err(e) => errors.push(format!("append: {e}")),
                }
            }
        }
        st.appends += appends;
        st.removes += removes;
        st.errors.extend(errors);
        let all_in = st
            .plans
            .iter()
            .all(|p| p.next >= p.segs.len() && p.pending.is_none() && !self.backend.updating(p.id));
        if !st.eos && all_in {
            match self.backend.end_of_stream(None) {
                Ok(()) => st.eos = true,
                Err(e) => st.errors.push(format!("end_of_stream: {e}")),
            }
        }
    }

    fn removed_to(&self, index: usize) -> f64 {
        self.st.lock().unwrap().plans[index].removed_to
    }
}

/// Plans for one source buffer: the init segment (appended first), then the media segments; segments
/// from `switch_at` on come from `other` with its own init segment in front.
fn plan_for(
    id: SourceBufferId,
    main: &SegmentedStream,
    other: Option<(&SegmentedStream, usize)>,
) -> Plan {
    let mut segs = Vec::new();
    segs.push(Seg {
        init: None,
        media: main.init.bytes.clone(),
        end_s: 0.0,
    });
    for (i, s) in main.media.iter().enumerate() {
        let end_s = s.end_ms().unwrap() as f64 / 1000.0;
        match other {
            Some((o, at)) if i >= at => segs.push(Seg {
                init: if i == at {
                    Some(o.init.bytes.clone())
                } else {
                    None
                },
                media: o.media[i].bytes.clone(),
                end_s,
            }),
            _ => segs.push(Seg {
                init: None,
                media: s.bytes.clone(),
                end_s,
            }),
        }
    }
    Plan {
        id,
        segs,
        next: 0,
        pending: None,
        end_s: 0.0,
        removed_to: 0.0,
    }
}

fn start(plans: impl FnOnce(&Rig) -> Vec<Plan>) -> (Rig, Arc<Dash>) {
    let rig = Rig::new();
    let plans = plans(&rig);
    let dash = Arc::new(Dash {
        backend: Arc::clone(&rig.backend),
        pos_ms: AtomicI64::new(0),
        st: Mutex::new(State {
            plans,
            eos: false,
            appends: 0,
            removes: 0,
            errors: Vec::new(),
            snapshots: Vec::new(),
        }),
    });
    let weak: Weak<Dash> = Arc::downgrade(&dash);
    rig.rec.add_hook(move |event| {
        if matches!(
            event,
            MseEvent::AppendDone { .. } | MseEvent::RemoveDone { .. }
        ) && let Some(dash) = weak.upgrade()
        {
            dash.snapshot();
            dash.pump();
        }
    });
    (rig, dash)
}

impl Dash {
    fn snapshot(&self) {
        let pos = self.position();
        let id = self.st.lock().unwrap().plans[0].id;
        let ranges = secs(&self.backend.buffered(id));
        self.st.lock().unwrap().snapshots.push((pos, ranges));
    }
}

struct Played {
    video: Vec<(f64, bool)>,
    audio: Vec<f64>,
    configs: usize,
    max_wait: Duration,
    waits: usize,
}

/// The pipeline: pulls both kinds until both end. `seek_at` makes it seek back to 3 s inside the
/// buffer once the first removal behind the position has happened and the position passed 6 s.
fn play(dash: &Arc<Dash>, has_audio: bool, seek: bool) -> Played {
    let feed = dash.backend.frame_feed();
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    feed.set_wake(Arc::new(move || {
        let _ = wake_tx.send(());
    }));
    let mut out = Played {
        video: Vec::new(),
        audio: Vec::new(),
        configs: 0,
        max_wait: Duration::ZERO,
        waits: 0,
    };
    let mut ended = [false; 2];
    let mut sought = !seek;
    let mut since_pump = 0;
    let (mut v_last, mut a_last) = (-1.0f64, -1.0f64);
    dash.pump();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut waiting_since: Option<Instant> = None;
    while !(ended[0] && (ended[1] || !has_audio)) {
        assert!(Instant::now() < deadline, "the loop did not finish in 60 s");
        let mut progressed = false;
        // One batch per turn, of the kind that is behind: audio and video stay within a batch of each
        // other, as a pipeline's queues keep them, so the player's removals (which follow the video
        // position) never reach audio that has not been consumed yet.
        let order = if !has_audio || ended[1] || v_last <= a_last {
            [(0, TrackKind::Video), (1, TrackKind::Audio)]
        } else {
            [(1, TrackKind::Audio), (0, TrackKind::Video)]
        };
        for (slot, kind) in order {
            if (slot == 1 && !has_audio) || ended[slot] {
                continue;
            }
            let batch = feed.pull(kind, 16);
            let got = !batch.items.is_empty();
            for item in batch.items {
                match item {
                    FeedItem::Config(_) if kind == TrackKind::Video => out.configs += 1,
                    FeedItem::Config(_) => {}
                    FeedItem::Frame { frame, discont } => {
                        let pts = frame.pts as f64 / 1e9;
                        if kind == TrackKind::Video {
                            out.video.push((pts, discont));
                            v_last = pts;
                            dash.set_position(pts);
                            since_pump += 1;
                            if since_pump >= 10 {
                                since_pump = 0;
                                dash.pump();
                            }
                        } else {
                            out.audio.push(pts);
                            a_last = pts;
                        }
                    }
                }
            }
            ended[slot] = batch.state == FeedState::Ended;
            if got {
                progressed = true;
                break;
            }
        }
        if ended[0] && (ended[1] || !has_audio) {
            break;
        }
        if progressed && let Some(since) = waiting_since.take() {
            out.max_wait = out.max_wait.max(since.elapsed());
        }
        if !sought {
            let pos = dash.position();
            if pos >= 6.0 {
                dash.pump();
                if dash.removed_to(0) >= 2.0 {
                    sought = true;
                    // If the stream had already ended, the seek reopens it and the player, like a real
                    // one, ends it again once it has what it wants.
                    if dash.backend.seeking(3.0) {
                        dash.st.lock().unwrap().eos = false;
                    }
                    dash.set_position(3.0);
                    feed.seek(3_000_000_000);
                    ended = [false; 2];
                    v_last = -1.0;
                    a_last = -1.0;
                    continue;
                }
            }
        }
        if !progressed {
            if waiting_since.is_none() {
                waiting_since = Some(Instant::now());
                out.waits += 1;
            }
            // A feed that says WaitingForData is woken when data arrives. A 5 s silence while the
            // buffered data reaches well past the position would be a lost wake.
            if wake_rx.recv_timeout(Duration::from_secs(5)).is_err() {
                let pos = dash.position();
                let ahead = dash
                    .backend
                    .element_buffered()
                    .last()
                    .map_or(0.0, |r| r.end as f64 / 1e9);
                let st = dash.st.lock().unwrap();
                let probe_v = feed.pull(TrackKind::Video, 4);
                let probe_a = feed.pull(TrackKind::Audio, 4);
                panic!(
                    "no wake for 5 s: position {pos}, buffered up to {ahead}, ended {ended:?}, \
                     ready state {:?}, player eos {} errors {:?}, probe video {:?} ({} items), probe audio {:?} ({} items)",
                    dash.backend.ready_state(),
                    st.eos,
                    st.errors,
                    probe_v.state,
                    probe_v.items.len(),
                    probe_a.state,
                    probe_a.items.len()
                );
            }
        }
    }
    out
}

fn assert_clean(dash: &Dash) {
    let st = dash.st.lock().unwrap();
    assert!(
        st.errors.is_empty(),
        "the player's own calls failed: {:?}",
        st.errors
    );
    assert!(st.eos, "the stream was ended");
}

#[test]
fn c15_fixture_loop_with_remove_behind_seek_quality_switch_and_end_of_stream() {
    let low = load("h264-low");
    let high = load("h264-high");
    let aac = load("aac");
    let (rig, dash) = start(|rig| {
        let v = rig.add(&low.mime);
        let a = rig.add(&aac.mime);
        // The video switches to the high rendition at segment 2 (4 s): a new init segment in the same
        // source buffer.
        vec![plan_for(v, &low, Some((&high, 2))), plan_for(a, &aac, None)]
    });
    let played = play(&dash, true, true);
    assert_clean(&dash);

    // Video: frames in decode order; before the seek every frame once; after it, from the keyframe at
    // 2 s with a discontinuity.
    let seek_at = played
        .video
        .iter()
        .position(|(_, discont)| *discont)
        .and_then(|first| {
            played.video[first + 1..]
                .iter()
                .position(|(_, d)| *d)
                .map(|i| i + first + 1)
        })
        .expect("the seek restarted the feed with a discontinuity");
    let before = &played.video[..seek_at];
    let after = &played.video[seek_at..];
    assert!(
        before.first().unwrap().1,
        "the first frame of the stream is a discontinuity"
    );
    for pair in before.windows(2) {
        let step = pair[1].0 - pair[0].0;
        assert!(
            (0.039..=0.041).contains(&step),
            "gap {step} before the seek at {}",
            pair[0].0
        );
    }
    assert!(after[0].1);
    assert!(
        (after[0].0 - 2.0).abs() < 0.001,
        "after the seek to 3 s the feed starts at the keyframe at 2 s, not {}",
        after[0].0
    );
    for pair in after.windows(2) {
        let step = pair[1].0 - pair[0].0;
        assert!(
            (0.039..=0.041).contains(&step),
            "gap {step} after the seek at {}",
            pair[0].0
        );
        assert!(!pair[1].1, "no discontinuity inside a run");
    }
    let last = after.last().unwrap().0;
    assert!(
        (last - 7.96).abs() < 0.001,
        "the last frame is the last of the stream: {last}"
    );
    // Two renditions: a Config item for the first frame, one for the switch at 4 s, and one more for the
    // restart after the seek (it restarts in the low rendition's range [2, 4): the consumer had the
    // high one, so the configuration differs and the feed says so).
    assert!(played.configs >= 2, "Config items: {}", played.configs);

    // Audio: restarts at or before 3 s after the seek, and ends at the end of the stream.
    let audio_restart = played
        .audio
        .windows(2)
        .position(|w| w[1] < w[0])
        .map(|i| i + 1)
        .expect("the audio restarted too");
    assert!(played.audio[audio_restart] <= 3.0 && played.audio[audio_restart] > 2.9);
    assert!((played.audio.last().unwrap() - (8.0 - 1024.0 / 48000.0)).abs() < 0.03);

    // The player did its work through the closures: segments appended, old data removed.
    let st = dash.st.lock().unwrap();
    assert!(st.appends >= 10, "appends {}", st.appends);
    assert!(st.removes >= 1, "removes {}", st.removes);
    // The buffered window stayed within what the player's rules allow.
    for (pos, ranges) in &st.snapshots {
        assert!(
            ranges.len() <= 1,
            "one contiguous range at {pos}: {ranges:?}"
        );
        if let Some((start, end)) = ranges.first() {
            assert!(
                end - pos <= AHEAD_S + 2.0 + 0.1,
                "buffered {end} is too far ahead of {pos}"
            );
            assert!(
                start - pos >= -(BEHIND_S + 3.0 * STEP_S),
                "buffered from {start} is too far behind {pos}"
            );
        }
    }
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Ended);
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendError { .. })),
        0
    );
    eprintln!(
        "c15 fixtures: {} appends, {} removes, {} waits on the wake closure, longest wait {:?}",
        st.appends, st.removes, played.waits, played.max_wait
    );
    assert!(
        played.max_wait < Duration::from_secs(2),
        "the feed stalled for {:?}",
        played.max_wait
    );
}

#[test]
fn c15_tiled_64_second_stream_plays_through_within_a_bounded_window() {
    let low = load("h264-low");
    let aac = load("aac");
    let v64 = tile(&low, 8).unwrap();
    let a64 = tile(&aac, 8).unwrap();
    let (rig, dash) = start(|rig| {
        let v = rig.add(&v64.mime);
        let a = rig.add(&a64.mime);
        vec![plan_for(v, &v64, None), plan_for(a, &a64, None)]
    });
    let played = play(&dash, true, false);
    assert_clean(&dash);
    assert_eq!(played.video.len(), 1600, "video frames over 64 s");
    assert_eq!(played.audio.len(), 3000, "AAC frames over 64 s");
    assert!(played.video[0].1);
    assert!(
        played.video[1..].iter().all(|(_, d)| !d),
        "no discontinuity in an unbroken stream"
    );
    for pair in played.video.windows(2) {
        let step = pair[1].0 - pair[0].0;
        assert!(
            (0.039..=0.041).contains(&step),
            "gap {step} at {}",
            pair[0].0
        );
    }
    let st = dash.st.lock().unwrap();
    assert!(
        st.removes >= 20,
        "old data was removed as the position moved: {} removes",
        st.removes
    );
    let widest = st
        .snapshots
        .iter()
        .filter_map(|(_, r)| r.first().map(|(s, e)| e - s))
        .fold(0.0f64, f64::max);
    assert!(
        widest <= 20.0,
        "the video buffer never held more than {widest} s of a 64 s stream"
    );
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Ended);
    let d = rig.backend.duration().unwrap();
    assert!((d - 64.0).abs() < 0.05, "duration {d}");
    eprintln!(
        "c15 tiled: {} appends, {} removes, widest buffer {widest:.1} s, {} waits, longest wait {:?}",
        st.appends, st.removes, played.waits, played.max_wait
    );
    assert!(played.max_wait < Duration::from_secs(2));
}
