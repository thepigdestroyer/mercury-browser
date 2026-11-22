// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Ending the browser: SIGTERM, a grace period, then SIGKILL; and never waiting on a process in
//! state `U` (uninterruptible: a GPU call that hung on a locked screen), which no signal ends.
//! Such a process is reported as stuck and the run stops there.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Term,
    Kill,
}

/// A process the watchdog can end: the real browser, or a fake in the tests.
pub trait Proc {
    fn id(&self) -> u32;
    /// Whether the process has exited (and been reaped).
    fn exited(&mut self) -> bool;
    fn signal(&mut self, signal: Signal);
    /// Kills what the process left behind (its process group). The default does nothing.
    fn sweep(&mut self) {}
    /// How it ended, once it has: `exit status 0`, `signal 11`.
    fn exit_description(&self) -> Option<String> {
        None
    }
}

/// What the watchdog may ask the system about a process.
pub trait Probe {
    /// The first character of `ps -o stat=`.
    fn state(&self, pid: u32) -> Option<char>;
    /// Resident set size in kilobytes.
    fn rss_kb(&self, pid: u32) -> Option<u64>;
}

/// [`Probe`] through `ps`.
pub struct PsProbe;

fn ps(field: &str, pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

impl Probe for PsProbe {
    fn state(&self, pid: u32) -> Option<char> {
        ps("stat", pid)?.chars().next()
    }

    fn rss_kb(&self, pid: u32) -> Option<u64> {
        ps("rss", pid)?.parse().ok()
    }
}

/// How [`terminate`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Termination {
    /// It had already exited.
    Gone,
    /// It exited after SIGTERM.
    Terminated,
    /// It ignored SIGTERM for the whole grace period and was killed.
    Killed,
    /// It could not be ended: it is in state `U`, or survived SIGKILL. Nothing waits on it.
    Stuck { pid: u32, state: Option<char> },
}

enum Waited {
    Exited,
    TimedOut,
    Uninterruptible(char),
}

fn wait_for_exit(
    proc: &mut dyn Proc,
    probe: &dyn Probe,
    limit: Duration,
    poll: Duration,
) -> Waited {
    let started = Instant::now();
    loop {
        if proc.exited() {
            return Waited::Exited;
        }
        if let Some(state) = probe.state(proc.id())
            && state == 'U'
        {
            return Waited::Uninterruptible(state);
        }
        if started.elapsed() >= limit {
            return Waited::TimedOut;
        }
        std::thread::sleep(
            poll.min(limit.saturating_sub(started.elapsed()))
                .max(Duration::from_millis(1)),
        );
    }
}

/// Ends `proc`: SIGTERM, up to `grace` for it to exit, then SIGKILL and up to `kill_wait`.
/// A process seen in state `U` is returned as stuck at once, without waiting for it.
pub fn terminate(
    proc: &mut dyn Proc,
    probe: &dyn Probe,
    grace: Duration,
    kill_wait: Duration,
    poll: Duration,
) -> Termination {
    if proc.exited() {
        proc.sweep();
        return Termination::Gone;
    }
    let pid = proc.id();
    proc.signal(Signal::Term);
    match wait_for_exit(proc, probe, grace, poll) {
        Waited::Exited => {
            proc.sweep();
            return Termination::Terminated;
        }
        Waited::Uninterruptible(state) => {
            proc.signal(Signal::Kill);
            return Termination::Stuck {
                pid,
                state: Some(state),
            };
        }
        Waited::TimedOut => {}
    }
    proc.signal(Signal::Kill);
    match wait_for_exit(proc, probe, kill_wait, poll) {
        Waited::Exited => {
            proc.sweep();
            Termination::Killed
        }
        Waited::Uninterruptible(state) => Termination::Stuck {
            pid,
            state: Some(state),
        },
        Waited::TimedOut => Termination::Stuck {
            pid,
            state: probe.state(pid),
        },
    }
}

/// A child started in its own process group, so that a signal reaches what it started too.
pub struct ChildGroup {
    child: Child,
    status: Option<ExitStatus>,
}

impl ChildGroup {
    /// Starts `command` as the leader of a new process group.
    pub fn spawn(command: &mut Command) -> std::io::Result<ChildGroup> {
        command.process_group(0);
        Ok(ChildGroup {
            child: command.spawn()?,
            status: None,
        })
    }

    fn send(&self, signal: libc::c_int) {
        // SAFETY: kill(2) with a process group of our own child; no memory is involved.
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), signal) };
    }
}

impl Proc for ChildGroup {
    fn id(&self) -> u32 {
        self.child.id()
    }

    fn exited(&mut self) -> bool {
        if self.status.is_some() {
            return true;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.status = Some(status);
                true
            }
            Ok(None) => false,
            // wait failed (already reaped elsewhere): nothing is left to wait for
            Err(_) => true,
        }
    }

    fn signal(&mut self, signal: Signal) {
        self.send(match signal {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        });
    }

    fn sweep(&mut self) {
        self.send(libc::SIGKILL);
    }

    fn exit_description(&self) -> Option<String> {
        use std::os::unix::process::ExitStatusExt;
        let status = self.status?;
        Some(match (status.code(), status.signal()) {
            (Some(code), _) => format!("exit status {code}"),
            (None, Some(signal)) => format!("signal {signal}"),
            _ => "unknown".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Stubborn {
        signals: Vec<Signal>,
        dies_on: Option<Signal>,
        gone: bool,
    }

    impl Proc for Stubborn {
        fn id(&self) -> u32 {
            4242
        }
        fn exited(&mut self) -> bool {
            self.gone
        }
        fn signal(&mut self, signal: Signal) {
            self.signals.push(signal);
            if self.dies_on == Some(signal) {
                self.gone = true;
            }
        }
    }

    struct FixedState(Option<char>, RefCell<u32>);

    impl Probe for FixedState {
        fn state(&self, _pid: u32) -> Option<char> {
            *self.1.borrow_mut() += 1;
            self.0
        }
        fn rss_kb(&self, _pid: u32) -> Option<u64> {
            None
        }
    }

    const POLL: Duration = Duration::from_millis(5);

    #[test]
    fn a_process_that_leaves_on_sigterm_is_not_killed() {
        let mut proc = Stubborn {
            signals: vec![],
            dies_on: Some(Signal::Term),
            gone: false,
        };
        let outcome = terminate(
            &mut proc,
            &FixedState(Some('S'), RefCell::new(0)),
            Duration::from_secs(1),
            Duration::from_secs(1),
            POLL,
        );
        assert_eq!(outcome, Termination::Terminated);
        assert_eq!(proc.signals, [Signal::Term]);
    }

    #[test]
    fn sigkill_follows_only_after_the_grace_period() {
        let mut proc = Stubborn {
            signals: vec![],
            dies_on: Some(Signal::Kill),
            gone: false,
        };
        let started = Instant::now();
        let outcome = terminate(
            &mut proc,
            &FixedState(Some('S'), RefCell::new(0)),
            Duration::from_millis(200),
            Duration::from_secs(1),
            POLL,
        );
        assert_eq!(outcome, Termination::Killed);
        assert_eq!(proc.signals, [Signal::Term, Signal::Kill]);
        assert!(started.elapsed() >= Duration::from_millis(200));
    }

    #[test]
    fn a_process_in_state_u_is_never_waited_for() {
        let mut proc = Stubborn {
            signals: vec![],
            dies_on: None,
            gone: false,
        };
        let started = Instant::now();
        let outcome = terminate(
            &mut proc,
            &FixedState(Some('U'), RefCell::new(0)),
            Duration::from_secs(30),
            Duration::from_secs(30),
            POLL,
        );
        assert_eq!(
            outcome,
            Termination::Stuck {
                pid: 4242,
                state: Some('U')
            }
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "returned at once, not after the grace period"
        );
    }

    #[test]
    fn a_process_that_survives_sigkill_is_stuck() {
        let mut proc = Stubborn {
            signals: vec![],
            dies_on: None,
            gone: false,
        };
        let outcome = terminate(
            &mut proc,
            &FixedState(Some('S'), RefCell::new(0)),
            Duration::from_millis(50),
            Duration::from_millis(50),
            POLL,
        );
        assert_eq!(
            outcome,
            Termination::Stuck {
                pid: 4242,
                state: Some('S')
            }
        );
        assert_eq!(proc.signals, [Signal::Term, Signal::Kill]);
    }

    #[test]
    fn a_process_that_has_gone_is_left_alone() {
        let mut proc = Stubborn {
            signals: vec![],
            dies_on: None,
            gone: true,
        };
        let outcome = terminate(
            &mut proc,
            &FixedState(None, RefCell::new(0)),
            Duration::from_secs(1),
            Duration::from_secs(1),
            POLL,
        );
        assert_eq!(outcome, Termination::Gone);
        assert!(proc.signals.is_empty());
    }

    fn shell(script: &str) -> ChildGroup {
        ChildGroup::spawn(Command::new("sh").args(["-c", script])).expect("spawn sh")
    }

    #[test]
    fn a_real_process_that_ignores_sigterm_is_killed_with_its_children() {
        let mut child = shell("trap '' TERM; sleep 60 & while :; do sleep 0.1; done");
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        let outcome = terminate(
            &mut child,
            &PsProbe,
            Duration::from_millis(400),
            Duration::from_secs(5),
            Duration::from_millis(20),
        );
        assert_eq!(outcome, Termination::Killed);
        assert!(started.elapsed() >= Duration::from_millis(400));
        assert!(child.exited());
        assert_eq!(child.exit_description().as_deref(), Some("signal 9"));
    }

    #[test]
    fn a_real_process_that_obeys_sigterm_ends_at_once() {
        let mut child = shell("sleep 60");
        std::thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        let outcome = terminate(
            &mut child,
            &PsProbe,
            Duration::from_secs(10),
            Duration::from_secs(5),
            Duration::from_millis(20),
        );
        assert_eq!(outcome, Termination::Terminated);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(child.exit_description().as_deref(), Some("signal 15"));
    }

    #[test]
    fn the_probe_reads_state_and_memory_of_a_live_process() {
        let mut child = shell("sleep 5");
        std::thread::sleep(Duration::from_millis(100));
        let state = PsProbe.state(child.id());
        assert!(matches!(state, Some('S' | 'I' | 'R')), "{state:?}");
        assert!(PsProbe.rss_kb(child.id()).is_some_and(|kb| kb > 0));
        let _ = terminate(
            &mut child,
            &PsProbe,
            Duration::from_secs(5),
            Duration::from_secs(5),
            POLL,
        );
        assert_eq!(PsProbe.state(child.id()).filter(|s| *s != 'Z'), None);
    }
}
