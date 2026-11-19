// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Fake camera and microphone for automated runs (`--fake-media-devices`).
//!
//! A run in this mode must not be able to reach a real device, so it is allowed only from an
//! executable whose `lib/` directory holds no GStreamer plugin that can capture from one
//! (`applemedia` has the camera source, `osxaudio` the microphone source) and does hold the
//! two test-source plugins that feed the fake devices. The browser process checks that before
//! it sets [`FAKE_MEDIA_DEVICES_VARIABLE`]; each content process checks it again before it
//! registers the fake devices.
//!
//! The fakes are a GStreamer device provider registered in the content process before Servo
//! starts its media backend. Servo's capture code then finds them as it would find real devices
//! (`servo-media-gstreamer` `media_capture.rs`): "Mercury fake camera" gives `videotestsrc`
//! frames (a moving ball) and "Mercury fake microphone" an `audiotestsrc` sine wave. Nothing in
//! this mode can play sound either: without `osxaudio` there is no audio output.

use std::path::Path;

/// Set (to `1`) by the browser process for its content processes when the fake devices may be
/// used. The browser removes any inherited value otherwise.
pub const FAKE_MEDIA_DEVICES_VARIABLE: &str = "MERCURY_FAKE_MEDIA_DEVICES";

/// Plugin files that can open a real camera or microphone; none may be present.
pub const CAPTURE_PLUGIN_FILES: [&str; 2] = ["libgstapplemedia.dylib", "libgstosxaudio.dylib"];

/// Plugin files the fake devices need; both must be present.
pub const TEST_SOURCE_PLUGIN_FILES: [&str; 2] =
    ["libgstvideotestsrc.dylib", "libgstaudiotestsrc.dylib"];

/// Why the fake devices may not be used with plugins from `lib_dir`, or `None` if they may.
pub fn fake_media_devices_problem(lib_dir: &Path) -> Option<String> {
    fake_media_devices_problem_with(lib_dir, |name| lib_dir.join(name).exists())
}

fn fake_media_devices_problem_with(
    lib_dir: &Path,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    let present = CAPTURE_PLUGIN_FILES
        .iter()
        .filter(|name| exists(name))
        .copied()
        .collect::<Vec<_>>();
    if !present.is_empty() {
        return Some(format!(
            "{} contains {}, which can open a real camera or microphone; use a copy of the \
             executable whose lib directory was collected without them \
             (scripts/macos/test-capture.sh)",
            lib_dir.display(),
            present.join(" and ")
        ));
    }
    let missing = TEST_SOURCE_PLUGIN_FILES
        .iter()
        .filter(|name| !exists(name))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Some(format!(
            "{} lacks {}, which the fake devices need (scripts/macos/test-capture.sh collects them)",
            lib_dir.display(),
            missing.join(" and ")
        ));
    }
    None
}

/// The `lib` directory beside this executable, where Servo loads GStreamer plugins from.
pub fn executable_lib_dir() -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?;
    Some(executable.parent()?.join("lib"))
}

/// In a content process: registers the fake devices if the browser asked for them and the
/// plugins next to this executable allow it. Must run before Servo starts its media backend.
/// Reports on standard error: Servo installs the logger only later in a content process.
pub(crate) fn install_if_requested() {
    if std::env::var_os(FAKE_MEDIA_DEVICES_VARIABLE).as_deref() != Some("1".as_ref()) {
        return;
    }
    let Some(lib_dir) = executable_lib_dir() else {
        eprintln!(
            "mercury-browser: fake media devices: cannot find this executable's lib directory"
        );
        return;
    };
    if let Some(problem) = fake_media_devices_problem(&lib_dir) {
        eprintln!("mercury-browser: fake media devices refused: {problem}");
        return;
    }
    match gst::install(&lib_dir) {
        Ok(()) => eprintln!(
            "mercury-browser: fake media devices registered in content process {}",
            std::process::id()
        ),
        Err(error) => eprintln!("mercury-browser: fake media devices: {error}"),
    }
}

mod gst {
    use std::path::Path;
    use std::sync::LazyLock;

    use gstreamer::glib;
    use gstreamer::prelude::*;
    use gstreamer::subclass::prelude::*;

    use super::TEST_SOURCE_PLUGIN_FILES;

    pub(super) fn install(lib_dir: &Path) -> Result<(), String> {
        gstreamer::init().map_err(|error| format!("GStreamer did not start: {error}"))?;
        let registry = gstreamer::Registry::get();
        for file in TEST_SOURCE_PLUGIN_FILES {
            let plugin = gstreamer::Plugin::load_file(lib_dir.join(file))
                .map_err(|error| format!("cannot load {file}: {error}"))?;
            registry
                .add_plugin(&plugin)
                .map_err(|error| format!("cannot register {file}: {error}"))?;
        }
        gstreamer::DeviceProvider::register(
            None,
            PROVIDER_NAME,
            gstreamer::Rank::PRIMARY,
            FakeDeviceProvider::static_type(),
        )
        .map_err(|error| format!("cannot register the device provider: {error}"))?;
        // Belt and braces: the lib directory has no capture plugin (checked), but GStreamer's
        // registry cache could still name one from elsewhere. Remove every plugin that can
        // capture and every other device provider, so that the fakes are the only devices.
        for name in CAPTURE_PLUGIN_NAMES {
            if let Some(plugin) = registry.find_plugin(name) {
                eprintln!("mercury-browser: fake media devices: removing the {name} plugin");
                registry.remove_plugin(&plugin);
            }
        }
        for factory in gstreamer::DeviceProviderFactory::factories(gstreamer::Rank::NONE) {
            if factory.name() != PROVIDER_NAME {
                eprintln!(
                    "mercury-browser: fake media devices: removing device provider {}",
                    factory.name()
                );
                registry.remove_feature(&factory);
            }
        }
        Ok(())
    }

    const PROVIDER_NAME: &str = "mercuryfakedeviceprovider";
    const CAPTURE_PLUGIN_NAMES: [&str; 2] = ["applemedia", "osxaudio"];

    #[derive(Clone, Copy, Debug)]
    pub enum Kind {
        Camera,
        Microphone,
    }

    glib::wrapper! {
        pub struct FakeDevice(ObjectSubclass<imp::FakeDevice>)
            @extends gstreamer::Device, gstreamer::Object;
    }

    impl FakeDevice {
        fn new(kind: Kind) -> Self {
            let (name, class, caps) = match kind {
                Kind::Camera => ("Mercury fake camera", "Video/Source", "video/x-raw"),
                Kind::Microphone => ("Mercury fake microphone", "Audio/Source", "audio/x-raw"),
            };
            let device: Self = glib::Object::builder()
                .property("display-name", name)
                .property("device-class", class)
                .property("caps", gstreamer::Caps::builder(caps).build())
                .build();
            device
                .imp()
                .kind
                .set(kind)
                .expect("a new device has no kind yet");
            device
        }
    }

    glib::wrapper! {
        pub struct FakeDeviceProvider(ObjectSubclass<imp::FakeDeviceProvider>)
            @extends gstreamer::DeviceProvider, gstreamer::Object;
    }

    mod imp {
        use std::sync::OnceLock;

        use super::*;

        #[derive(Default)]
        pub struct FakeDevice {
            pub(super) kind: OnceLock<Kind>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for FakeDevice {
            const NAME: &'static str = "MercuryFakeDevice";
            type Type = super::FakeDevice;
            type ParentType = gstreamer::Device;
        }

        impl ObjectImpl for FakeDevice {}
        impl GstObjectImpl for FakeDevice {}

        impl DeviceImpl for FakeDevice {
            fn create_element(
                &self,
                name: Option<&str>,
            ) -> Result<gstreamer::Element, gstreamer::LoggableError> {
                let builder = match self.kind.get() {
                    Some(Kind::Camera) => gstreamer::ElementFactory::make("videotestsrc")
                        .property_from_str("pattern", "ball"),
                    Some(Kind::Microphone) => gstreamer::ElementFactory::make("audiotestsrc")
                        .property_from_str("wave", "sine"),
                    None => {
                        return Err(gstreamer::LoggableError::new(
                            *gstreamer::CAT_RUST,
                            glib::bool_error!("fake device without a kind"),
                        ));
                    }
                };
                // The marker scripts/macos/test-capture.sh looks for: the engine opened a device.
                eprintln!(
                    "mercury-browser: fake media devices: the engine opened the fake {}",
                    match self.kind.get() {
                        Some(Kind::Camera) => "camera",
                        _ => "microphone",
                    }
                );
                let builder = builder.property("is-live", true);
                let builder = match name {
                    Some(name) => builder.name(name),
                    None => builder,
                };
                builder
                    .build()
                    .map_err(|error| gstreamer::LoggableError::new(*gstreamer::CAT_RUST, error))
            }
        }

        #[derive(Default)]
        pub struct FakeDeviceProvider;

        #[glib::object_subclass]
        impl ObjectSubclass for FakeDeviceProvider {
            const NAME: &'static str = "MercuryFakeDeviceProvider";
            type Type = super::FakeDeviceProvider;
            type ParentType = gstreamer::DeviceProvider;
        }

        impl ObjectImpl for FakeDeviceProvider {}
        impl GstObjectImpl for FakeDeviceProvider {}

        impl DeviceProviderImpl for FakeDeviceProvider {
            fn metadata() -> Option<&'static gstreamer::subclass::DeviceProviderMetadata> {
                static METADATA: LazyLock<gstreamer::subclass::DeviceProviderMetadata> =
                    LazyLock::new(|| {
                        gstreamer::subclass::DeviceProviderMetadata::new(
                            "Mercury fake devices",
                            "Source/Audio/Video",
                            "A fake camera and microphone for Mercury Browser's automated tests",
                            "Mercury Browser",
                        )
                    });
                Some(&METADATA)
            }

            fn probe(&self) -> Vec<gstreamer::Device> {
                // The marker scripts/macos/test-capture.sh looks for: the engine listed devices.
                eprintln!("mercury-browser: fake media devices: probed");
                vec![
                    super::FakeDevice::new(Kind::Camera).upcast(),
                    super::FakeDevice::new(Kind::Microphone).upcast(),
                ]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::fake_media_devices_problem_with;

    fn problem(present: &[&str]) -> Option<String> {
        fake_media_devices_problem_with(Path::new("/x/lib"), |name| present.contains(&name))
    }

    const TEST_SOURCES: [&str; 2] = ["libgstvideotestsrc.dylib", "libgstaudiotestsrc.dylib"];

    #[test]
    fn a_lib_directory_with_test_sources_and_no_capture_plugins_is_allowed() {
        assert_eq!(problem(&TEST_SOURCES), None);
    }

    #[test]
    fn the_camera_plugin_is_refused() {
        let message = problem(&[TEST_SOURCES[0], TEST_SOURCES[1], "libgstapplemedia.dylib"])
            .expect("refused");
        assert!(message.contains("libgstapplemedia.dylib"), "{message}");
        assert!(message.contains("real camera or microphone"), "{message}");
    }

    #[test]
    fn the_microphone_plugin_is_refused() {
        let message =
            problem(&[TEST_SOURCES[0], TEST_SOURCES[1], "libgstosxaudio.dylib"]).expect("refused");
        assert!(message.contains("libgstosxaudio.dylib"), "{message}");
    }

    #[test]
    fn a_normal_build_directory_is_refused() {
        // `make build-release` collects both capture plugins and no test sources.
        let message =
            problem(&["libgstapplemedia.dylib", "libgstosxaudio.dylib"]).expect("refused");
        assert!(
            message.contains("libgstapplemedia.dylib and libgstosxaudio.dylib"),
            "{message}"
        );
    }

    #[test]
    fn missing_test_sources_are_refused() {
        let message = problem(&[TEST_SOURCES[0]]).expect("refused");
        assert!(
            message.contains("lacks libgstaudiotestsrc.dylib"),
            "{message}"
        );
        assert!(
            problem(&[])
                .expect("refused")
                .contains("libgstvideotestsrc.dylib and libgstaudiotestsrc.dylib")
        );
    }
}
