// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The full-page capture as a state machine. The driver answers each action with an event; the
//! machine never reads a clock and never touches the engine. Every failure, and a cancel, runs the
//! restore (viewport, scroll, `afterprint`) before the outcome is reported.

use std::collections::VecDeque;

use image::RgbaImage;
use thiserror::Error;

use crate::assembler::RowAssembler;
use crate::setup::MAX_TILE_DEVICE_PX;

/// The viewport height the page is laid out at to measure it, in device pixels. It is also the
/// tile height when the measurement is unusable.
const PROBE_VIEWPORT_PX: u32 = 1000;
/// The page is always captured at 100 percent zoom.
const CAPTURE_ZOOM_PERCENT: u16 = 100;

/// Reads the scroll position.
pub const SCROLL_STATE: &str = "window.scrollY";
/// Reads the document height in CSS pixels.
pub const HEIGHT_PROBE: &str = "Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0)";
/// Fires `beforeprint` in the page, so the page can prepare for print.
pub const BEFORE_PRINT: &str = "window.dispatchEvent(new Event('beforeprint')); 0";
/// Fires `afterprint` in the page.
pub const AFTER_PRINT: &str = "window.dispatchEvent(new Event('afterprint')); 0";

/// The script that scrolls the page to `y_css` and reads back where it landed.
pub fn scroll_script(y_css: f64) -> String {
    format!("window.scrollTo(0, {y_css}); window.scrollY")
}

/// What a capture needs to know before it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureConfig {
    /// Layout width of the capture, in CSS pixels.
    pub width_css: u32,
    /// Device pixels per CSS pixel.
    pub device_scale: f32,
    /// What to put back afterwards.
    pub restore: ViewState,
    /// The total height cap (MAX_PAGES times the page height, or MAX_SCREENSHOT_DEVICE_PX).
    pub max_height_device_px: u32,
    /// Whether the page gets `beforeprint` and `afterprint` (printing yes, screenshots no).
    pub fire_print_events: bool,
}

/// The view of the tab, so that the capture can put it back exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewState {
    pub width_px: u32,
    pub height_px: u32,
    pub device_scale: f32,
    pub zoom_percent: u16,
    pub scroll_y_css: Option<f64>,
}

/// What the driver must do next. Every action except `SetViewport` and `Finished` gets an answer.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureAction {
    /// Run a script in the page; answer with `CaptureEvent::Evaluated`.
    Evaluate(String),
    /// Resize the rendering context; no answer.
    SetViewport {
        width_px: u32,
        height_px: u32,
        device_scale: f32,
        zoom_percent: u16,
    },
    /// Wait until the engine has rendered the new size; answer with `CaptureEvent::Settled`.
    Settle,
    /// Take the picture of the current viewport; answer with `CaptureEvent::Tile`.
    Screenshot,
    /// The capture is over, after the restore actions.
    Finished(CaptureOutcome),
}

/// What the driver tells the machine.
#[derive(Debug)]
pub enum CaptureEvent {
    Evaluated(Result<f64, String>),
    Settled,
    Tile(RgbaImage),
    Failed(String),
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    Done { rows: u32, truncated: bool },
    Failed(CaptureError),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("the engine failed: {0}")]
    Engine(String),
    #[error("the page did not scroll to new rows")]
    NoProgress,
    #[error("a tile could not be placed: {0}")]
    Assembly(String),
}

/// Rows of the capture, in order, already without the overlap of the last tile.
#[derive(Debug)]
pub struct TileRows {
    pub start_row_px: u32,
    pub image: RgbaImage,
}

/// Why rows could not be written.
#[derive(Debug, Error)]
pub enum SinkError {
    #[error("rows arrived out of order: row {got} came when row {expected} was next")]
    OutOfOrder { expected: u32, got: u32 },
    #[error("rows are {found} px wide, the image is {expected} px wide")]
    WidthChanged { expected: u32, found: u32 },
    #[error("the image is {height} rows tall, more rows arrived")]
    TooManyRows { height: u32 },
    #[error("{written} rows were written, {expected} were promised")]
    Incomplete { written: u32, expected: u32 },
    #[error("could not encode the PNG: {0}")]
    Encode(#[from] png::EncodingError),
    #[error("could not write the file: {0}")]
    Io(#[from] std::io::Error),
}

/// Receives the rows of a capture as they are produced.
pub trait RowSink {
    fn push_rows(&mut self, start_row: u32, rows: &RgbaImage) -> Result<(), SinkError>;
    fn finish(self: Box<Self>) -> Result<(), SinkError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    ReadingState,
    BeforePrint,
    SettlingProbe,
    Probing,
    Scrolling,
    SettlingTile,
    Shooting,
    Restoring,
    Finished,
}

/// The capture, driven by the events of the engine.
#[derive(Debug)]
pub struct CaptureMachine {
    config: CaptureConfig,
    phase: Phase,
    pending: VecDeque<CaptureAction>,
    scroll_before: Option<f64>,
    before_sent: bool,
    total_px: Option<u32>,
    truncated: bool,
    tile_px: u32,
    tile_top_px: u32,
    assembler: RowAssembler,
    rows: Vec<TileRows>,
}

impl CaptureMachine {
    /// Starts a capture; the first actions are returned.
    pub fn new(config: CaptureConfig) -> (CaptureMachine, Vec<CaptureAction>) {
        let scale = if config.device_scale.is_finite() && config.device_scale > 0.0 {
            config.device_scale
        } else {
            1.0
        };
        let mut machine = CaptureMachine {
            config: CaptureConfig {
                device_scale: scale,
                ..config
            },
            phase: Phase::ReadingState,
            pending: VecDeque::from([CaptureAction::Evaluate(SCROLL_STATE.to_owned())]),
            scroll_before: None,
            before_sent: false,
            total_px: None,
            truncated: false,
            tile_px: 0,
            tile_top_px: 0,
            assembler: RowAssembler::new(),
            rows: Vec::new(),
        };
        let first = machine.run();
        (machine, first)
    }

    /// Feeds one answer or command; returns the actions to run next.
    pub fn on_event(&mut self, event: CaptureEvent) -> Vec<CaptureAction> {
        match self.phase {
            Phase::Finished => return Vec::new(),
            Phase::Restoring => {}
            _ => match event {
                CaptureEvent::Cancel => self.restore(CaptureOutcome::Cancelled),
                CaptureEvent::Failed(message) => {
                    self.restore(CaptureOutcome::Failed(CaptureError::Engine(message)));
                }
                event => self.advance(event),
            },
        }
        self.run()
    }

    /// The tiles captured since the last call, in order and without overlap.
    pub fn take_rows(&mut self) -> Vec<TileRows> {
        std::mem::take(&mut self.rows)
    }

    pub fn is_finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    /// The page height in device pixels, once it has been measured (after the cap).
    pub fn total_rows_px(&self) -> Option<u32> {
        self.total_px
    }

    fn advance(&mut self, event: CaptureEvent) {
        match (self.phase, event) {
            (Phase::ReadingState, CaptureEvent::Evaluated(Ok(y))) => {
                self.scroll_before = Some(y);
                if self.config.fire_print_events {
                    self.before_sent = true;
                    self.pending
                        .push_back(CaptureAction::Evaluate(BEFORE_PRINT.to_owned()));
                    self.phase = Phase::BeforePrint;
                } else {
                    self.start_probe();
                }
            }
            (Phase::BeforePrint, CaptureEvent::Evaluated(Ok(_))) => self.start_probe(),
            (Phase::SettlingProbe, CaptureEvent::Settled) => {
                self.pending
                    .push_back(CaptureAction::Evaluate(HEIGHT_PROBE.to_owned()));
                self.phase = Phase::Probing;
            }
            (Phase::Probing, CaptureEvent::Evaluated(Ok(height))) => self.start_tiles(height),
            (Phase::Scrolling, CaptureEvent::Evaluated(Ok(reached))) => {
                self.tile_top_px = device_px(reached, self.config.device_scale);
                self.pending.push_back(CaptureAction::Settle);
                self.phase = Phase::SettlingTile;
            }
            (Phase::SettlingTile, CaptureEvent::Settled) => {
                self.pending.push_back(CaptureAction::Screenshot);
                self.phase = Phase::Shooting;
            }
            (Phase::Shooting, CaptureEvent::Tile(image)) => self.place(image),
            (_, CaptureEvent::Evaluated(Err(message))) => {
                self.restore(CaptureOutcome::Failed(CaptureError::Engine(message)));
            }
            _ => self.restore(CaptureOutcome::Failed(CaptureError::Engine(
                "the engine answered out of turn".to_owned(),
            ))),
        }
    }

    fn start_probe(&mut self) {
        self.pending.push_back(CaptureAction::SetViewport {
            width_px: self.width_px(),
            height_px: PROBE_VIEWPORT_PX,
            device_scale: self.config.device_scale,
            zoom_percent: CAPTURE_ZOOM_PERCENT,
        });
        self.pending.push_back(CaptureAction::Settle);
        self.phase = Phase::SettlingProbe;
    }

    fn start_tiles(&mut self, height_css: f64) {
        let scale = self.config.device_scale;
        let measured = measured_device_px(height_css, scale);
        let cap = self.config.max_height_device_px.max(1);
        self.truncated = measured > cap;
        let total = measured.min(cap);
        self.total_px = Some(total);
        self.tile_px = tile_device_px(total, scale);
        self.pending.push_back(CaptureAction::SetViewport {
            width_px: self.width_px(),
            height_px: self.tile_px,
            device_scale: scale,
            zoom_percent: CAPTURE_ZOOM_PERCENT,
        });
        self.schedule_tile();
    }

    /// Asks for the viewport that starts at the first row not yet delivered, so that the tiles
    /// join with no gap even when the engine scrolls less than asked.
    fn schedule_tile(&mut self) {
        let y_css = f64::from(self.rows_delivered()) / f64::from(self.config.device_scale);
        self.pending
            .push_back(CaptureAction::Evaluate(scroll_script(y_css)));
        self.phase = Phase::Scrolling;
    }

    fn place(&mut self, image: RgbaImage) {
        let total = self.total_px.unwrap_or(0);
        let top = self.tile_top_px;
        let before = self.rows_delivered();
        let visible = image.height().min(total.saturating_sub(top));
        let tile = if visible < image.height() {
            image::imageops::crop_imm(&image, 0, 0, image.width(), visible).to_image()
        } else {
            image
        };
        match self.assembler.push_tile(top, &tile) {
            Err(error) => self.restore(CaptureOutcome::Failed(CaptureError::Assembly(
                error.to_string(),
            ))),
            Ok(fresh) => {
                if let Some(image) = fresh {
                    self.rows.push(TileRows {
                        start_row_px: before,
                        image,
                    });
                }
                let delivered = self.rows_delivered();
                if delivered >= total {
                    let truncated = self.truncated;
                    self.restore(CaptureOutcome::Done {
                        rows: delivered,
                        truncated,
                    });
                } else if delivered == before {
                    self.restore(CaptureOutcome::Failed(CaptureError::NoProgress));
                } else {
                    self.schedule_tile();
                }
            }
        }
    }

    /// Replaces the pending actions with the restore and the outcome.
    fn restore(&mut self, outcome: CaptureOutcome) {
        let view = self.config.restore;
        self.pending.clear();
        self.pending.push_back(CaptureAction::SetViewport {
            width_px: view.width_px,
            height_px: view.height_px,
            device_scale: view.device_scale,
            zoom_percent: view.zoom_percent,
        });
        if let Some(y) = view.scroll_y_css.or(self.scroll_before) {
            self.pending
                .push_back(CaptureAction::Evaluate(scroll_script(y)));
        }
        if self.before_sent {
            self.pending
                .push_back(CaptureAction::Evaluate(AFTER_PRINT.to_owned()));
        }
        self.pending.push_back(CaptureAction::Finished(outcome));
        self.phase = Phase::Restoring;
    }

    /// Runs pending actions up to the next one that needs an answer, or to `Finished`.
    fn run(&mut self) -> Vec<CaptureAction> {
        let mut batch = Vec::new();
        while let Some(action) = self.pending.pop_front() {
            let waits = matches!(
                action,
                CaptureAction::Evaluate(_) | CaptureAction::Settle | CaptureAction::Screenshot
            );
            let finished = matches!(action, CaptureAction::Finished(_));
            batch.push(action);
            if finished {
                self.phase = Phase::Finished;
                break;
            }
            if waits {
                break;
            }
        }
        batch
    }

    fn width_px(&self) -> u32 {
        device_px(f64::from(self.config.width_css), self.config.device_scale)
    }

    fn rows_delivered(&self) -> u32 {
        self.assembler.rows_delivered()
    }
}

/// CSS pixels to device pixels, rounded; negative and NaN values become 0.
fn device_px(css: f64, scale: f32) -> u32 {
    (css.max(0.0) * f64::from(scale)).round() as u32
}

/// The document height in device pixels, or the probe viewport when the measurement is unusable.
fn measured_device_px(height_css: f64, scale: f32) -> u32 {
    let device = height_css * f64::from(scale);
    if device.is_finite() && (1.0..=f64::from(u32::MAX)).contains(&device) {
        device.round() as u32
    } else {
        PROBE_VIEWPORT_PX
    }
}

/// The tile height in device pixels: at most the page and MAX_TILE_DEVICE_PX, whole CSS pixels.
fn tile_device_px(total: u32, scale: f32) -> u32 {
    let limit = total.min(MAX_TILE_DEVICE_PX);
    let css = (limit as f32 / scale).floor().max(1.0);
    (css * scale).floor() as u32
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    const DEVICE_SCALE: f32 = 2.0;
    const NO_CAP: u32 = 1_000_000;

    fn restore_view() -> ViewState {
        ViewState {
            width_px: 2560,
            height_px: 1000,
            device_scale: DEVICE_SCALE,
            zoom_percent: 100,
            scroll_y_css: Some(0.0),
        }
    }

    fn config(width_css: u32, max_height_device_px: u32, fire_print_events: bool) -> CaptureConfig {
        CaptureConfig {
            width_css,
            device_scale: DEVICE_SCALE,
            restore: restore_view(),
            max_height_device_px,
            fire_print_events,
        }
    }

    fn viewport(width_px: u32, height_px: u32, zoom_percent: u16) -> CaptureAction {
        CaptureAction::SetViewport {
            width_px,
            height_px,
            device_scale: DEVICE_SCALE,
            zoom_percent,
        }
    }

    fn eval(script: &str) -> CaptureAction {
        CaptureAction::Evaluate(script.to_owned())
    }

    fn scroll(y_css: f64) -> CaptureAction {
        CaptureAction::Evaluate(scroll_script(y_css))
    }

    fn finished(rows: u32, truncated: bool) -> CaptureAction {
        CaptureAction::Finished(CaptureOutcome::Done { rows, truncated })
    }

    /// Plays the engine for a page that is `page_css_height` CSS pixels tall.
    struct FakeEngine {
        page_css_height: f64,
        scroll_css: f64,
        viewport: (u32, u32),
        scroll_requests: usize,
        /// The nth scroll request (from 1) reaches `short` CSS pixels less than it asked for.
        short_scroll_at: Option<(usize, f64)>,
        /// Every scroll request reaches 0.
        frozen: bool,
    }

    impl FakeEngine {
        fn new(page_css_height: f64) -> Self {
            FakeEngine {
                page_css_height,
                scroll_css: 0.0,
                viewport: (0, 0),
                scroll_requests: 0,
                short_scroll_at: None,
                frozen: false,
            }
        }

        fn answer(&mut self, action: &CaptureAction) -> Option<CaptureEvent> {
            match action {
                CaptureAction::Evaluate(script) => {
                    Some(CaptureEvent::Evaluated(Ok(self.evaluate(script))))
                }
                CaptureAction::SetViewport {
                    width_px,
                    height_px,
                    ..
                } => {
                    self.viewport = (*width_px, *height_px);
                    None
                }
                CaptureAction::Settle => Some(CaptureEvent::Settled),
                CaptureAction::Screenshot => Some(CaptureEvent::Tile(self.screenshot())),
                CaptureAction::Finished(_) => None,
            }
        }

        fn evaluate(&mut self, script: &str) -> f64 {
            match script {
                SCROLL_STATE => return self.scroll_css,
                HEIGHT_PROBE => return self.page_css_height,
                BEFORE_PRINT | AFTER_PRINT => return 0.0,
                _ => {}
            }
            self.scroll_requests += 1;
            let asked = parse_scroll(script);
            let view_css = f64::from(self.viewport.1) / f64::from(DEVICE_SCALE);
            let mut reached = asked.min((self.page_css_height - view_css).max(0.0));
            if let Some((_, short)) = self
                .short_scroll_at
                .filter(|(nth, _)| *nth == self.scroll_requests)
            {
                reached = (reached - short).max(0.0);
            }
            if self.frozen {
                reached = 0.0;
            }
            self.scroll_css = reached;
            reached
        }

        fn screenshot(&self) -> RgbaImage {
            let (width, height) = self.viewport;
            let top = (self.scroll_css * f64::from(DEVICE_SCALE)).round() as u32;
            RgbaImage::from_fn(width, height, |_, y| row_colour(top + y))
        }
    }

    fn parse_scroll(script: &str) -> f64 {
        let asked = script.strip_prefix("window.scrollTo(0, ").unwrap();
        asked.split(')').next().unwrap().parse().unwrap()
    }

    /// Every pixel of absolute row `row` has the colour that encodes `row`.
    fn row_colour(row: u32) -> Rgba<u8> {
        Rgba([row as u8, (row >> 8) as u8, (row >> 16) as u8, 255])
    }

    fn row_of(pixel: &Rgba<u8>) -> u32 {
        let [r, g, b, _] = pixel.0;
        u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
    }

    #[derive(Default)]
    struct Run {
        actions: Vec<CaptureAction>,
        rows: Vec<TileRows>,
    }

    /// Runs the machine against the engine. `inject` replaces the answer with the given index
    /// (counted from 0 over the actions that get an answer) with an event.
    fn drive(
        config: CaptureConfig,
        engine: &mut FakeEngine,
        mut inject: Option<(usize, CaptureEvent)>,
    ) -> (Run, CaptureMachine) {
        let (mut machine, first) = CaptureMachine::new(config);
        let mut queue: VecDeque<CaptureAction> = first.into();
        let mut run = Run::default();
        let mut answers = 0;
        while let Some(action) = queue.pop_front() {
            let answer = match &action {
                CaptureAction::Evaluate(_) | CaptureAction::Settle | CaptureAction::Screenshot => {
                    let index = answers;
                    answers += 1;
                    match inject.take_if(|(at, _)| *at == index) {
                        Some((_, event)) => Some(event),
                        None => engine.answer(&action),
                    }
                }
                _ => engine.answer(&action),
            };
            let finished = matches!(action, CaptureAction::Finished(_));
            run.actions.push(action);
            if finished {
                break;
            }
            if let Some(event) = answer {
                queue.extend(machine.on_event(event));
                run.rows.extend(machine.take_rows());
            }
        }
        assert!(machine.is_finished(), "the machine stalled");
        (run, machine)
    }

    fn capture(config: CaptureConfig, engine: &mut FakeEngine) -> Run {
        drive(config, engine, None).0
    }

    fn assert_rows_cover(rows: &[TileRows], total: u32) {
        let mut next = 0;
        for tile in rows {
            assert_eq!(
                tile.start_row_px, next,
                "tiles must join with no gap and no overlap"
            );
            for y in 0..tile.image.height() {
                assert_eq!(row_of(tile.image.get_pixel(0, y)), next + y);
            }
            next += tile.image.height();
        }
        assert_eq!(next, total);
    }

    #[test]
    fn a_normal_capture_runs_the_documented_sequence() {
        let run = capture(config(718, NO_CAP, true), &mut FakeEngine::new(3000.0));
        assert_eq!(
            run.actions,
            vec![
                eval(SCROLL_STATE),
                eval(BEFORE_PRINT),
                viewport(1436, 1000, 100),
                CaptureAction::Settle,
                eval(HEIGHT_PROBE),
                viewport(1436, 6000, 100),
                scroll(0.0),
                CaptureAction::Settle,
                CaptureAction::Screenshot,
                viewport(2560, 1000, 100),
                scroll(0.0),
                eval(AFTER_PRINT),
                finished(6000, false),
            ]
        );
        assert_rows_cover(&run.rows, 6000);
    }

    #[test]
    fn tall_pages_use_several_tiles() {
        let run = capture(config(200, NO_CAP, false), &mut FakeEngine::new(30_000.0));
        let reached: Vec<f64> = run
            .actions
            .iter()
            .filter_map(|action| match action {
                CaptureAction::Evaluate(script) if script.starts_with("window.scrollTo(0, ") => {
                    Some(parse_scroll(script))
                }
                _ => None,
            })
            .collect();
        // The engine clamps the last request to 25904 CSS px; the request itself is tile index x
        // tile height. The last entry is the restore's scroll back to 0.
        assert_eq!(
            reached,
            vec![
                0.0, 4096.0, 8192.0, 12288.0, 16384.0, 20480.0, 24576.0, 28672.0, 0.0
            ]
        );
        assert_eq!(run.rows.len(), 8);
        assert_rows_cover(&run.rows, 60_000);
        assert_eq!(run.actions.last(), Some(&finished(60_000, false)));
    }

    #[test]
    fn last_tile_overlap_is_removed() {
        let run = capture(config(200, NO_CAP, false), &mut FakeEngine::new(30_000.0));
        let last = run.rows.last().unwrap();
        assert_eq!((last.start_row_px, last.image.height()), (57_344, 2_656));
        assert_rows_cover(&run.rows, 60_000);
    }

    #[test]
    fn short_scroll_overlap_is_removed() {
        let mut engine = FakeEngine::new(30_000.0);
        engine.short_scroll_at = Some((2, 96.0));
        let run = capture(config(200, NO_CAP, false), &mut engine);
        assert_rows_cover(&run.rows, 60_000);
        assert_eq!(run.actions.last(), Some(&finished(60_000, false)));
    }

    #[test]
    fn a_frozen_page_fails_instead_of_looping() {
        let mut engine = FakeEngine::new(30_000.0);
        engine.frozen = true;
        let run = capture(config(200, NO_CAP, false), &mut engine);
        assert_eq!(
            run.actions.last(),
            Some(&CaptureAction::Finished(CaptureOutcome::Failed(
                CaptureError::NoProgress
            )))
        );
        assert_eq!(run.rows.len(), 1);
    }

    #[test]
    fn probe_garbage_is_one_viewport() {
        for height in [f64::NAN, -5.0, 1e12] {
            let run = capture(config(200, NO_CAP, false), &mut FakeEngine::new(height));
            assert_eq!(
                run.actions.last(),
                Some(&finished(1000, false)),
                "probe {height}"
            );
            assert_rows_cover(&run.rows, 1000);
        }
    }

    #[test]
    fn empty_page_gives_one_viewport_of_rows() {
        let run = capture(config(200, NO_CAP, false), &mut FakeEngine::new(0.0));
        assert_eq!(run.rows.len(), 1);
        assert_rows_cover(&run.rows, 1000);
        assert_eq!(run.actions.last(), Some(&finished(1000, false)));
    }

    #[test]
    fn height_cap_truncates() {
        let run = capture(config(200, 20_000, false), &mut FakeEngine::new(30_000.0));
        assert_eq!(
            run.actions.last(),
            Some(&CaptureAction::Finished(CaptureOutcome::Done {
                rows: 20_000,
                truncated: true
            }))
        );
        assert_rows_cover(&run.rows, 20_000);
    }

    #[test]
    fn restore_is_always_the_last_action() {
        for print in [true, false] {
            // The answer that delivers the first tile's picture, counted from 0.
            let tile_answer = if print { 6 } else { 5 };
            for failed in [true, false] {
                for at in 0..=8 {
                    let event = if failed {
                        CaptureEvent::Failed("x".to_owned())
                    } else {
                        CaptureEvent::Cancel
                    };
                    let mut engine = FakeEngine::new(3000.0);
                    let (run, machine) =
                        drive(config(718, NO_CAP, print), &mut engine, Some((at, event)));
                    assert!(machine.is_finished());
                    let restore_at = run
                        .actions
                        .iter()
                        .position(|action| *action == viewport(2560, 1000, 100))
                        .unwrap();
                    let had_before = run.actions[..restore_at].contains(&eval(BEFORE_PRINT));
                    let outcome = if at > tile_answer {
                        CaptureOutcome::Done {
                            rows: 6000,
                            truncated: false,
                        }
                    } else if failed {
                        CaptureOutcome::Failed(CaptureError::Engine("x".to_owned()))
                    } else {
                        CaptureOutcome::Cancelled
                    };
                    let mut expected = vec![viewport(2560, 1000, 100), scroll(0.0)];
                    if had_before {
                        expected.push(eval(AFTER_PRINT));
                    }
                    expected.push(CaptureAction::Finished(outcome));
                    assert_eq!(
                        &run.actions[restore_at..],
                        expected.as_slice(),
                        "print {print}, event at answer {at}"
                    );
                }
            }
        }
    }

    #[test]
    fn later_events_after_finish_are_ignored() {
        let (_, mut machine) = drive(
            config(718, NO_CAP, true),
            &mut FakeEngine::new(3000.0),
            None,
        );
        assert!(machine.is_finished());
        assert!(machine.on_event(CaptureEvent::Cancel).is_empty());
        assert!(
            machine
                .on_event(CaptureEvent::Failed("late".to_owned()))
                .is_empty()
        );
        assert!(machine.take_rows().is_empty());
    }

    #[test]
    fn no_print_events_for_screenshots() {
        let run = capture(config(718, NO_CAP, false), &mut FakeEngine::new(3000.0));
        assert!(!run.actions.contains(&eval(BEFORE_PRINT)));
        assert!(!run.actions.contains(&eval(AFTER_PRINT)));
        assert_eq!(run.actions.last(), Some(&finished(6000, false)));
    }

    #[test]
    fn zoom_is_reset_and_restored() {
        let mut restore = restore_view();
        restore.zoom_percent = 50;
        let run = capture(
            CaptureConfig {
                restore,
                ..config(718, NO_CAP, true)
            },
            &mut FakeEngine::new(3000.0),
        );
        let zooms: Vec<u16> = run
            .actions
            .iter()
            .filter_map(|action| match action {
                CaptureAction::SetViewport { zoom_percent, .. } => Some(*zoom_percent),
                _ => None,
            })
            .collect();
        assert_eq!(zooms, vec![100, 100, 50]);
    }

    #[test]
    fn a_second_settle_timeout_is_the_drivers_job() {
        // The machine has no clock: it waits after a Settle, and only the driver's timeout (a
        // Failed event) moves it on to the restore.
        let (mut machine, _) = CaptureMachine::new(config(718, NO_CAP, true));
        machine.on_event(CaptureEvent::Evaluated(Ok(0.0)));
        let to_settle = machine.on_event(CaptureEvent::Evaluated(Ok(0.0)));
        assert_eq!(to_settle.last(), Some(&CaptureAction::Settle));
        let after_timeout = machine.on_event(CaptureEvent::Failed("timeout".to_owned()));
        assert_eq!(after_timeout.first(), Some(&viewport(2560, 1000, 100)));
        assert!(!machine.is_finished());
    }
}
