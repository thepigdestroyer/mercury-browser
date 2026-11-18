// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A location source that never touches the operating system: for tests, for the diagnostic
//! run (`GeolocationSetup::Diagnostic`), and as the "no location service" source on platforms
//! without one.

use std::cell::RefCell;

use super::{LocationAccuracy, LocationError, LocationFix, LocationSink, LocationSource};

/// What a `FakeLocationSource` was asked to do, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceCall {
    Start(LocationAccuracy),
    SetAccuracy(LocationAccuracy),
    Stop,
}

pub(crate) struct FakeLocationSource {
    /// Delivered from inside every `start`, if set.
    answer: Option<Result<LocationFix, LocationError>>,
    sink: RefCell<Option<LocationSink>>,
    /// The sink of the latest `start`, kept after `stop`.
    latest_sink: RefCell<Option<LocationSink>>,
    calls: RefCell<Vec<SourceCall>>,
}

impl FakeLocationSource {
    /// A source that reports `answer` as soon as it starts.
    pub(crate) fn answering(answer: Result<LocationFix, LocationError>) -> Self {
        Self {
            answer: Some(answer),
            sink: RefCell::default(),
            latest_sink: RefCell::default(),
            calls: RefCell::default(),
        }
    }

    /// A source that reports only what `deliver` is given.
    #[cfg(test)]
    pub(crate) fn silent() -> Self {
        Self {
            answer: None,
            sink: RefCell::default(),
            latest_sink: RefCell::default(),
            calls: RefCell::default(),
        }
    }

    /// Reports `result` as a running source would; does nothing while stopped.
    #[cfg(test)]
    pub(crate) fn deliver(&self, result: Result<LocationFix, LocationError>) {
        let sink = self.sink.borrow().clone();
        if let Some(sink) = sink {
            sink.deliver(result);
        }
    }

    /// Reports `result` through the latest start's sink even after `stop`, as a late
    /// callback from the operating system would.
    #[cfg(test)]
    pub(crate) fn deliver_late(&self, result: Result<LocationFix, LocationError>) {
        let sink = self.latest_sink.borrow().clone();
        if let Some(sink) = sink {
            sink.deliver(result);
        }
    }

    /// Everything asked of the source since the last call.
    #[cfg(test)]
    pub(crate) fn take_calls(&self) -> Vec<SourceCall> {
        std::mem::take(&mut self.calls.borrow_mut())
    }
}

impl LocationSource for FakeLocationSource {
    fn start(&self, accuracy: LocationAccuracy, sink: LocationSink) {
        self.calls.borrow_mut().push(SourceCall::Start(accuracy));
        *self.sink.borrow_mut() = Some(sink.clone());
        *self.latest_sink.borrow_mut() = Some(sink.clone());
        if let Some(answer) = &self.answer {
            sink.deliver(answer.clone());
        }
    }

    fn set_accuracy(&self, accuracy: LocationAccuracy) {
        self.calls
            .borrow_mut()
            .push(SourceCall::SetAccuracy(accuracy));
    }

    fn stop(&self) {
        self.calls.borrow_mut().push(SourceCall::Stop);
        *self.sink.borrow_mut() = None;
    }
}
