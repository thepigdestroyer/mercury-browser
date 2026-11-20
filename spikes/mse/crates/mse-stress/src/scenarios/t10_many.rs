// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T10: many at once. Four rigs in one process playing at the same time (four tabs) while sixteen
//! more sit idle: no cross-talk (every rig's counts equal the single-rig result, its event log holds
//! only its own source buffers), element names do not collide, and the cost of an idle rig in
//! memory, threads and file descriptors is recorded.

use std::collections::BTreeSet;
use std::sync::mpsc;
use std::time::Duration;

use mse_rig::events::MseEvent;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_segments::SegmentedStream;
use serde_json::json;

use crate::common::{
    Expect, FIXTURE_FRAMES, Findings, ManagedRig, Played, Verdict, fast_config, frames_ok, new_rig,
    play_through,
};
use crate::proc::{self, Sample};

pub const BUSY_RIGS: usize = 4;
pub const IDLE_RIGS: usize = 16;

pub fn run(ctx: &Ctx) -> ScenarioResult {
    crate::common::run_isolated(
        "T10",
        "Many rigs at once",
        ctx,
        "MSE_STRESS_T10_NESTED",
        std::time::Duration::from_secs(150),
        body,
    )
}

/// What one busy rig reports back.
struct RigReport {
    index: usize,
    played: Played,
    /// Source buffer ids that appear in its event log.
    ids_seen: BTreeSet<usize>,
    update_starts: usize,
    error_events: usize,
    msesrc_names: Vec<String>,
}

/// True when `a` and `b` differ by at most `slack`.
pub fn within(a: u64, b: u64, slack: u64) -> bool {
    a.abs_diff(b) <= slack
}

/// The element names that occur in more than one list.
pub fn shared_names(lists: &[Vec<String>]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut shared = BTreeSet::new();
    for list in lists {
        let unique: BTreeSet<&String> = list.iter().collect();
        for name in unique {
            if !seen.insert(name.clone()) {
                shared.insert(name.clone());
            }
        }
    }
    shared.into_iter().collect()
}

/// `msesrc0:msesrc` -> `msesrc0`.
fn element_name(description: &str) -> String {
    description
        .split(':')
        .next()
        .unwrap_or(description)
        .to_owned()
}

fn id_of(event: &MseEvent) -> Option<usize> {
    match event {
        MseEvent::SourceBufferAdded(id)
        | MseEvent::SourceBufferRemoved(id)
        | MseEvent::UpdateStart(id)
        | MseEvent::Update(id)
        | MseEvent::UpdateEnd(id)
        | MseEvent::Abort(id)
        | MseEvent::Error(id) => Some(*id),
        _ => None,
    }
}

fn busy_rig(
    ctx: &Ctx,
    index: usize,
    video: &SegmentedStream,
    audio: &SegmentedStream,
) -> Result<RigReport, String> {
    let rig = new_rig(fast_config(ctx))?;
    let played = play_through(
        &rig,
        Some(video),
        Some(audio),
        Expect {
            video_frames: FIXTURE_FRAMES,
        },
        Duration::from_secs(60),
    )?;
    let elements: Vec<String> = rig
        .describe_pipeline()
        .iter()
        .map(|d| element_name(d))
        .collect();
    let events = rig.log.snapshot();
    let ids_seen: BTreeSet<usize> = events.iter().filter_map(|t| id_of(&t.event)).collect();
    let update_starts = events
        .iter()
        .filter(|t| matches!(t.event, MseEvent::UpdateStart(_)))
        .count();
    let msesrc_names: Vec<String> = elements
        .iter()
        .filter(|name| name.starts_with("msesrc"))
        .cloned()
        .collect();
    let error_events = events
        .iter()
        .filter(|t| matches!(t.event, MseEvent::Error(_)))
        .count();
    let _ = rig.shutdown();
    Ok(RigReport {
        index,
        played,
        ids_seen,
        update_starts,
        error_events,
        msesrc_names,
    })
}

fn per_rig(before: Sample, after: Sample, count: usize) -> serde_json::Value {
    let n = count.max(1) as f64;
    json!({
        "rss_kb_per_rig": ((after.rss_kb as f64 - before.rss_kb as f64) / n).round(),
        "threads_per_rig": (after.threads as f64 - before.threads as f64) / n,
        "fds_per_rig": (after.fds as f64 - before.fds as f64) / n,
    })
}

fn body(ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    let video = crate::common::fixture("h264-low")?;
    let audio = crate::common::fixture("aac")?;
    let mut findings = Findings::new();

    // The single-rig result every other rig is compared with.
    let baseline = busy_rig(ctx, 0, &video, &audio)?;
    let base_video = baseline.played.video.count;
    let base_audio = baseline.played.audio.count;
    result.metric("single_rig_video_frames", base_video);
    result.metric("single_rig_audio_buffers", base_audio);
    result.metric("single_rig_update_starts", baseline.update_starts);
    findings.check(
        frames_ok(base_video, FIXTURE_FRAMES),
        format!("the single rig delivered {base_video} video frames, expected {FIXTURE_FRAMES}: the baseline itself is off"),
    );

    // Sixteen idle rigs.
    let before_idle = proc::sample()?;
    let mut idle: Vec<ManagedRig> = Vec::new();
    for index in 0..IDLE_RIGS {
        match new_rig(fast_config(ctx)) {
            Ok(rig) => idle.push(rig),
            Err(error) => {
                findings.fail(format!("idle rig {index} could not be created: {error}"));
                break;
            }
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    let with_idle = proc::sample()?;
    result.metric("idle_rigs", idle.len());
    result.metric("idle_cost", per_rig(before_idle, with_idle, idle.len()));
    result.note(format!(
        "{} idle rigs cost {:.1} MB of RSS, {} threads and {} file descriptors in all: {:.1} MB, {:.1} threads, {:.1} fds per rig",
        idle.len(),
        (with_idle.rss_kb as f64 - before_idle.rss_kb as f64) / 1024.0,
        with_idle.threads as i64 - before_idle.threads as i64,
        with_idle.fds as i64 - before_idle.fds as i64,
        (with_idle.rss_kb as f64 - before_idle.rss_kb as f64) / 1024.0 / idle.len().max(1) as f64,
        (with_idle.threads as f64 - before_idle.threads as f64) / idle.len().max(1) as f64,
        (with_idle.fds as f64 - before_idle.fds as f64) / idle.len().max(1) as f64,
    ));

    // Four busy rigs at the same time, with the idle ones alive.
    let (sender, receiver) = mpsc::channel::<Result<RigReport, String>>();
    for index in 0..BUSY_RIGS {
        let (ctx, video, audio, sender) =
            (ctx.clone(), video.clone(), audio.clone(), sender.clone());
        std::thread::Builder::new()
            .name(format!("t10-rig-{index}"))
            .spawn(move || {
                let _ = sender.send(busy_rig(&ctx, index, &video, &audio));
            })
            .map_err(|e| format!("cannot start a thread: {e}"))?;
    }
    drop(sender);
    let mut reports = Vec::new();
    for _ in 0..BUSY_RIGS {
        match receiver.recv_timeout(Duration::from_secs(120)) {
            Ok(Ok(report)) => reports.push(report),
            Ok(Err(error)) => findings.fail(format!("a concurrent rig failed: {error}")),
            Err(_) => {
                findings.fail("a concurrent rig did not finish within 120 s: deadlock or stall with several rigs in one process");
                break;
            }
        }
    }
    reports.sort_by_key(|r| r.index);
    let after_busy = proc::sample()?;
    result.metric("rss_kb_with_idle_rigs_after_busy_run", after_busy.rss_kb);

    for report in &reports {
        let prefix = format!("busy_{}", report.index);
        report
            .played
            .video
            .record(result, &format!("{prefix}_video"));
        report
            .played
            .audio
            .record(result, &format!("{prefix}_audio"));
        findings.check(
            report.played.reached_expected,
            format!(
                "busy rig {}: the expected frames did not arrive within 60 s",
                report.index
            ),
        );
        findings.check(
            frames_ok(report.played.video.count, base_video),
            format!(
                "busy rig {}: {} video frames, the single rig had {base_video} (cross-talk or loss)",
                report.index, report.played.video.count
            ),
        );
        findings.check(
            within(report.played.audio.count, base_audio, 2),
            format!(
                "busy rig {}: {} audio buffers, the single rig had {base_audio}",
                report.index, report.played.audio.count
            ),
        );
        findings.check(
            report.ids_seen.iter().all(|id| *id < 2),
            format!(
                "busy rig {}: its event log mentions source buffer ids {:?}, but it only has 0 and 1 (events of another rig?)",
                report.index, report.ids_seen
            ),
        );
        findings.check(
            report.update_starts == baseline.update_starts,
            format!(
                "busy rig {}: {} UpdateStart events, the single rig had {}",
                report.index, report.update_starts, baseline.update_starts
            ),
        );
        findings.check(
            report.error_events == 0,
            format!(
                "busy rig {}: {} source buffer Error event(s)",
                report.index, report.error_events
            ),
        );
    }
    // Only the elements GstMSE and GStreamer number globally can collide: every `msesrc` must be unique. (The
    // inner elements of each playbin3 carry the same names in every pipeline, under different parents.)
    let mut lists: Vec<Vec<String>> = reports.iter().map(|r| r.msesrc_names.clone()).collect();
    lists.push(baseline.msesrc_names.clone());
    let collisions = shared_names(&lists);
    result.metric("element_name_collisions", json!(collisions));
    findings.check(
        collisions.is_empty(),
        format!("element names that occur in more than one rig's pipeline: {collisions:?}"),
    );
    result.note(format!(
        "{} of {BUSY_RIGS} concurrent rigs finished; video frames {:?}, audio buffers {:?} (single rig: {base_video} and {base_audio}); element name collisions: {}",
        reports.len(),
        reports.iter().map(|r| r.played.video.count).collect::<Vec<_>>(),
        reports.iter().map(|r| r.played.audio.count).collect::<Vec<_>>(),
        collisions.len()
    ));
    findings.check(
        reports.len() == BUSY_RIGS,
        format!(
            "only {} of {BUSY_RIGS} concurrent rigs reported a result",
            reports.len()
        ),
    );

    // Drop the idle rigs and see what comes back.
    for rig in idle {
        let _ = rig.shutdown();
    }
    std::thread::sleep(Duration::from_millis(800));
    let after_idle = proc::sample()?;
    result.metric(
        "after_dropping_idle_rigs",
        json!({
            "rss_kb": after_idle.rss_kb,
            "threads": after_idle.threads,
            "fds": after_idle.fds,
            "threads_before_idle_rigs": before_idle.threads,
            "fds_before_idle_rigs": before_idle.fds,
        }),
    );
    result.note(format!(
        "after dropping the idle rigs: threads {} (before them {}), fds {} (before {})",
        after_idle.threads, before_idle.threads, after_idle.fds, before_idle.fds
    ));
    Ok(findings.verdict())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_is_symmetric() {
        assert!(within(10, 12, 2));
        assert!(within(12, 10, 2));
        assert!(!within(10, 13, 2));
    }

    #[test]
    fn shared_names_finds_names_in_two_lists_only() {
        let lists = vec![
            vec![
                "msesrc0".to_owned(),
                "playbin3-0".to_owned(),
                "playbin3-0".to_owned(),
            ],
            vec!["msesrc1".to_owned(), "playbin3-1".to_owned()],
            vec!["msesrc1".to_owned()],
        ];
        assert_eq!(shared_names(&lists), ["msesrc1"]);
        assert!(shared_names(&[vec!["a".into(), "a".into()]]).is_empty());
    }

    #[test]
    fn element_names_drop_the_factory() {
        assert_eq!(element_name("msesrc0:msesrc"), "msesrc0");
        assert_eq!(element_name("plain"), "plain");
    }

    #[test]
    fn only_buffer_events_carry_an_id() {
        assert_eq!(id_of(&MseEvent::UpdateEnd(3)), Some(3));
        assert_eq!(id_of(&MseEvent::SourceOpen), None);
    }

    #[test]
    fn the_counts_are_the_briefs() {
        assert_eq!((BUSY_RIGS, IDLE_RIGS), (4, 16));
    }
}
