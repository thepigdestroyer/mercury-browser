// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Two peer connections of one factory, connected to each other in this process.
//!
//! The "signalling" is a function call: the offer goes from one peer to the other, the answer
//! comes back, and each peer's trickled ICE candidates are forwarded to the other by a task. No
//! ICE server is configured, so the only candidates are host candidates and the connection runs
//! over the machine's own interface addresses.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libwebrtc::ice_candidate::IceCandidate;
use libwebrtc::peer_connection::{
    AnswerOptions, IceGatheringState, OfferOptions, PeerConnection, PeerConnectionState,
};
use libwebrtc::peer_connection_factory::{PeerConnectionFactory, RtcConfiguration};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

/// The text of a candidate and where it belongs, copied out of libwebrtc's callback thread.
type CandidateParts = (String, i32, String);

/// One side of a [`Pair`].
pub struct Peer {
    /// Name used in messages.
    pub name: &'static str,
    /// The peer connection.
    pub pc: PeerConnection,
    /// Every local candidate line (`candidate:...`) the peer has reported, in order.
    pub local_candidates: Arc<Mutex<Vec<String>>>,
    state: watch::Receiver<PeerConnectionState>,
    gathering: watch::Receiver<IceGatheringState>,
    candidates: Mutex<Option<mpsc::UnboundedReceiver<CandidateParts>>>,
}

impl Peer {
    /// Creates a peer connection with `config` and hooks its state and candidate callbacks.
    pub fn new(
        factory: &PeerConnectionFactory,
        name: &'static str,
        config: RtcConfiguration,
    ) -> Result<Peer, String> {
        let pc = factory
            .create_peer_connection(config)
            .map_err(|e| format!("{name}: create_peer_connection: {e:?}"))?;
        let (state_tx, state) = watch::channel(pc.connection_state());
        let (gather_tx, gathering) = watch::channel(pc.ice_gathering_state());
        let (cand_tx, cand_rx) = mpsc::unbounded_channel::<CandidateParts>();
        let local_candidates = Arc::new(Mutex::new(Vec::new()));
        {
            let seen = local_candidates.clone();
            pc.on_ice_candidate(Some(Box::new(move |candidate| {
                let line = candidate.candidate();
                if let Ok(mut all) = seen.lock() {
                    all.push(line.clone());
                }
                let _ = cand_tx.send((candidate.sdp_mid(), candidate.sdp_mline_index(), line));
            })));
        }
        pc.on_connection_state_change(Some(Box::new(move |s| {
            let _ = state_tx.send(s);
        })));
        pc.on_ice_gathering_state_change(Some(Box::new(move |s| {
            let _ = gather_tx.send(s);
        })));
        Ok(Peer {
            name,
            pc,
            local_candidates,
            state,
            gathering,
            candidates: Mutex::new(Some(cand_rx)),
        })
    }

    /// Waits until the connection state is `want`, or fails after `limit`.
    pub async fn wait_state(
        &self,
        want: PeerConnectionState,
        limit: Duration,
    ) -> Result<(), String> {
        let mut rx = self.state.clone();
        let result = timeout(limit, async {
            loop {
                if *rx.borrow_and_update() == want {
                    return Ok(());
                }
                if rx.changed().await.is_err() {
                    return Err("state channel closed".to_owned());
                }
            }
        })
        .await;
        match result {
            Ok(inner) => inner,
            Err(_) => Err(format!(
                "{}: connection state is {:?}, not {want:?} after {limit:?}",
                self.name,
                *self.state.borrow()
            )),
        }
    }

    /// Waits until ICE gathering is complete, or fails after `limit`.
    pub async fn wait_gathered(&self, limit: Duration) -> Result<(), String> {
        let mut rx = self.gathering.clone();
        let result = timeout(limit, async {
            loop {
                if *rx.borrow_and_update() == IceGatheringState::Complete {
                    return Ok(());
                }
                if rx.changed().await.is_err() {
                    return Err("gathering channel closed".to_owned());
                }
            }
        })
        .await;
        match result {
            Ok(inner) => inner,
            Err(_) => Err(format!(
                "{}: ICE gathering is {:?} after {limit:?}",
                self.name,
                *self.gathering.borrow()
            )),
        }
    }

    /// The local candidate lines reported so far.
    pub fn candidates(&self) -> Vec<String> {
        self.local_candidates
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    fn take_candidates(&self) -> Option<mpsc::UnboundedReceiver<CandidateParts>> {
        self.candidates.lock().ok().and_then(|mut rx| rx.take())
    }
}

/// Two connected peers. Dropping the pair stops the forwarding tasks and closes both connections.
pub struct Pair {
    /// The side that makes the offer.
    pub offerer: Arc<Peer>,
    /// The side that answers.
    pub answerer: Arc<Peer>,
    forwarders: Mutex<Vec<JoinHandle<()>>>,
}

/// The two session descriptions of a negotiation, as text.
#[derive(Debug, Clone)]
pub struct Sdps {
    /// The offer.
    pub offer: String,
    /// The answer.
    pub answer: String,
}

impl Pair {
    /// Creates both peers with the same configuration.
    pub fn new(factory: &PeerConnectionFactory, config: &RtcConfiguration) -> Result<Pair, String> {
        Ok(Pair {
            offerer: Arc::new(Peer::new(factory, "offerer", config.clone())?),
            answerer: Arc::new(Peer::new(factory, "answerer", config.clone())?),
            forwarders: Mutex::new(Vec::new()),
        })
    }

    fn forward(&self, from: &Arc<Peer>, to: &Arc<Peer>) {
        let Some(mut rx) = from.take_candidates() else {
            return;
        };
        let to = to.clone();
        let handle = tokio::spawn(async move {
            while let Some((mid, index, line)) = rx.recv().await {
                // The other side may not have its remote description yet (the candidate can be
                // gathered before the offer reaches it): try again for up to two seconds.
                for _ in 0..100 {
                    let Ok(candidate) = IceCandidate::parse(&mid, index, &line) else {
                        break;
                    };
                    if to.pc.add_ice_candidate(candidate).await.is_ok() {
                        break;
                    }
                    sleep(Duration::from_millis(20)).await;
                }
            }
        });
        if let Ok(mut all) = self.forwarders.lock() {
            all.push(handle);
        }
    }

    /// Runs one offer/answer exchange with trickled ICE, whatever the peers have added so far
    /// (tracks, transceivers, data channels).
    pub async fn negotiate(&self, offer_options: OfferOptions) -> Result<Sdps, String> {
        self.forward(&self.offerer, &self.answerer);
        self.forward(&self.answerer, &self.offerer);
        let (a, b) = (&self.offerer.pc, &self.answerer.pc);
        let err = |what: &str, e: libwebrtc::RtcError| format!("{what}: {e:?}");
        let offer = a
            .create_offer(offer_options)
            .await
            .map_err(|e| err("create_offer", e))?;
        let offer_text = offer.to_string();
        a.set_local_description(offer.clone())
            .await
            .map_err(|e| err("offerer set_local_description", e))?;
        b.set_remote_description(offer)
            .await
            .map_err(|e| err("answerer set_remote_description", e))?;
        let answer = b
            .create_answer(AnswerOptions::default())
            .await
            .map_err(|e| err("create_answer", e))?;
        let answer_text = answer.to_string();
        b.set_local_description(answer.clone())
            .await
            .map_err(|e| err("answerer set_local_description", e))?;
        a.set_remote_description(answer)
            .await
            .map_err(|e| err("offerer set_remote_description", e))?;
        Ok(Sdps {
            offer: offer_text,
            answer: answer_text,
        })
    }

    /// Waits until both peers are `Connected`; returns how long that took.
    pub async fn wait_connected(&self, limit: Duration) -> Result<Duration, String> {
        let started = Instant::now();
        self.offerer
            .wait_state(PeerConnectionState::Connected, limit)
            .await?;
        let left = limit.saturating_sub(started.elapsed());
        self.answerer
            .wait_state(
                PeerConnectionState::Connected,
                left.max(Duration::from_millis(100)),
            )
            .await?;
        Ok(started.elapsed())
    }

    /// Closes both connections (in the order given by `offerer_first`).
    pub fn close(&self, offerer_first: bool) {
        if offerer_first {
            self.offerer.pc.close();
            self.answerer.pc.close();
        } else {
            self.answerer.pc.close();
            self.offerer.pc.close();
        }
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        if let Ok(all) = self.forwarders.lock() {
            for handle in all.iter() {
                handle.abort();
            }
        }
        self.offerer.pc.close();
        self.answerer.pc.close();
    }
}
