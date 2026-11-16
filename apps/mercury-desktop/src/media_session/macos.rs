//! The macOS services behind the media-session lane's traits: `MPNowPlayingInfoCenter` and
//! `MPRemoteCommandCenter` (MediaPlayer framework) and an IOKit power assertion. Only created in
//! an interactive launch (see `system_services`); nothing here runs in tests or automated runs.

use std::ffi::{CStr, c_void};
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use super::{NowPlayingCenter, NowPlayingInfo, PowerAssertions, RemoteCommand};

#[link(name = "MediaPlayer", kind = "framework")]
unsafe extern "C" {
    static MPMediaItemPropertyTitle: &'static NSString;
    static MPMediaItemPropertyArtist: &'static NSString;
    static MPMediaItemPropertyAlbumTitle: &'static NSString;
    static MPMediaItemPropertyPlaybackDuration: &'static NSString;
    static MPNowPlayingInfoPropertyElapsedPlaybackTime: &'static NSString;
    static MPNowPlayingInfoPropertyPlaybackRate: &'static NSString;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: *const c_void,
        level: u32,
        name: *const c_void,
        assertion_id: *mut u32,
    ) -> i32;
    fn IOPMAssertionRelease(assertion_id: u32) -> i32;
}

/// `kIOPMAssertionTypePreventUserIdleDisplaySleep`, a macro in `IOPMLib.h`.
const PREVENT_USER_IDLE_DISPLAY_SLEEP: &str = "PreventUserIdleDisplaySleep";
const ASSERTION_LEVEL_ON: u32 = 255;
const IO_RETURN_SUCCESS: i32 = 0;

/// `MPNowPlayingPlaybackState`.
const PLAYBACK_STATE_PLAYING: usize = 1;
const PLAYBACK_STATE_PAUSED: usize = 2;
const PLAYBACK_STATE_STOPPED: usize = 3;
/// `MPRemoteCommandHandlerStatus`.
const HANDLER_STATUS_SUCCESS: isize = 0;

type CommandSink = std::sync::Arc<dyn Fn(RemoteCommand) + Send + Sync>;

pub(super) struct SystemNowPlaying {
    sink: CommandSink,
    /// The registered targets: (command object, target token), removed on `clear`.
    targets: Vec<(Retained<AnyObject>, Retained<AnyObject>)>,
}

impl SystemNowPlaying {
    pub(super) fn new(sink: impl Fn(RemoteCommand) + Send + Sync + 'static) -> Self {
        Self {
            sink: std::sync::Arc::new(sink),
            targets: Vec::new(),
        }
    }

    fn class(name: &CStr) -> Option<&'static AnyClass> {
        let class = AnyClass::get(name);
        if class.is_none() {
            log::warn!("{name:?} is not available; Now Playing is off");
        }
        class
    }

    fn info_center() -> Option<Retained<AnyObject>> {
        let class = Self::class(c"MPNowPlayingInfoCenter")?;
        // SAFETY: `+defaultCenter` takes no arguments and returns the shared centre.
        unsafe { msg_send![class, defaultCenter] }
    }

    fn register_commands(&mut self) {
        if !self.targets.is_empty() {
            return;
        }
        let Some(class) = Self::class(c"MPRemoteCommandCenter") else {
            return;
        };
        // SAFETY: `+sharedCommandCenter` takes no arguments and returns the shared centre.
        let Some(center): Option<Retained<AnyObject>> =
            (unsafe { msg_send![class, sharedCommandCenter] })
        else {
            return;
        };
        let commands = [
            (RemoteCommand::Play, c"playCommand"),
            (RemoteCommand::Pause, c"pauseCommand"),
            (RemoteCommand::TogglePlayPause, c"togglePlayPauseCommand"),
            (RemoteCommand::NextTrack, c"nextTrackCommand"),
            (RemoteCommand::PreviousTrack, c"previousTrackCommand"),
        ];
        for (command, getter) in commands {
            let selector = objc2::runtime::Sel::register(getter);
            // SAFETY: each getter is a property of `MPRemoteCommandCenter` returning an
            // `MPRemoteCommand`.
            let Some(remote_command): Option<Retained<AnyObject>> =
                (unsafe { msg_send![&*center, performSelector: selector] })
            else {
                continue;
            };
            let sink = std::sync::Arc::clone(&self.sink);
            // The handler only hands the command on; it captures nothing tied to a thread.
            let handler = RcBlock::new(move |_event: NonNull<AnyObject>| -> isize {
                sink(command);
                HANDLER_STATUS_SUCCESS
            });
            // SAFETY: `-addTargetWithHandler:` takes a block
            // `MPRemoteCommandHandlerStatus (^)(MPRemoteCommandEvent *)` and returns the
            // target to remove later; `-setEnabled:` takes a BOOL.
            let target: Option<Retained<AnyObject>> =
                unsafe { msg_send![&*remote_command, addTargetWithHandler: &*handler] };
            unsafe {
                let () = msg_send![&*remote_command, setEnabled: Bool::YES];
            }
            if let Some(target) = target {
                self.targets.push((remote_command, target));
            }
        }
        log::info!("now playing: taking media keys");
    }

    fn unregister_commands(&mut self) {
        for (remote_command, target) in self.targets.drain(..) {
            // SAFETY: `target` was returned by `-addTargetWithHandler:` on this command.
            unsafe {
                let () = msg_send![&*remote_command, removeTarget: &*target];
                let () = msg_send![&*remote_command, setEnabled: Bool::NO];
            }
        }
    }
}

impl NowPlayingCenter for SystemNowPlaying {
    fn publish(&mut self, info: &NowPlayingInfo) {
        let Some(center) = Self::info_center() else {
            return;
        };
        // SAFETY: the keys are the MediaPlayer framework's constant strings.
        let (title, artist, album, duration, elapsed, rate) = unsafe {
            (
                MPMediaItemPropertyTitle,
                MPMediaItemPropertyArtist,
                MPMediaItemPropertyAlbumTitle,
                MPMediaItemPropertyPlaybackDuration,
                MPNowPlayingInfoPropertyElapsedPlaybackTime,
                MPNowPlayingInfoPropertyPlaybackRate,
            )
        };
        let mut keys: Vec<&NSString> = vec![title, rate];
        let mut values: Vec<Retained<AnyObject>> = vec![
            Retained::into_super(Retained::into_super(NSString::from_str(&info.title))),
            Retained::into_super(Retained::into_super(Retained::into_super(
                NSNumber::numberWithDouble(info.playback_rate),
            ))),
        ];
        for (key, text) in [(artist, &info.artist), (album, &info.album)] {
            if !text.is_empty() {
                keys.push(key);
                values.push(Retained::into_super(Retained::into_super(
                    NSString::from_str(text),
                )));
            }
        }
        for (key, seconds) in [(duration, info.duration), (elapsed, info.elapsed)] {
            if let Some(seconds) = seconds {
                keys.push(key);
                values.push(Retained::into_super(Retained::into_super(
                    Retained::into_super(NSNumber::numberWithDouble(seconds)),
                )));
            }
        }
        let dictionary = NSDictionary::<NSString, AnyObject>::from_retained_objects(&keys, &values);
        let state = if info.playing {
            PLAYBACK_STATE_PLAYING
        } else {
            PLAYBACK_STATE_PAUSED
        };
        // SAFETY: `nowPlayingInfo` is an `NSDictionary<NSString *, id>` property and
        // `playbackState` an `MPNowPlayingPlaybackState` (NSUInteger) property.
        unsafe {
            let () = msg_send![&*center, setNowPlayingInfo: &*dictionary];
            let () = msg_send![&*center, setPlaybackState: state];
        }
        self.register_commands();
    }

    fn clear(&mut self) {
        self.unregister_commands();
        let Some(center) = Self::info_center() else {
            return;
        };
        let none: Option<&NSDictionary<NSString, AnyObject>> = None;
        // SAFETY: as in `publish`; `nil` clears the information.
        unsafe {
            let () = msg_send![&*center, setNowPlayingInfo: none];
            let () = msg_send![&*center, setPlaybackState: PLAYBACK_STATE_STOPPED];
        }
        log::info!("now playing: cleared, media keys released");
    }
}

impl Drop for SystemNowPlaying {
    fn drop(&mut self) {
        self.unregister_commands();
    }
}

#[derive(Default)]
pub(super) struct SystemPowerAssertions {
    assertion: Option<u32>,
}

impl PowerAssertions for SystemPowerAssertions {
    fn prevent_display_sleep(&mut self) -> bool {
        if self.assertion.is_some() {
            return true;
        }
        let assertion_type = NSString::from_str(PREVENT_USER_IDLE_DISPLAY_SLEEP);
        let name = NSString::from_str("Mercury Browser is playing a fullscreen video");
        let mut assertion_id = 0;
        // SAFETY: both strings are `NSString`s, toll-free bridged to `CFStringRef`, alive for
        // the call; `assertion_id` is a valid out pointer.
        let result = unsafe {
            IOPMAssertionCreateWithName(
                Retained::as_ptr(&assertion_type).cast(),
                ASSERTION_LEVEL_ON,
                Retained::as_ptr(&name).cast(),
                &mut assertion_id,
            )
        };
        if result != IO_RETURN_SUCCESS {
            log::warn!("could not keep the display awake (IOReturn {result:#x})");
            return false;
        }
        self.assertion = Some(assertion_id);
        log::info!("display sleep: prevented while a fullscreen video plays");
        true
    }

    fn allow_display_sleep(&mut self) {
        if let Some(assertion_id) = self.assertion.take() {
            // SAFETY: `assertion_id` came from `IOPMAssertionCreateWithName` and is released once.
            let result = unsafe { IOPMAssertionRelease(assertion_id) };
            if result != IO_RETURN_SUCCESS {
                log::warn!("could not release the display assertion (IOReturn {result:#x})");
            }
            log::info!("display sleep: allowed again");
        }
    }
}

impl Drop for SystemPowerAssertions {
    fn drop(&mut self) {
        self.allow_display_sleep();
    }
}
