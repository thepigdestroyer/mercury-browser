// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The smallest program that uses the route: one factory, one peer connection, a data channel and
//! an offer. Its stripped size minus `probe_baseline`'s is the size the route adds (G7). It opens
//! no device and connects to nothing.

use libwebrtc::data_channel::DataChannelInit;
use libwebrtc::peer_connection::OfferOptions;
use libwebrtc::peer_connection_factory::{PeerConnectionFactory, RtcConfiguration};

fn main() {
    let factory = PeerConnectionFactory::default();
    let pc = factory
        .create_peer_connection(RtcConfiguration::default())
        .expect("peer connection");
    let _dc = pc
        .create_data_channel("probe", DataChannelInit::default())
        .expect("data channel");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let offer = runtime
        .block_on(pc.create_offer(OfferOptions::default()))
        .expect("offer");
    println!(
        "offer of {} bytes, state {:?}",
        offer.to_string().len(),
        pc.connection_state()
    );
    pc.close();
}
