// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::time::Duration;

/// What the platform says about one wheel event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelPhase {
    Began,
    Changed,
    Ended,
    Cancelled,
    Momentum,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelSample {
    /// Monotonic time from any origin.
    pub at: Duration,
    /// Logical px, finger direction: positive = fingers moved right.
    pub delta_x: f32,
    pub delta_y: f32,
    pub phase: WheelPhase,
    /// Pixel deltas (trackpad, Magic Mouse); false for a notched wheel.
    pub precise: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeDirection {
    /// Fingers moved right.
    Back,
    /// Fingers moved left.
    Forward,
}

/// Whether history allows each direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reach {
    pub back: bool,
    pub forward: bool,
}

impl Reach {
    fn allows(self, direction: SwipeDirection) -> bool {
        match direction {
            SwipeDirection::Back => self.back,
            SwipeDirection::Forward => self.forward,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Travel (|x| + |y|) before the direction is decided: smaller jitter is not a gesture.
    pub intent_distance: f32,
    /// |x| must be this many times |y| to count as horizontal: keeps vertical scrolls out.
    pub intent_ratio: f32,
    /// Along-travel before the chip shows: avoids flicker on a tiny drift.
    pub show_after: f32,
    /// Along-travel to arm: the distance a deliberate swipe covers.
    pub arm_distance: f32,
    /// A Began this soon after an Ended is the inertial tail, not a new gesture.
    pub momentum_join: Duration,
    /// Silence longer than this in a live gesture means it was lost.
    pub max_gap: Duration,
}

impl Tuning {
    pub const DEFAULT: Tuning = Tuning {
        intent_distance: 24.0,
        intent_ratio: 2.0,
        show_after: 16.0,
        arm_distance: 120.0,
        momentum_join: Duration::from_millis(80),
        max_gap: Duration::from_millis(400),
    };
}

impl Default for Tuning {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Feedback {
    Hidden,
    Shown {
        direction: SwipeDirection,
        /// 0..=1.
        progress: f32,
        armed: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Update {
    pub feedback: Feedback,
    pub commit: Option<SwipeDirection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Tracking,
    Swiping(SwipeDirection),
    /// Vertical, no history that way, consumed by the page, or inertial tail.
    Ignored,
}

pub struct SwipeRecogniser {
    tuning: Tuning,
    stage: Stage,
    feedback: Feedback,
    sum_x: f32,
    sum_y: f32,
    armed: bool,
    last_at: Option<Duration>,
    ended_at: Option<Duration>,
}

impl SwipeRecogniser {
    pub fn new(tuning: Tuning) -> Self {
        Self {
            tuning,
            stage: Stage::Idle,
            feedback: Feedback::Hidden,
            sum_x: 0.0,
            sum_y: 0.0,
            armed: false,
            last_at: None,
            ended_at: None,
        }
    }

    pub fn on_wheel(&mut self, sample: WheelSample, reach: Reach) -> Update {
        if !sample.precise || sample.phase == WheelPhase::Momentum {
            return self.update(None);
        }
        if self.stage != Stage::Idle
            && self
                .last_at
                .is_some_and(|last| sample.at.saturating_sub(last) > self.tuning.max_gap)
        {
            self.reset();
        }
        let dx = finite(sample.delta_x);
        let dy = finite(sample.delta_y);
        let commit = match sample.phase {
            WheelPhase::Began => {
                self.began(sample.at, dx, dy, reach);
                None
            }
            WheelPhase::Changed => {
                self.moved(dx, dy, reach);
                None
            }
            WheelPhase::Ended => self.finish(sample.at, reach, true),
            WheelPhase::Cancelled => self.finish(sample.at, reach, false),
            WheelPhase::Momentum => None,
        };
        self.last_at = Some(sample.at);
        self.update(commit)
    }

    /// A wheel event of the live gesture came back `DefaultPrevented`: the page has it.
    pub fn page_consumed(&mut self) -> Update {
        if matches!(self.stage, Stage::Tracking | Stage::Swiping(_)) {
            self.stage = Stage::Ignored;
            self.feedback = Feedback::Hidden;
            self.armed = false;
        }
        self.update(None)
    }

    /// The gesture cannot continue (focus lost, tab or modifier changed, a guard turned false).
    pub fn cancel(&mut self) -> Update {
        self.reset();
        self.update(None)
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn feedback(&self) -> Feedback {
        self.feedback
    }

    fn update(&self, commit: Option<SwipeDirection>) -> Update {
        Update {
            feedback: self.feedback,
            commit,
        }
    }

    fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.feedback = Feedback::Hidden;
        self.sum_x = 0.0;
        self.sum_y = 0.0;
        self.armed = false;
        self.last_at = None;
    }

    fn began(&mut self, at: Duration, dx: f32, dy: f32, reach: Reach) {
        match self.stage {
            Stage::Idle => {
                if self
                    .ended_at
                    .is_some_and(|ended| at.saturating_sub(ended) <= self.tuning.momentum_join)
                {
                    self.stage = Stage::Ignored;
                    return;
                }
                self.stage = Stage::Tracking;
                self.sum_x = dx;
                self.sum_y = dy;
                self.decide(reach);
            }
            Stage::Tracking => {
                self.sum_x = dx;
                self.sum_y = dy;
                self.decide(reach);
            }
            Stage::Swiping(_) | Stage::Ignored => self.moved(dx, dy, reach),
        }
    }

    fn moved(&mut self, dx: f32, dy: f32, reach: Reach) {
        match self.stage {
            Stage::Idle | Stage::Ignored => {}
            Stage::Tracking => {
                self.sum_x += dx;
                self.sum_y += dy;
                self.decide(reach);
            }
            Stage::Swiping(direction) => {
                self.sum_x += dx;
                self.sum_y += dy;
                if self.sum_y.abs() > self.sum_x.abs() || !reach.allows(direction) {
                    self.stage = Stage::Ignored;
                    self.feedback = Feedback::Hidden;
                    self.armed = false;
                } else {
                    self.show(direction);
                }
            }
        }
    }

    fn decide(&mut self, reach: Reach) {
        let travel = self.sum_x.abs() + self.sum_y.abs();
        if travel < self.tuning.intent_distance {
            return;
        }
        let horizontal = self.sum_x.abs() >= self.tuning.intent_ratio * self.sum_y.abs();
        let direction = if self.sum_x > 0.0 {
            Some(SwipeDirection::Back)
        } else if self.sum_x < 0.0 {
            Some(SwipeDirection::Forward)
        } else {
            None
        };
        match direction {
            Some(direction) if horizontal && reach.allows(direction) => self.show(direction),
            _ => {
                self.stage = Stage::Ignored;
                self.feedback = Feedback::Hidden;
            }
        }
    }

    fn show(&mut self, direction: SwipeDirection) {
        self.stage = Stage::Swiping(direction);
        let along = match direction {
            SwipeDirection::Back => self.sum_x,
            SwipeDirection::Forward => -self.sum_x,
        }
        .max(0.0);
        let progress = (along / self.tuning.arm_distance).min(1.0);
        self.armed = along >= self.tuning.arm_distance;
        self.feedback = if along >= self.tuning.show_after {
            Feedback::Shown {
                direction,
                progress,
                armed: self.armed,
            }
        } else {
            Feedback::Hidden
        };
    }

    fn finish(&mut self, at: Duration, reach: Reach, lifted: bool) -> Option<SwipeDirection> {
        self.ended_at = Some(at);
        let commit = match self.stage {
            Stage::Swiping(direction) if lifted && self.armed && reach.allows(direction) => {
                Some(direction)
            }
            _ => None,
        };
        self.reset();
        commit
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// The line format of a wheel trace: `<microseconds> <dx> <dy> <phase> <0|1>`.
pub mod fixture {
    use super::{WheelPhase, WheelSample};
    use std::time::Duration;

    pub fn parse(text: &str) -> Result<Vec<WheelSample>, String> {
        let mut samples = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let n = index + 1;
            let parts: Vec<&str> = line.split_whitespace().collect();
            let [us, dx, dy, phase, precise] = parts.as_slice() else {
                return Err(format!(
                    "line {n}: expected 5 fields, found {}",
                    parts.len()
                ));
            };
            let micros: u64 = us
                .parse()
                .map_err(|_| format!("line {n}: bad time {us:?}"))?;
            let delta_x: f32 = dx.parse().map_err(|_| format!("line {n}: bad dx {dx:?}"))?;
            let delta_y: f32 = dy.parse().map_err(|_| format!("line {n}: bad dy {dy:?}"))?;
            let phase = match *phase {
                "Started" => WheelPhase::Began,
                "Moved" => WheelPhase::Changed,
                "Ended" => WheelPhase::Ended,
                "Cancelled" => WheelPhase::Cancelled,
                "Momentum" => WheelPhase::Momentum,
                other => return Err(format!("line {n}: unknown phase {other:?}")),
            };
            let precise = match *precise {
                "1" => true,
                "0" => false,
                other => return Err(format!("line {n}: bad precise flag {other:?}")),
            };
            samples.push(WheelSample {
                at: Duration::from_micros(micros),
                delta_x,
                delta_y,
                phase,
                precise,
            });
        }
        Ok(samples)
    }

    /// Writes `sample` as one trace line; `winit_phase` is the platform's name for the phase.
    pub fn format(sample: &WheelSample, winit_phase: &str) -> String {
        format!(
            "{} {} {} {} {}",
            sample.at.as_micros(),
            sample.delta_x,
            sample.delta_y,
            winit_phase,
            u8::from(sample.precise)
        )
    }
}
