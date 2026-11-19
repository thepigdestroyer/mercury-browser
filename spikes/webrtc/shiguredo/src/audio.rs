// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The virtual audio device, and a meter for remote audio.
//!
//! `shiguredo_webrtc` has no "native audio source" that takes frames: libwebrtc's audio enters a
//! call through an `AudioDeviceModule` (the recording callback `AudioTransport::
//! RecordedDataIsAvailable`) and leaves it through the same module (`NeedMorePlayData` pulls the
//! mixed remote audio). The wrapper lets Rust implement the module (`new_with_handler`), so this
//! is a device without hardware: [`VirtualAudio::push_block`] plays the part of the microphone,
//! and a thread started by `StartPlayout` plays the part of the speaker, pulling and discarding
//! 10 ms blocks every 10 ms (without that pull, remote audio is never decoded and no sink on a
//! remote track is ever called).
//!
//! This is the only module with `unsafe`: the raw `AudioTransport` pointer.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mercury_frame_sink::{
    AUDIO_BLOCK_MS, AUDIO_SAMPLE_RATE, AUDIO_SAMPLES_PER_BLOCK, MediaFrameSink, Micros,
    RawAudioFrame,
};
use shiguredo_webrtc::{
    AudioDeviceModuleHandler, AudioParameters, AudioTrack, AudioTrackSink, AudioTrackSinkHandler,
    AudioTransportPtr,
};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// State shared by the device (on libwebrtc's network thread), the producer thread that pushes
/// blocks, the playout thread and whoever reads the counters.
#[derive(Default)]
struct Shared {
    /// Set between `RegisterAudioCallback(Some)` and `RegisterAudioCallback(None)`. The mutex is
    /// held for the whole of every call through the pointer, so the pointer is never used after
    /// libwebrtc has withdrawn it.
    transport: Mutex<Option<AudioTransportPtr>>,
    recording: AtomicBool,
    playing: AtomicBool,
    pushed_blocks: AtomicU64,
    dropped_blocks: AtomicU64,
    pulled_blocks: AtomicU64,
    /// The names of the device methods libwebrtc called, in order (consecutive repeats once).
    calls: Mutex<Vec<&'static str>>,
}

impl Shared {
    fn note(&self, call: &'static str) {
        let mut calls = lock(&self.calls);
        if calls.last() != Some(&call) {
            calls.push(call);
        }
    }
}

/// A handle to the virtual device; clone it freely.
#[derive(Clone)]
pub struct VirtualAudio {
    shared: Arc<Shared>,
}

impl VirtualAudio {
    /// The handler for `AudioDeviceModule::new_with_handler` and the handle to talk to it.
    pub fn new() -> (VirtualDevice, VirtualAudio) {
        let shared = Arc::new(Shared::default());
        let device = VirtualDevice {
            shared: Arc::clone(&shared),
            pump: None,
        };
        (device, VirtualAudio { shared })
    }

    /// True while libwebrtc has started recording (a sending audio track exists).
    pub fn recording(&self) -> bool {
        self.shared.recording.load(Ordering::Acquire)
    }

    /// True while libwebrtc has started playout (a receiving audio track exists).
    pub fn playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    /// Blocks given to libwebrtc, blocks refused (not recording yet, or no transport), blocks
    /// pulled by the playout thread.
    pub fn counters(&self) -> (u64, u64, u64) {
        (
            self.shared.pushed_blocks.load(Ordering::Relaxed),
            self.shared.dropped_blocks.load(Ordering::Relaxed),
            self.shared.pulled_blocks.load(Ordering::Relaxed),
        )
    }

    /// The device methods libwebrtc called (the evidence that no real device was involved: this
    /// list is everything that ever "opened" a device).
    pub fn calls(&self) -> Vec<&'static str> {
        lock(&self.shared.calls).clone()
    }

    /// Hands one 10 ms block of interleaved S16 samples (1 or 2 channels, 48 kHz) to libwebrtc as
    /// microphone data. Returns false when the block was refused (nobody is recording).
    ///
    /// `capture_time` is on libwebrtc's clock (microseconds, see `rtc::time_millis`), if known.
    pub fn push_block(&self, samples: &[i16], channels: u16, capture_time: Option<Micros>) -> bool {
        let shared = &self.shared;
        let frames = samples.len() / usize::from(channels.max(1));
        if channels == 0
            || frames != AUDIO_SAMPLES_PER_BLOCK
            || !shared.recording.load(Ordering::Acquire)
        {
            shared.dropped_blocks.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let transport = lock(&shared.transport);
        let Some(transport) = transport.as_ref() else {
            shared.dropped_blocks.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        let mut new_mic_level = 0u32;
        let bytes_per_frame = 2 * usize::from(channels);
        // SAFETY: `samples` holds `frames * channels` S16 values, `bytes_per_frame` is the size of
        // one frame (the convention of `AudioTransport`), and the guard keeps libwebrtc from
        // withdrawing the transport while this call runs.
        let status = unsafe {
            transport.recorded_data_is_available(
                samples.as_ptr().cast::<u8>(),
                frames,
                bytes_per_frame,
                usize::from(channels),
                AUDIO_SAMPLE_RATE,
                0,
                0,
                0,
                false,
                &mut new_mic_level,
                capture_time.map(|time| time.0.saturating_mul(1000)),
            )
        };
        shared.pushed_blocks.fetch_add(1, Ordering::Relaxed);
        status == 0
    }
}

/// The `AudioDeviceModuleHandler`; libwebrtc owns it and calls it on its network thread.
pub struct VirtualDevice {
    shared: Arc<Shared>,
    pump: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
}

impl VirtualDevice {
    fn stop_pump(&mut self) {
        if let Some((stop, thread)) = self.pump.take() {
            stop.store(true, Ordering::Release);
            let _ = thread.join();
        }
    }
}

/// The speaker: every 10 ms (absolute deadlines, so no drift) pull one block of mixed remote audio
/// and throw it away.
fn spawn_playout_pump(shared: Arc<Shared>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("virtual-audio-playout".into())
        .spawn(move || {
            let mut block = vec![0i16; AUDIO_SAMPLES_PER_BLOCK];
            let started = Instant::now();
            let mut ticks = 0u64;
            while !stop.load(Ordering::Acquire) {
                ticks += 1;
                let due = started + Duration::from_millis(u64::from(AUDIO_BLOCK_MS) * ticks);
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    thread::sleep(wait);
                }
                let transport = lock(&shared.transport);
                let Some(transport) = transport.as_ref() else {
                    continue;
                };
                let (mut samples_out, mut elapsed_ms, mut ntp_ms) = (0usize, 0i64, 0i64);
                // SAFETY: `block` has room for one mono 10 ms block (480 S16 samples = 2 bytes per
                // frame, 1 channel) and the guard keeps the transport valid during the call.
                let status = unsafe {
                    transport.need_more_play_data(
                        AUDIO_SAMPLES_PER_BLOCK,
                        2,
                        1,
                        AUDIO_SAMPLE_RATE,
                        block.as_mut_ptr().cast::<u8>(),
                        &mut samples_out,
                        &mut elapsed_ms,
                        &mut ntp_ms,
                    )
                };
                if status == 0 {
                    shared.pulled_blocks.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
        .expect("cannot start the playout thread")
}

impl AudioDeviceModuleHandler for VirtualDevice {
    fn register_audio_callback(&mut self, audio_transport: Option<AudioTransportPtr>) -> i32 {
        self.shared.note(if audio_transport.is_some() {
            "register_audio_callback"
        } else {
            "unregister_audio_callback"
        });
        *lock(&self.shared.transport) = audio_transport;
        0
    }

    fn init(&mut self) -> i32 {
        self.shared.note("init");
        0
    }

    fn terminate(&mut self) -> i32 {
        self.shared.note("terminate");
        self.stop_pump();
        0
    }

    fn initialized(&self) -> bool {
        true
    }

    fn playout_is_available(&mut self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn recording_is_available(&mut self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn init_playout(&mut self) -> i32 {
        self.shared.note("init_playout");
        0
    }

    fn init_recording(&mut self) -> i32 {
        self.shared.note("init_recording");
        0
    }

    fn start_playout(&mut self) -> i32 {
        self.shared.note("start_playout");
        if self.pump.is_none() {
            let stop = Arc::new(AtomicBool::new(false));
            let thread = spawn_playout_pump(Arc::clone(&self.shared), Arc::clone(&stop));
            self.pump = Some((stop, thread));
        }
        self.shared.playing.store(true, Ordering::Release);
        0
    }

    fn stop_playout(&mut self) -> i32 {
        self.shared.note("stop_playout");
        self.shared.playing.store(false, Ordering::Release);
        self.stop_pump();
        0
    }

    fn playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    fn start_recording(&mut self) -> i32 {
        self.shared.note("start_recording");
        self.shared.recording.store(true, Ordering::Release);
        0
    }

    fn stop_recording(&mut self) -> i32 {
        self.shared.note("stop_recording");
        self.shared.recording.store(false, Ordering::Release);
        0
    }

    fn recording(&self) -> bool {
        self.shared.recording.load(Ordering::Acquire)
    }

    fn get_playout_audio_parameters(&self, params: &mut Option<AudioParameters>) -> i32 {
        *params = Some(AudioParameters::new(
            AUDIO_SAMPLE_RATE as i32,
            1,
            AUDIO_SAMPLES_PER_BLOCK,
        ));
        0
    }

    fn get_record_audio_parameters(&self, params: &mut Option<AudioParameters>) -> i32 {
        *params = Some(AudioParameters::new(
            AUDIO_SAMPLE_RATE as i32,
            1,
            AUDIO_SAMPLES_PER_BLOCK,
        ));
        0
    }
}

impl Drop for VirtualDevice {
    fn drop(&mut self) {
        self.stop_pump();
    }
}

/// What a remote audio track delivered.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AudioMeter {
    /// Callbacks (libwebrtc delivers 10 ms blocks).
    pub blocks: u64,
    /// Samples per channel summed over all blocks.
    pub frames: u64,
    /// Root mean square of all samples (S16 scale), channels mixed.
    pub rms: f64,
    /// Largest absolute sample.
    pub peak: i32,
    /// Sign changes per second of audio (twice the tone's frequency, for a sine).
    pub zero_crossings_per_second: f64,
    /// Rate and channels of the last block.
    pub sample_rate: i32,
    /// Channels of the last block.
    pub channels: usize,
    /// Blocks that were not 10 ms of 16-bit 48 kHz audio and were not forwarded.
    pub unexpected_format: u64,
}

#[derive(Default)]
struct MeterState {
    meter: AudioMeter,
    sum_squares: f64,
    samples: u64,
    crossings: u64,
    last_sign: i8,
}

struct MeterHandler {
    state: Arc<Mutex<MeterState>>,
    forward: Option<Arc<dyn MediaFrameSink>>,
}

impl AudioTrackSinkHandler for MeterHandler {
    fn on_data(
        &mut self,
        audio_data: &[u8],
        bits_per_sample: i32,
        sample_rate: i32,
        number_of_channels: usize,
        number_of_frames: usize,
    ) {
        let samples: Vec<i16> = audio_data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect();
        let mut state = lock(&self.state);
        state.meter.blocks += 1;
        state.meter.frames += number_of_frames as u64;
        state.meter.sample_rate = sample_rate;
        state.meter.channels = number_of_channels;
        let first_channel = samples.iter().step_by(number_of_channels.max(1));
        for &sample in first_channel {
            let sign = if sample >= 0 { 1i8 } else { -1i8 };
            if state.last_sign != 0 && sign != state.last_sign {
                state.crossings += 1;
            }
            state.last_sign = sign;
        }
        for &sample in &samples {
            state.sum_squares += f64::from(sample) * f64::from(sample);
            state.meter.peak = state.meter.peak.max(i32::from(sample).abs());
        }
        state.samples += samples.len() as u64;
        let well_formed = bits_per_sample == 16
            && sample_rate == AUDIO_SAMPLE_RATE as i32
            && number_of_frames == AUDIO_SAMPLES_PER_BLOCK
            && (1..=2).contains(&number_of_channels);
        if !well_formed {
            state.meter.unexpected_format += 1;
        }
        drop(state);
        if let (true, Some(sink)) = (well_formed, self.forward.as_ref()) {
            sink.audio_frame(RawAudioFrame {
                samples: &samples,
                channels: number_of_channels as u16,
                sample_rate: AUDIO_SAMPLE_RATE,
                timestamp: Micros(shiguredo_webrtc::time_millis() * 1000),
            });
        }
    }
}

/// A sink on a remote audio track that measures what arrives and optionally forwards each block
/// to a [`MediaFrameSink`] (the reverse direction of the bridge: remote frames into an `appsrc`).
pub struct RemoteAudio {
    // Declared first so that it is dropped (and the handler with it) before the track goes away.
    sink: AudioTrackSink,
    track: AudioTrack,
    state: Arc<Mutex<MeterState>>,
}

impl RemoteAudio {
    /// Attaches the meter to `track`.
    pub fn attach(track: &AudioTrack, forward: Option<Arc<dyn MediaFrameSink>>) -> RemoteAudio {
        let state = Arc::new(Mutex::new(MeterState::default()));
        let sink = AudioTrackSink::new_with_handler(Box::new(MeterHandler {
            state: Arc::clone(&state),
            forward,
        }));
        track.add_sink(&sink);
        RemoteAudio {
            sink,
            track: track.clone(),
            state,
        }
    }

    /// The numbers so far.
    pub fn meter(&self) -> AudioMeter {
        let state = lock(&self.state);
        let mut meter = state.meter;
        if state.samples > 0 {
            meter.rms = (state.sum_squares / state.samples as f64).sqrt();
        }
        if meter.frames > 0 && meter.sample_rate > 0 {
            let seconds = meter.frames as f64 / f64::from(meter.sample_rate);
            meter.zero_crossings_per_second = state.crossings as f64 / seconds;
        }
        meter
    }
}

impl Drop for RemoteAudio {
    fn drop(&mut self) {
        self.track.remove_sink(&self.sink);
    }
}
