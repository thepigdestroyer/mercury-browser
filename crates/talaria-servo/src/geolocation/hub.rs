//! One location source shared by every page, running only while some page waits for a
//! position (`getCurrentPosition`) or watches it (`watchPosition`).
//!
//! Generic over the request, watch and watch-key types so the rules can be tested without
//! Servo, whose `PositionRequest` and `PositionWatch` only Servo can construct.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use log::info;

use super::{
    LocationAccuracy, LocationError, LocationFix, LocationReceiver, LocationSink, LocationSource,
};

/// A one-shot request. Dropping it unanswered must answer it with a failure (Servo's
/// `PositionRequest` does: "No response sent for request.").
pub(crate) trait PendingPosition {
    fn succeed(self, fix: &LocationFix);
    fn fail(self, error: LocationError);
}

/// An ongoing watch. Must not call back into the hub.
pub(crate) trait PositionSubscriber {
    fn update(&self, fix: &LocationFix);
    fn error(&self, error: LocationError);
}

/// Whose requests these are: one per web view, so closing or navigating it forgets exactly
/// its own requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnerId(u64);

struct Consumer<T> {
    owner: OwnerId,
    high_accuracy: bool,
    item: T,
}

struct Running {
    generation: u64,
    accuracy: LocationAccuracy,
    /// The latest position of this run, given at once to a page that asks while running.
    /// Forgotten when the source stops.
    last_fix: Option<LocationFix>,
}

struct HubState<R, W, K> {
    source: Option<Rc<dyn LocationSource>>,
    requests: Vec<Consumer<R>>,
    watches: Vec<(K, Consumer<W>)>,
    running: Option<Running>,
    next_generation: u64,
}

enum SourceAction {
    Start(Rc<dyn LocationSource>, LocationAccuracy, u64),
    SetAccuracy(Rc<dyn LocationSource>, LocationAccuracy),
    Stop(Rc<dyn LocationSource>),
}

type SourceFactory = Box<dyn Fn() -> Rc<dyn LocationSource>>;

pub(crate) struct LocationHub<R, W, K> {
    this: Weak<Self>,
    make_source: SourceFactory,
    next_owner: Cell<u64>,
    state: RefCell<HubState<R, W, K>>,
}

impl<R, W, K> LocationHub<R, W, K>
where
    R: PendingPosition + 'static,
    W: PositionSubscriber + 'static,
    K: PartialEq + 'static,
{
    /// `make_source` runs once, when a page first asks for a position.
    pub(crate) fn new(make_source: impl Fn() -> Rc<dyn LocationSource> + 'static) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            make_source: Box::new(make_source),
            next_owner: Cell::new(1),
            state: RefCell::new(HubState {
                source: None,
                requests: Vec::new(),
                watches: Vec::new(),
                running: None,
                next_generation: 1,
            }),
        })
    }

    pub(crate) fn new_owner(&self) -> OwnerId {
        let owner = self.next_owner.get();
        self.next_owner.set(owner + 1);
        OwnerId(owner)
    }

    pub(crate) fn request_position(&self, owner: OwnerId, high_accuracy: bool, request: R) {
        if let Some(fix) = self.latest_fix() {
            request.succeed(&fix);
            return;
        }
        self.state.borrow_mut().requests.push(Consumer {
            owner,
            high_accuracy,
            item: request,
        });
        self.update_source();
    }

    pub(crate) fn start_watch(&self, owner: OwnerId, high_accuracy: bool, key: K, watch: W) {
        if let Some(fix) = self.latest_fix() {
            watch.update(&fix);
        }
        self.state.borrow_mut().watches.push((
            key,
            Consumer {
                owner,
                high_accuracy,
                item: watch,
            },
        ));
        self.update_source();
    }

    pub(crate) fn stop_watch(&self, key: &K) {
        let removed = {
            let mut state = self.state.borrow_mut();
            let index = state
                .watches
                .iter()
                .position(|(watch_key, _)| watch_key == key);
            index.map(|index| state.watches.remove(index))
        };
        if removed.is_some() {
            drop(removed);
            self.update_source();
        }
    }

    /// The owner's web view closed or started loading a new document.
    pub(crate) fn forget_owner(&self, owner: OwnerId) {
        let (requests, watches) = {
            let mut state = self.state.borrow_mut();
            let (forgotten_requests, requests) = std::mem::take(&mut state.requests)
                .into_iter()
                .partition::<Vec<_>, _>(|request| request.owner == owner);
            state.requests = requests;
            let (forgotten_watches, watches) = std::mem::take(&mut state.watches)
                .into_iter()
                .partition::<Vec<_>, _>(|(_, watch)| watch.owner == owner);
            state.watches = watches;
            (forgotten_requests, forgotten_watches)
        };
        if requests.is_empty() && watches.is_empty() {
            return;
        }
        // Dropped outside the borrow: an unanswered Servo request answers itself on drop.
        drop(requests);
        drop(watches);
        self.update_source();
    }

    fn latest_fix(&self) -> Option<LocationFix> {
        let state = self.state.borrow();
        state
            .running
            .as_ref()
            .and_then(|running| running.last_fix.clone())
    }

    /// Starts, retunes or stops the source to match who is waiting. The source is called
    /// outside the borrow, since it may report from inside `start`.
    fn update_source(&self) {
        let action = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let waiting = state
                .requests
                .iter()
                .map(|request| request.high_accuracy)
                .chain(state.watches.iter().map(|(_, watch)| watch.high_accuracy));
            let mut anyone = false;
            let mut high = false;
            for high_accuracy in waiting {
                anyone = true;
                high |= high_accuracy;
            }
            let accuracy = if high {
                LocationAccuracy::High
            } else {
                LocationAccuracy::Coarse
            };
            match (&mut state.running, anyone) {
                (None, false) => None,
                (Some(_), false) => {
                    state.running = None;
                    state.source.clone().map(SourceAction::Stop)
                }
                (None, true) => {
                    let generation = state.next_generation;
                    state.next_generation += 1;
                    state.running = Some(Running {
                        generation,
                        accuracy,
                        last_fix: None,
                    });
                    let source = state
                        .source
                        .get_or_insert_with(|| (self.make_source)())
                        .clone();
                    Some(SourceAction::Start(source, accuracy, generation))
                }
                (Some(running), true) if running.accuracy != accuracy => {
                    running.accuracy = accuracy;
                    state
                        .source
                        .clone()
                        .map(|source| SourceAction::SetAccuracy(source, accuracy))
                }
                (Some(_), true) => None,
            }
        };
        match action {
            None => {}
            Some(SourceAction::Start(source, accuracy, generation)) => {
                info!("geolocation: location source started ({accuracy:?} accuracy)");
                let receiver: Weak<dyn LocationReceiver> = self.this.clone();
                source.start(accuracy, LocationSink::new(receiver, generation));
            }
            Some(SourceAction::SetAccuracy(source, accuracy)) => {
                info!("geolocation: location source now asked for {accuracy:?} accuracy");
                source.set_accuracy(accuracy);
            }
            Some(SourceAction::Stop(source)) => {
                info!("geolocation: location source stopped: no page is waiting for a position");
                source.stop();
            }
        }
    }

    #[cfg(test)]
    fn is_running(&self) -> bool {
        self.state.borrow().running.is_some()
    }
}

/// Forgets its owner's requests and watches when dropped, as a web view's delegate is when
/// the web view closes.
pub(crate) struct OwnerGuard<R, W, K>
where
    R: PendingPosition + 'static,
    W: PositionSubscriber + 'static,
    K: PartialEq + 'static,
{
    hub: Rc<LocationHub<R, W, K>>,
    owner: OwnerId,
}

impl<R, W, K> OwnerGuard<R, W, K>
where
    R: PendingPosition + 'static,
    W: PositionSubscriber + 'static,
    K: PartialEq + 'static,
{
    pub(crate) fn new(hub: &Rc<LocationHub<R, W, K>>) -> Self {
        Self {
            hub: Rc::clone(hub),
            owner: hub.new_owner(),
        }
    }

    pub(crate) fn hub(&self) -> &LocationHub<R, W, K> {
        &self.hub
    }

    pub(crate) fn owner(&self) -> OwnerId {
        self.owner
    }
}

impl<R, W, K> Drop for OwnerGuard<R, W, K>
where
    R: PendingPosition + 'static,
    W: PositionSubscriber + 'static,
    K: PartialEq + 'static,
{
    fn drop(&mut self) {
        self.hub.forget_owner(self.owner);
    }
}

impl<R, W, K> LocationReceiver for LocationHub<R, W, K>
where
    R: PendingPosition + 'static,
    W: PositionSubscriber + 'static,
    K: PartialEq + 'static,
{
    fn receive(&self, generation: u64, result: Result<LocationFix, LocationError>) {
        let requests = {
            let mut state = self.state.borrow_mut();
            let Some(running) = state
                .running
                .as_mut()
                .filter(|running| running.generation == generation)
            else {
                return;
            };
            if let Ok(fix) = &result {
                running.last_fix = Some(fix.clone());
            }
            std::mem::take(&mut state.requests)
        };
        for request in requests {
            match &result {
                Ok(fix) => request.item.succeed(fix),
                Err(error) => request.item.fail(*error),
            }
        }
        let ended_watches = {
            let state = self.state.borrow();
            for (_, watch) in &state.watches {
                match &result {
                    Ok(fix) => watch.item.update(fix),
                    Err(error) => watch.item.error(*error),
                }
            }
            drop(state);
            // The system refused: no position will come, so the watches end here. (The page's
            // watch stays registered in script, and gets nothing more.)
            if result == Err(LocationError::PermissionDenied) {
                std::mem::take(&mut self.state.borrow_mut().watches)
            } else {
                Vec::new()
            }
        };
        drop(ended_watches);
        self.update_source();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::geolocation::fake::{FakeLocationSource, SourceCall};

    #[derive(Debug, PartialEq)]
    enum Seen {
        Position(&'static str, f64),
        Error(&'static str, LocationError),
        Dropped(&'static str),
    }

    type Log = Rc<RefCell<Vec<Seen>>>;

    struct Request {
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

    struct Watch {
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

    type TestHub = LocationHub<Request, Watch, u32>;

    struct Harness {
        hub: Rc<TestHub>,
        source: Rc<FakeLocationSource>,
        sources_made: Rc<Cell<u32>>,
        log: Log,
    }

    impl Harness {
        fn new(source: FakeLocationSource) -> Self {
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

        fn silent() -> Self {
            Self::new(FakeLocationSource::silent())
        }

        fn request(&self, name: &'static str) -> Request {
            Request {
                name,
                log: Rc::clone(&self.log),
                answered: false,
            }
        }

        fn watch(&self, name: &'static str) -> Watch {
            Watch {
                name,
                log: Rc::clone(&self.log),
            }
        }

        fn seen(&self) -> Vec<Seen> {
            std::mem::take(&mut self.log.borrow_mut())
        }
    }

    fn fix(latitude: f64) -> LocationFix {
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

    use LocationAccuracy::{Coarse, High};

    #[test]
    fn no_source_exists_until_a_page_asks() {
        let harness = Harness::silent();
        let _owner = harness.hub.new_owner();

        assert_eq!(harness.sources_made.get(), 0);
        assert!(harness.source.take_calls().is_empty());
    }

    #[test]
    fn owners_are_distinct() {
        let harness = Harness::silent();

        assert_ne!(harness.hub.new_owner(), harness.hub.new_owner());
    }

    #[test]
    fn a_one_shot_request_starts_the_source_gets_the_position_and_stops_it() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();

        harness
            .hub
            .request_position(owner, false, harness.request("once"));
        assert_eq!(harness.source.take_calls(), [SourceCall::Start(Coarse)]);
        assert!(harness.hub.is_running());

        harness.source.deliver(Ok(fix(1.0)));

        assert_eq!(harness.seen(), [Seen::Position("once", 1.0)]);
        assert_eq!(harness.source.take_calls(), [SourceCall::Stop]);
        assert!(!harness.hub.is_running());
        assert_eq!(harness.sources_made.get(), 1);
    }

    #[test]
    fn requests_waiting_together_share_one_start_and_one_answer() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();

        harness
            .hub
            .request_position(owner, false, harness.request("a"));
        harness
            .hub
            .request_position(owner, false, harness.request("b"));
        harness.source.deliver(Ok(fix(1.0)));

        assert_eq!(
            harness.seen(),
            [Seen::Position("a", 1.0), Seen::Position("b", 1.0)]
        );
        assert_eq!(
            harness.source.take_calls(),
            [SourceCall::Start(Coarse), SourceCall::Stop]
        );
    }

    #[test]
    fn a_source_that_answers_while_starting_serves_the_request_and_stops() {
        let harness = Harness::new(FakeLocationSource::answering(Ok(fix(7.0))));
        let owner = harness.hub.new_owner();

        harness
            .hub
            .request_position(owner, false, harness.request("once"));

        assert_eq!(harness.seen(), [Seen::Position("once", 7.0)]);
        assert_eq!(
            harness.source.take_calls(),
            [SourceCall::Start(Coarse), SourceCall::Stop]
        );
        assert!(!harness.hub.is_running());
    }

    #[test]
    fn a_watch_gets_every_position_and_keeps_the_source_running() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();

        harness
            .hub
            .start_watch(owner, false, 1, harness.watch("watch"));
        harness.source.deliver(Ok(fix(1.0)));
        harness.source.deliver(Ok(fix(2.0)));

        assert_eq!(
            harness.seen(),
            [Seen::Position("watch", 1.0), Seen::Position("watch", 2.0)]
        );
        assert_eq!(harness.source.take_calls(), [SourceCall::Start(Coarse)]);
        assert!(harness.hub.is_running());
    }

    #[test]
    fn clearing_the_last_watch_stops_the_source() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();
        harness.hub.start_watch(owner, false, 1, harness.watch("a"));
        harness.hub.start_watch(owner, false, 2, harness.watch("b"));
        harness.source.take_calls();

        harness.hub.stop_watch(&1);
        assert_eq!(harness.seen(), [Seen::Dropped("a")]);
        assert!(harness.source.take_calls().is_empty());
        assert!(harness.hub.is_running());

        harness.hub.stop_watch(&2);
        assert_eq!(harness.seen(), [Seen::Dropped("b")]);
        assert_eq!(harness.source.take_calls(), [SourceCall::Stop]);
        assert!(!harness.hub.is_running());
    }

    #[test]
    fn clearing_an_unknown_watch_changes_nothing() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();
        harness.hub.start_watch(owner, false, 1, harness.watch("a"));
        harness.source.take_calls();

        harness.hub.stop_watch(&9);

        assert!(harness.seen().is_empty());
        assert!(harness.source.take_calls().is_empty());
        assert!(harness.hub.is_running());
    }

    #[test]
    fn a_page_asking_while_the_source_runs_gets_the_latest_position_at_once() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();
        harness
            .hub
            .start_watch(owner, false, 1, harness.watch("watch"));
        harness.source.deliver(Ok(fix(3.0)));
        harness.seen();

        harness
            .hub
            .request_position(owner, false, harness.request("once"));
        harness
            .hub
            .start_watch(owner, false, 2, harness.watch("second"));

        assert_eq!(
            harness.seen(),
            [Seen::Position("once", 3.0), Seen::Position("second", 3.0)]
        );
        assert_eq!(harness.source.take_calls(), [SourceCall::Start(Coarse)]);
    }

    #[test]
    fn a_stopped_source_forgets_its_last_position() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();
        harness
            .hub
            .request_position(owner, false, harness.request("a"));
        harness.source.deliver(Ok(fix(1.0)));
        harness.seen();
        harness.source.take_calls();

        harness
            .hub
            .request_position(owner, false, harness.request("b"));

        assert!(harness.seen().is_empty());
        assert_eq!(harness.source.take_calls(), [SourceCall::Start(Coarse)]);
        harness.source.deliver(Ok(fix(2.0)));
        assert_eq!(harness.seen(), [Seen::Position("b", 2.0)]);
        assert_eq!(harness.sources_made.get(), 1, "the source is made once");
    }

    #[test]
    fn high_accuracy_is_asked_for_while_any_page_wants_it() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();

        harness.hub.start_watch(owner, false, 1, harness.watch("a"));
        harness.hub.start_watch(owner, true, 2, harness.watch("b"));
        harness.hub.stop_watch(&2);

        assert_eq!(
            harness.source.take_calls(),
            [
                SourceCall::Start(Coarse),
                SourceCall::SetAccuracy(High),
                SourceCall::SetAccuracy(Coarse),
            ]
        );
    }

    #[test]
    fn a_high_accuracy_request_starts_the_source_with_high_accuracy() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();

        harness
            .hub
            .request_position(owner, true, harness.request("a"));

        assert_eq!(harness.source.take_calls(), [SourceCall::Start(High)]);
    }

    #[test]
    fn an_unavailable_position_fails_requests_and_tells_watches_which_keep_watching() {
        for error in [LocationError::Unavailable, LocationError::Timeout] {
            let harness = Harness::silent();
            let owner = harness.hub.new_owner();
            harness
                .hub
                .request_position(owner, false, harness.request("once"));
            harness
                .hub
                .start_watch(owner, false, 1, harness.watch("watch"));

            harness.source.deliver(Err(error));
            assert_eq!(
                harness.seen(),
                [Seen::Error("once", error), Seen::Error("watch", error)]
            );
            assert!(harness.hub.is_running());

            harness.source.deliver(Ok(fix(4.0)));
            assert_eq!(harness.seen(), [Seen::Position("watch", 4.0)]);
        }
    }

    #[test]
    fn a_system_refusal_fails_everything_ends_the_watches_and_stops_the_source() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();
        harness
            .hub
            .request_position(owner, false, harness.request("once"));
        harness
            .hub
            .start_watch(owner, false, 1, harness.watch("watch"));
        harness.source.take_calls();

        harness.source.deliver(Err(LocationError::PermissionDenied));

        assert_eq!(
            harness.seen(),
            [
                Seen::Error("once", LocationError::PermissionDenied),
                Seen::Error("watch", LocationError::PermissionDenied),
                Seen::Dropped("watch"),
            ]
        );
        assert_eq!(harness.source.take_calls(), [SourceCall::Stop]);
        assert!(!harness.hub.is_running());
    }

    #[test]
    fn a_source_refusing_while_starting_fails_the_request() {
        let harness = Harness::new(FakeLocationSource::answering(Err(
            LocationError::PermissionDenied,
        )));
        let owner = harness.hub.new_owner();

        harness
            .hub
            .start_watch(owner, false, 1, harness.watch("watch"));

        assert_eq!(
            harness.seen(),
            [
                Seen::Error("watch", LocationError::PermissionDenied),
                Seen::Dropped("watch"),
            ]
        );
        assert!(!harness.hub.is_running());
    }

    #[test]
    fn forgetting_an_owner_drops_its_requests_and_watches_only() {
        let harness = Harness::silent();
        let closing = harness.hub.new_owner();
        let staying = harness.hub.new_owner();
        harness
            .hub
            .request_position(closing, false, harness.request("closing once"));
        harness
            .hub
            .start_watch(closing, false, 1, harness.watch("closing watch"));
        harness
            .hub
            .start_watch(staying, false, 2, harness.watch("staying watch"));
        harness.source.take_calls();

        harness.hub.forget_owner(closing);

        assert_eq!(
            harness.seen(),
            [
                Seen::Dropped("closing once"),
                Seen::Dropped("closing watch")
            ]
        );
        assert!(harness.source.take_calls().is_empty());
        harness.source.deliver(Ok(fix(5.0)));
        assert_eq!(harness.seen(), [Seen::Position("staying watch", 5.0)]);
    }

    #[test]
    fn forgetting_the_last_owner_stops_the_source() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();
        harness
            .hub
            .start_watch(owner, true, 1, harness.watch("watch"));
        harness.source.take_calls();

        harness.hub.forget_owner(owner);

        assert_eq!(harness.seen(), [Seen::Dropped("watch")]);
        assert_eq!(harness.source.take_calls(), [SourceCall::Stop]);
        assert!(!harness.hub.is_running());
    }

    #[test]
    fn dropping_an_owner_guard_forgets_its_owner() {
        let harness = Harness::silent();
        let guard = OwnerGuard::new(&harness.hub);
        harness
            .hub
            .start_watch(guard.owner(), false, 1, harness.watch("watch"));
        harness.source.take_calls();

        drop(guard);

        assert_eq!(harness.seen(), [Seen::Dropped("watch")]);
        assert_eq!(harness.source.take_calls(), [SourceCall::Stop]);
    }

    #[test]
    fn a_late_report_from_a_stopped_source_reaches_nobody() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();
        harness.hub.start_watch(owner, false, 1, harness.watch("a"));
        harness.hub.stop_watch(&1);
        harness.hub.start_watch(owner, false, 2, harness.watch("b"));
        harness.seen();
        harness.source.take_calls();
        harness.hub.stop_watch(&2);
        harness.seen();

        harness.source.deliver_late(Ok(fix(6.0)));
        harness
            .hub
            .request_position(owner, false, harness.request("c"));

        assert!(harness.seen().is_empty(), "the late position is not kept");
        assert_eq!(
            harness.source.take_calls(),
            [SourceCall::Stop, SourceCall::Start(Coarse)]
        );
    }

    #[test]
    fn a_report_through_an_earlier_starts_sink_is_ignored() {
        let harness = Harness::silent();
        let owner = harness.hub.new_owner();
        harness
            .hub
            .request_position(owner, false, harness.request("a"));
        let earlier =
            LocationSink::new(Rc::downgrade(&harness.hub) as Weak<dyn LocationReceiver>, 1);
        harness.source.deliver(Ok(fix(1.0)));
        harness
            .hub
            .request_position(owner, false, harness.request("b"));
        harness.seen();

        earlier.deliver(Ok(fix(9.0)));

        assert!(harness.seen().is_empty());
        assert!(harness.hub.is_running());
    }
}
