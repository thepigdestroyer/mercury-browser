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
//!
//! Three variants of the same stream, because a clear stream cannot be a positive control: `clear` (the
//! fixture as it is), `pssh` (clear, with `pssh` boxes for ClearKey and Widevine in the `moov`) and `cenc`
//! (the same `pssh` boxes and the first track declared Common Encryption protected: `encv`, `sinf`, `tenc`,
//! see `mse_segments::with_cenc_protection`; the samples are not encrypted, only the event path is tested).
//!
//! Positive control (added after a skeptic noted that "no event at msesrc" proved nothing while no run
//! showed the probe could see such an event at all): every variant, with the same probe, through a plain
//! `appsrc ! qtdemux` and a plain `appsrc ! parsebin` (what GstMSE's append pipeline is made of), with no
//! GstMSE in the pipeline. Measured on GStreamer 1.28.9, and read in `qtdemux.c` of that release: `qtdemux`
//! emits no protection event for the clear stream with `pssh` boxes (it queues each `pssh` event per stream and
//! pushes the queue only for a track declared protected), so the first version of this scenario could not have
//! seen one; and for the `cenc` variant it refuses the stream, because `gst_protection_select_system` finds no
//! element of class Decryptor in the registry (the bundled GStreamer has none). So no run in this scenario
//! shows a protection event anywhere, and the answer about `msesrc` is UNDETERMINED until a Decryptor element
//! is registered (a pass-through one would do; EA15 needs a real one). The scenario reports that, and turns
//! into a real answer by itself when the control passes.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use mse_rig::harness::Ctx;
use mse_rig::report::ScenarioResult;
use mse_rig::rig::{Rig, RigConfig, Shape};
use mse_segments::{with_cenc_protection, with_pssh};
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
        std::time::Duration::from_secs(100),
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
    /// The caps of every Caps event, cut to 90 characters: a clear stream says `video/x-h264`, an encrypted
    /// one `application/x-cenc`.
    caps: Vec<String>,
}

fn install_probes(rig: &Rig, seen: &Arc<Mutex<Seen>>) -> Result<(), String> {
    let msesrc = rig.msesrc().ok_or("the rig has no msesrc element")?;
    let seen = Arc::clone(seen);
    msesrc.connect_pad_added(move |_, pad| {
        crate::common::trace(&format!("T08 pad-added {}", pad.name()));
        attach_probe(pad, &seen);
    });
    Ok(())
}

/// The one probe of this scenario, on `msesrc`'s pads and on the control pipelines' pads alike: it records
/// the pad, every downstream event by type (with the system ID of each protection event and the caps of each
/// caps event) and every buffer with its protection meta.
fn attach_probe(pad: &gst::Pad, seen: &Arc<Mutex<Seen>>) {
    seen.lock()
        .unwrap_or_else(|e| e.into_inner())
        .pads
        .push(pad.name().to_string());
    let seen = Arc::clone(seen);
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
                if let gst::EventView::Caps(caps) = event.view() {
                    let text = caps.caps().to_string();
                    seen.caps.push(text.chars().take(90).collect());
                }
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
}

/// Which version of the stream a run uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// The fixture as it is.
    Clear,
    /// Clear samples, `pssh` boxes for ClearKey and Widevine in the `moov`.
    Pssh,
    /// The `pssh` boxes and the first track declared protected (`encv`, `sinf`, `tenc`); samples not encrypted.
    Cenc,
}

impl Variant {
    const ALL: [Variant; 3] = [Variant::Clear, Variant::Pssh, Variant::Cenc];

    fn label(self) -> &'static str {
        match self {
            Variant::Clear => "clear",
            Variant::Pssh => "pssh",
            Variant::Cenc => "cenc",
        }
    }

    fn has_pssh(self) -> bool {
        self != Variant::Clear
    }
}

/// The context type a demuxer asks for to learn which protection system the application prefers.
const PREFERRED_SYSTEM_CONTEXT: &str = "drm-preferred-decryption-system-id";

/// The context that tells a demuxer to use ClearKey.
fn preferred_system_context() -> gst::Context {
    let mut context = gst::Context::new(PREFERRED_SYSTEM_CONTEXT, false);
    context
        .get_mut()
        .expect("a new context is writable")
        .structure_mut()
        .set("decryption-system-id", CLEARKEY_SYSTEM_ID);
    context
}

/// The key ID of the `cenc` variant (arbitrary; nothing is encrypted with it).
const PROBE_KEY_ID: [u8; 16] = [
    0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
];

struct RunSummary {
    shape: Shape,
    variant: Variant,
    seen: Seen,
    notes: Vec<String>,
}

/// The type string, the init segment (as the variant says) and the first two media segments.
type ProbeStream = (String, Vec<u8>, Vec<Vec<u8>>);

/// One stream for the GstMSE runs and the controls alike: `h264-low` in the given variant.
fn probe_stream(variant: Variant) -> Result<ProbeStream, String> {
    let video = mse_segments::load("h264-low").map_err(|e| format!("fixture h264-low: {e}"))?;
    let mut init = video.init.clone();
    if variant == Variant::Cenc {
        init = with_cenc_protection(&init, PROBE_KEY_ID, 8)
            .map_err(|e| format!("with_cenc_protection: {e}"))?;
    }
    if variant.has_pssh() {
        for (id, payload) in [
            (CLEARKEY_SYSTEM_ID, &b"stress-probe-clearkey"[..]),
            (WIDEVINE_SYSTEM_ID, &b"stress-probe-widevine"[..]),
        ] {
            let system = uuid_bytes(id).ok_or("bad system id")?;
            init =
                with_pssh(&init, system, payload).map_err(|e| format!("with_pssh({id}): {e}"))?;
        }
    }
    let media = video
        .media
        .iter()
        .take(2)
        .map(|segment| segment.bytes.clone())
        .collect();
    Ok((video.mime.clone(), init.bytes, media))
}

/// How long the rig is left alone before its pipeline goes to NULL. GstMSE's source-buffer task crashes the
/// process (SIGSEGV, T04) when the teardown follows an append or a seek within about 0.8 s; a quiet second
/// before NULL avoided it in every launch T04 measured. Without this the dynamic runs of this scenario died
/// before they could report, and the answer rested on one early run.
const QUIET_BEFORE_TEARDOWN: Duration = Duration::from_millis(1100);

fn dynamic_run(shape: Shape, variant: Variant) -> Result<RunSummary, String> {
    let (mime, init, media) = probe_stream(variant)?;
    let config = RigConfig {
        shape,
        sync: false,
        video: true,
        audio: false,
        ..RigConfig::default()
    };
    crate::common::trace(&format!(
        "T08 {} {}: Rig::new",
        shape.label(),
        variant.label()
    ));
    let rig = crate::common::new_rig(config)?;
    let seen = Arc::new(Mutex::new(Seen::default()));
    if std::env::var_os("MSE_STRESS_T08_NO_PROBES").is_none() {
        install_probes(&rig, &seen)?;
    }
    let id = rig
        .backend
        .add_source_buffer(&mime)
        .map_err(|e| format!("add_source_buffer: {e:?}"))?;
    let mut notes = Vec::new();
    crate::common::trace("T08: appending the initialisation segment");
    if let Err(error) = rig.append_and_wait(id, &init, APPEND_TIMEOUT) {
        notes.push(format!(
            "the {} initialisation segment was refused or failed: {error}",
            variant.label()
        ));
    }
    for segment in &media {
        if let Err(error) = rig.append_and_wait(id, segment, APPEND_TIMEOUT) {
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
    // Read what the probes saw before anything can go wrong in the teardown.
    let seen = std::mem::take(&mut *seen.lock().unwrap_or_else(|e| e.into_inner()));
    crate::common::trace("T08: a quiet second, then shutdown");
    std::thread::sleep(QUIET_BEFORE_TEARDOWN);
    let _ = rig.shutdown();
    crate::common::trace("T08: shut down");
    Ok(RunSummary {
        shape,
        variant,
        seen,
        notes,
    })
}

/// What stands in front of the probe in a control pipeline (no GstMSE): the demuxer alone, or the bin GstMSE
/// puts its appended bytes through (`appsrc ! parsebin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Front {
    Qtdemux,
    Parsebin,
}

impl Front {
    fn label(self) -> &'static str {
        match self {
            Front::Qtdemux => "qtdemux",
            Front::Parsebin => "parsebin",
        }
    }
}

struct ControlSummary {
    front: Front,
    variant: Variant,
    seen: Seen,
    notes: Vec<String>,
}

/// The same bytes and the same probe through `appsrc ! <front>` with a `fakesink` on every pad that appears,
/// with GstMSE nowhere in the pipeline: the positive control for the probe. Runs to the end of the stream or
/// five seconds.
fn control_run(front: Front, variant: Variant) -> Result<ControlSummary, String> {
    let (_, init, media) = probe_stream(variant)?;
    let pipeline = gst::Pipeline::with_name("t08-control");
    let appsrc = gst_app::AppSrc::builder()
        .name("t08-control-src")
        .format(gst::Format::Bytes)
        .build();
    if front == Front::Qtdemux {
        appsrc.set_caps(Some(&gst::Caps::builder("video/quicktime").build()));
    }
    let demux = gst::ElementFactory::make(front.label())
        .name("t08-control-front")
        .build()
        .map_err(|e| format!("{}: {e}", front.label()))?;
    pipeline
        .add_many([appsrc.upcast_ref::<gst::Element>(), &demux])
        .map_err(|e| format!("add: {e}"))?;
    appsrc
        .link(&demux)
        .map_err(|e| format!("link appsrc to {}: {e}", front.label()))?;
    // Also tell the demuxer up front: a context set on the element is used without asking.
    demux.set_context(&preferred_system_context());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let weak = pipeline.downgrade();
    {
        let seen = Arc::clone(&seen);
        demux.connect_pad_added(move |_, pad| {
            attach_probe(pad, &seen);
            let Some(pipeline) = weak.upgrade() else {
                return;
            };
            let Ok(sink) = gst::ElementFactory::make("fakesink")
                .property("sync", false)
                .property("async", false)
                .build()
            else {
                return;
            };
            if pipeline.add(&sink).is_err() {
                return;
            }
            let _ = sink.sync_state_with_parent();
            if let Some(sink_pad) = sink.static_pad("sink") {
                let _ = pad.link(&sink_pad);
            }
        });
    }
    let bus = pipeline.bus().ok_or("the control pipeline has no bus")?;
    // A demuxer that finds a protected track asks (a NEED_CONTEXT message) which protection system the
    // application prefers; the control answers ClearKey, as an EME implementation would. Measured: this does
    // not help while the registry has no Decryptor element for that system (qtdemux still refuses the stream:
    // "no suitable decryptor element has been found"), but it is what a registered decryptor needs.
    bus.set_sync_handler(|_, message| {
        if let gst::MessageView::NeedContext(need) = message.view()
            && need.context_type() == PREFERRED_SYSTEM_CONTEXT
            && let Some(element) = message
                .src()
                .and_then(|src| src.downcast_ref::<gst::Element>())
        {
            element.set_context(&preferred_system_context());
        }
        gst::BusSyncReply::Pass
    });
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("set_state(Playing): {e}"))?;
    for bytes in std::iter::once(&init).chain(media.iter()) {
        appsrc
            .push_buffer(gst::Buffer::from_slice(bytes.clone()))
            .map_err(|e| format!("push_buffer: {e:?}"))?;
    }
    let _ = appsrc.end_of_stream();
    let mut notes = Vec::new();
    let message = bus.timed_pop_filtered(
        gst::ClockTime::from_seconds(5),
        &[gst::MessageType::Error, gst::MessageType::Eos],
    );
    match message.as_ref().map(|message| message.view()) {
        Some(gst::MessageView::Eos(_)) => notes.push("the stream ended (EOS)".to_owned()),
        Some(gst::MessageView::Error(error)) => {
            notes.push(format!("pipeline error: {}", error.error()));
        }
        _ => notes.push("no EOS and no error within 5 s".to_owned()),
    }
    let _ = pipeline.set_state(gst::State::Null);
    let seen = std::mem::take(&mut *seen.lock().unwrap_or_else(|e| e.into_inner()));
    Ok(ControlSummary {
        front,
        variant,
        seen,
        notes,
    })
}

/// Whether `protection` (the descriptions the probe wrote) holds an event for both ClearKey and Widevine.
fn has_both_systems(protection: &[String]) -> bool {
    [CLEARKEY_SYSTEM_ID, WIDEVINE_SYSTEM_ID]
        .iter()
        .all(|id| protection.iter().any(|p| p.contains(id)))
}

/// The answer to R14a 15.1, from the measured facts: whether the probe is known to be able to see the events
/// (`positive_control`: `qtdemux` alone emitted both for the `cenc` variant), whether `parsebin` forwarded them,
/// and in how many shapes they reached `msesrc` for the `cenc` variant.
fn answer(positive_control: bool, parsebin_forwards: bool, reached_in: usize) -> String {
    match (positive_control, reached_in) {
        (false, _) => "answer to R14a section 15.1: UNDETERMINED. qtdemux alone emitted no protection event for either test stream: never for the clear stream with pssh boxes (qtdemux pushes the queued events only for a track declared protected), and for the protected-shaped stream it refused the stream because the registry has no Decryptor element. So the probe has never been shown to see a protection event and msesrc's silence says nothing about GstMSE. A positive control needs a registered Decryptor element (a pass-through one is about 150 lines; EA15 needs a real one); not built because the answer does not change the verdict: Plan B is unaffected, the DOM parses pssh itself".to_owned(),
        (true, 2) => "answer to R14a section 15.1: protection events DO reach the msesrc pads in both shapes for a protected-shaped stream (qtdemux alone emits them, so the probe can see them), so EA15 can take pssh init data from them".to_owned(),
        (true, 0) => format!(
            "answer to R14a section 15.1: protection events do NOT reach the msesrc pads in either shape for a protected-shaped stream (encv/sinf/tenc, unencrypted samples), measured against a positive control (qtdemux alone emits them{}), so GstMSE drops them; real encrypted samples were not tested. EA15 must extract pssh init data from the appended bytes itself",
            if parsebin_forwards { ", and parsebin forwards them" } else { ", and parsebin does NOT forward them" }
        ),
        (true, _) => "answer to R14a section 15.1: protection events reach the msesrc pads in only one of the two shapes for a protected-shaped stream (the positive control passed): partial, see the per-shape lines".to_owned(),
    }
}

fn body(_ctx: &Ctx, result: &mut ScenarioResult) -> Result<Verdict, String> {
    crate::common::trace("T08: static part");
    let static_report = static_part(result);
    result.metric("static", static_report);

    let mut findings = Findings::new();

    // The positive control first: no GstMSE in it, so nothing here can crash in a GstMSE teardown.
    let mut controls = serde_json::Map::new();
    let mut qtdemux_clear_has_protection = false;
    let mut qtdemux_pssh_protection: Vec<String> = Vec::new();
    let mut qtdemux_cenc_protection: Vec<String> = Vec::new();
    let mut parsebin_cenc_protection: Vec<String> = Vec::new();
    for front in [Front::Qtdemux, Front::Parsebin] {
        for variant in Variant::ALL {
            crate::common::trace(&format!(
                "T08 control {} {}",
                front.label(),
                variant.label()
            ));
            let run = control_run(front, variant)?;
            let key = format!("{}_{}", run.front.label(), run.variant.label());
            result.note(format!(
                "control {key} (no GstMSE): pads {:?}; caps {:?}; downstream events {:?}; protection events {:?}; {} buffers; {}",
                run.seen.pads,
                run.seen.caps,
                run.seen.events,
                run.seen.protection,
                run.seen.buffers,
                run.notes.join("; ")
            ));
            controls.insert(
                key,
                json!({
                    "pads": run.seen.pads,
                    "caps": run.seen.caps,
                    "events": run.seen.events,
                    "protection_events": run.seen.protection,
                    "buffers": run.seen.buffers,
                    "notes": run.notes,
                }),
            );
            match (run.front, run.variant) {
                (Front::Qtdemux, Variant::Clear) => {
                    qtdemux_clear_has_protection = !run.seen.protection.is_empty();
                }
                (Front::Qtdemux, Variant::Pssh) => qtdemux_pssh_protection = run.seen.protection,
                (Front::Qtdemux, Variant::Cenc) => qtdemux_cenc_protection = run.seen.protection,
                (Front::Parsebin, Variant::Cenc) => parsebin_cenc_protection = run.seen.protection,
                (Front::Parsebin, _) => {}
            }
        }
    }
    result.metric("controls_without_gstmse", Value::Object(controls));
    let positive_control = has_both_systems(&qtdemux_cenc_protection);
    let parsebin_forwards = has_both_systems(&parsebin_cenc_protection);
    let clear_pssh_emits = !qtdemux_pssh_protection.is_empty();
    result.metric(
        "positive_control_qtdemux_emits_protection",
        positive_control,
    );
    result.metric("parsebin_forwards_protection", parsebin_forwards);
    result.metric(
        "qtdemux_emits_protection_for_clear_stream_with_pssh",
        clear_pssh_emits,
    );
    if qtdemux_clear_has_protection {
        result.note("a protection event also appeared in the control run without pssh boxes: the probe is not specific");
    }
    result.note(format!(
        "positive control: qtdemux alone emits protection events for the protected-shaped stream (cenc): {positive_control}; for the clear stream with pssh boxes: {clear_pssh_emits}; parsebin forwards the cenc ones: {parsebin_forwards}"
    ));

    let mut dynamic = serde_json::Map::new();
    let mut reached_in = Vec::new();
    let mut clear_has_protection = false;
    let mut any_meta = false;
    for shape in [Shape::Bare, Shape::PlayBin] {
        for variant in Variant::ALL {
            let run = dynamic_run(shape, variant)?;
            let key = format!("{}_{}", run.shape.label(), run.variant.label());
            result.note(format!(
                "{key}: pads {:?}; caps {:?}; downstream events {:?}; protection events {:?}; {} buffers, {} with a GstProtectionMeta; {}",
                run.seen.pads,
                run.seen.caps,
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
                    "caps": run.seen.caps,
                    "events": run.seen.events,
                    "protection_events": run.seen.protection,
                    "buffers": run.seen.buffers,
                    "buffers_with_protection_meta": run.seen.buffers_with_protection_meta,
                    "notes": run.notes,
                }),
            );
            any_meta |= run.seen.buffers_with_protection_meta > 0;
            match run.variant {
                Variant::Clear => clear_has_protection |= !run.seen.protection.is_empty(),
                Variant::Pssh => {}
                Variant::Cenc if has_both_systems(&run.seen.protection) => {
                    reached_in.push(run.shape.label());
                }
                Variant::Cenc => {
                    let found: Vec<bool> = [CLEARKEY_SYSTEM_ID, WIDEVINE_SYSTEM_ID]
                        .iter()
                        .map(|id| run.seen.protection.iter().any(|p| p.contains(id)))
                        .collect();
                    findings.fail(format!(
                        "{} shape: with the track declared protected and pssh boxes for ClearKey and Widevine in the initialisation segment, protection events seen downstream of msesrc: {:?} (ClearKey found: {}, Widevine found: {})",
                        run.shape.label(),
                        run.seen.protection,
                        found[0],
                        found[1]
                    ));
                }
            }
        }
    }
    result.metric("dynamic", Value::Object(dynamic));
    result.metric(
        "protection_events_reach_downstream_of_msesrc_in",
        json!(reached_in),
    );
    result.metric("protection_meta_seen_on_buffers", any_meta);
    if clear_has_protection {
        result.note("a protection event also appeared in the clear run at msesrc: the probe is not specific");
    }
    if !positive_control {
        findings.fail(
            "no positive control: qtdemux alone emitted no protection event for the protected-shaped stream, so the probe has never been shown to see one",
        );
    }
    result.note(
        "GstProtectionMeta needs encrypted samples (senc/saiz); the cenc variant declares the track protected but its samples are clear, so only the event path could be tested. Real ClearKey content is MED-1's job (EA15)",
    );
    let text = answer(positive_control, parsebin_forwards, reached_in.len());
    result.metric("answer", json!(text));
    result.note(text);
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
    fn both_systems_must_be_present_for_a_protection_set_to_count() {
        let one = vec![format!(
            "system_id={CLEARKEY_SYSTEM_ID} origin=\"isobmff/moov\" data_bytes=Some(40)"
        )];
        assert!(!has_both_systems(&one));
        let mut both = one.clone();
        both.push(format!(
            "system_id={WIDEVINE_SYSTEM_ID} origin=\"isobmff/moov\" data_bytes=Some(40)"
        ));
        assert!(has_both_systems(&both));
        assert!(!has_both_systems(&[]));
    }

    #[test]
    fn the_answer_follows_the_positive_control() {
        // Without a positive control msesrc's silence proves nothing, whatever it saw.
        assert!(answer(false, false, 0).contains("UNDETERMINED"));
        assert!(answer(false, true, 0).contains("UNDETERMINED"));
        // With one, silence is a measured "no", and parsebin's behaviour is part of the sentence.
        let none = answer(true, true, 0);
        assert!(none.contains("do NOT reach") && none.contains("parsebin forwards them"));
        assert!(answer(true, false, 0).contains("parsebin does NOT forward"));
        assert!(answer(true, true, 2).contains("DO reach"));
        assert!(answer(true, true, 1).contains("only one of the two shapes"));
    }

    #[test]
    fn the_fronts_are_named_for_their_factories() {
        assert_eq!(Front::Qtdemux.label(), "qtdemux");
        assert_eq!(Front::Parsebin.label(), "parsebin");
    }

    #[test]
    fn the_variants_differ_only_in_the_initialisation_segment() {
        let (mime, clear, media) = probe_stream(Variant::Clear).unwrap();
        let (_, pssh, media_pssh) = probe_stream(Variant::Pssh).unwrap();
        let (_, cenc, media_cenc) = probe_stream(Variant::Cenc).unwrap();
        assert!(mime.starts_with("video/mp4"));
        assert!(pssh.len() > clear.len());
        // The protected-shaped one adds the sinf (80 bytes) to the pssh one.
        assert_eq!(cenc.len(), pssh.len() + 80);
        assert_eq!(media.len(), 2);
        assert_eq!(media, media_pssh);
        assert_eq!(media, media_cenc);
    }

    #[test]
    fn only_the_cenc_variant_declares_the_track_protected() {
        let (_, init, _) = probe_stream(Variant::Cenc).unwrap();
        assert!(init.windows(4).any(|w| w == b"encv"));
        assert!(init.windows(4).any(|w| w == b"tenc"));
        for variant in [Variant::Clear, Variant::Pssh] {
            let (_, init, _) = probe_stream(variant).unwrap();
            assert!(!init.windows(4).any(|w| w == b"encv"));
            assert_eq!(variant.has_pssh(), variant == Variant::Pssh);
        }
        assert_eq!(Variant::ALL.map(Variant::label), ["clear", "pssh", "cenc"]);
    }

    #[test]
    fn the_arch_argument_names_the_slice() {
        let args = arch_args();
        assert_eq!(args[0], "-arch");
        assert!(args[1] == "arm64" || args[1] == "x86_64");
    }
}
