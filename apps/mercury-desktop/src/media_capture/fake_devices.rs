// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The `--fake-media-devices` diagnostic (media-capture lane).

use talaria_servo::{FAKE_MEDIA_DEVICES_VARIABLE, executable_lib_dir, fake_media_devices_problem};

/// Whether this run's content processes get the fake camera and microphone, or why they may
/// not: `requested` is `--fake-media-devices`, `problem` what `fake_media_devices_problem` said
/// about the plugins beside the executable.
fn fake_media_devices_decision(requested: bool, problem: Option<String>) -> Result<bool, String> {
    match (requested, problem) {
        (false, _) => Ok(false),
        (true, None) => Ok(true),
        (true, Some(problem)) => Err(format!("--fake-media-devices refused: {problem}")),
    }
}

/// Sets or clears `MERCURY_FAKE_MEDIA_DEVICES` for the content processes. An inherited value is
/// always removed, so only a run that passed the check can turn the fakes on.
///
/// Must be called before any thread exists and before Servo starts.
pub(crate) fn select_fake_media_devices(requested: bool) -> Result<(), String> {
    let problem = if requested {
        match executable_lib_dir() {
            Some(lib_dir) => fake_media_devices_problem(&lib_dir),
            None => Some("cannot find the lib directory beside this executable".to_owned()),
        }
    } else {
        None
    };
    let enabled = fake_media_devices_decision(requested, problem)?;
    // SAFETY: called in `main` before any thread starts (see the call site); nothing reads the
    // environment concurrently.
    unsafe {
        if enabled {
            std::env::set_var(FAKE_MEDIA_DEVICES_VARIABLE, "1");
        } else {
            std::env::remove_var(FAKE_MEDIA_DEVICES_VARIABLE);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::fake_media_devices_decision;

    #[test]
    fn without_the_flag_the_fakes_stay_off_whatever_the_plugins() {
        assert_eq!(fake_media_devices_decision(false, None), Ok(false));
        assert_eq!(
            fake_media_devices_decision(false, Some("real plugins".to_owned())),
            Ok(false)
        );
    }

    #[test]
    fn the_flag_turns_them_on_only_without_a_problem() {
        assert_eq!(fake_media_devices_decision(true, None), Ok(true));
        assert_eq!(
            fake_media_devices_decision(
                true,
                Some("lib contains libgstapplemedia.dylib".to_owned())
            ),
            Err("--fake-media-devices refused: lib contains libgstapplemedia.dylib".to_owned())
        );
    }
}
