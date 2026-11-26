// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The print backend seam: the desktop hands a finished PDF and its settings to a backend, which
//! lists printers and submits the job. The CUPS backend is in `cups.rs`; the macOS print panel is
//! a second backend in the desktop. Tests use `FakePrintBackend`, which records jobs and never
//! touches a printer.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::ranges::PageSelection;
use crate::setup::PageSetup;

/// A printer the system knows about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrinterInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub accepting: bool,
}

/// One print job: a finished PDF, its title and the copies and pages to print.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintJob {
    pub pdf: PathBuf,
    pub title: String,
    pub copies: u32,
    pub selection: PageSelection,
    pub setup: PageSetup,
    /// `None` means the system default printer.
    pub printer: Option<String>,
}

/// The id the backend gave a submitted job; empty when the backend did not report one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobHandle {
    pub id: String,
}

/// Why a print backend could not list printers or submit a job.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PrintError {
    #[error("the printing system (lp) is not installed")]
    NotInstalled,
    #[error("printing failed: {0}")]
    Failed(String),
}

/// Lists printers and submits print jobs.
pub trait PrintBackend {
    fn printers(&self) -> Result<Vec<PrinterInfo>, PrintError>;
    fn submit(&self, job: &PrintJob) -> Result<JobHandle, PrintError>;
}

/// A backend that records the jobs it is given and never prints. Failures can be scripted.
#[derive(Debug, Default)]
pub struct FakePrintBackend {
    printers: Vec<PrinterInfo>,
    failure: Option<PrintError>,
    jobs: Mutex<Vec<PrintJob>>,
}

impl FakePrintBackend {
    pub fn new(printers: Vec<PrinterInfo>) -> Self {
        Self {
            printers,
            failure: None,
            jobs: Mutex::new(Vec::new()),
        }
    }

    /// Makes every `submit` fail with `error`; the job is not recorded.
    pub fn with_failure(mut self, error: PrintError) -> Self {
        self.failure = Some(error);
        self
    }

    /// The jobs submitted so far, in order.
    pub fn jobs(&self) -> Vec<PrintJob> {
        self.jobs
            .lock()
            .map(|jobs| jobs.clone())
            .unwrap_or_default()
    }
}

impl PrintBackend for FakePrintBackend {
    fn printers(&self) -> Result<Vec<PrinterInfo>, PrintError> {
        Ok(self.printers.clone())
    }

    fn submit(&self, job: &PrintJob) -> Result<JobHandle, PrintError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| PrintError::Failed("the fake job log is poisoned".to_owned()))?;
        jobs.push(job.clone());
        Ok(JobHandle {
            id: format!("fake-{}", jobs.len()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper::{MarginPreset, Orientation, PaperSize};

    fn printer(id: &str) -> PrinterInfo {
        PrinterInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            is_default: false,
            accepting: true,
        }
    }

    fn job(printer: Option<&str>) -> PrintJob {
        PrintJob {
            pdf: PathBuf::from("/nonexistent/mercury-print-test.pdf"),
            title: "My page".to_owned(),
            copies: 1,
            selection: PageSelection::all(3),
            setup: PageSetup {
                paper: PaperSize::A4,
                orientation: Orientation::Portrait,
                margins: MarginPreset::Default,
                scale_percent: 100,
            },
            printer: printer.map(str::to_owned),
        }
    }

    #[test]
    fn fake_records_the_job_and_never_touches_anything() {
        let backend = FakePrintBackend::new(vec![printer("Brother_HL")]);

        let handle = backend.submit(&job(Some("Brother_HL"))).unwrap();

        assert!(!handle.id.is_empty());
        let jobs = backend.jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].printer.as_deref(), Some("Brother_HL"));
        assert_eq!(jobs[0].title, "My page");
    }

    #[test]
    fn fake_lists_its_scripted_printers() {
        let backend = FakePrintBackend::new(vec![printer("Brother_HL"), printer("Canon")]);

        let listed = backend.printers().unwrap();

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].id, "Canon");
    }

    #[test]
    fn fake_scripted_failure() {
        let backend = FakePrintBackend::new(vec![printer("Brother_HL")])
            .with_failure(PrintError::Failed("paper jam".to_owned()));

        let error = backend.submit(&job(Some("Brother_HL"))).unwrap_err();

        assert_eq!(error, PrintError::Failed("paper jam".to_owned()));
        assert!(backend.jobs().is_empty());
    }
}
