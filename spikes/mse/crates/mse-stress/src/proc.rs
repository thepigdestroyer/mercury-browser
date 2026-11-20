// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the current process is using: resident memory, open file descriptors, threads and CPU
//! time, read with the operating system's own tools (`ps -o rss=`, `lsof -p`, `ps -M`,
//! `ps -o time=`) so that the spike needs no extra dependency. The parsers are pure and tested
//! on captured output; the samplers run the tools.

use std::process::Command;

/// One sample of the current process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub rss_kb: u64,
    pub fds: usize,
    pub threads: usize,
    /// User plus system CPU time of the whole process, in milliseconds (10 ms resolution from `ps`).
    pub cpu_ms: u64,
}

/// `ps -o rss= -p PID`: the resident set size in kilobytes.
pub fn parse_rss_kb(output: &str) -> Option<u64> {
    output.split_whitespace().next()?.parse().ok()
}

/// `ps -M -p PID`: a header line and one line per thread.
pub fn parse_thread_count(output: &str) -> Option<usize> {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next()?;
    if !header.contains("PID") {
        return None;
    }
    Some(lines.count())
}

/// `lsof -n -P -p PID -Ffd`: `f<fd>` lines; only numeric descriptors count (not `cwd`, `txt`, `mem`).
pub fn parse_lsof_fd_count(output: &str) -> usize {
    output
        .lines()
        .filter_map(|line| line.strip_prefix('f'))
        .filter(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()))
        .count()
}

/// `ps -o time= -p PID`: `[[DD-]HH:]MM:SS[.cc]`, as milliseconds.
pub fn parse_cpu_time_ms(output: &str) -> Option<u64> {
    let text = output.split_whitespace().next()?;
    let (days, rest) = match text.split_once('-') {
        Some((days, rest)) => (days.parse::<u64>().ok()?, rest),
        None => (0, text),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        [h, m, s] => (
            h.parse::<f64>().ok()?,
            m.parse::<f64>().ok()?,
            s.parse::<f64>().ok()?,
        ),
        _ => return None,
    };
    let total = days as f64 * 86_400.0 + hours * 3600.0 + minutes * 60.0 + seconds;
    Some((total * 1000.0).round() as u64)
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn pid() -> String {
    std::process::id().to_string()
}

pub fn rss_kb() -> Result<u64, String> {
    let text = run("ps", &["-o", "rss=", "-p", &pid()])?;
    parse_rss_kb(&text).ok_or_else(|| format!("cannot read rss from {text:?}"))
}

pub fn thread_count() -> Result<usize, String> {
    let text = run("ps", &["-M", "-p", &pid()])?;
    parse_thread_count(&text).ok_or_else(|| format!("cannot read threads from {text:?}"))
}

pub fn fd_count() -> Result<usize, String> {
    let text = run("lsof", &["-n", "-P", "-p", &pid(), "-Ffd"])?;
    let count = parse_lsof_fd_count(&text);
    if count == 0 {
        Err(format!("lsof listed no descriptors: {text:?}"))
    } else {
        Ok(count)
    }
}

pub fn cpu_time_ms() -> Result<u64, String> {
    let text = run("ps", &["-o", "time=", "-p", &pid()])?;
    parse_cpu_time_ms(&text).ok_or_else(|| format!("cannot read cpu time from {text:?}"))
}

/// All four numbers (three `ps` calls and one `lsof`: about 100 to 300 ms in all).
pub fn sample() -> Result<Sample, String> {
    Ok(Sample {
        rss_kb: rss_kb()?,
        fds: fd_count()?,
        threads: thread_count()?,
        cpu_ms: cpu_time_ms()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_is_the_first_number() {
        assert_eq!(parse_rss_kb("  123456\n"), Some(123_456));
        assert_eq!(parse_rss_kb(""), None);
        assert_eq!(parse_rss_kb("abc"), None);
    }

    #[test]
    fn threads_are_the_lines_after_the_header() {
        let text = "USER     PID   TT   %CPU STAT PRI     STIME     UTIME COMMAND\n\
                    me     100   ??    0.0 S    31T   0:00.00   0:00.00 /bin/app\n\
                    me     100         0.0 S    31T   0:00.00   0:00.00\n\
                    me     100         0.0 S    31T   0:00.00   0:00.00\n";
        assert_eq!(parse_thread_count(text), Some(3));
        assert_eq!(parse_thread_count("nothing"), None);
        assert_eq!(parse_thread_count(""), None);
    }

    #[test]
    fn only_numeric_descriptors_count() {
        let text = "p83450\nfcwd\nd0x1\nftxt\nd0x2\nf0\nd0x3\nf1\nf2\nf10\nfmem\nf255\n";
        assert_eq!(parse_lsof_fd_count(text), 5);
        assert_eq!(parse_lsof_fd_count(""), 0);
    }

    #[test]
    fn cpu_time_formats() {
        assert_eq!(parse_cpu_time_ms("0:03.45"), Some(3450));
        assert_eq!(parse_cpu_time_ms("  12:03.50 \n"), Some(723_500));
        assert_eq!(parse_cpu_time_ms("1:02:03.25"), Some(3_723_250));
        assert_eq!(
            parse_cpu_time_ms("2-01:00:00"),
            Some(2 * 86_400_000 + 3_600_000)
        );
        assert_eq!(parse_cpu_time_ms("garbage"), None);
        assert_eq!(parse_cpu_time_ms(""), None);
    }

    #[test]
    fn the_samplers_read_this_process() {
        // These run `ps` and `lsof`; both exist on every Mac the spike runs on.
        let sample = sample().expect("ps and lsof work on this machine");
        assert!(sample.rss_kb > 1000);
        assert!(sample.threads >= 1);
        assert!(sample.fds >= 3);
    }
}
