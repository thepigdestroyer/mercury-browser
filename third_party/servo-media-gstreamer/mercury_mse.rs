// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Mercury: this file is new (lane med-1, MSE by Plan B of R14a). It is the whole of the glue between
// Servo's media layer and Mercury's Media Source Extensions backend: `crates/mercury-mse` (parsers,
// coded-frame processing, track buffers, the MediaSource model, a threaded `MseBackend`) and
// `crates/mercury-mse-gst` (the `mercury-mse://` source element that feeds GStreamer's `appsrc`).
// Nothing in it is copied from Servo or another project. The hooks are in `lib.rs` (registration,
// `create_media_source`, `is_type_supported_mse`) and `player.rs` (`StreamType::MediaSource`,
// `connect_media_source`, `source-setup`); `MERCURY.md` lists them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use ::mercury_mse::{
    EventSink, Limits, MediaSourceBackend, MemoryBudget, MseBackend, MseError, MseEvent,
    TypeSupport,
};
use ::mercury_mse_gst::{GstTypeSupport, MSE_URI_SCHEME};
use log::{debug, warn};
use servo_base::generic_channel::GenericCallback;

/// Set once `mercury_mse_gst::register()` succeeded: the `MseSrc` element and its `mercury-mse://` URI
/// handler exist. Without it MSE is reported as unsupported and a MediaSource player refuses to start.
static REGISTERED: AtomicBool = AtomicBool::new(false);

/// The most coded-frame memory all the MediaSources of this process may hold together (decision D17 of
/// lane med-1, review S3): 1 GiB. One video source buffer may hold 150 MiB and a page may have several
/// players, so a per-source quota alone let a few tabs (or one hostile page with many elements) take
/// gigabytes; Servo runs the pages of a Mercury window in one process, so the budget is per process and
/// shared by every tab. 1 GiB leaves room for about six full 1080p players and is a small part of a
/// 16 GB Mac; a tab that hits it gets `QuotaExceededError` and must `remove()`, as for its own quota.
const PROCESS_BUDGET_BYTES: usize = 1024 * 1024 * 1024;

/// The budget every MediaSource backend of the process charges.
fn process_budget() -> &'static Arc<MemoryBudget> {
    static BUDGET: OnceLock<Arc<MemoryBudget>> = OnceLock::new();
    BUDGET.get_or_init(|| MemoryBudget::new(PROCESS_BUDGET_BYTES))
}

/// Registers the `MseSrc` element with GStreamer (idempotent). Called after `gstreamer::init()` and the
/// plugin loading. A failure is logged and leaves MSE unsupported; it never stops the media backend.
pub(crate) fn register() {
    match ::mercury_mse_gst::register() {
        Ok(()) => REGISTERED.store(true, Ordering::Release),
        Err(error) => {
            REGISTERED.store(false, Ordering::Release);
            warn!("Media Source Extensions are unavailable: the MseSrc element did not register: {error}");
        },
    }
}

/// `true` when the `MseSrc` element is registered.
pub(crate) fn is_available() -> bool {
    REGISTERED.load(Ordering::Acquire)
}

/// The URI `playbin3` is given for the MediaSource player `player_id`: `mercury-mse://<id>`. GStreamer
/// finds the `MseSrc` element through its URI handler; the id only keeps the names apart.
pub(crate) fn uri_for(player_id: usize) -> String {
    format!("{MSE_URI_SCHEME}://{player_id}")
}

/// `MediaSource.isTypeSupported(mime)`: the container and every codec are ones the backend feeds and the
/// GStreamer registry has a decoder for.
pub(crate) fn is_type_supported(mime: &str) -> bool {
    is_available() && MseBackend::is_type_supported(mime, &GstTypeSupport as &dyn TypeSupport)
}

/// A new MediaSource backend. Its events go to `events`, from the backend's worker thread: the callback the
/// script thread made only queues a task there, and nothing in this crate calls back into the backend on
/// that thread's behalf (the rule of `mercury_mse::MseEvent`).
pub(crate) fn create_media_source(
    events: GenericCallback<MseEvent>,
) -> Result<Arc<dyn MediaSourceBackend>, MseError> {
    if !is_available() {
        return Err(MseError::NotSupported(
            "the MseSrc GStreamer element is not registered".to_owned(),
        ));
    }
    // The callback may not be `Sync` (it can wrap an IPC sender), so it sits behind a mutex like the
    // player's own observer does.
    let events = Mutex::new(events);
    let sink: EventSink = Arc::new(move |event| {
        let result = events
            .lock()
            .expect("the MSE event callback mutex is never poisoned")
            .send(event);
        if let Err(error) = result {
            debug!("An MSE event could not be delivered (the media element is gone): {error:?}");
        }
    });
    Ok(Arc::new(MseBackend::with_budget(
        sink,
        Arc::new(GstTypeSupport),
        Limits::default(),
        Arc::clone(process_budget()),
    )))
}

/// Connects the source element `playbin3` made for the `mercury-mse://` URI (the `source-setup`
/// argument) to the backend's frame feed. Runs on a GStreamer thread: it only hands the feed over.
pub(crate) fn attach(
    source: &gstreamer::Element,
    backend: &Arc<dyn MediaSourceBackend>,
) -> Result<(), String> {
    ::mercury_mse_gst::attach(source, backend.frame_feed())
}
