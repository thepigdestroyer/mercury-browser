// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::Sender;

use servo::{JSValue, JavaScriptEvaluationError, LoadStatus};
use talaria_core::{BrowserEvent, PageIcon, TabEvent, TabId};

/// The script that reads the current document's title.
pub(crate) const DOCUMENT_TITLE_SCRIPT: &str = "document.title";

/// Receives the result of one title evaluation.
pub(crate) type TitleCallback = Box<dyn FnOnce(Result<JSValue, JavaScriptEvaluationError>)>;

/// Keeps the tab's title and icon right where Servo's notifications are not enough.
///
/// Servo notifies a title only when its cached title changes, and never resets that cache
/// between documents, so a new document with the same title, or with none, sends nothing.
/// Its cached favicon is never cleared either. The title is therefore read from the document
/// (`document.title`) when a document's head has been parsed, when it has loaded, and after a
/// same-document navigation, with at most one evaluation in flight; the icon is remembered per
/// document here and sent again where an address change may have cleared it.
///
/// Holds the tab's event sender, never a `WebView`: evaluation callbacks keep this alive, and a
/// `WebView` in them would keep Servo alive.
pub(crate) struct PageFeedback {
    tab_id: TabId,
    sender: Sender<TabEvent>,
    /// Changes whenever the document or its address changes, so that an answer about an
    /// earlier one is dropped.
    generation: Cell<u64>,
    icon: RefCell<Option<PageIcon>>,
    /// The generation of the title read in flight. Servo leaves an evaluation unanswered when
    /// the content process crashes or the page's script thread is stuck, so a new document
    /// forgets it: a stuck read costs one document, not the tab.
    title_in_flight: Cell<Option<u64>>,
    /// A title read was asked for while one was in flight; `refresh_title` starts it.
    title_wanted: Cell<bool>,
    /// How many title changes Servo has notified. A read is asked at one moment and answered at
    /// a later one, by a different route than the notifications; an answer is current only if no
    /// notification came in between (see `title_answer`).
    title_notices: Cell<u64>,
    /// history lane: the address shown (as `AddressChanged` sent it), so that a title read
    /// also goes out as `PageTitleAt` for history recording. A read answers only while the
    /// address it was asked for is still shown (`generation`).
    address: RefCell<Option<String>>,
}

impl PageFeedback {
    pub(crate) fn new(tab_id: TabId, sender: Sender<TabEvent>) -> Rc<Self> {
        Rc::new(Self {
            tab_id,
            sender,
            generation: Cell::new(0),
            icon: RefCell::new(None),
            title_in_flight: Cell::new(None),
            title_wanted: Cell::new(false),
            title_notices: Cell::new(0),
            address: RefCell::new(None),
        })
    }

    /// Servo notified a title change. A read that is in flight now answers with the title as it
    /// was when the page ran it, which this notification supersedes or repeats.
    pub(crate) fn title_notified(&self) {
        self.title_notices
            .set(self.title_notices.get().wrapping_add(1));
    }

    /// A new document is loading: it has no icon yet, and the tab must not keep the last one.
    pub(crate) fn document_started(&self) {
        self.bump();
        self.title_in_flight.set(None);
        self.icon.replace(None);
        self.send(BrowserEvent::PageIconChanged { icon: None });
    }

    pub(crate) fn icon_changed(&self, icon: Option<PageIcon>) {
        self.icon.replace(icon);
    }

    /// The document finished loading: its icon again, in case a `pushState` during the load
    /// cleared the tab's copy.
    pub(crate) fn load_completed(&self) {
        self.send_icon();
    }

    // history lane
    /// The address now shown, as `AddressChanged` sent it.
    pub(crate) fn set_address(&self, url: String) {
        self.address.replace(Some(url));
    }

    /// The address changed. For a same-document navigation the icon is sent again (the
    /// address change cleared it); returns whether the title should be read.
    pub(crate) fn address_changed(&self, status: LoadStatus) -> bool {
        self.bump();
        let same_document = is_same_document_navigation(status);
        if same_document {
            self.send_icon();
        }
        same_document
    }

    /// Reads the title through `evaluate`, unless a read is already in flight, in which case
    /// one more read is owed (see `refresh_title`).
    pub(crate) fn request_title(self: &Rc<Self>, evaluate: impl FnOnce(TitleCallback)) {
        if self.title_in_flight.get().is_some() {
            self.title_wanted.set(true);
            return;
        }
        let generation = self.generation.get();
        let notices = self.title_notices.get();
        self.title_in_flight.set(Some(generation));
        let feedback = Rc::clone(self);
        evaluate(Box::new(move |result| {
            feedback.title_answer(generation, notices, result);
        }));
    }

    /// Starts the read that was owed while another was in flight, if it is still owed and the
    /// other has answered.
    pub(crate) fn refresh_title(self: &Rc<Self>, evaluate: impl FnOnce(TitleCallback)) {
        if self.title_wanted.get() && self.title_in_flight.get().is_none() {
            self.title_wanted.set(false);
            self.request_title(evaluate);
        }
    }

    /// `notices` is the notification count when the read was asked for. A page can change its
    /// title right after it loads, and the notification of that change can reach the browser
    /// before the answer of a read that the page ran just before it: the answer would then put
    /// the older title back and the tab would keep it. Every later change is notified too, so
    /// the tab already has the page's latest title once one notification came in.
    fn title_answer(
        &self,
        generation: u64,
        notices: u64,
        result: Result<JSValue, JavaScriptEvaluationError>,
    ) {
        if self.title_in_flight.get() == Some(generation) {
            self.title_in_flight.set(None);
        }
        if generation != self.generation.get() || notices != self.title_notices.get() {
            return;
        }
        if let Ok(JSValue::String(title)) = result {
            self.send(BrowserEvent::PageTitleChanged {
                title: Some(title.clone()),
            });
            // history lane: Servo notifies no title for a new document whose title equals the
            // last one, so history takes the titles read here too.
            let address = self.address.borrow().clone();
            if let Some(url) = address {
                self.send(BrowserEvent::PageTitleAt {
                    url,
                    title: Some(title),
                });
            }
        }
    }

    fn bump(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
    }

    fn send_icon(&self) {
        let icon = self.icon.borrow().clone();
        self.send(BrowserEvent::PageIconChanged { icon });
    }

    fn send(&self, event: BrowserEvent) {
        let _send_result = self.sender.send(TabEvent::new(self.tab_id, event));
    }
}

/// An address change while no load is in progress is a same-document navigation (fragment,
/// `pushState`, `replaceState`); during a load it is the new document committing.
pub(crate) fn is_same_document_navigation(status: LoadStatus) -> bool {
    status == LoadStatus::Complete
}

/// Whether a load status means the document's title can now be read.
pub(crate) fn reads_title_at(status: LoadStatus) -> bool {
    matches!(status, LoadStatus::HeadParsed | LoadStatus::Complete)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver};

    use super::*;

    const TAB: TabId = TabId::new(3);

    fn feedback() -> (Rc<PageFeedback>, Receiver<TabEvent>) {
        let (sender, receiver) = mpsc::channel();
        (PageFeedback::new(TAB, sender), receiver)
    }

    fn events(receiver: &Receiver<TabEvent>) -> Vec<BrowserEvent> {
        receiver.try_iter().map(TabEvent::into_event).collect()
    }

    fn icon() -> PageIcon {
        PageIcon::from_rgba(1, 1, vec![1, 2, 3, 255]).expect("the test icon is valid")
    }

    fn title(text: &str) -> BrowserEvent {
        BrowserEvent::PageTitleChanged {
            title: Some(text.to_owned()),
        }
    }

    /// A fake evaluator: keeps the callbacks so the test answers them.
    #[derive(Default)]
    struct Evaluations(RefCell<Vec<TitleCallback>>);

    impl Evaluations {
        fn evaluate(&self) -> impl FnOnce(TitleCallback) + '_ {
            |callback| self.0.borrow_mut().push(callback)
        }

        fn count(&self) -> usize {
            self.0.borrow().len()
        }

        fn answer(&self, result: Result<JSValue, JavaScriptEvaluationError>) {
            let callback = self.0.borrow_mut().remove(0);
            callback(result);
        }
    }

    #[test]
    fn a_title_answer_for_the_current_document_becomes_a_title_event() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();

        feedback.request_title(evaluations.evaluate());
        evaluations.answer(Ok(JSValue::String("Same title".to_owned())));

        assert_eq!(events(&receiver), vec![title("Same title")]);
    }

    // history lane
    #[test]
    fn a_title_read_also_goes_to_history_with_the_address_it_was_read_at() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();
        feedback.set_address("https://a.test/".to_owned());

        feedback.request_title(evaluations.evaluate());
        evaluations.answer(Ok(JSValue::String("Same title".to_owned())));

        assert_eq!(
            events(&receiver),
            vec![
                title("Same title"),
                BrowserEvent::PageTitleAt {
                    url: "https://a.test/".to_owned(),
                    title: Some("Same title".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn a_title_answer_does_not_replace_a_title_notified_after_the_read_was_asked() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();

        // The read runs in the page while it still has its first title; the page then changes
        // it, and that notification overtakes the answer.
        feedback.request_title(evaluations.evaluate());
        feedback.title_notified();
        evaluations.answer(Ok(JSValue::String("First title".to_owned())));

        assert_eq!(events(&receiver), Vec::new());
        // The read is over, so the next one is asked for and answered as usual.
        feedback.request_title(evaluations.evaluate());
        evaluations.answer(Ok(JSValue::String("Second title".to_owned())));
        assert_eq!(events(&receiver), vec![title("Second title")]);
    }

    #[test]
    fn a_title_notified_before_the_read_was_asked_does_not_drop_its_answer() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();

        feedback.title_notified();
        feedback.request_title(evaluations.evaluate());
        evaluations.answer(Ok(JSValue::String("Same title".to_owned())));

        assert_eq!(events(&receiver), vec![title("Same title")]);
    }

    #[test]
    fn an_empty_document_title_is_sent_so_the_placeholder_shows() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();

        feedback.request_title(evaluations.evaluate());
        evaluations.answer(Ok(JSValue::String(String::new())));

        assert_eq!(events(&receiver), vec![title("")]);
    }

    #[test]
    fn a_title_answer_about_an_earlier_document_or_address_is_dropped() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();

        feedback.request_title(evaluations.evaluate());
        feedback.document_started();
        evaluations.answer(Ok(JSValue::String("Old".to_owned())));
        assert_eq!(
            events(&receiver),
            vec![BrowserEvent::PageIconChanged { icon: None }]
        );

        feedback.request_title(evaluations.evaluate());
        assert!(!feedback.address_changed(LoadStatus::Started));
        evaluations.answer(Ok(JSValue::String("Old".to_owned())));
        assert_eq!(events(&receiver), Vec::new());
    }

    #[test]
    fn a_failed_title_script_sends_nothing() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();

        feedback.request_title(evaluations.evaluate());
        evaluations.answer(Err(JavaScriptEvaluationError::CompilationFailure));
        feedback.request_title(evaluations.evaluate());
        evaluations.answer(Ok(JSValue::Null));

        assert_eq!(events(&receiver), Vec::new());
    }

    #[test]
    fn at_most_one_title_evaluation_is_in_flight() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();

        // A page calling replaceState on every scroll event.
        for _ in 0..50 {
            assert!(feedback.address_changed(LoadStatus::Complete));
            feedback.request_title(evaluations.evaluate());
        }
        assert_eq!(evaluations.count(), 1);
        feedback.refresh_title(evaluations.evaluate());
        assert_eq!(evaluations.count(), 1);

        // The answer is about an earlier address, so it is dropped, and one more read follows.
        evaluations.answer(Ok(JSValue::String("Feed".to_owned())));
        feedback.refresh_title(evaluations.evaluate());
        assert_eq!(evaluations.count(), 1);
        evaluations.answer(Ok(JSValue::String("Feed".to_owned())));
        feedback.refresh_title(evaluations.evaluate());
        assert_eq!(evaluations.count(), 0);

        let titles = events(&receiver)
            .into_iter()
            .filter(|event| matches!(event, BrowserEvent::PageTitleChanged { .. }))
            .collect::<Vec<_>>();
        assert_eq!(titles, vec![title("Feed")]);
    }

    #[test]
    fn a_stuck_evaluation_only_costs_its_document() {
        let (feedback, receiver) = feedback();
        let evaluations = Evaluations::default();

        // The first document's script thread never answers.
        feedback.request_title(evaluations.evaluate());
        feedback.request_title(evaluations.evaluate());
        assert_eq!(evaluations.count(), 1);

        // The next document reads its title at once.
        feedback.document_started();
        feedback.request_title(evaluations.evaluate());
        assert_eq!(evaluations.count(), 2);
        let stuck = evaluations.0.borrow_mut().remove(0);

        // A late answer about the old document neither counts nor ends the new read.
        stuck(Ok(JSValue::String("Old".to_owned())));
        feedback.request_title(evaluations.evaluate());
        assert_eq!(evaluations.count(), 1);

        evaluations.answer(Ok(JSValue::String("New".to_owned())));
        let titles = events(&receiver)
            .into_iter()
            .filter(|event| matches!(event, BrowserEvent::PageTitleChanged { .. }))
            .collect::<Vec<_>>();
        assert_eq!(titles, vec![title("New")]);
        // The read owed from the request made while the new one was in flight follows.
        feedback.refresh_title(evaluations.evaluate());
        assert_eq!(evaluations.count(), 1);
    }

    #[test]
    fn a_same_document_navigation_resends_the_documents_icon() {
        let (feedback, receiver) = feedback();
        feedback.document_started();
        feedback.icon_changed(Some(icon()));
        let _ = events(&receiver);

        assert!(feedback.address_changed(LoadStatus::Complete));

        assert_eq!(
            events(&receiver),
            vec![BrowserEvent::PageIconChanged { icon: Some(icon()) }]
        );
    }

    #[test]
    fn a_new_document_clears_the_tabs_icon_and_forgets_the_previous_one() {
        let (feedback, receiver) = feedback();
        feedback.icon_changed(Some(icon()));

        feedback.document_started();
        assert_eq!(
            events(&receiver),
            vec![BrowserEvent::PageIconChanged { icon: None }]
        );

        assert!(!feedback.address_changed(LoadStatus::Started));
        assert_eq!(events(&receiver), Vec::new());
        assert!(feedback.address_changed(LoadStatus::Complete));
        assert_eq!(
            events(&receiver),
            vec![BrowserEvent::PageIconChanged { icon: None }]
        );
    }

    #[test]
    fn a_completed_load_resends_the_documents_icon() {
        let (feedback, receiver) = feedback();
        feedback.document_started();
        feedback.icon_changed(Some(icon()));
        let _ = events(&receiver);

        feedback.load_completed();

        assert_eq!(
            events(&receiver),
            vec![BrowserEvent::PageIconChanged { icon: Some(icon()) }]
        );
    }

    #[test]
    fn titles_are_read_when_the_head_is_parsed_and_on_load() {
        assert!(!reads_title_at(LoadStatus::Started));
        assert!(reads_title_at(LoadStatus::HeadParsed));
        assert!(reads_title_at(LoadStatus::Complete));
        assert!(!reads_title_at(LoadStatus::Stopped));
        assert!(is_same_document_navigation(LoadStatus::Complete));
        assert!(!is_same_document_navigation(LoadStatus::HeadParsed));
    }
}
