// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A peer connection with its observers turned into shared state, blocking offer/answer calls,
//! a candidate relay for two peers in one process, and stats as JSON.
//!
//! The wrapper reports everything through `Box<dyn Handler>` callbacks that run on libwebrtc's
//! signaling thread. This module keeps what the callbacks learn in one [`PeerState`] behind a
//! mutex and a condition variable, so a caller waits with a predicate instead of juggling
//! channels. Everything needed to run an offer/answer exchange in one process is here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use shiguredo_webrtc::{
    CreateSessionDescriptionObserver, CreateSessionDescriptionObserverHandler, CxxString,
    DataChannel, IceCandidate, IceCandidateRef, IceConnectionState, IceGatheringState,
    IceTransportsType, MediaStreamTrack, PeerConnection, PeerConnectionDependencies,
    PeerConnectionObserver, PeerConnectionObserverHandler, PeerConnectionOfferAnswerOptions,
    PeerConnectionRtcConfiguration, PeerConnectionState, RtcError, RtpEncodingParameters,
    RtpEncodingParametersVector, RtpReceiver, RtpSender, RtpTransceiver, RtpTransceiverDirection,
    RtpTransceiverInit, SdpType, SessionDescription, SetLocalDescriptionObserver,
    SetLocalDescriptionObserverHandler, SetRemoteDescriptionObserver,
    SetRemoteDescriptionObserverHandler, SignalingState, StringVector, VideoTrack,
};

use crate::channel::{Channel, ChannelOptions};
use crate::engine::Engine;
use crate::{Result, error};

/// libwebrtc's `IceTransportsType::kNoHost`. The wrapper names only `kRelay`; `set_type` casts an
/// integer straight to the C++ enum (`kNone, kRelay, kNoHost, kAll` in `api/peer_connection_
/// interface.h` of the pinned archive), so the other values go through `Unknown`.
pub const ICE_TRANSPORTS_NO_HOST: i32 = 2;
/// libwebrtc's `IceTransportsType::kAll`, the default.
pub const ICE_TRANSPORTS_ALL: i32 = 3;

/// Which candidates the peer may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IceTransports {
    /// Host, server reflexive and relay candidates (libwebrtc's default).
    #[default]
    All,
    /// No host candidates.
    NoHost,
    /// Relay candidates only.
    Relay,
}

impl IceTransports {
    fn raw(self) -> IceTransportsType {
        match self {
            IceTransports::All => IceTransportsType::Unknown(ICE_TRANSPORTS_ALL),
            IceTransports::NoHost => IceTransportsType::Unknown(ICE_TRANSPORTS_NO_HOST),
            IceTransports::Relay => IceTransportsType::Relay,
        }
    }
}

/// What can be set on a new peer (the `RTCConfiguration` fields the wrapper exposes that this
/// spike uses; there are no ICE servers anywhere in the tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct PeerConfig {
    /// Candidate policy.
    pub ice_transports: IceTransports,
}

/// A local ICE candidate as the observer saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// `sdp_mid`.
    pub mid: String,
    /// `sdp_mline_index`.
    pub index: i32,
    /// The `candidate:` line.
    pub sdp: String,
}

impl Candidate {
    /// The candidate type (`host`, `srflx`, `relay`, `prflx`) from the SDP line.
    pub fn kind(&self) -> &str {
        let mut words = self.sdp.split_whitespace();
        while let Some(word) = words.next() {
            if word == "typ" {
                return words.next().unwrap_or("");
            }
        }
        ""
    }

    /// The connection address field (an IP address, or an mDNS name ending in `.local`).
    pub fn address(&self) -> &str {
        self.sdp.split_whitespace().nth(4).unwrap_or("")
    }
}

/// A track the remote side added (from `on_track`).
pub struct RemoteTrack {
    /// `audio` or `video`.
    pub kind: String,
    /// The receiving track.
    pub track: MediaStreamTrack,
    /// The transceiver it arrived on.
    pub transceiver: RtpTransceiver,
}

/// Everything the observer learned.
pub struct PeerState {
    /// Signaling state.
    pub signaling: SignalingState,
    /// Peer connection state.
    pub connection: PeerConnectionState,
    /// ICE connection state (standardised).
    pub ice_connection: IceConnectionState,
    /// ICE gathering state.
    pub gathering: IceGatheringState,
    /// Local candidates in the order they were gathered.
    pub candidates: Vec<Candidate>,
    /// Remote tracks, in the order they were announced.
    pub tracks: Vec<RemoteTrack>,
    /// `on_remove_track` calls.
    pub removed_tracks: usize,
    /// Data channels the remote side created and nobody took yet, with their labels (read in the
    /// callback: no call into libwebrtc is made while the state lock is held).
    pub data_channels: Vec<(String, DataChannel)>,
    /// A readable log of every callback, oldest first.
    pub events: Vec<String>,
}

struct Shared {
    state: Mutex<PeerState>,
    changed: Condvar,
    remote_description_set: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, PeerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, event: String, change: impl FnOnce(&mut PeerState)) {
        let mut state = self.lock();
        change(&mut state);
        state.events.push(event);
        drop(state);
        self.changed.notify_all();
    }
}

struct Observer(Arc<Shared>);

impl PeerConnectionObserverHandler for Observer {
    fn on_signaling_change(&mut self, new_state: SignalingState) {
        self.0.update(format!("signaling {new_state:?}"), |s| {
            s.signaling = new_state
        });
    }

    fn on_connection_change(&mut self, new_state: PeerConnectionState) {
        self.0.update(format!("connection {new_state:?}"), |s| {
            s.connection = new_state
        });
    }

    fn on_standardized_ice_connection_change(&mut self, new_state: IceConnectionState) {
        self.0.update(format!("ice {new_state:?}"), |s| {
            s.ice_connection = new_state
        });
    }

    fn on_ice_gathering_change(&mut self, new_state: IceGatheringState) {
        self.0.update(format!("gathering {new_state:?}"), |s| {
            s.gathering = new_state
        });
    }

    fn on_track(&mut self, transceiver: RtpTransceiver) {
        let track = transceiver.receiver().track();
        let kind = track.kind().unwrap_or_default();
        self.0.update(format!("track {kind}"), |s| {
            s.tracks.push(RemoteTrack {
                kind,
                track,
                transceiver,
            });
        });
    }

    fn on_remove_track(&mut self, _receiver: RtpReceiver) {
        self.0
            .update("remove_track".into(), |s| s.removed_tracks += 1);
    }

    fn on_ice_candidate(&mut self, candidate: IceCandidateRef<'_>) {
        let found = Candidate {
            mid: candidate.sdp_mid().unwrap_or_default(),
            index: candidate.sdp_mline_index(),
            sdp: candidate.to_string().unwrap_or_default(),
        };
        self.0.update(format!("candidate {}", found.sdp), |s| {
            s.candidates.push(found)
        });
    }

    fn on_data_channel(&mut self, data_channel: DataChannel) {
        let label = data_channel.label().unwrap_or_default();
        self.0.update(format!("data_channel {label}"), |s| {
            s.data_channels.push((label, data_channel))
        });
    }
}

/// How long the blocking helpers wait for libwebrtc's answer to a call.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

type Reply = std::result::Result<String, String>;

struct DescriptionHandler(mpsc::Sender<Reply>);

impl CreateSessionDescriptionObserverHandler for DescriptionHandler {
    fn on_success(&mut self, desc: SessionDescription) {
        let _ = self.0.send(desc.to_string().map_err(|e| e.to_string()));
    }

    fn on_failure(&mut self, error: RtcError) {
        let _ = self
            .0
            .send(Err(error.message().unwrap_or_else(|_| "RTCError".into())));
    }
}

struct SetHandler(mpsc::Sender<Reply>);

fn completion(error: RtcError) -> Reply {
    if error.ok() {
        Ok(String::new())
    } else {
        Err(error.message().unwrap_or_else(|_| "RTCError".into()))
    }
}

impl SetLocalDescriptionObserverHandler for SetHandler {
    fn on_set_local_description_complete(&mut self, error: RtcError) {
        let _ = self.0.send(completion(error));
    }
}

impl SetRemoteDescriptionObserverHandler for SetHandler {
    fn on_set_remote_description_complete(&mut self, error: RtcError) {
        let _ = self.0.send(completion(error));
    }
}

fn await_reply(what: &str, replies: &mpsc::Receiver<Reply>) -> Result<String> {
    match replies.recv_timeout(CALL_TIMEOUT) {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(message)) => Err(error(format!("{what} failed: {message}"))),
        Err(_) => Err(error(format!(
            "{what} did not complete in {CALL_TIMEOUT:?}"
        ))),
    }
}

/// One layer of a simulcast send.
#[derive(Debug, Clone)]
pub struct SimulcastLayer {
    /// The `rid`.
    pub rid: String,
    /// `scaleResolutionDownBy`.
    pub scale_down_by: f64,
    /// `scalabilityMode`, when set.
    pub scalability_mode: Option<String>,
}

/// A peer connection.
pub struct Peer {
    name: String,
    pc: PeerConnection,
    shared: Arc<Shared>,
    closed: AtomicBool,
    // libwebrtc keeps a pointer to the observer until `close`; it is dropped after `pc`.
    _observer: PeerConnectionObserver,
}

impl Peer {
    /// Creates a peer connection on `engine`'s factory, without ICE servers.
    pub fn new(engine: &Engine, name: &str, config: PeerConfig) -> Result<Peer> {
        let shared = Arc::new(Shared {
            state: Mutex::new(PeerState {
                signaling: SignalingState::Stable,
                connection: PeerConnectionState::New,
                ice_connection: IceConnectionState::New,
                gathering: IceGatheringState::New,
                candidates: Vec::new(),
                tracks: Vec::new(),
                removed_tracks: 0,
                data_channels: Vec::new(),
                events: Vec::new(),
            }),
            changed: Condvar::new(),
            remote_description_set: AtomicBool::new(false),
        });
        let observer =
            PeerConnectionObserver::new_with_handler(Box::new(Observer(Arc::clone(&shared))));
        let mut rtc_config = PeerConnectionRtcConfiguration::new();
        rtc_config.set_type(config.ice_transports.raw());
        let deps = PeerConnectionDependencies::new(&observer);
        let pc = PeerConnection::create(engine.factory(), &rtc_config, deps)?;
        Ok(Peer {
            name: name.to_owned(),
            pc,
            shared,
            closed: AtomicBool::new(false),
            _observer: observer,
        })
    }

    /// The name given at creation (for messages).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The underlying peer connection.
    pub fn pc(&self) -> &PeerConnection {
        &self.pc
    }

    /// Creates an offer and returns its SDP.
    pub fn create_offer(&self) -> Result<String> {
        let (tx, rx) = mpsc::channel();
        let mut observer =
            CreateSessionDescriptionObserver::new_with_handler(Box::new(DescriptionHandler(tx)));
        self.pc
            .create_offer(&mut observer, &PeerConnectionOfferAnswerOptions::new());
        await_reply("create_offer", &rx)
    }

    /// Creates an ICE-restart offer (the wrapper has no `restart_ice`; the offer option does it).
    pub fn create_restart_offer(&self) -> Result<String> {
        let (tx, rx) = mpsc::channel();
        let mut observer =
            CreateSessionDescriptionObserver::new_with_handler(Box::new(DescriptionHandler(tx)));
        let mut options = PeerConnectionOfferAnswerOptions::new();
        options.set_ice_restart(true);
        self.pc.create_offer(&mut observer, &options);
        await_reply("create_offer(ice_restart)", &rx)
    }

    /// Creates an answer to the remote offer and returns its SDP.
    pub fn create_answer(&self) -> Result<String> {
        let (tx, rx) = mpsc::channel();
        let mut observer =
            CreateSessionDescriptionObserver::new_with_handler(Box::new(DescriptionHandler(tx)));
        self.pc
            .create_answer(&mut observer, &PeerConnectionOfferAnswerOptions::new());
        await_reply("create_answer", &rx)
    }

    /// Sets the local description.
    pub fn set_local(&self, kind: SdpType, sdp: &str) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let observer = SetLocalDescriptionObserver::new_with_handler(Box::new(SetHandler(tx)));
        self.pc
            .set_local_description(SessionDescription::new(kind, sdp)?, &observer);
        await_reply("set_local_description", &rx).map(|_| ())
    }

    /// Sets the remote description (candidates may be added from now on).
    pub fn set_remote(&self, kind: SdpType, sdp: &str) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let observer = SetRemoteDescriptionObserver::new_with_handler(Box::new(SetHandler(tx)));
        self.pc
            .set_remote_description(SessionDescription::new(kind, sdp)?, &observer);
        await_reply("set_remote_description", &rx)?;
        self.shared
            .remote_description_set
            .store(true, Ordering::Release);
        Ok(())
    }

    /// Adds a remote ICE candidate.
    pub fn add_remote_candidate(&self, candidate: &Candidate) -> Result<()> {
        let candidate = IceCandidate::new(&candidate.mid, candidate.index, &candidate.sdp)?;
        self.pc.add_ice_candidate(&candidate)
    }

    /// Creates a data channel and attaches an inbox to it.
    pub fn create_channel(&self, label: &str, options: &ChannelOptions) -> Result<Channel> {
        Ok(Channel::attach(
            self.pc.create_data_channel(label, &options.to_init())?,
        ))
    }

    /// Adds `track` with `sender`'s stream id; returns the sender (the only way to reach one: the
    /// wrapper's `RtpTransceiver` has no `sender()`).
    pub fn add_track(&self, track: &MediaStreamTrack, stream_id: &str) -> Result<RtpSender> {
        let mut ids = StringVector::new(0);
        ids.push(&CxxString::from_str(stream_id));
        self.pc.add_track(track, &ids)
    }

    /// Adds a send-only video transceiver with one encoding per simulcast layer.
    pub fn add_simulcast_video(
        &self,
        track: &VideoTrack,
        stream_id: &str,
        layers: &[SimulcastLayer],
    ) -> Result<RtpTransceiver> {
        let mut encodings = RtpEncodingParametersVector::new(0);
        for layer in layers {
            let mut encoding = RtpEncodingParameters::new();
            encoding.set_rid(&layer.rid);
            encoding.set_scale_resolution_down_by(Some(layer.scale_down_by));
            encoding.set_scalability_mode(layer.scalability_mode.as_deref());
            encoding.set_active(true);
            encodings.push(&encoding);
        }
        let mut init = RtpTransceiverInit::new();
        init.set_direction(RtpTransceiverDirection::SendOnly);
        init.set_send_encodings(&encodings);
        init.stream_ids_mut().push(&CxxString::from_str(stream_id));
        self.pc.add_transceiver_with_track(track, &init)
    }

    /// Runs `read` on the state under the lock.
    pub fn with_state<R>(&self, read: impl FnOnce(&PeerState) -> R) -> R {
        read(&self.shared.lock())
    }

    /// Waits until `check` returns `Some` (it is called with the state locked, after every
    /// callback), or until `timeout`.
    pub fn wait_for<R>(
        &self,
        timeout: Duration,
        mut check: impl FnMut(&PeerState) -> Option<R>,
    ) -> Option<R> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.lock();
        loop {
            if let Some(found) = check(&state) {
                return Some(found);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            state = self
                .shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Waits for the peer connection to be `Connected`.
    pub fn wait_connected(&self, timeout: Duration) -> bool {
        self.wait_for(timeout, |s| {
            (s.connection == PeerConnectionState::Connected).then_some(())
        })
        .is_some()
    }

    /// Waits for ICE gathering to complete.
    pub fn wait_gathered(&self, timeout: Duration) -> bool {
        self.wait_for(timeout, |s| {
            (s.gathering == IceGatheringState::Complete).then_some(())
        })
        .is_some()
    }

    /// Waits for a remote data channel with `label`, takes it and attaches an inbox.
    pub fn accept_channel(&self, label: &str, timeout: Duration) -> Option<Channel> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.lock();
        let (_, channel) = loop {
            if let Some(position) = state.data_channels.iter().position(|(l, _)| l == label) {
                break state.data_channels.remove(position);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            state = self
                .shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        };
        // Registering the observer calls into libwebrtc: never with the state lock held.
        drop(state);
        Some(Channel::attach(channel))
    }

    /// Every remote track announced so far, as `(kind, track)`. The handles are copied under the
    /// lock; ask them for their state afterwards (a call into libwebrtc must never run with the
    /// state lock held: the signaling thread may be waiting for it in a callback).
    pub fn remote_tracks(&self) -> Vec<(String, MediaStreamTrack)> {
        self.with_state(|state| {
            state
                .tracks
                .iter()
                .map(|t| (t.kind.clone(), t.track.clone()))
                .collect()
        })
    }

    /// The first remote track of `kind` that has been announced.
    pub fn remote_track(&self, kind: &str, timeout: Duration) -> Option<MediaStreamTrack> {
        self.wait_for(timeout, |state| {
            state
                .tracks
                .iter()
                .find(|t| t.kind == kind)
                .map(|t| t.track.clone())
        })
    }

    /// `getStats()` as the JSON array libwebrtc prints (W3C member names, one object per stat).
    pub fn stats_json(&self) -> Result<String> {
        let (tx, rx) = mpsc::channel();
        self.pc.get_stats(move |report| {
            let _ = tx.send(report.to_json().map_err(|e| e.to_string()));
        });
        await_reply("get_stats", &rx)
    }

    /// Closes the peer connection (idempotent). After this no observer callback runs.
    pub fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.pc.close();
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.close();
    }
}

/// Moves local candidates of each side to the other, once the other has a remote description.
/// A real signalling channel would do the same through a server; here it is a thread in the
/// process.
pub struct CandidateRelay {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CandidateRelay {
    /// Starts relaying between `a` and `b`.
    pub fn start(a: &Peer, b: &Peer) -> CandidateRelay {
        let stop = Arc::new(AtomicBool::new(false));
        let ends = [
            (Arc::clone(&a.shared), b.pc.clone(), Arc::clone(&b.shared)),
            (Arc::clone(&b.shared), a.pc.clone(), Arc::clone(&a.shared)),
        ];
        let flag = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("candidate-relay".into())
            .spawn(move || {
                let mut sent = [0usize; 2];
                while !flag.load(Ordering::Acquire) {
                    for (index, (from, to_pc, to)) in ends.iter().enumerate() {
                        if !to.remote_description_set.load(Ordering::Acquire) {
                            continue;
                        }
                        let batch: Vec<Candidate> = from.lock().candidates[sent[index]..].to_vec();
                        for candidate in batch {
                            if let Ok(ice) =
                                IceCandidate::new(&candidate.mid, candidate.index, &candidate.sdp)
                            {
                                let _ = to_pc.add_ice_candidate(&ice);
                            }
                            sent[index] += 1;
                        }
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })
            .expect("cannot start the candidate relay thread");
        CandidateRelay {
            stop,
            thread: Some(thread),
        }
    }

    /// Stops the relay thread.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for CandidateRelay {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The result of [`negotiate`].
pub struct Negotiated {
    /// The offer SDP.
    pub offer: String,
    /// The answer SDP the answerer wrote.
    pub answer: String,
    /// The answer the offerer applied (the same unless the negotiation rewrote it).
    pub applied_answer: String,
}

/// One offer/answer exchange, `offerer` to `answerer`. Candidates are not moved: start a
/// [`CandidateRelay`] first (or use [`connect`]).
pub fn negotiate(offerer: &Peer, answerer: &Peer) -> Result<Negotiated> {
    negotiate_with_answer(offerer, answerer, None)
}

/// Like [`negotiate`], but the offerer applies `rewrite(answer)` instead of the answer libwebrtc
/// wrote. libwebrtc as an answerer does not accept simulcast, so t06 rewrites its answer the way a
/// server's would look, as webrtc's own `PeerConnectionSimulcastTests` do.
pub fn negotiate_with_answer(
    offerer: &Peer,
    answerer: &Peer,
    rewrite: Option<&dyn Fn(&str) -> String>,
) -> Result<Negotiated> {
    let offer = offerer.create_offer()?;
    offerer.set_local(SdpType::Offer, &offer)?;
    answerer.set_remote(SdpType::Offer, &offer)?;
    let answer = answerer.create_answer()?;
    answerer.set_local(SdpType::Answer, &answer)?;
    let applied_answer = rewrite.map_or_else(|| answer.clone(), |rewrite| rewrite(&answer));
    offerer.set_remote(SdpType::Answer, &applied_answer)?;
    Ok(Negotiated {
        offer,
        answer,
        applied_answer,
    })
}

/// Two connected peers: the relay stays alive as long as this does.
pub struct Link {
    /// What was exchanged.
    pub negotiated: Negotiated,
    /// Time from the start of the offer to both sides being `Connected`.
    pub connect_time: Duration,
    _relay: CandidateRelay,
}

/// Negotiates `offerer` to `answerer`, relays candidates and waits until both are `Connected`.
pub fn connect(offerer: &Peer, answerer: &Peer, timeout: Duration) -> Result<Link> {
    connect_with_answer(offerer, answerer, timeout, None)
}

/// [`connect`] with an answer rewrite, see [`negotiate_with_answer`].
pub fn connect_with_answer(
    offerer: &Peer,
    answerer: &Peer,
    timeout: Duration,
    rewrite: Option<&dyn Fn(&str) -> String>,
) -> Result<Link> {
    let started = Instant::now();
    let relay = CandidateRelay::start(offerer, answerer);
    let negotiated = negotiate_with_answer(offerer, answerer, rewrite)?;
    for peer in [offerer, answerer] {
        if !peer.wait_connected(timeout.saturating_sub(started.elapsed())) {
            let events = peer.with_state(|s| s.events.join(" | "));
            return Err(error(format!(
                "{} did not connect in {timeout:?}; events: {events}",
                peer.name()
            )));
        }
    }
    Ok(Link {
        negotiated,
        connect_time: started.elapsed(),
        _relay: relay,
    })
}
