//! Fakes shared by the geolocation tests: recording one-shot requests and watches, and a hub
//! over them with a fake source.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::fake::FakeLocationSource;
use super::hub::{LocationHub, PendingPosition, PositionSubscriber, WatchKey};
use super::{LocationError, LocationFix, LocationSource};

#[derive(Debug, PartialEq)]
pub(crate) enum Seen {
    Position(&'static str, f64),
    Error(&'static str, LocationError),
    Dropped(&'static str),
}

type Log = Rc<RefCell<Vec<Seen>>>;

pub(crate) struct Request {
    name: &'static str,
    log: Log,
    answered: bool,
}

impl PendingPosition for Request {
    fn succeed(mut self, fix: &LocationFix) {
        self.answered = true;
        self.log
            .borrow_mut()
            .push(Seen::Position(self.name, fix.latitude));
    }

    fn fail(mut self, error: LocationError) {
        self.answered = true;
        self.log.borrow_mut().push(Seen::Error(self.name, error));
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if !self.answered {
            self.log.borrow_mut().push(Seen::Dropped(self.name));
        }
    }
}

pub(crate) struct Watch {
    name: &'static str,
    log: Log,
}

impl PositionSubscriber for Watch {
    fn update(&self, fix: &LocationFix) {
        self.log
            .borrow_mut()
            .push(Seen::Position(self.name, fix.latitude));
    }

    fn error(&self, error: LocationError) {
        self.log.borrow_mut().push(Seen::Error(self.name, error));
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.log.borrow_mut().push(Seen::Dropped(self.name));
    }
}

impl WatchKey for u32 {
    type Document = u32;
    fn document(&self) -> u32 {
        self / 100
    }
}

pub(crate) type TestHub = LocationHub<Request, Watch, u32>;

pub(crate) struct Harness {
    pub(crate) hub: Rc<TestHub>,
    pub(crate) source: Rc<FakeLocationSource>,
    pub(crate) sources_made: Rc<Cell<u32>>,
    log: Log,
}

impl Harness {
    pub(crate) fn new(source: FakeLocationSource) -> Self {
        let source = Rc::new(source);
        let sources_made = Rc::new(Cell::new(0));
        let factory_source = Rc::clone(&source);
        let factory_count = Rc::clone(&sources_made);
        let hub = TestHub::new(move || {
            factory_count.set(factory_count.get() + 1);
            Rc::clone(&factory_source) as Rc<dyn LocationSource>
        });
        Self {
            hub,
            source,
            sources_made,
            log: Rc::default(),
        }
    }

    pub(crate) fn silent() -> Self {
        Self::new(FakeLocationSource::silent())
    }

    pub(crate) fn request(&self, name: &'static str) -> Request {
        Request {
            name,
            log: Rc::clone(&self.log),
            answered: false,
        }
    }

    pub(crate) fn watch(&self, name: &'static str) -> Watch {
        Watch {
            name,
            log: Rc::clone(&self.log),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Seen> {
        std::mem::take(&mut self.log.borrow_mut())
    }
}

pub(crate) fn fix(latitude: f64) -> LocationFix {
    LocationFix {
        latitude,
        longitude: 2.0,
        accuracy: 10.0,
        altitude: None,
        altitude_accuracy: None,
        heading: None,
        speed: None,
    }
}
