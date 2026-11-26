// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The one call into the system: `sandbox_init_with_parameters`.
//!
//! The SDK header declares only the deprecated `sandbox_init`; this function is not in the
//! public header but is exported by `libsystem_sandbox` and is what Chromium and Firefox call.
//! `flags` 0 means the profile is SBPL text, as for `sandbox_init` in gaol and Servo. The
//! process tests are the proof that it works on the macOS in use.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use crate::confine::EnterError;
use crate::profile::Profile;

unsafe extern "C" {
    fn sandbox_init_with_parameters(
        profile: *const c_char,
        flags: u64,
        parameters: *const *const c_char,
        errorbuf: *mut *mut c_char,
    ) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// Applies `profile` to this process, for good: a sandbox cannot be left or loosened.
pub(crate) fn apply(profile: &Profile) -> Result<(), EnterError> {
    let text = CString::new(profile.text.as_str())
        .map_err(|_| EnterError::InvalidParameter("the profile text contains a NUL".to_owned()))?;
    let mut owned: Vec<CString> = Vec::with_capacity(profile.parameters.len() * 2);
    for (name, value) in &profile.parameters {
        for part in [name, value] {
            owned.push(CString::new(part.as_str()).map_err(|_| {
                EnterError::InvalidParameter(format!("parameter {name} contains a NUL"))
            })?);
        }
    }
    let mut pointers: Vec<*const c_char> = owned.iter().map(|part| part.as_ptr()).collect();
    pointers.push(ptr::null());

    let mut error: *mut c_char = ptr::null_mut();
    // SAFETY: `text` and every element of `pointers` are NUL-terminated strings that outlive
    // the call; `pointers` ends with a null pointer as the function requires; `error` is a
    // valid out-pointer.
    let status =
        unsafe { sandbox_init_with_parameters(text.as_ptr(), 0, pointers.as_ptr(), &mut error) };
    if status == 0 {
        return Ok(());
    }
    let message = if error.is_null() {
        "sandbox_init_with_parameters failed without a message".to_owned()
    } else {
        // SAFETY: on failure the function sets `error` to a NUL-terminated string that we
        // free with `sandbox_free_error` right after copying it.
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe { sandbox_free_error(error) };
        message
    };
    Err(EnterError::Refused(message))
}
