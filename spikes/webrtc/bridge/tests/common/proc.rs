// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Process measurements for the tests and the `measure` example: resident memory, peak memory,
//! thread count and CPU time of this process, read through the macOS `libproc` and `getrusage`
//! calls. Test support only; the bridge library itself has no `unsafe`.
#![allow(dead_code)]

use std::ffi::c_void;

#[repr(C)]
struct TimeVal {
    tv_sec: i64,
    tv_usec: i32,
}

#[repr(C)]
struct RUsage {
    ru_utime: TimeVal,
    ru_stime: TimeVal,
    ru_maxrss: i64,
    rest: [i64; 13],
}

#[repr(C)]
#[derive(Default)]
struct TaskInfo {
    virtual_size: u64,
    resident_size: u64,
    total_user: u64,
    total_system: u64,
    threads_user: u64,
    threads_system: u64,
    policy: i32,
    faults: i32,
    pageins: i32,
    cow_faults: i32,
    messages_sent: i32,
    messages_received: i32,
    syscalls_mach: i32,
    syscalls_unix: i32,
    csw: i32,
    threadnum: i32,
    numrunning: i32,
    priority: i32,
}

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut RUsage) -> i32;
    fn getpid() -> i32;
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, size: i32) -> i32;
}

const RUSAGE_SELF: i32 = 0;
const PROC_PIDTASKINFO: i32 = 4;

fn task_info() -> TaskInfo {
    let mut info = TaskInfo::default();
    let size = std::mem::size_of::<TaskInfo>() as i32;
    // SAFETY: `info` is a plain struct of the size passed; proc_pidinfo writes at most `size`
    // bytes into it for the current process.
    let written = unsafe {
        proc_pidinfo(
            getpid(),
            PROC_PIDTASKINFO,
            0,
            (&mut info as *mut TaskInfo).cast::<c_void>(),
            size,
        )
    };
    assert_eq!(written, size, "proc_pidinfo(PROC_PIDTASKINFO) failed");
    info
}

/// Resident set size now, in bytes.
pub fn rss_bytes() -> u64 {
    task_info().resident_size
}

/// Threads in this process now.
pub fn thread_count() -> usize {
    task_info().threadnum.max(0) as usize
}

/// Peak resident set size since the process started, in bytes (macOS reports bytes).
pub fn peak_rss_bytes() -> u64 {
    usage().ru_maxrss.max(0) as u64
}

/// User plus system CPU time of this process so far, in seconds.
pub fn cpu_seconds() -> f64 {
    let usage = usage();
    let timeval = |t: &TimeVal| t.tv_sec as f64 + f64::from(t.tv_usec) / 1e6;
    timeval(&usage.ru_utime) + timeval(&usage.ru_stime)
}

fn usage() -> RUsage {
    let mut usage = RUsage {
        ru_utime: TimeVal {
            tv_sec: 0,
            tv_usec: 0,
        },
        ru_stime: TimeVal {
            tv_sec: 0,
            tv_usec: 0,
        },
        ru_maxrss: 0,
        rest: [0; 13],
    };
    // SAFETY: `usage` matches the layout of `struct rusage` on macOS (two timevals, then 14 longs).
    let result = unsafe { getrusage(RUSAGE_SELF, &mut usage) };
    assert_eq!(result, 0, "getrusage failed");
    usage
}
