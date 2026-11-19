// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Serve the repository's `tests/` directory on the loopback interface, for the media and
//! dialog checks.
//!
//!     scripts/serve-tests.sh [--port N]
//!
//! Binds 127.0.0.1 on an ephemeral port (or `--port`), prints the port on the first line of
//! standard output, and serves `tests/` (so `tests/pages/media.html` is `/pages/media.html`).
//! Single byte ranges are answered with 206 Partial Content, `/private` asks for HTTP Basic
//! authentication (`alice` / `secret`); see `mercury_devtools::serve`. Exits with status 0 on
//! SIGINT, SIGTERM or SIGHUP, and when the process that started it goes away.
//!
//! Signals and the parent-process check are Unix APIs, so on other systems this prints a
//! message and exits with status 1 (the workspace still builds there).

use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    unix::main()
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    eprintln!("serve-tests: needs a Unix system (signal handling and parent-process watch)");
    ExitCode::FAILURE
}

#[cfg(unix)]
mod unix {
    use std::io::Write;
    use std::process::ExitCode;
    use std::time::Duration;

    use mercury_devtools::serve::{self, Command, Server};

    extern "C" fn stop(_signal: libc::c_int) {
        // SAFETY: _exit is async-signal-safe. Responses in progress are cut off, as the Python
        // server's daemon threads were.
        unsafe { libc::_exit(0) }
    }

    fn parent_pid() -> libc::pid_t {
        // SAFETY: getppid has no preconditions and cannot fail.
        unsafe { libc::getppid() }
    }

    pub fn main() -> ExitCode {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let port = match serve::parse_arguments(&arguments) {
            Ok(Command::Help) => {
                print!("{}", serve::help_text());
                return ExitCode::SUCCESS;
            }
            Ok(Command::Serve { port }) => port,
            Err(error) => {
                eprint!("{}", error.message);
                return ExitCode::from(error.status as u8);
            }
        };
        let Ok(port) = u16::try_from(port) else {
            eprintln!("serve-tests: port must be 0-65535");
            return ExitCode::FAILURE;
        };
        let root = serve::default_root();
        let server = match Server::bind(&root, port) {
            Ok(server) => server,
            Err(error) => {
                eprintln!(
                    "serve-tests: cannot serve {} on 127.0.0.1:{port}: {error}",
                    root.display()
                );
                return ExitCode::FAILURE;
            }
        };

        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            // SAFETY: the handler only calls _exit.
            unsafe { libc::signal(signal, stop as *const () as libc::sighandler_t) };
        }

        let parent = parent_pid();
        if parent == 1 {
            // The starting process has already gone: nobody would stop this server.
            return ExitCode::SUCCESS;
        }
        std::thread::spawn(move || {
            while parent_pid() == parent {
                std::thread::sleep(Duration::from_millis(500));
            }
            std::process::exit(0);
        });

        let mut stdout = std::io::stdout().lock();
        if writeln!(stdout, "{}", server.port())
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return ExitCode::FAILURE;
        }
        drop(stdout);
        server.run();
        ExitCode::SUCCESS
    }
}
