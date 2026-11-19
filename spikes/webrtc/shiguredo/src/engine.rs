// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Threads, audio device, codec factories and the peer connection factory.
//!
//! `unsafe` appears twice, for the two constructors of `AudioDeviceModule`, which the binding crate
//! marks `unsafe` because the module must not outlive the thread it runs on.

#![allow(unsafe_code)]

use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer, AudioEncoderFactory,
    AudioProcessingBuilder, ConnectionContext, Environment, MediaType, PeerConnectionFactory,
    PeerConnectionFactoryDependencies, PeerConnectionFactoryDependenciesParams, RtpCapabilities,
    Thread, VideoDecoderFactory, VideoEncoderFactory,
};

use crate::audio::VirtualAudio;
use crate::{Result, error};

/// Which audio device module the factory gets. A factory without one would create the platform
/// default (CoreAudio) when media is enabled, so one of these two is always passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioMode {
    /// libwebrtc's `kDummyAudio` layer: no devices, no audio flows (calls without audio).
    #[default]
    Dummy,
    /// The device of [`crate::audio`]: no hardware, audio is pushed in and pulled out by us.
    Virtual,
}

/// Which video codec factories the factory gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoCodecs {
    /// libwebrtc's built-in software factories (libvpx, libaom, dav1d; no H.264 in this build).
    #[default]
    Builtin,
    /// `RTCDefaultVideoEncoderFactory` and decoder factory of the Apple SDK: the built-in codecs
    /// plus H.264 through VideoToolbox.
    ObjcDefault,
}

/// What [`Engine::new`] builds.
#[derive(Debug, Clone, Copy, Default)]
pub struct EngineConfig {
    /// The audio device module.
    pub audio: AudioMode,
    /// The video codec factories.
    pub video: VideoCodecs,
}

/// The factory and everything that must outlive it.
pub struct Engine {
    // Field order is the drop order: the factory goes first, the threads (stopped in `drop`) last.
    factory: Option<PeerConnectionFactory>,
    context: Option<ConnectionContext>,
    virtual_audio: Option<VirtualAudio>,
    recording_devices: i16,
    config: EngineConfig,
    signaling: Thread,
    network: Thread,
}

impl Engine {
    /// Starts the network and signaling threads and creates the factory.
    ///
    /// Devices: the audio device module is `Dummy` or the virtual one, both report zero devices
    /// (asserted here and recorded in [`Engine::recording_devices`]); no camera or screen capture
    /// API of the wrapper is called.
    pub fn new(config: EngineConfig) -> Result<Engine> {
        let mut network = Thread::new_with_socket_server();
        let mut signaling = Thread::new();
        if !network.start() || !signaling.start() {
            return Err(error("cannot start the libwebrtc threads"));
        }
        let env = Environment::new();
        let (adm, virtual_audio) = match config.audio {
            // SAFETY: `network` is started and outlives the module (the factory is dropped first).
            AudioMode::Dummy => (
                unsafe {
                    AudioDeviceModule::new(&network, &env, AudioDeviceModuleAudioLayer::Dummy)
                }?,
                None,
            ),
            AudioMode::Virtual => {
                let (handler, handle) = VirtualAudio::new();
                // SAFETY: as above.
                (
                    unsafe { AudioDeviceModule::new_with_handler(&network, Box::new(handler)) },
                    Some(handle),
                )
            }
        };
        adm.init()?;
        let recording_devices = adm.recording_devices();
        // 0 for the virtual module; the `Dummy` layer's AudioDeviceDummy answers -1 ("not supported"),
        // as the first CI run showed. Both mean that no device exists, so only a positive count fails.
        if recording_devices > 0 {
            return Err(error(format!(
                "the audio device module reports {recording_devices} recording devices, expected none"
            )));
        }

        let mut deps = PeerConnectionFactoryDependencies::from_params(
            PeerConnectionFactoryDependenciesParams {
                network_thread: &network,
                signaling_thread: &signaling,
                adm: Some(adm),
            },
        );
        // Kept in locals until the factory exists, as the binding crate's own tests do.
        let audio_encoders = AudioEncoderFactory::builtin();
        let audio_decoders = AudioDecoderFactory::builtin();
        deps.set_audio_encoder_factory(&audio_encoders);
        deps.set_audio_decoder_factory(&audio_decoders);
        deps.set_audio_processing_builder(AudioProcessingBuilder::new_builtin());
        let (encoders, decoders) = match config.video {
            VideoCodecs::Builtin => (
                Some(VideoEncoderFactory::builtin()),
                Some(VideoDecoderFactory::builtin()),
            ),
            VideoCodecs::ObjcDefault => (
                VideoEncoderFactory::from_objc_default(),
                VideoDecoderFactory::from_objc_default(),
            ),
        };
        let (Some(encoders), Some(decoders)) = (encoders, decoders) else {
            return Err(error(
                "the Apple default video codec factories are not available",
            ));
        };
        deps.set_video_encoder_factory(encoders);
        deps.set_video_decoder_factory(decoders);
        deps.enable_media();
        let (factory, context) = PeerConnectionFactory::create_modular_with_context(deps)?;
        Ok(Engine {
            factory: Some(factory),
            context: Some(context),
            virtual_audio,
            recording_devices,
            config,
            signaling,
            network,
        })
    }

    /// The peer connection factory.
    pub fn factory(&self) -> &PeerConnectionFactory {
        self.factory
            .as_ref()
            .expect("factory is only taken in drop")
    }

    /// The connection context (default network manager and socket factory).
    pub fn context(&self) -> &ConnectionContext {
        self.context
            .as_ref()
            .expect("context is only taken in drop")
    }

    /// The handle to the virtual audio device, when the engine was made with [`AudioMode::Virtual`].
    pub fn virtual_audio(&self) -> Option<&VirtualAudio> {
        self.virtual_audio.as_ref()
    }

    /// Recording devices the audio device module reported after `Init` (always 0).
    pub fn recording_devices(&self) -> i16 {
        self.recording_devices
    }

    /// The configuration this engine was made with.
    pub fn config(&self) -> EngineConfig {
        self.config
    }

    /// Codec names the factory offers for sending (`VP8`, `opus`, ...), in the factory's order.
    pub fn sender_codecs(&self, media: MediaType) -> Vec<String> {
        codec_names(&self.factory().get_rtp_sender_capabilities(media))
    }

    /// Codec names the factory accepts for receiving.
    pub fn receiver_codecs(&self, media: MediaType) -> Vec<String> {
        codec_names(&self.factory().get_rtp_receiver_capabilities(media))
    }
}

fn codec_names(capabilities: &RtpCapabilities) -> Vec<String> {
    let codecs = capabilities.codecs();
    (0..codecs.len())
        .filter_map(|index| codecs.get(index))
        .filter_map(|codec| codec.name().ok())
        .collect()
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Peers must be closed and dropped before this runs (they hold the factory alive).
        drop(self.context.take());
        drop(self.factory.take());
        self.signaling.stop();
        self.network.stop();
    }
}
