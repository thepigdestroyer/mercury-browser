// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What a confined content process can do with the system: start programs, signal its parent,
//! reach Mach services, and the things it needs to run (threads, memory, sysctl, fonts). Each
//! test starts a child that confines itself and then tries; the child's exit code is the verdict.

#[path = "support/mod.rs"]
mod support;

use support::{Test, Tree};

const TESTS: &[Test] = &[
    ("cannot_start_programs", cannot_start_programs),
    (
        "mach_negative_refused_and_fonts_lay_out",
        mach_negative_refused_and_fonts_lay_out,
    ),
    ("cannot_signal_the_parent", cannot_signal_the_parent),
    (
        "threads_mmap_sysctl_and_time_work",
        threads_mmap_sysctl_and_time_work,
    ),
    (
        "coretext_font_is_created_and_measured",
        coretext_font_is_created_and_measured,
    ),
];

const JOBS: &[Test] = &[
    ("programs", job_programs),
    ("mach", job_mach),
    ("signal", job_signal),
    ("runtime", job_runtime),
    ("coretext", job_coretext),
];

fn main() {
    support::run(TESTS, JOBS);
}

#[cfg(not(target_os = "macos"))]
fn cannot_start_programs() {}
#[cfg(not(target_os = "macos"))]
fn mach_negative_refused_and_fonts_lay_out() {}
#[cfg(not(target_os = "macos"))]
fn cannot_signal_the_parent() {}
#[cfg(not(target_os = "macos"))]
fn threads_mmap_sysctl_and_time_work() {}
#[cfg(not(target_os = "macos"))]
fn coretext_font_is_created_and_measured() {}

#[cfg(target_os = "macos")]
fn cannot_start_programs() {
    let tree = Tree::new("programs", "home");
    support::verdict("programs", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn mach_negative_refused_and_fonts_lay_out() {
    // The control: unconfined, this process reaches the service the child must not reach. A
    // lookup only reads a port right; nothing is sent, and no clipboard is touched.
    assert!(
        mach_lookup("com.apple.coreservices.launchservicesd"),
        "the control service is not registered on this machine"
    );
    let tree = Tree::new("mach", "home");
    support::verdict("mach", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn cannot_signal_the_parent() {
    let tree = Tree::new("signal", "home");
    support::verdict(
        "signal",
        &tree,
        &[("MERCURY_TEST_PARENT", &std::process::id().to_string())],
    );
}

#[cfg(target_os = "macos")]
fn threads_mmap_sysctl_and_time_work() {
    let tree = Tree::new("runtime", "home");
    support::verdict("runtime", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn coretext_font_is_created_and_measured() {
    let tree = Tree::new("coretext", "home");
    support::verdict("coretext", &tree, &[]);
}

#[cfg(target_os = "macos")]
fn job_programs() {
    use std::process::Command;

    support::child_confine();
    support::expect_eperm(
        Command::new("/bin/ls").arg("/").output().map(|_| ()),
        "starting /bin/ls",
    );
}

#[cfg(target_os = "macos")]
fn job_mach() {
    support::child_confine();
    // A negative by differential check. The service is registered with launchd on this
    // machine (`launchctl print system` lists it) and is not in `profiles/content.sb`, so a
    // refusal is the sandbox's doing. A missing or unregistered name would prove nothing,
    // which is why the service is picked from a live registry, not from memory.
    let registered = mach_lookup("com.apple.coreservices.launchservicesd");
    support::check(
        !registered,
        "a mach service outside the profile was reachable",
    );
}

#[cfg(target_os = "macos")]
fn job_signal() {
    support::child_confine();
    let parent: i32 = support::child_env("MERCURY_TEST_PARENT")
        .parse()
        .unwrap_or_else(|_| support::fail("MERCURY_TEST_PARENT is not a pid"));
    // SAFETY: `kill` with signal 0 only checks whether the process may be signalled.
    let status = unsafe { libc::kill(parent, 0) };
    support::check(status == -1, "kill of the parent succeeded");
    let error = std::io::Error::last_os_error();
    support::check(
        support::is_eperm(&error),
        "kill of the parent failed, but not with EPERM",
    );
}

#[cfg(target_os = "macos")]
fn job_runtime() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    support::child_confine();
    let counter = Arc::new(AtomicU64::new(0));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let counter = Arc::clone(&counter);
            thread::spawn(move || {
                for _ in 0..1000 {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();
    for worker in workers {
        worker
            .join()
            .unwrap_or_else(|_| support::fail("a thread panicked"));
    }
    support::check(
        counter.load(Ordering::SeqCst) == 4000,
        "threads lost updates",
    );

    // SAFETY: an anonymous private mapping, touched and then unmapped by this function.
    unsafe {
        let length = 1 << 20;
        let memory = libc::mmap(
            std::ptr::null_mut(),
            length,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        );
        support::check(memory != libc::MAP_FAILED, "mmap failed");
        memory.cast::<u8>().write(7);
        support::check(
            memory.cast::<u8>().read() == 7,
            "mapped memory did not hold a value",
        );
        libc::munmap(memory, length);
    }

    let mut cores: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: `hw.ncpu` is an integer and `cores`/`size` are valid for the call.
    let status = unsafe {
        libc::sysctlbyname(
            c"hw.ncpu".as_ptr(),
            std::ptr::addr_of_mut!(cores).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    support::check(status == 0 && cores > 0, "sysctl hw.ncpu failed");

    support::check(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .is_ok(),
        "the clock is before the epoch",
    );
}

#[cfg(target_os = "macos")]
fn job_coretext() {
    use core_text::font;
    use core_text::font_descriptor::kCTFontOrientationHorizontal;

    support::child_confine();
    let font = font::new_from_name("Helvetica", 12.0)
        .unwrap_or_else(|()| support::fail("CoreText could not create Helvetica"));
    support::check(font.ascent() > 0.0, "the font has no ascent");
    support::check(font.descent() >= 0.0, "the font has a negative descent");

    // Laying out a glyph goes through the font service the profile allows, so it is the
    // behaviour that matters: a character maps to a glyph and the glyph has an advance.
    let characters: [u16; 1] = [u16::from(b'A')];
    let mut glyphs = [0_u16; 1];
    // SAFETY: `characters` and `glyphs` each hold one element, matching `count` = 1.
    let mapped =
        unsafe { font.get_glyphs_for_characters(characters.as_ptr(), glyphs.as_mut_ptr(), 1) };
    support::check(mapped && glyphs[0] != 0, "CoreText mapped no glyph for 'A'");
    let mut advances = [core_graphics::geometry::CGSize::new(0.0, 0.0)];
    // SAFETY: `glyphs` and `advances` each hold one element, matching `count` = 1.
    unsafe {
        font.get_advances_for_glyphs(
            kCTFontOrientationHorizontal,
            glyphs.as_ptr(),
            advances.as_mut_ptr(),
            1,
        );
    }
    support::check(advances[0].width > 0.0, "the glyph for 'A' has no advance");
}

/// Whether the current process can look up a Mach service by name.
#[cfg(target_os = "macos")]
fn mach_lookup(name: &str) -> bool {
    use std::ffi::CString;

    unsafe extern "C" {
        fn bootstrap_look_up(
            bootstrap: u32,
            name: *const libc::c_char,
            port: *mut u32,
        ) -> libc::c_int;
        static bootstrap_port: u32;
    }
    let name = CString::new(name).expect("a service name has no NUL");
    let mut port = 0_u32;
    // SAFETY: `bootstrap_port` is the process's bootstrap port, set by the system at start;
    // `name` is NUL-terminated and `port` is a valid out-pointer.
    let status = unsafe { bootstrap_look_up(bootstrap_port, name.as_ptr(), &mut port) };
    status == 0
}
