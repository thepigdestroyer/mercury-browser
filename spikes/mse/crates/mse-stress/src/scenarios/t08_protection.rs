// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! T08: protection events (EME). Answers R14a §15 point 1: do `GST_EVENT_PROTECTION` events (and
//! `GstProtectionMeta` on buffers) reach the pads downstream of `msesrc`, so that EA15 could use
//! them, or must `appendBuffer` extract the initialisation data itself?
//!
//! Static part: what `nm` and `strings` say about protection in `libgstmse-1.0` and the plugin, and
//! whether the MP4 demuxer inside the append pipeline can emit such events at all. Dynamic part: a
//! `pssh` box (ClearKey's and Widevine's system IDs) is put into the `moov` of `h264-low`'s
//! initialisation segment with `mse_segments::with_pssh`, and pad probes on every `msesrc` pad record
//! the events and buffer metas that arrive, in both pipeline shapes, with a control run on the
//! unmodified initialisation segment.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::{Rig, RigConfig, Shape};
use mse_segments::with_pssh;
use serde_json::{Value, json};

use crate::common::{APPEND_TIMEOUT, Findings, Verdict};

/// ClearKey (W3C), Widevine: the system IDs EA15 and a CDM would ask for.
pub const CLEARKEY_SYSTEM_ID: &str = "1077efec-c0b2-4d02-ace3-3c1e52e2fb4b";
pub const WIDEVINE_SYSTEM_ID: &str = "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed";

/// Words that mark protection handling in a binary's strings and symbols.
pub const NEEDLES: &[&str] = &[
    "protect", "encrypt", "pssh", "cenc", "drm", "decrypt", "clearkey", "cdm",
];

pub fn run(ctx: &Ctx) -> ScenarioResult {
    crate::common::run_isolated(
        "T08",
        "Protection events (EME)",
        ctx,
        "MSE_STRESS_T08_NESTED",
        std::time::Duration::from_secs(60),
        body,
    )
}

/// `1077efec-c0b2-4d02-ace3-3c1e52e2fb4b` -> 16 bytes.
pub fn uuid_bytes(text: &str) -> Option<[u8; 16]> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The lines of `text` that contain any needle (case-insensitive), trimmed, at most `limit`.
pub fn matching_lines(text: &str, needles: &[&str], limit: usize) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && line.len() < 200)
        .filter(|line| {
            let lower = line.to_lowercase();
            needles.iter().any(|needle| lower.contains(needle))
        })
        .take(limit)
        .map(str::to_owned)
        .collect()
}

fn framework_prefix() -> Option<PathBuf> {
    std::env::var("MERCURY_GSTREAMER_PREFIX")
        .ok()
        .map(PathBuf::from)
}

fn tool_output(program: &str, args: &[String]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The `-arch` argument `nm` needs on a universal binary for this machine.
fn arch_args() -> Vec<String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    vec!["-arch".to_owned(), arch.to_owned()]
}

fn static_part(result: &mut ScenarioResult) -> Value {
    let Some(prefix) = framework_prefix() else {
        result.note("static part skipped: MERCURY_GSTREAMER_PREFIX is not set (run through spikes/mse/run.sh)");
        return json!({ "skipped": "MERCURY_GSTREAMER_PREFIX is not set" });
    };
    let mut report = serde_json::Map::new();
    for (name, relative) in [
        ("libgstmse-1.0", "lib/libgstmse-1.0.0.dylib"),
        ("libgstmse plugin", "lib/gstreamer-1.0/libgstmse.dylib"),
        (
            "qtdemux (libgstisomp4)",
            "lib/gstreamer-1.0/libgstisomp4.dylib",
        ),
    ] {
        let path = prefix.join(relative);
        let path_text = path.display().to_string();
        let mut entry = serde_json::Map::new();
        let mut args = arch_args();
        args.push(path_text.clone());
        let mut defined_args = vec!["-gU".to_owned()];
        defined_args.extend(args.clone());
        let mut undefined_args = vec!["-u".to_owned()];
        undefined_args.extend(args.clone());
        let defined = tool_output("nm", &defined_args);
        let undefined = tool_output("nm", &undefined_args);
        let strings = tool_output("strings", &["-a".to_owned(), path_text.clone()]);
        match (&defined, &undefined, &strings) {
            (Ok(defined), Ok(undefined), Ok(strings)) => {
                let defined_hits = matching_lines(defined, NEEDLES, 20);
                let imported_hits = matching_lines(undefined, &["protection", "protect"], 20);
                let string_hits = matching_lines(strings, NEEDLES, 25);
                entry.insert(
                    "exported_symbols_total".to_owned(),
                    json!(defined.lines().count()),
                );
                entry.insert(
                    "exported_symbols_about_protection".to_owned(),
                    json!(defined_hits),
                );
                entry.insert(
                    "imported_protection_symbols".to_owned(),
                    json!(imported_hits),
                );
                entry.insert("strings_about_protection".to_owned(), json!(string_hits));
                result.note(format!(
                    "static, {name}: {} exported symbols, {} about protection; imports {} protection symbols ({}); {} protection-related strings{}",
                    defined.lines().count(),
                    defined_hits.len(),
                    imported_hits.len(),
                    imported_hits.first().map_or("none".to_owned(), |s| s.clone()),
                    string_hits.len(),
                    if string_hits.is_empty() { String::new() } else { format!(" (first: {:?})", string_hits[0]) }
                ));
            }
            _ => {
                entry.insert("error".to_owned(), json!("nm or strings failed"));
                result.note(format!(
                    "static, {name}: nm or strings failed on {path_text}"
                ));
            }
        }
        report.insert(name.to_owned(), Value::Object(entry));
    }
    Value::Object(report)
}

/// What the probes on the `msesrc` pads saw.
#[derive(Debug, Default)]
struct Seen {
    /// Event type name -> count.
    events: std::collections::BTreeMap<String, u64>,
    protection: Vec<String>,
    buffers: u64,
    buffers_with_protection_meta: u64,
    pads: Vec<String>,
}

fn install_probes(rig: &Rig, seen: &Arc<Mutex<Seen>>) -> Result<(), String> {
    let msesrc = rig.msesrc().ok_or("the rig has no msesrc element")?;
    let seen = Arc::clone(seen);
    msesrc.connect_pad_added(move |_, pad| {
        crate::common::trace(&format!("T08 pad-added {}", pad.name()));
        seen.lock()
            .unwrap_or_else(|e| e.into_inner())
            .pads
            .push(pad.name().to_string());
        let seen = Arc::clone(&seen);
        pad.add_probe(
            gst::PadProbeType::EVENT_DOWNSTREAM | gst::PadProbeType::BUFFER,
            move |_, info| {
                let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
                crate::common::trace("T08 probe called");
                if let Some(event) = info.event() {
                    crate::common::trace(&format!("T08 probe event {:?}", event.type_()));
                    *seen
                        .events
                        .entry(format!("{:?}", event.type_()))
                        .or_default() += 1;
                    if let gst::EventView::Protection(protection) = event.view() {
                        crate::common::trace("T08 probe: a protection event");
                        let data_bytes = event
                            .structure()
                            .and_then(|s| s.get::<Option<gst::Buffer>>("data").ok().flatten())
                            .map(|b| b.size());
                        seen.protection.push(format!(
                            "system_id={} origin={:?} data_bytes={:?}",
                            protection.system_id(),
                            protection.origin(),
                            data_bytes
                        ));
                    }
                }
                if let Some(buffer) = info.buffer() {
                    seen.buffers += 1;
                    if std::env::var_os("MSE_STRESS_T08_NO_META").is_none()
                        && buffer.meta::<gst::meta::ProtectionMeta>().is_some()
                    {
                        seen.buffers_with_protection_meta += 1;
                    }
                }
                gst::PadProbeReturn::Ok
            },
        );
    });
    Ok(())
}

struct RunSummary {
    shape: Shape,
    with_pssh: bool,
    seen: Seen,
    notes: Vec<String>,
}

fn dynamic_run(shape: Shape, with_pssh_box: bool) -> Result<RunSummary, String> {
    let video = mse_segments::load("h264-low").map_err(|e| format!("fixture h264-low: {e}"))?;
    let mut init = video.init.clone();
    if with_pssh_box {
        for (id, payload) in [
            (CLEARKEY_SYSTEM_ID, &b"stress-probe-clearkey"[..]),
            (WIDEVINE_SYSTEM_ID, &b"stress-probe-widevine"[..]),
        ] {
            let system = uuid_bytes(id).ok_or("bad system id")?;
            init =
                with_pssh(&init, system, payload).map_err(|e| format!("with_pssh({id}): {e}"))?;
        }
    }
    let config = RigConfig {
        shape,
        sync: false,
        video: true,
        audio: false,
        ..RigConfig::default()
    };
    crate::common::trace(&format!(
        "T08 {} pssh={with_pssh_box}: Rig::new",
        shape.label()
    ));
    let rig = crate::common::new_rig(config)?;
    let seen = Arc::new(Mutex::new(Seen::default()));
    if std::env::var_os("MSE_STRESS_T08_NO_PROBES").is_none() {
        install_probes(&rig, &seen)?;
    }
    let id = rig
        .backend
        .add_source_buffer(&video.mime)
        .map_err(|e| format!("add_source_buffer: {e:?}"))?;
    let mut notes = Vec::new();
    crate::common::trace("T08: appending the initialisation segment");
    match rig.append_and_wait(id, &init.bytes, APPEND_TIMEOUT) {
        Ok(_) => {}
        Err(error) => notes.push(format!(
            "the initialisation segment{} was refused or failed: {error}",
            if with_pssh_box {
                " with pssh boxes"
            } else {
                ""
            }
        )),
    }
    for segment in video.media.iter().take(2) {
        if let Err(error) = rig.append_and_wait(id, &segment.bytes, APPEND_TIMEOUT) {
            notes.push(format!("a media segment append failed: {error}"));
        }
    }
    let _ = rig.play();
    rig.wait_until(Duration::from_secs(3), |rig| {
        crate::common::video_frames(rig) >= 5
    });
    std::thread::sleep(Duration::from_millis(300));
    let frames = crate::common::video_frames(&rig);
    notes.push(format!("{frames} video frames reached the sink"));
    crate::common::trace("T08: shutdown");
    let _ = rig.shutdown();
    crate::common::trace("T08: shut down");
    let seen = std::mem::take(&mut *seen.lock().unwrap_or_else(|e| e.into_inner()));
    Ok(RunSummary {
        shape,
        with_pssh: with_pssh_box,
        seen,
        notes,
    })
}

fn body(_ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    crate::common::trace("T08: static part");
    let static_report = static_part(result);
    result.metric("static", static_report);

    let mut findings = Findings::new();
    let mut dynamic = serde_json::Map::new();
    let mut reached_in = Vec::new();
    let mut control_has_protection = false;
    let mut any_meta = false;
    for shape in [Shape::Bare, Shape::PlayBin] {
        for with_pssh_box in [false, true] {
            let run = dynamic_run(shape, with_pssh_box)?;
            let key = format!(
                "{}_{}",
                run.shape.label(),
                if run.with_pssh { "pssh" } else { "control" }
            );
            result.note(format!(
                "{key}: pads {:?}; downstream events {:?}; protection events {:?}; {} buffers, {} with a GstProtectionMeta; {}",
                run.seen.pads,
                run.seen.events,
                run.seen.protection,
                run.seen.buffers,
                run.seen.buffers_with_protection_meta,
                run.notes.join("; ")
            ));
            dynamic.insert(
                key,
                json!({
                    "pads": run.seen.pads,
                    "events": run.seen.events,
                    "protection_events": run.seen.protection,
                    "buffers": run.seen.buffers,
                    "buffers_with_protection_meta": run.seen.buffers_with_protection_meta,
                    "notes": run.notes,
                }),
            );
            any_meta |= run.seen.buffers_with_protection_meta > 0;
            if run.with_pssh {
                let wanted: Vec<bool> = [CLEARKEY_SYSTEM_ID, WIDEVINE_SYSTEM_ID]
                    .iter()
                    .map(|id| run.seen.protection.iter().any(|p| p.contains(id)))
                    .collect();
                if wanted.iter().all(|w| *w) {
                    reached_in.push(run.shape.label());
                } else {
                    findings.fail(format!(
                        "{} shape: with pssh boxes for ClearKey and Widevine in the initialisation segment, protection events seen downstream of msesrc: {:?} (ClearKey found: {}, Widevine found: {})",
                        run.shape.label(),
                        run.seen.protection,
                        wanted[0],
                        wanted[1]
                    ));
                }
            } else if !run.seen.protection.is_empty() {
                control_has_protection = true;
            }
        }
    }
    result.metric("dynamic", Value::Object(dynamic));
    result.metric(
        "protection_events_reach_downstream_of_msesrc_in",
        json!(reached_in),
    );
    result.metric("protection_meta_seen_on_buffers", any_meta);
    if control_has_protection {
        result.note("a protection event also appeared in the control run (no pssh box): the probe is not specific");
    }
    result.note(
        "GstProtectionMeta needs encrypted samples (senc/saiz); the fixtures are clear, so only the event path could be tested. A ClearKey fixture is MED-1's job (EA15)",
    );
    result.note(if reached_in.len() == 2 {
        "answer to R14a section 15.1: protection events DO reach the msesrc pads in both shapes, so EA15 can take pssh init data from them".to_owned()
    } else {
        "answer to R14a section 15.1: protection events do NOT (fully) reach the msesrc pads: EA15 must extract pssh init data from the appended bytes itself (mp4parse) before or while calling append".to_owned()
    });
    Ok(findings.verdict())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_parse_into_sixteen_bytes() {
        let clearkey = uuid_bytes(CLEARKEY_SYSTEM_ID).unwrap();
        assert_eq!(clearkey[0], 0x10);
        assert_eq!(clearkey[15], 0x4b);
        let widevine = uuid_bytes(WIDEVINE_SYSTEM_ID).unwrap();
        assert_eq!(&widevine[..4], &[0xed, 0xef, 0x8b, 0xa9]);
        assert_eq!(uuid_bytes("nope"), None);
        assert_eq!(uuid_bytes("1077efec-c0b2-4d02-ace3-3c1e52e2fb4"), None);
        assert_eq!(uuid_bytes("zz77efec-c0b2-4d02-ace3-3c1e52e2fb4b"), None);
    }

    #[test]
    fn matching_lines_is_case_insensitive_and_limited() {
        let text =
            "gst_qtdemux_protection\nunrelated\nPSSH box\n\n   Encrypted sample   \nmore protect\n";
        let hits = matching_lines(text, NEEDLES, 10);
        assert_eq!(
            hits,
            [
                "gst_qtdemux_protection",
                "PSSH box",
                "Encrypted sample",
                "more protect"
            ]
        );
        assert_eq!(matching_lines(text, NEEDLES, 2).len(), 2);
        assert!(matching_lines("nothing here", NEEDLES, 5).is_empty());
    }

    #[test]
    fn very_long_lines_are_not_reported() {
        let long = "protect".repeat(100);
        assert!(matching_lines(&long, NEEDLES, 5).is_empty());
    }

    #[test]
    fn the_arch_argument_names_the_slice() {
        let args = arch_args();
        assert_eq!(args[0], "-arch");
        assert!(args[1] == "arm64" || args[1] == "x86_64");
    }
}
