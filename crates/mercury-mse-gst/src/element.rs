// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `MseSrc`: a `GstBin` that is the URI handler of `mercury-mse://` and holds one `appsrc` per track.
//!
//! Shape (the choice the brief's risk 1 is about): the bin has no pads when it is created. When the feed
//! knows its tracks (`FrameFeed::all_initialized`), the feeder thread adds one `appsrc` per track
//! (`format=time`, `stream-type=seekable`, not live, not blocking), a sometimes ghost pad for each
//! (`video_0`, `audio_0`), and says `no-more-pads`. Every pad carries the same `stream-start` group id
//! (a probe replaces the one `GstBaseSrc` would send with its own group) and a stream collection that
//! lists all the tracks, as GStreamer's own `msesrc` does.
//!
//! A seek event arrives on one pad; the element sends it to every appsrc (as a demuxer flushes all its
//! pads) and answers it once; the appsrcs' `seek-data` callbacks tell the feeder, which calls
//! `FrameFeed::seek` once and pushes again from the random access point.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::{JoinHandle, ThreadId};

use glib::subclass::prelude::*;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::subclass::prelude::*;
use gstreamer_app as gst_app;
use mercury_mse::{FrameFeed, TrackConfig, TrackKind};

use crate::caps::{caps_for, stream_type};
use crate::feeder::{Feeder, TrackOut};
use crate::shared::Shared;
use crate::{ELEMENT_NAME, MSE_URI_SCHEME};

/// The debug category (`GST_DEBUG=mercurymsesrc:6`).
static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        ELEMENT_NAME,
        gst::DebugColorFlags::empty(),
        Some("Mercury MSE source"),
    )
});

/// Numbers the elements of every `MseSrc` of the process, so that the appsrcs of two pipelines never share a
/// name (`playbin3` calls every source `source`).
static SERIAL: AtomicU32 = AtomicU32::new(0);

/// The appsrc queue: bytes and time. Both limits apply; `enough-data` fires when either is passed.
const VIDEO_MAX_BYTES: u64 = 8 * 1024 * 1024;
const AUDIO_MAX_BYTES: u64 = 1024 * 1024;
const MAX_TIME: gst::ClockTime = gst::ClockTime::from_seconds(3);
/// `need-data` fires again when the queue falls under this share of its limit.
const MIN_PERCENT: u32 = 50;

mod imp {
    use super::*;

    #[derive(Default)]
    struct Attached {
        shared: Option<Arc<Shared>>,
        thread: Option<(JoinHandle<()>, ThreadId)>,
    }

    /// What the first announcement fixed, for the tracks that come later: the numbering of the stream
    /// ids, the group id every pad shares, and the streams announced so far.
    struct Announced {
        serial: u32,
        group_id: gst::GroupId,
        streams: Vec<(TrackKind, gst::Stream)>,
    }

    /// One track before its elements exist: caps and stream.
    struct Plan {
        config: TrackConfig,
        caps: gst::Caps,
        stream: gst::Stream,
        stream_id: String,
    }

    /// The elements of one track.
    struct Output {
        kind: TrackKind,
        appsrc: gst_app::AppSrc,
        ghost: gst::GhostPad,
        config: TrackConfig,
    }

    /// The stream collection that lists these streams.
    fn collection_of(name: &str, streams: &[(TrackKind, gst::Stream)]) -> gst::StreamCollection {
        gst::StreamCollection::builder(Some(name))
            .streams(streams.iter().map(|(_, stream)| stream.clone()))
            .build()
    }

    #[derive(Default)]
    pub struct MseSrc {
        uri: Mutex<Option<String>>,
        attached: Mutex<Attached>,
        /// The appsrcs, for the seek handler (the feeder thread keeps its own handles).
        outputs: Mutex<Vec<gst_app::AppSrc>>,
        announced: Mutex<Option<Announced>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MseSrc {
        const NAME: &'static str = "MercuryMseSrc";
        type Type = super::MseSrc;
        type ParentType = gst::Bin;
        type Interfaces = (gst::URIHandler,);
    }

    impl ObjectImpl for MseSrc {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_element_flags(gst::ElementFlags::SOURCE);
        }

        fn dispose(&self) {
            self.shutdown();
        }
    }

    impl GstObjectImpl for MseSrc {}

    impl ElementImpl for MseSrc {
        fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
            static METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
                gst::subclass::ElementMetadata::new(
                    "Mercury MSE Source",
                    "Source/Audio/Video",
                    "Feeds the coded frames of a MediaSource to the pipeline, one appsrc per track",
                    "Mercury Browser contributors",
                )
            });
            Some(&*METADATA)
        }

        fn pad_templates() -> &'static [gst::PadTemplate] {
            static TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
                let video: gst::Caps =
                    "video/x-h264; video/x-h265; video/x-vp8; video/x-vp9; video/x-av1"
                        .parse()
                        .expect("static caps");
                let audio: gst::Caps =
                    "audio/mpeg; audio/x-opus; audio/x-ac3; audio/x-eac3; audio/x-flac; audio/x-vorbis"
                        .parse()
                        .expect("static caps");
                vec![
                    gst::PadTemplate::new(
                        "video_%u",
                        gst::PadDirection::Src,
                        gst::PadPresence::Sometimes,
                        &video,
                    )
                    .expect("video pad template"),
                    gst::PadTemplate::new(
                        "audio_%u",
                        gst::PadDirection::Src,
                        gst::PadPresence::Sometimes,
                        &audio,
                    )
                    .expect("audio pad template"),
                ]
            });
            TEMPLATES.as_ref()
        }

        fn change_state(
            &self,
            transition: gst::StateChange,
        ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
            let result = self.parent_change_state(transition);
            if transition == gst::StateChange::ReadyToNull {
                // Ask the feeder to end; it is joined when the element is disposed. Joining here could
                // wait for a feeder that is itself waiting for the state lock this call holds.
                if let Some(shared) = self.attached.lock().unwrap().shared.clone() {
                    shared.request_stop();
                }
            }
            result
        }
    }

    impl BinImpl for MseSrc {}

    impl URIHandlerImpl for MseSrc {
        const URI_TYPE: gst::URIType = gst::URIType::Src;

        fn protocols() -> &'static [&'static str] {
            &[MSE_URI_SCHEME]
        }

        fn uri(&self) -> Option<String> {
            Some(
                self.uri
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| format!("{MSE_URI_SCHEME}://")),
            )
        }

        fn set_uri(&self, uri: &str) -> Result<(), glib::Error> {
            let prefix = format!("{MSE_URI_SCHEME}:");
            let valid =
                uri.len() >= prefix.len() && uri[..prefix.len()].eq_ignore_ascii_case(&prefix);
            if !valid {
                return Err(glib::Error::new(
                    gst::URIError::BadUri,
                    &format!("Invalid URI '{uri}': not a {MSE_URI_SCHEME} URI"),
                ));
            }
            *self.uri.lock().unwrap() = Some(uri.to_owned());
            Ok(())
        }
    }

    impl MseSrc {
        /// Connects the element to its feed and starts the feeder thread.
        pub(super) fn attach(&self, feed: Arc<dyn FrameFeed>) -> Result<(), String> {
            let mut attached = self.attached.lock().unwrap();
            if attached.shared.is_some() {
                return Err("this MseSrc is already attached to a feed".to_owned());
            }
            let shared = Shared::new();
            // A weak reference: the feed outlives the element, and a stale wake must do nothing.
            let weak = Arc::downgrade(&shared);
            feed.set_wake(Arc::new(move || {
                if let Some(shared) = weak.upgrade() {
                    shared.wake();
                }
            }));
            let feeder = Feeder::new(Arc::clone(&shared), feed, self.obj().downgrade());
            let handle = std::thread::Builder::new()
                .name(format!("mse-feed-{}", self.obj().name()))
                .spawn(move || feeder.run())
                .map_err(|error| format!("cannot start the feeder thread: {error}"))?;
            let id = handle.thread().id();
            attached.shared = Some(shared);
            attached.thread = Some((handle, id));
            Ok(())
        }

        /// Ends the feeder thread and waits for it (it never sleeps, so this is immediate). A feeder
        /// that holds the last reference to the element (it upgrades a weak one while it announces)
        /// runs this itself: it does not wait for itself.
        pub(super) fn shutdown(&self) {
            let (shared, thread) = {
                let mut attached = self.attached.lock().unwrap();
                (attached.shared.take(), attached.thread.take())
            };
            if let Some(shared) = shared {
                shared.request_stop();
            }
            if let Some((handle, id)) = thread
                && std::thread::current().id() != id
            {
                let _ = handle.join();
            }
            self.outputs.lock().unwrap().clear();
        }

        /// Adds an appsrc and a pad for each track the feed knows, from the feeder thread.
        pub(super) fn announce(
            &self,
            shared: &Arc<Shared>,
            configs: &[TrackConfig],
            duration: Option<u64>,
        ) -> Result<Vec<TrackOut>, String> {
            let obj = self.obj();
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let group_id = gst::GroupId::next();

            // Everything that can fail first, so that nothing is half done.
            let mut plans: Vec<Plan> = Vec::new();
            for config in configs {
                if config.kind == TrackKind::Text {
                    gst::info!(CAT, imp = self, "a text track is not fed in this round");
                    continue;
                }
                if plans.iter().any(|plan| plan.config.kind == config.kind) {
                    gst::warning!(
                        CAT,
                        imp = self,
                        "a second {} track is ignored",
                        config.kind.name()
                    );
                    continue;
                }
                plans.push(self.plan(serial, config)?);
            }
            if plans.is_empty() {
                return Err("the source has no track GStreamer can play".to_owned());
            }
            let streams: Vec<(TrackKind, gst::Stream)> = plans
                .iter()
                .map(|plan| (plan.config.kind, plan.stream.clone()))
                .collect();
            let collection = collection_of(obj.name().as_str(), &streams);

            let mut outputs = Vec::new();
            for plan in &plans {
                outputs.push(self.make_output(
                    shared,
                    plan,
                    serial,
                    group_id,
                    &collection,
                    duration,
                )?);
            }
            *self.outputs.lock().unwrap() =
                outputs.iter().map(|output| output.appsrc.clone()).collect();
            for output in &outputs {
                shared.mark_present(output.kind);
            }
            // Pads out first (the pipeline links them while this runs), then the appsrcs start.
            for output in &outputs {
                obj.add_pad(&output.ghost)
                    .map_err(|error| format!("cannot add a pad: {error}"))?;
            }
            obj.no_more_pads();
            let mut tracks = Vec::new();
            for output in outputs {
                output
                    .appsrc
                    .sync_state_with_parent()
                    .map_err(|error| format!("cannot start the appsrc: {error}"))?;
                tracks.push(TrackOut::new(output.kind, output.appsrc, &output.config));
            }
            *self.announced.lock().unwrap() = Some(Announced {
                serial,
                group_id,
                streams,
            });
            gst::info!(CAT, imp = self, "announced {} track(s)", tracks.len());
            Ok(tracks)
        }

        /// Adds the track of a kind the feed did not have when the streams were announced (a source
        /// buffer added or initialized later: decision D17): one more appsrc, its pad, and a stream
        /// collection that lists every track, so that `decodebin3` and `playbin3` see the update. Called
        /// from the feeder thread. `Ok(None)` when the kind is already announced or nothing was announced.
        pub(super) fn add_track(
            &self,
            shared: &Arc<Shared>,
            config: &TrackConfig,
            duration: Option<u64>,
        ) -> Result<Option<TrackOut>, String> {
            let obj = self.obj();
            let (serial, group_id, mut streams) = {
                let announced = self.announced.lock().unwrap();
                let Some(announced) = announced.as_ref() else {
                    return Ok(None);
                };
                (
                    announced.serial,
                    announced.group_id,
                    announced.streams.clone(),
                )
            };
            if config.kind == TrackKind::Text
                || streams.iter().any(|(kind, _)| *kind == config.kind)
            {
                return Ok(None);
            }
            let plan = self.plan(serial, config)?;
            streams.push((config.kind, plan.stream.clone()));
            let collection = collection_of(obj.name().as_str(), &streams);
            let output =
                self.make_output(shared, &plan, serial, group_id, &collection, duration)?;
            self.outputs.lock().unwrap().push(output.appsrc.clone());
            shared.mark_present(output.kind);
            obj.add_pad(&output.ghost)
                .map_err(|error| format!("cannot add a pad: {error}"))?;
            obj.no_more_pads();
            output
                .appsrc
                .sync_state_with_parent()
                .map_err(|error| format!("cannot start the appsrc: {error}"))?;
            if let Some(announced) = self.announced.lock().unwrap().as_mut() {
                announced.streams = streams;
            }
            gst::info!(CAT, imp = self, "added a late {} track", config.kind.name());
            Ok(Some(TrackOut::new(
                output.kind,
                output.appsrc,
                &output.config,
            )))
        }

        /// The caps and the `GstStream` of one track, or why there are none.
        fn plan(&self, serial: u32, config: &TrackConfig) -> Result<Plan, String> {
            let caps = caps_for(config)?;
            let stream_id = format!("{}/{serial}/{}", self.obj().name(), config.kind.name());
            let stream = gst::Stream::new(
                Some(&stream_id),
                Some(&caps),
                stream_type(config.kind),
                gst::StreamFlags::SELECT,
            );
            Ok(Plan {
                config: config.clone(),
                caps,
                stream,
                stream_id,
            })
        }

        /// The appsrc and the ghost pad of one planned track, added to the bin (the pad is not added
        /// yet).
        fn make_output(
            &self,
            shared: &Arc<Shared>,
            plan: &Plan,
            serial: u32,
            group_id: gst::GroupId,
            collection: &gst::StreamCollection,
            duration: Option<u64>,
        ) -> Result<Output, String> {
            let obj = self.obj();
            let kind = plan.config.kind;
            let appsrc = gst::ElementFactory::make("appsrc")
                .name(format!("{}-{serial}-{}", obj.name(), kind.name()))
                .build()
                .map_err(|error| format!("cannot create an appsrc: {error}"))?
                .downcast::<gst_app::AppSrc>()
                .map_err(|_| "appsrc is not an AppSrc".to_owned())?;
            appsrc.set_format(gst::Format::Time);
            appsrc.set_stream_type(gst_app::AppStreamType::Seekable);
            appsrc.set_is_live(false);
            appsrc.set_block(false);
            appsrc.set_max_bytes(match kind {
                TrackKind::Audio => AUDIO_MAX_BYTES,
                _ => VIDEO_MAX_BYTES,
            });
            appsrc.set_property("max-time", MAX_TIME.nseconds());
            appsrc.set_min_percent(MIN_PERCENT);
            appsrc.set_caps(Some(&plan.caps));
            appsrc.set_duration(duration.map(gst::ClockTime::from_nseconds));

            // The callbacks only record and notify: they never call the feed.
            let (need, enough, seek) = (Arc::clone(shared), Arc::clone(shared), Arc::clone(shared));
            appsrc.set_callbacks(
                gst_app::AppSrcCallbacks::builder()
                    .need_data(move |_, _| need.on_need_data(kind))
                    .enough_data(move |_| enough.on_enough_data(kind))
                    .seek_data(move |_, position| {
                        seek.on_seek_data(kind, position);
                        true
                    })
                    .build(),
            );

            let src_pad = appsrc
                .static_pad("src")
                .ok_or_else(|| "appsrc has no src pad".to_owned())?;
            install_stream_probe(
                &src_pad,
                plan.stream_id.clone(),
                group_id,
                plan.stream.clone(),
                collection.clone(),
            );

            obj.add(&appsrc)
                .map_err(|error| format!("cannot add the appsrc to the bin: {error}"))?;

            let template_name = match kind {
                TrackKind::Audio => "audio_%u",
                _ => "video_%u",
            };
            let template = obj
                .pad_template(template_name)
                .ok_or_else(|| format!("no pad template {template_name}"))?;
            let ghost = gst::GhostPad::builder_from_template(&template)
                .name(format!("{}_0", kind.name()))
                .query_function(|pad, parent, query| {
                    MseSrc::catch_panic_pad_function(
                        parent,
                        || false,
                        |this| this.src_query(pad, query),
                    )
                })
                .event_function(|pad, parent, event| {
                    MseSrc::catch_panic_pad_function(
                        parent,
                        || false,
                        |this| this.src_event(pad, event),
                    )
                })
                .build();
            ghost
                .set_target(Some(&src_pad))
                .map_err(|error| format!("cannot target the ghost pad: {error}"))?;
            ghost
                .set_active(true)
                .map_err(|error| format!("cannot activate the ghost pad: {error}"))?;
            Ok(Output {
                kind,
                appsrc,
                ghost,
                config: plan.config.clone(),
            })
        }

        /// Posts a stream error on the bus (the pipeline stops with it).
        pub(super) fn fail(&self, message: &str) {
            gst::element_imp_error!(
                self,
                gst::StreamError::Format,
                ("The media source cannot be played"),
                ["{message}"]
            );
        }

        fn shared(&self) -> Option<Arc<Shared>> {
            self.attached.lock().unwrap().shared.clone()
        }

        /// The queries the pads answer themselves; the rest goes to the appsrc behind the pad.
        fn src_query(&self, pad: &gst::GhostPad, query: &mut gst::QueryRef) -> bool {
            let duration = self.shared().and_then(|shared| shared.duration());
            match query.view_mut() {
                gst::QueryViewMut::Duration(q) if q.format() == gst::Format::Time => match duration
                {
                    Some(ns) => {
                        q.set(gst::ClockTime::from_nseconds(ns));
                        true
                    }
                    None => false,
                },
                gst::QueryViewMut::Seeking(q) if q.format() == gst::Format::Time => {
                    q.set(
                        true,
                        gst::ClockTime::ZERO,
                        duration.map(gst::ClockTime::from_nseconds),
                    );
                    true
                }
                gst::QueryViewMut::Scheduling(q) => {
                    // A local, seekable source: no download buffering in front of it.
                    q.set(gst::SchedulingFlags::SEEKABLE, 1, -1, 0);
                    q.add_scheduling_modes([gst::PadMode::Push]);
                    true
                }
                _ => gst::Pad::query_default(pad, Some(&*self.obj()), query),
            }
        }

        /// A seek reaches every appsrc through this one pad (the other pads' copy of it is dropped).
        fn src_event(&self, pad: &gst::GhostPad, event: gst::Event) -> bool {
            if event.type_() != gst::EventType::Seek {
                return gst::Pad::event_default(pad, Some(&*self.obj()), event);
            }
            let Some(shared) = self.shared() else {
                return false;
            };
            if !shared.begin_seek(event.seqnum()) {
                // The same seek, already delivered through another pad.
                return true;
            }
            let appsrcs: Vec<gst_app::AppSrc> = self.outputs.lock().unwrap().clone();
            let mut all_ok = true;
            for appsrc in appsrcs {
                all_ok &= appsrc.send_event(event.clone());
            }
            if !all_ok {
                gst::warning!(CAT, imp = self, "an appsrc refused the seek");
                shared.abort_seek();
            }
            all_ok
        }
    }
}

glib::wrapper! {
    /// The source element of `mercury-mse://` URIs.
    pub struct MseSrc(ObjectSubclass<imp::MseSrc>)
        @extends gst::Bin, gst::Element, gst::Object, @implements gst::URIHandler;
}

impl MseSrc {
    pub(crate) fn attach(&self, feed: Arc<dyn FrameFeed>) -> Result<(), String> {
        self.imp().attach(feed)
    }

    pub(crate) fn announce(
        &self,
        shared: &Arc<Shared>,
        configs: &[TrackConfig],
        duration: Option<u64>,
    ) -> Result<Vec<TrackOut>, String> {
        self.imp().announce(shared, configs, duration)
    }

    pub(crate) fn add_track(
        &self,
        shared: &Arc<Shared>,
        config: &TrackConfig,
        duration: Option<u64>,
    ) -> Result<Option<TrackOut>, String> {
        self.imp().add_track(shared, config, duration)
    }

    pub(crate) fn fail(&self, message: &str) {
        self.imp().fail(message);
    }
}

/// Replaces the `stream-start` `GstBaseSrc` sends (its own stream id and a group id of its own) with the
/// track's: the shared group id and the `GstStream`; and puts the stream collection in front of the first
/// caps. Stays on the appsrc's pad for as long as the pad lives (sticky events are not sent twice).
fn install_stream_probe(
    pad: &gst::Pad,
    stream_id: String,
    group_id: gst::GroupId,
    stream: gst::Stream,
    collection: gst::StreamCollection,
) {
    let collection_sent = AtomicBool::new(false);
    pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |pad, info| {
        let Some(gst::PadProbeData::Event(event)) = &info.data else {
            return gst::PadProbeReturn::Ok;
        };
        let event = event.clone();
        match event.view() {
            gst::EventView::StreamStart(_) => {
                let replacement = gst::event::StreamStart::builder(&stream_id)
                    .seqnum(event.seqnum())
                    .group_id(group_id)
                    .flags(gst::StreamFlags::SELECT)
                    .stream(stream.clone())
                    .build();
                info.data = Some(gst::PadProbeData::Event(replacement));
            }
            gst::EventView::Caps(_) if !collection_sent.swap(true, Ordering::AcqRel) => {
                let _ = pad.push_event(gst::event::StreamCollection::new(&collection));
            }
            _ => {}
        }
        gst::PadProbeReturn::Ok
    });
}
