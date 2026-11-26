// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The CUPS print backend for Linux (and macOS when the panel is not used): `lpstat` lists the
//! printers, `lp` submits a job. Every command goes through a `CommandRunner` with a vector of
//! arguments (no shell), so the tests can replace the runner and no process is started.

use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::backend::{JobHandle, PrintBackend, PrintError, PrintJob, PrinterInfo};
use crate::paper::Orientation;
use crate::ranges::clamp_copies;

const LPSTAT: &str = "lpstat";
const LP: &str = "lp";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// What a command printed and how it exited.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a program with arguments. An `io::ErrorKind::NotFound` means the program is not installed.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[String]) -> io::Result<CommandOutput>;
}

/// Runs real programs, with no shell and a 10 second limit. Tests never construct it.
#[derive(Debug, Default)]
pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(&self, program: &str, args: &[String]) -> io::Result<CommandOutput> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = read_in_thread(child.stdout.take());
        let stderr = read_in_thread(child.stderr.take());
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the printing command took too long",
                ));
            }
            thread::sleep(POLL_INTERVAL);
        };
        Ok(CommandOutput {
            status: status.code().unwrap_or(-1),
            stdout: join_text(stdout),
            stderr: join_text(stderr),
        })
    }
}

fn read_in_thread<R: Read + Send + 'static>(stream: Option<R>) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut stream) = stream {
            let _ = stream.read_to_string(&mut text);
        }
        text
    })
}

fn join_text(handle: JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}

/// Scripted commands for tests: each call is recorded, and no process is started.
#[derive(Debug, Default)]
pub struct FakeRunner {
    replies: Vec<(String, CommandOutput)>,
    missing: Vec<String>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// The output of `program` with exactly these arguments, for example `reply("lpstat", &["-e"], ..)`.
    pub fn reply(mut self, program: &str, args: &[&str], output: CommandOutput) -> Self {
        self.replies.push((command_key(program, args), output));
        self
    }

    /// Makes every call to `program` fail as if it were not installed.
    pub fn missing(mut self, program: &str) -> Self {
        self.missing.push(program.to_owned());
        self
    }

    /// Every call made so far, as (program, arguments).
    pub fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[String]) -> io::Result<CommandOutput> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push((program.to_owned(), args.to_vec()));
        }
        if self.missing.iter().any(|name| name == program) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let key = std::iter::once(program)
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        self.replies
            .iter()
            .find(|(reply, _)| *reply == key)
            .map(|(_, output)| output.clone())
            .ok_or_else(|| io::Error::other(format!("no scripted reply for `{key}`")))
    }
}

fn command_key(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Printers from `lpstat -e` (names) and `lpstat -p` (state: a disabled printer does not accept jobs).
pub fn parse_lpstat_printers(names: &str, status: &str) -> Vec<PrinterInfo> {
    names
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            let accepting = status
                .lines()
                .find(|line| {
                    line.strip_prefix("printer ")
                        .and_then(|rest| rest.split_whitespace().next())
                        == Some(name)
                })
                .is_none_or(|line| !line.contains("disabled"));
            PrinterInfo {
                id: name.to_owned(),
                name: name.to_owned(),
                is_default: false,
                accepting,
            }
        })
        .collect()
}

/// The name from `lpstat -d`, or `None` when there is no system default destination.
pub fn parse_lpstat_default(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("system default destination:"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// The job id from `lp`'s `request id is <id> (<n> file(s))` line.
fn parse_request_id(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("request id is "))
        .and_then(|rest| rest.split_whitespace().next())
        .map(str::to_owned)
}

/// The `lp` arguments for a job. The title and the path are separate arguments; the path comes
/// after `--`, so it can never be read as an option.
pub fn lp_arguments(job: &PrintJob) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(printer) = &job.printer {
        args.push("-d".to_owned());
        args.push(printer.clone());
    }
    args.push("-n".to_owned());
    args.push(clamp_copies(job.copies).to_string());
    args.push("-t".to_owned());
    args.push(job.title.clone());
    option(&mut args, format!("media={}", job.setup.paper.name()));
    let orientation = match job.setup.orientation {
        Orientation::Portrait => 3,
        Orientation::Landscape => 4,
    };
    option(&mut args, format!("orientation-requested={orientation}"));
    if !job.selection.is_empty() {
        let ranges = job.selection.to_text().replace(' ', "");
        option(&mut args, format!("page-ranges={ranges}"));
    }
    option(&mut args, "fit-to-page".to_owned());
    args.push("--".to_owned());
    args.push(job.pdf.to_string_lossy().into_owned());
    args
}

fn option(args: &mut Vec<String>, value: String) {
    args.push("-o".to_owned());
    args.push(value);
}

/// The CUPS backend, driven through a runner.
#[derive(Debug)]
pub struct CupsBackend<R: CommandRunner> {
    runner: R,
}

impl<R: CommandRunner> CupsBackend<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput, PrintError> {
        let output = self.runner.run(program, args).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                PrintError::NotInstalled
            } else {
                PrintError::Failed(error.to_string())
            }
        })?;
        if output.status != 0 {
            let reason = output
                .stderr
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .unwrap_or("the command failed");
            return Err(PrintError::Failed(reason.to_owned()));
        }
        Ok(output)
    }
}

impl<R: CommandRunner> PrintBackend for CupsBackend<R> {
    fn printers(&self) -> Result<Vec<PrinterInfo>, PrintError> {
        let names = self.run(LPSTAT, &["-e".to_owned()])?.stdout;
        let status = self
            .run(LPSTAT, &["-p".to_owned()])
            .map(|output| output.stdout)
            .unwrap_or_default();
        let default = self
            .run(LPSTAT, &["-d".to_owned()])
            .ok()
            .and_then(|output| parse_lpstat_default(&output.stdout));
        let mut printers = parse_lpstat_printers(&names, &status);
        for printer in &mut printers {
            printer.is_default = default.as_deref() == Some(printer.id.as_str());
        }
        Ok(printers)
    }

    fn submit(&self, job: &PrintJob) -> Result<JobHandle, PrintError> {
        let output = self.run(LP, &lp_arguments(job))?;
        Ok(JobHandle {
            id: parse_request_id(&output.stdout).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::paper::{MarginPreset, PaperSize};
    use crate::ranges::PageSelection;
    use crate::setup::PageSetup;

    fn setup(paper: PaperSize, orientation: Orientation) -> PageSetup {
        PageSetup {
            paper,
            orientation,
            margins: MarginPreset::Default,
            scale_percent: 100,
        }
    }

    fn job(title: &str, copies: u32, selection: PageSelection, setup: PageSetup) -> PrintJob {
        PrintJob {
            pdf: PathBuf::from("/tmp/x.pdf"),
            title: title.to_owned(),
            copies,
            selection,
            setup,
            printer: Some("Brother_HL".to_owned()),
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }

    fn failed(stderr: &str) -> CommandOutput {
        CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn lpstat_lists_printers() {
        let runner = FakeRunner::new()
            .reply("lpstat", &["-e"], ok("Brother_HL\nCanon_MX\n"))
            .reply(
                "lpstat",
                &["-p"],
                ok(
                    "printer Brother_HL is idle.  enabled since Mon Nov 23 09:00:00 2026\n\
                    printer Canon_MX disabled since Mon Nov 23 09:00:00 2026 -\n\
                    \tDescription: Office\n",
                ),
            );
        let backend = CupsBackend::new(runner);

        let printers = backend.printers().unwrap();

        assert_eq!(printers.len(), 2);
        assert_eq!(printers[0].id, "Brother_HL");
        assert!(printers[0].accepting);
        assert_eq!(printers[1].id, "Canon_MX");
        assert!(!printers[1].accepting);
    }

    #[test]
    fn lpstat_default_destination() {
        assert_eq!(
            parse_lpstat_default("system default destination: Brother_HL\n"),
            Some("Brother_HL".to_owned())
        );
        assert_eq!(
            parse_lpstat_default("no system default destination\n"),
            None
        );
        assert_eq!(parse_lpstat_default(""), None);

        let runner = FakeRunner::new()
            .reply("lpstat", &["-e"], ok("Brother_HL\nCanon_MX\n"))
            .reply(
                "lpstat",
                &["-d"],
                ok("system default destination: Canon_MX\n"),
            );
        let printers = CupsBackend::new(runner).printers().unwrap();

        assert!(!printers[0].is_default);
        assert!(printers[1].is_default);
    }

    #[test]
    fn lp_arguments_for_a_full_job() {
        let selection = PageSelection::parse("1-3, 5", 5).unwrap();
        let args = lp_arguments(&job(
            "My page",
            2,
            selection,
            setup(PaperSize::A4, Orientation::Landscape),
        ));

        assert_eq!(
            args,
            [
                "-d",
                "Brother_HL",
                "-n",
                "2",
                "-t",
                "My page",
                "-o",
                "media=A4",
                "-o",
                "orientation-requested=4",
                "-o",
                "page-ranges=1-3,5",
                "-o",
                "fit-to-page",
                "--",
                "/tmp/x.pdf",
            ]
        );
    }

    #[test]
    fn lp_arguments_use_letter_and_portrait() {
        let args = lp_arguments(&job(
            "Page",
            1,
            PageSelection::all(1),
            setup(PaperSize::Letter, Orientation::Portrait),
        ));

        assert!(args.contains(&"media=Letter".to_owned()));
        assert!(args.contains(&"orientation-requested=3".to_owned()));
    }

    #[test]
    fn all_pages_send_their_range_too() {
        let args = lp_arguments(&job(
            "Page",
            1,
            PageSelection::all(3),
            setup(PaperSize::A4, Orientation::Portrait),
        ));

        assert!(args.contains(&"page-ranges=1-3".to_owned()));
    }

    #[test]
    fn copies_are_clamped_in_the_arguments() {
        let args = lp_arguments(&job(
            "Page",
            1000,
            PageSelection::all(1),
            setup(PaperSize::A4, Orientation::Portrait),
        ));

        let index = args.iter().position(|arg| arg == "-n").unwrap();
        assert_eq!(args[index + 1], "99");
    }

    #[test]
    fn no_printer_means_the_default_destination() {
        let mut no_printer = job(
            "Page",
            1,
            PageSelection::all(1),
            setup(PaperSize::A4, Orientation::Portrait),
        );
        no_printer.printer = None;

        assert!(!lp_arguments(&no_printer).contains(&"-d".to_owned()));
    }

    #[test]
    fn title_with_leading_dash_is_one_argument() {
        let args = lp_arguments(&job(
            "-rf",
            1,
            PageSelection::all(1),
            setup(PaperSize::A4, Orientation::Portrait),
        ));

        let index = args.iter().position(|arg| arg == "-t").unwrap();
        assert_eq!(args[index + 1], "-rf");
        let separator = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(args[separator + 1], "/tmp/x.pdf");
    }

    #[test]
    fn submit_with_lp_missing_reports_not_installed() {
        let runner = FakeRunner::new().missing("lp");
        let backend = CupsBackend::new(runner);

        let error = backend
            .submit(&job(
                "Page",
                1,
                PageSelection::all(1),
                setup(PaperSize::A4, Orientation::Portrait),
            ))
            .unwrap_err();

        assert_eq!(error, PrintError::NotInstalled);
    }

    #[test]
    fn submit_with_nonzero_exit_reports_first_stderr_line() {
        let runner = FakeRunner::new().reply(
            "lp",
            &[
                "-d",
                "Brother_HL",
                "-n",
                "1",
                "-t",
                "Page",
                "-o",
                "media=A4",
                "-o",
                "orientation-requested=3",
                "-o",
                "page-ranges=1",
                "-o",
                "fit-to-page",
                "--",
                "/tmp/x.pdf",
            ],
            failed("lp: Printer is not accepting jobs.\nsecond line\n"),
        );
        let backend = CupsBackend::new(runner);

        let error = backend
            .submit(&job(
                "Page",
                1,
                PageSelection::all(1),
                setup(PaperSize::A4, Orientation::Portrait),
            ))
            .unwrap_err();

        assert_eq!(
            error,
            PrintError::Failed("lp: Printer is not accepting jobs.".to_owned())
        );
    }

    #[test]
    fn submit_parses_request_id() {
        assert_eq!(
            parse_request_id("request id is Brother_HL-12 (1 file(s))\n"),
            Some("Brother_HL-12".to_owned())
        );
        assert_eq!(parse_request_id("something else"), None);

        let runner = FakeRunner::new().reply(
            "lp",
            &[
                "-d",
                "Brother_HL",
                "-n",
                "1",
                "-t",
                "Page",
                "-o",
                "media=A4",
                "-o",
                "orientation-requested=3",
                "-o",
                "page-ranges=1",
                "-o",
                "fit-to-page",
                "--",
                "/tmp/x.pdf",
            ],
            ok("request id is Brother_HL-12 (1 file(s))\n"),
        );
        let handle = CupsBackend::new(runner)
            .submit(&job(
                "Page",
                1,
                PageSelection::all(1),
                setup(PaperSize::A4, Orientation::Portrait),
            ))
            .unwrap();

        assert_eq!(handle.id, "Brother_HL-12");
    }

    #[test]
    fn the_runner_is_the_only_way_out() {
        let runner = FakeRunner::new()
            .reply("lpstat", &["-e"], ok("Brother_HL\n"))
            .reply(
                "lp",
                &[
                    "-d",
                    "Brother_HL",
                    "-n",
                    "1",
                    "-t",
                    "Page",
                    "-o",
                    "media=A4",
                    "-o",
                    "orientation-requested=3",
                    "-o",
                    "page-ranges=1",
                    "-o",
                    "fit-to-page",
                    "--",
                    "/tmp/x.pdf",
                ],
                ok("request id is Brother_HL-1 (1 file(s))\n"),
            );
        let backend = CupsBackend::new(runner);

        backend.printers().unwrap();
        backend
            .submit(&job(
                "Page",
                1,
                PageSelection::all(1),
                setup(PaperSize::A4, Orientation::Portrait),
            ))
            .unwrap();

        let programs: Vec<String> = backend
            .runner
            .calls()
            .into_iter()
            .map(|(program, _)| program)
            .collect();
        assert!(!programs.is_empty());
        assert!(
            programs
                .iter()
                .all(|program| program == "lpstat" || program == "lp")
        );
    }

    #[test]
    fn real_runner_is_a_command_runner() {
        fn assert_runner<R: CommandRunner>() {}
        assert_runner::<RealRunner>();
    }
}
