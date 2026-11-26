// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The print session: one Print window at a time, its state, and the effects the desktop
//! controller must carry out. Pure: the clock is part of each command, so the tests replay a
//! session without a window, an engine or a printer.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::backend::PrintJob;
use crate::capture::{CaptureConfig, ViewState};
use crate::ranges::{PageSelection, RangeError};
use crate::setup::{MAX_PAGES, PRINT_DEVICE_SCALE, PageSetup};

/// How long the window waits after the last setup change before it captures again.
const RECAPTURE_DELAY: Duration = Duration::from_millis(300);
/// After a finished or cancelled print, script requests are ignored for this long.
const SCRIPT_THROTTLE: Duration = Duration::from_secs(1);

/// Where the flow is in one print session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowState {
    Idle,
    Preparing,
    Ready,
    Working,
    Done,
    Failed(String),
}

/// What asked for the print: a page (possibly from its own script) or a full-page screenshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Page { initiated_by_script: bool },
    Screenshot,
}

/// Where the job goes: a saved PDF, or a printer (`None` is the system default; on macOS the
/// panel chooses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    SavePdf,
    Printer(Option<String>),
}

/// What a capture produced, reported back to the flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureSummary {
    pub pages: usize,
    pub truncated: bool,
}

/// Something that happened, and when it happened (the clock is part of the command).
#[derive(Clone, Debug, PartialEq)]
pub enum FlowCommand {
    Request {
        tab: u64,
        source: Source,
        view: ViewState,
        now: Instant,
    },
    SetupChanged(PageSetup, Instant),
    SelectionChanged(String),
    CopiesChanged(u32),
    DestinationChanged(Destination),
    CaptureProgress {
        pages_ready: usize,
    },
    CaptureFinished(Result<CaptureSummary, String>),
    RecaptureDue(Instant),
    SaveAs(PathBuf),
    Print,
    JobFinished(Result<(), String>, Instant),
    Cancel(Instant),
    TabClosed(u64),
}

/// What the desktop must do in response to a command.
#[derive(Clone, Debug, PartialEq)]
pub enum FlowEffect {
    StartCapture(CaptureConfig),
    CancelCapture,
    Repaginate,
    ShowWindow,
    FocusWindow,
    CloseWindow,
    WritePdf {
        path: PathBuf,
        selection: PageSelection,
    },
    SubmitJob(PrintJob),
    ShowPanel(PrintJob),
    Notify(String),
}

/// The state of one print session.
#[derive(Debug)]
pub struct PrintFlow {
    state: FlowState,
    setup: PageSetup,
    selection_text: String,
    copies: u32,
    destination: Destination,
    tab: Option<u64>,
    source: Option<Source>,
    view: Option<ViewState>,
    pages: usize,
    truncated: bool,
    capturing: bool,
    pending_recapture: Option<Instant>,
    throttle_until: Option<Instant>,
}

impl PrintFlow {
    pub fn new(default_setup: PageSetup) -> Self {
        Self {
            state: FlowState::Idle,
            setup: default_setup.sanitized(),
            selection_text: String::new(),
            copies: 1,
            destination: Destination::SavePdf,
            tab: None,
            source: None,
            view: None,
            pages: 0,
            truncated: false,
            capturing: false,
            pending_recapture: None,
            throttle_until: None,
        }
    }

    pub fn state(&self) -> &FlowState {
        &self.state
    }

    pub fn setup(&self) -> &PageSetup {
        &self.setup
    }

    pub fn tab(&self) -> Option<u64> {
        self.tab
    }

    pub fn source(&self) -> Option<Source> {
        self.source
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn handle(&mut self, command: FlowCommand) -> Vec<FlowEffect> {
        match command {
            FlowCommand::Request {
                tab,
                source,
                view,
                now,
            } => self.request(tab, source, view, now),
            FlowCommand::SetupChanged(setup, now) => self.setup_changed(setup, now),
            FlowCommand::SelectionChanged(text) => {
                self.selection_text = text;
                Vec::new()
            }
            FlowCommand::CopiesChanged(copies) => {
                self.copies = crate::ranges::clamp_copies(copies);
                Vec::new()
            }
            FlowCommand::DestinationChanged(destination) => {
                self.destination = destination;
                Vec::new()
            }
            FlowCommand::CaptureProgress { pages_ready } => {
                if self.capturing {
                    self.pages = self.pages.max(pages_ready);
                }
                Vec::new()
            }
            FlowCommand::CaptureFinished(result) => self.capture_finished(result),
            FlowCommand::RecaptureDue(now) => self.recapture_due(now),
            FlowCommand::SaveAs(path) => self.save_as(path),
            FlowCommand::Print => self.print(),
            FlowCommand::JobFinished(result, now) => self.job_finished(result, now),
            FlowCommand::Cancel(now) => self.cancel(now),
            FlowCommand::TabClosed(tab) => self.tab_closed(tab),
        }
    }

    fn request(
        &mut self,
        tab: u64,
        source: Source,
        view: ViewState,
        now: Instant,
    ) -> Vec<FlowEffect> {
        if self.state != FlowState::Idle {
            return if self.tab == Some(tab) {
                vec![FlowEffect::FocusWindow]
            } else {
                Vec::new()
            };
        }
        if let Source::Page {
            initiated_by_script: true,
        } = source
            && self.throttle_until.is_some_and(|until| now < until)
        {
            return Vec::new();
        }
        self.tab = Some(tab);
        self.source = Some(source);
        self.view = Some(view);
        self.pages = 0;
        self.truncated = false;
        self.state = FlowState::Preparing;
        let mut effects = vec![FlowEffect::ShowWindow];
        effects.push(self.start_capture());
        effects
    }

    fn start_capture(&mut self) -> FlowEffect {
        self.capturing = true;
        self.pending_recapture = None;
        FlowEffect::StartCapture(self.capture_config())
    }

    fn capture_config(&self) -> CaptureConfig {
        let page_device_px = self.setup.content_height_css_px() as f32 * PRINT_DEVICE_SCALE;
        let max_height = (MAX_PAGES as f32 * page_device_px).min(u32::MAX as f32) as u32;
        CaptureConfig {
            width_css: self.setup.content_width_css_px(),
            device_scale: PRINT_DEVICE_SCALE,
            restore: self.view.unwrap_or(ViewState {
                width_px: 0,
                height_px: 0,
                device_scale: PRINT_DEVICE_SCALE,
                zoom_percent: 100,
                scroll_y_css: None,
            }),
            max_height_device_px: max_height,
            fire_print_events: true,
        }
    }

    fn setup_changed(&mut self, setup: PageSetup, now: Instant) -> Vec<FlowEffect> {
        let setup = setup.sanitized();
        let changed = setup.needs_recapture(&self.setup);
        self.setup = setup;
        if self.state == FlowState::Idle || !changed {
            return Vec::new();
        }
        self.pending_recapture = Some(now + RECAPTURE_DELAY);
        let mut effects = Vec::new();
        if self.capturing {
            self.capturing = false;
            effects.push(FlowEffect::CancelCapture);
        }
        self.state = FlowState::Preparing;
        effects.push(FlowEffect::Repaginate);
        effects
    }

    fn recapture_due(&mut self, now: Instant) -> Vec<FlowEffect> {
        match self.pending_recapture {
            Some(due) if now >= due && self.state != FlowState::Idle => {
                vec![self.start_capture()]
            }
            _ => Vec::new(),
        }
    }

    fn capture_finished(&mut self, result: Result<CaptureSummary, String>) -> Vec<FlowEffect> {
        if !self.capturing {
            return Vec::new();
        }
        self.capturing = false;
        match result {
            Ok(summary) => {
                self.pages = summary.pages;
                self.truncated = summary.truncated;
                self.state = FlowState::Ready;
                Vec::new()
            }
            Err(message) => {
                self.state = FlowState::Failed(message.clone());
                vec![FlowEffect::Notify(message)]
            }
        }
    }

    fn save_as(&mut self, path: PathBuf) -> Vec<FlowEffect> {
        if self.state != FlowState::Ready {
            return Vec::new();
        }
        match self.selection() {
            Ok(selection) => {
                self.state = FlowState::Working;
                vec![FlowEffect::WritePdf { path, selection }]
            }
            Err(message) => vec![FlowEffect::Notify(message)],
        }
    }

    fn print(&mut self) -> Vec<FlowEffect> {
        if self.state != FlowState::Ready {
            return Vec::new();
        }
        let selection = match self.selection() {
            Ok(selection) => selection,
            Err(message) => return vec![FlowEffect::Notify(message)],
        };
        let job = PrintJob {
            pdf: PathBuf::new(),
            title: String::new(),
            copies: self.copies,
            selection,
            setup: self.setup,
            printer: None,
        };
        match &self.destination {
            Destination::SavePdf => Vec::new(),
            Destination::Printer(Some(name)) => {
                let job = PrintJob {
                    printer: Some(name.clone()),
                    ..job
                };
                self.state = FlowState::Working;
                vec![FlowEffect::SubmitJob(job)]
            }
            Destination::Printer(None) => {
                self.state = FlowState::Working;
                vec![FlowEffect::ShowPanel(job)]
            }
        }
    }

    fn selection(&self) -> Result<PageSelection, String> {
        PageSelection::parse(&self.selection_text, self.pages).map_err(|error| match error {
            RangeError::Empty => "the document has no pages".to_owned(),
            RangeError::Invalid(item) => format!("\"{item}\" is not a page range"),
            RangeError::OutOfRange { page, total } => {
                format!("page {page} is past the last page ({total})")
            }
            RangeError::Reversed(start, end) => {
                format!("the range {start}-{end} ends before it starts")
            }
        })
    }

    fn job_finished(&mut self, result: Result<(), String>, now: Instant) -> Vec<FlowEffect> {
        match result {
            Ok(()) => {
                self.state = FlowState::Done;
                self.end_session(now)
            }
            Err(message) => {
                self.state = FlowState::Failed(message.clone());
                vec![FlowEffect::Notify(message)]
            }
        }
    }

    fn cancel(&mut self, now: Instant) -> Vec<FlowEffect> {
        let mut effects = Vec::new();
        if self.capturing {
            self.capturing = false;
            effects.push(FlowEffect::CancelCapture);
        }
        effects.extend(self.end_session(now));
        effects
    }

    fn tab_closed(&mut self, tab: u64) -> Vec<FlowEffect> {
        if self.tab != Some(tab) || self.state == FlowState::Idle {
            return Vec::new();
        }
        let mut effects = Vec::new();
        if self.capturing {
            self.capturing = false;
            effects.push(FlowEffect::CancelCapture);
        }
        effects.extend(self.reset_session());
        effects.push(FlowEffect::CloseWindow);
        effects
    }

    fn end_session(&mut self, now: Instant) -> Vec<FlowEffect> {
        if let Some(Source::Page {
            initiated_by_script: true,
        }) = self.source
        {
            self.throttle_until = Some(now + SCRIPT_THROTTLE);
        }
        let mut effects = self.reset_session();
        effects.push(FlowEffect::CloseWindow);
        effects
    }

    fn reset_session(&mut self) -> Vec<FlowEffect> {
        self.state = FlowState::Idle;
        self.tab = None;
        self.source = None;
        self.view = None;
        self.pending_recapture = None;
        self.pages = 0;
        self.truncated = false;
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper::{MarginPreset, Orientation, PaperSize};

    fn setup() -> PageSetup {
        PageSetup {
            paper: PaperSize::A4,
            orientation: Orientation::Portrait,
            margins: MarginPreset::Default,
            scale_percent: 100,
        }
    }

    fn view() -> ViewState {
        ViewState {
            width_px: 1000,
            height_px: 800,
            device_scale: 1.0,
            zoom_percent: 125,
            scroll_y_css: Some(40.0),
        }
    }

    fn page(initiated_by_script: bool, now: Instant) -> FlowCommand {
        FlowCommand::Request {
            tab: 7,
            source: Source::Page {
                initiated_by_script,
            },
            view: view(),
            now,
        }
    }

    fn ready(flow: &mut PrintFlow, pages: usize, now: Instant) {
        flow.handle(page(false, now));
        flow.handle(FlowCommand::CaptureFinished(Ok(CaptureSummary {
            pages,
            truncated: false,
        })));
    }

    fn start_capture_of(effects: &[FlowEffect]) -> Option<&CaptureConfig> {
        effects.iter().find_map(|effect| match effect {
            FlowEffect::StartCapture(config) => Some(config),
            _ => None,
        })
    }

    #[test]
    fn request_starts_a_capture_with_the_page_width() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());

        let effects = flow.handle(page(false, t0));

        assert_eq!(effects[0], FlowEffect::ShowWindow);
        let config = start_capture_of(&effects).unwrap();
        assert_eq!(config.width_css, setup().content_width_css_px());
        assert_eq!(config.device_scale, 2.0);
        assert!(config.fire_print_events);
        assert_eq!(config.restore, view());
        let page_px = setup().content_height_css_px() * 2;
        assert_eq!(config.max_height_device_px, MAX_PAGES as u32 * page_px);
        assert_eq!(flow.state(), &FlowState::Preparing);
        assert_eq!(flow.tab(), Some(7));
    }

    #[test]
    fn second_request_while_open_focuses_the_window() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));

        let effects = flow.handle(page(false, t0));

        assert_eq!(effects, vec![FlowEffect::FocusWindow]);
    }

    #[test]
    fn second_request_from_another_tab_is_ignored() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));

        let effects = flow.handle(FlowCommand::Request {
            tab: 9,
            source: Source::Page {
                initiated_by_script: false,
            },
            view: view(),
            now: t0,
        });

        assert!(effects.is_empty());
        assert_eq!(flow.tab(), Some(7));
    }

    #[test]
    fn script_requests_are_throttled() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(true, t0));
        flow.handle(FlowCommand::CaptureFinished(Ok(CaptureSummary {
            pages: 2,
            truncated: false,
        })));
        flow.handle(FlowCommand::Cancel(t0));

        let during = flow.handle(page(true, t0 + Duration::from_millis(500)));
        assert!(during.is_empty());

        let after = flow.handle(page(true, t0 + Duration::from_millis(1100)));
        assert!(start_capture_of(&after).is_some());
    }

    #[test]
    fn user_request_is_honoured_during_the_script_throttle() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(true, t0));
        flow.handle(FlowCommand::CaptureFinished(Ok(CaptureSummary {
            pages: 2,
            truncated: false,
        })));
        flow.handle(FlowCommand::Cancel(t0));

        let effects = flow.handle(page(false, t0 + Duration::from_millis(100)));

        assert!(start_capture_of(&effects).is_some());
    }

    #[test]
    fn setup_change_recaptures_after_300_ms() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        ready(&mut flow, 2, t0);
        let landscape = PageSetup {
            orientation: Orientation::Landscape,
            ..setup()
        };

        let changed = flow.handle(FlowCommand::SetupChanged(landscape, t0));
        assert_eq!(changed, vec![FlowEffect::Repaginate]);

        let early = flow.handle(FlowCommand::RecaptureDue(t0 + Duration::from_millis(299)));
        assert!(early.is_empty());

        let due = flow.handle(FlowCommand::RecaptureDue(t0 + Duration::from_millis(300)));
        let config = start_capture_of(&due).unwrap();
        assert_eq!(config.width_css, landscape.content_width_css_px());
    }

    #[test]
    fn setup_change_during_capture_cancels_it_first() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));
        let scaled = PageSetup {
            scale_percent: 50,
            ..setup()
        };

        let effects = flow.handle(FlowCommand::SetupChanged(scaled, t0));

        assert_eq!(
            effects,
            vec![FlowEffect::CancelCapture, FlowEffect::Repaginate]
        );
    }

    #[test]
    fn setup_change_that_needs_no_recapture_does_nothing() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        ready(&mut flow, 2, t0);

        let effects = flow.handle(FlowCommand::SetupChanged(setup(), t0));

        assert!(effects.is_empty());
        assert_eq!(flow.state(), &FlowState::Ready);
    }

    #[test]
    fn selection_copies_and_destination_change_do_not_recapture() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        ready(&mut flow, 4, t0);

        assert!(
            flow.handle(FlowCommand::SelectionChanged("1-2".into()))
                .is_empty()
        );
        assert!(flow.handle(FlowCommand::CopiesChanged(3)).is_empty());
        assert!(
            flow.handle(FlowCommand::DestinationChanged(Destination::Printer(None)))
                .is_empty()
        );
        assert_eq!(flow.state(), &FlowState::Ready);
    }

    #[test]
    fn capture_finished_moves_to_ready() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));

        let effects = flow.handle(FlowCommand::CaptureFinished(Ok(CaptureSummary {
            pages: 3,
            truncated: true,
        })));

        assert!(effects.is_empty());
        assert_eq!(flow.state(), &FlowState::Ready);
    }

    #[test]
    fn capture_failed_shows_error_and_stays_open() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));

        let effects = flow.handle(FlowCommand::CaptureFinished(Err("timeout".into())));

        assert_eq!(effects, vec![FlowEffect::Notify("timeout".into())]);
        assert_eq!(flow.state(), &FlowState::Failed("timeout".into()));
        assert_eq!(flow.tab(), Some(7));
    }

    #[test]
    fn save_as_writes_the_selection() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        ready(&mut flow, 4, t0);
        flow.handle(FlowCommand::SelectionChanged("1-2".into()));

        let effects = flow.handle(FlowCommand::SaveAs(PathBuf::from("/tmp/a.pdf")));

        assert_eq!(
            effects,
            vec![FlowEffect::WritePdf {
                path: PathBuf::from("/tmp/a.pdf"),
                selection: PageSelection::parse("1-2", 4).unwrap(),
            }]
        );
        assert_eq!(flow.state(), &FlowState::Working);

        let done = flow.handle(FlowCommand::JobFinished(Ok(()), t0));
        assert_eq!(done, vec![FlowEffect::CloseWindow]);
        assert_eq!(flow.state(), &FlowState::Idle);
    }

    #[test]
    fn print_builds_a_job_with_copies_and_selection() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        ready(&mut flow, 4, t0);
        flow.handle(FlowCommand::SelectionChanged("2-3".into()));
        flow.handle(FlowCommand::CopiesChanged(2));
        flow.handle(FlowCommand::DestinationChanged(Destination::Printer(Some(
            "Brother_HL".into(),
        ))));

        let effects = flow.handle(FlowCommand::Print);

        assert_eq!(effects.len(), 1);
        match &effects[0] {
            FlowEffect::SubmitJob(job) => {
                assert_eq!(job.copies, 2);
                assert_eq!(job.selection, PageSelection::parse("2-3", 4).unwrap());
                assert_eq!(job.setup, setup());
                assert_eq!(job.printer.as_deref(), Some("Brother_HL"));
            }
            other => panic!("expected SubmitJob, got {other:?}"),
        }
    }

    #[test]
    fn print_to_the_panel_destination_shows_the_panel() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        ready(&mut flow, 2, t0);
        flow.handle(FlowCommand::DestinationChanged(Destination::Printer(None)));

        let effects = flow.handle(FlowCommand::Print);

        match effects.as_slice() {
            [FlowEffect::ShowPanel(job)] => assert_eq!(job.printer, None),
            other => panic!("expected ShowPanel, got {other:?}"),
        }
    }

    #[test]
    fn invalid_range_text_keeps_the_old_selection_and_notifies() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        ready(&mut flow, 4, t0);

        flow.handle(FlowCommand::SelectionChanged("3-1".into()));
        let effects = flow.handle(FlowCommand::Print);

        assert_eq!(
            effects,
            vec![FlowEffect::Notify(
                "the range 3-1 ends before it starts".to_owned()
            )]
        );
        assert_eq!(flow.state(), &FlowState::Ready);

        flow.handle(FlowCommand::DestinationChanged(Destination::Printer(None)));
        flow.handle(FlowCommand::SelectionChanged("1-2".into()));
        let effects = flow.handle(FlowCommand::Print);

        assert!(matches!(effects.as_slice(), [FlowEffect::ShowPanel(_)]));
    }

    #[test]
    fn cancel_during_capture_restores() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));

        let effects = flow.handle(FlowCommand::Cancel(t0));

        assert_eq!(
            effects,
            vec![FlowEffect::CancelCapture, FlowEffect::CloseWindow]
        );
        assert_eq!(flow.state(), &FlowState::Idle);
    }

    #[test]
    fn cancel_when_ready_closes() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        ready(&mut flow, 2, t0);

        let effects = flow.handle(FlowCommand::Cancel(t0));

        assert_eq!(effects, vec![FlowEffect::CloseWindow]);
        assert_eq!(flow.state(), &FlowState::Idle);
    }

    #[test]
    fn print_while_capturing_is_ignored() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));

        assert!(flow.handle(FlowCommand::Print).is_empty());
        assert!(
            flow.handle(FlowCommand::SaveAs(PathBuf::from("/tmp/a.pdf")))
                .is_empty()
        );
        assert_eq!(flow.state(), &FlowState::Preparing);
    }

    #[test]
    fn request_after_failure_works() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));
        flow.handle(FlowCommand::CaptureFinished(Err("timeout".into())));

        let effects = flow.handle(FlowCommand::Cancel(t0));
        assert_eq!(effects, vec![FlowEffect::CloseWindow]);

        let again = flow.handle(page(false, t0 + Duration::from_secs(5)));
        assert!(start_capture_of(&again).is_some());
    }

    #[test]
    fn tab_closed_while_capturing_cancels_silently() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));

        let effects = flow.handle(FlowCommand::TabClosed(7));

        assert_eq!(
            effects,
            vec![FlowEffect::CancelCapture, FlowEffect::CloseWindow]
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, FlowEffect::Notify(_)))
        );
        assert_eq!(flow.state(), &FlowState::Idle);
    }

    #[test]
    fn tab_closed_for_another_tab_is_ignored() {
        let t0 = Instant::now();
        let mut flow = PrintFlow::new(setup());
        flow.handle(page(false, t0));

        assert!(flow.handle(FlowCommand::TabClosed(9)).is_empty());
        assert_eq!(flow.state(), &FlowState::Preparing);
    }
}
