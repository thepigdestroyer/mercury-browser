// SPDX-License-Identifier: MPL-2.0
use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

use crate::address::{AddressResolutionError, NavigationTarget, SearchUrlBuilder, resolve_address};
use crate::browser::LoadState;
use crate::browser::{BrowserCommand, BrowserEvent, BrowserState, placeholder_title};
use crate::page_request::{Answered, decision_to_remember};
use crate::visits::{NavigationOrigin, VisitTracker, VisitUpdate};
// media-capture
use crate::CaptureAccess;
use crate::{
    ColorScheme, ControlAnswer, ControlId, ControlRequest, DEFAULT_ZOOM_PERCENT, RestoredTab, Tab,
    TabActivation, TabCommand, TabId, ZOOM_STEPS,
};
use crate::{
    PageRequestAnswer, PageRequestId, PageRequestReply, PendingPageRequest, PermissionDecisions,
    SessionPermissionDecisions,
};
// site security
use crate::site_security::{PendingUpgrade, same_address};
use crate::{
    ErrorPageKind, HttpsOnlyInterstitial, HttpsPolicy, PagePermission, PermissionDecision,
    SecurityIndicator, security_indicator, typed_https_upgrade,
};

const NO_SEARCH: &str = "no search engine is set";
// openurl
use crate::ExternalLinkRequest;
// groups-core: the group operations and the status flags live in child modules (new files);
// this file keeps only the hook lines.
use crate::GroupBook;
mod group_ops;
mod group_status;

/// What `close_tab` did.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseTabOutcome {
    Closed,
    NotFound,
    /// The tab is the only one and stays open; closing it means quitting.
    LastTab,
}

#[derive(Debug)]
pub struct BrowserController {
    tabs: Vec<Tab>,
    active_tab_id: TabId,
    next_tab_id: u64,
    command_queue: VecDeque<TabCommand>,
    color_scheme: ColorScheme,
    /// downloads: top-level requests and pages offered as downloads, oldest first.
    download_signals: Vec<(TabId, crate::DownloadSignal)>,
    // settings
    /// The system's scheme, as last reported; `color_scheme` is what pages see.
    system_color_scheme: ColorScheme,
    forced_color_scheme: Option<ColorScheme>,
    default_zoom_percent: u16,
    search: Option<SearchUrlBuilder>,
    // dialogs
    permission_decisions: Box<dyn PermissionDecisions>,
    visits: VisitTracker,
    // openurl
    external_link_requests: Vec<(TabId, ExternalLinkRequest)>,
    // site security
    https_policy: HttpsPolicy,
    /// Hosts whose invalid certificate the user accepted this session.
    certificate_exceptions: BTreeSet<String>,
    // groups-core
    /// The window's tab groups, in strip order; see `group_ops`.
    groups: GroupBook,
    /// Counts activations: a tab's `last_shown` is the count when it last became active.
    activation_clock: u64,
}

impl BrowserController {
    pub fn new(state: BrowserState) -> Self {
        let initial_tab_id = TabId::new(1);

        let mut controller = Self {
            tabs: vec![Tab::new(initial_tab_id, state)],
            active_tab_id: initial_tab_id,
            next_tab_id: 2,
            command_queue: VecDeque::new(),
            color_scheme: ColorScheme::default(),
            download_signals: Vec::new(),
            system_color_scheme: ColorScheme::default(),
            forced_color_scheme: None,
            default_zoom_percent: DEFAULT_ZOOM_PERCENT,
            search: None,
            permission_decisions: Box::new(SessionPermissionDecisions::default()),
            visits: VisitTracker::default(),
            external_link_requests: Vec::new(),
            https_policy: HttpsPolicy::default(),
            certificate_exceptions: BTreeSet::new(),
            groups: GroupBook::new(),
            activation_clock: 0,
        };
        // groups-core: the first tab is the one shown
        controller.activate(initial_tab_id);
        controller
    }

    /// Navigates the active tab. If the tab's first page has not committed yet, the engine
    /// cannot navigate it, so the pending page is replaced instead: the URL becomes the tab's
    /// initial URL again and the engine recreates the tab's web view with it.
    pub fn queue_navigation(&mut self, input: &str) -> Result<(), AddressResolutionError> {
        let url = self.resolve_address(input)?.to_string();
        // site security: a typed http:// address is tried over HTTPS first
        let (url, upgrade) = self.typed_upgrade(url);
        let tab_id = self.active_tab_id;
        self.visits
            .set_origin(tab_id, NavigationOrigin::Typed, Some(&url));
        let tab = self
            .tab_mut(tab_id)
            .expect("the controller always owns an active tab");

        if tab.is_engine_ready() {
            tab.state_mut()
                .apply(BrowserEvent::AddressChanged { url: url.clone() });
            self.queue_command(BrowserCommand::Navigate { url });
        } else {
            tab.replace_initial_url(url.clone());
            show_loading(tab, url);
            self.command_queue.retain(|queued| {
                queued.tab_id() != tab_id || !queued.command().targets_the_current_page()
            });
        }
        // site security
        self.start_site_security_navigation(tab_id, upgrade);
        Ok(())
    }

    pub fn state(&self) -> &BrowserState {
        self.active_state()
    }

    pub fn active_state(&self) -> &BrowserState {
        self.active_tab()
            .expect("the controller always owns an active tab")
            .state()
    }

    pub const fn active_tab_id(&self) -> TabId {
        self.active_tab_id
    }

    pub fn tabs(&self) -> impl ExactSizeIterator<Item = &Tab> {
        self.tabs.iter()
    }

    pub fn open_tab(&mut self) -> TabId {
        self.open_blank_tab(TabActivation::Foreground)
    }

    /// Opens a tab. With `Some(address)` the address is resolved like `queue_navigation`
    /// and becomes the tab's initial URL; the tab starts in `LoadState::Loading`.
    pub fn open_tab_with(
        &mut self,
        initial_address: Option<&str>,
        activation: TabActivation,
    ) -> Result<TabId, AddressResolutionError> {
        let initial_url = initial_address
            .map(|address| self.resolve_address(address).map(|url| url.to_string()))
            .transpose()?;
        let tab_id = self.open_blank_tab(activation);

        if let Some(initial_url) = initial_url {
            let tab = self
                .tab_mut(tab_id)
                .expect("the tab was opened by this call");
            set_tab_initial_url(tab, initial_url);
        }

        Ok(tab_id)
    }

    /// Sets the initial URL of a tab whose web view has not been created yet.
    /// Returns Ok(false) if the tab does not exist or its initial URL was already taken.
    pub fn set_initial_address(
        &mut self,
        tab_id: TabId,
        address: &str,
    ) -> Result<bool, AddressResolutionError> {
        let url = self.resolve_address(address)?.to_string();
        Ok(self
            .tab_mut(tab_id)
            .is_some_and(|tab| set_tab_initial_url(tab, url)))
    }

    /// Returns the tab's initial URL exactly once (None afterwards, and None for a blank tab),
    /// unless a navigation before the first commit made a new one pending. When it returns a
    /// URL the tab is no longer engine-ready and shows that URL as loading.
    ///
    /// A restored tab that has not been selected yet keeps its URL (see `is_load_deferred`).
    pub fn take_initial_url(&mut self, tab_id: TabId) -> Option<String> {
        if self.is_load_deferred(tab_id) {
            return None;
        }
        let tab = self.tab_mut(tab_id)?;
        tab.end_load_deferral();
        let url = tab.take_initial_url()?;
        // controls: the new web view has asked for nothing yet.
        tab.state_mut().forget_controls();
        show_loading(tab, url.clone());
        Some(url)
    }

    pub fn select_tab(&mut self, tab_id: TabId) -> bool {
        if self.tabs.iter().any(|tab| tab.id() == tab_id) {
            self.activate(tab_id); // groups-core
            true
        } else {
            false
        }
    }

    pub fn select_tab_at(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(index) else {
            return false;
        };

        let tab_id = tab.id();
        self.activate(tab_id); // groups-core
        true
    }

    pub fn select_next_tab(&mut self, backwards: bool) {
        let active_index = self
            .tabs
            .iter()
            .position(|tab| tab.id() == self.active_tab_id)
            .expect("the active tab is always present");
        let next_index = if backwards {
            active_index.checked_sub(1).unwrap_or(self.tabs.len() - 1)
        } else {
            (active_index + 1) % self.tabs.len()
        };

        let tab_id = self.tabs[next_index].id();
        self.activate(tab_id); // groups-core
    }

    pub fn select_last_tab(&mut self) {
        if let Some(tab_id) = self.tabs.last().map(Tab::id) {
            self.activate(tab_id); // groups-core
        }
    }

    /// Zooms the active tab in by one step; see `zoom_in`.
    pub fn zoom_in(&mut self) {
        self.set_active_zoom(crate::zoom_in(self.active_state().zoom_percent()));
    }

    /// Zooms the active tab out by one step; see `zoom_out`.
    pub fn zoom_out(&mut self) {
        self.set_active_zoom(crate::zoom_out(self.active_state().zoom_percent()));
    }

    pub fn reset_zoom(&mut self) {
        self.set_active_zoom(self.default_zoom_percent);
    }

    /// The colour scheme pages should see; the engine gives it to every new web view.
    pub const fn color_scheme(&self) -> ColorScheme {
        self.color_scheme
    }

    /// Records the scheme and queues it for every tab. Tabs that are not engine-ready hold it
    /// until they are, like other non-pointer commands.
    /// While a scheme is forced (`set_forced_color_scheme`), the system's is only remembered.
    pub fn set_color_scheme(&mut self, scheme: ColorScheme) {
        self.system_color_scheme = scheme;
        if self.forced_color_scheme.is_some() {
            return;
        }
        self.send_color_scheme(scheme);
    }

    fn send_color_scheme(&mut self, scheme: ColorScheme) {
        self.color_scheme = scheme;
        let tab_ids = self.tabs.iter().map(Tab::id).collect::<Vec<_>>();
        for tab_id in tab_ids {
            self.queue_command_for(tab_id, BrowserCommand::SetColorScheme(scheme));
        }
    }

    /// Records the zoom and queues it for the engine. A zoom requested before the tab is
    /// engine-ready is held like other non-pointer commands; the engine also applies the
    /// recorded zoom whenever it creates the tab's web view.
    fn set_active_zoom(&mut self, percent: u16) {
        let tab_id = self.active_tab_id;
        let tab = self
            .tab_mut(tab_id)
            .expect("the controller always owns an active tab");
        if tab.state().zoom_percent() == percent {
            return;
        }
        tab.state_mut().set_zoom_percent(percent);
        self.queue_command(BrowserCommand::SetPageZoom { percent });
    }

    /// Closes a tab. The only tab is never closed; `LastTab` tells the caller to quit instead.
    pub fn close_tab(&mut self, tab_id: TabId) -> CloseTabOutcome {
        let Some(index) = self.tabs.iter().position(|tab| tab.id() == tab_id) else {
            return CloseTabOutcome::NotFound;
        };
        if self.tabs.len() == 1 {
            return CloseTabOutcome::LastTab;
        }

        let was_active = self.active_tab_id == tab_id;
        self.tabs.remove(index);
        self.forget_closed_tab(tab_id);

        if was_active {
            let replacement_index = index.saturating_sub(1).min(self.tabs.len() - 1);
            let replacement = self.tabs[replacement_index].id();
            self.activate(replacement); // groups-core
        }

        CloseTabOutcome::Closed
    }

    /// Drops what the controller keeps for a tab that has just left the strip: its history
    /// state (the last titles are written), the commands queued for it and its requests to open
    /// links in other apps. Every way of closing a tab goes through here.
    fn forget_closed_tab(&mut self, tab_id: TabId) {
        self.visits.forget_tab(tab_id);
        self.command_queue
            .retain(|queued| queued.tab_id() != tab_id);
        // openurl
        self.external_link_requests
            .retain(|(request_tab_id, _)| *request_tab_id != tab_id);
        // groups-core: a group the closed tab was the last of is deleted, its key freed
        self.settle_groups();
    }

    pub fn close_tab_at(&mut self, index: usize) -> CloseTabOutcome {
        let Some(tab_id) = self.tabs.get(index).map(Tab::id) else {
            return CloseTabOutcome::NotFound;
        };

        self.close_tab(tab_id)
    }

    pub fn queue_command(&mut self, command: BrowserCommand) {
        let tab_id = self.active_tab_id;
        let queued = self.queue_command_for(tab_id, command);
        debug_assert!(queued, "the active tab must accept commands");
    }

    pub fn queue_command_for(&mut self, tab_id: TabId, command: BrowserCommand) -> bool {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id() == tab_id) else {
            return false;
        };

        let engine_ready = tab.is_engine_ready();
        if !engine_ready && command.is_discarded_before_engine_ready() {
            return true;
        }
        if let Some(origin) = command.navigation_origin() {
            // A reload goes to the page shown; back and forward go wherever they lead.
            let target = (origin == NavigationOrigin::Reload).then(|| tab.state().url());
            self.visits.set_origin(tab_id, origin, target);
        }
        // dialogs
        self.observe_command_for_page_requests(tab_id, &command);

        // A tab that is not ready (a restored tab may wait a long time) holds one colour
        // scheme, the latest, rather than every change.
        if !engine_ready
            && let BrowserCommand::SetColorScheme(scheme) = command
            && let Some(held) = self.command_queue.iter_mut().find(|queued| {
                queued.tab_id() == tab_id
                    && matches!(queued.command(), BrowserCommand::SetColorScheme(_))
            })
        {
            *held.command_mut() = BrowserCommand::SetColorScheme(scheme);
            return true;
        }

        if let Some(last_command) = self.command_queue.back_mut() {
            if last_command.tab_id() != tab_id {
                self.command_queue
                    .push_back(TabCommand::new(tab_id, command));
                return true;
            }

            match (last_command.command_mut(), &command) {
                (
                    BrowserCommand::PointerMoved { x, y },
                    BrowserCommand::PointerMoved {
                        x: latest_x,
                        y: latest_y,
                    },
                ) => {
                    *x = *latest_x;
                    *y = *latest_y;
                    return true;
                }
                (
                    BrowserCommand::Scrolled {
                        delta_x,
                        delta_y,
                        x,
                        y,
                    },
                    BrowserCommand::Scrolled {
                        delta_x: latest_delta_x,
                        delta_y: latest_delta_y,
                        x: latest_x,
                        y: latest_y,
                    },
                ) => {
                    *delta_x += latest_delta_x;
                    *delta_y += latest_delta_y;
                    *x = *latest_x;
                    *y = *latest_y;
                    return true;
                }
                (
                    BrowserCommand::PinchZoom { delta, x, y },
                    BrowserCommand::PinchZoom {
                        delta: latest_delta,
                        x: latest_x,
                        y: latest_y,
                    },
                ) => {
                    *delta *= latest_delta;
                    *x = *latest_x;
                    *y = *latest_y;
                    return true;
                }
                (
                    BrowserCommand::FocusChanged { focused },
                    BrowserCommand::FocusChanged {
                        focused: latest_focused,
                    },
                ) => {
                    *focused = *latest_focused;
                    return true;
                }
                _ => {}
            }
        }

        self.command_queue
            .push_back(TabCommand::new(tab_id, command));
        true
    }

    /// Returns the oldest queued command whose tab is engine-ready. Commands for tabs whose
    /// browsing context does not exist yet stay queued, in order, until it does.
    pub fn next_command(&mut self) -> Option<TabCommand> {
        let index = self
            .command_queue
            .iter()
            .position(|queued| self.is_dispatchable(queued))?;
        self.command_queue.remove(index)
    }

    /// Reports whether `next_command` would return a command.
    pub fn has_pending_commands(&self) -> bool {
        self.command_queue
            .iter()
            .any(|queued| self.is_dispatchable(queued))
    }

    /// Applies an engine event to its tab. `CloseRequested` changes nothing here: whether and
    /// how the tab closes is the embedder's decision.
    pub fn handle_event(&mut self, tab_id: TabId, event: BrowserEvent) -> bool {
        if self.tabs.iter().any(|tab| tab.id() == tab_id) {
            self.visits.observe(tab_id, &event);
        }
        // downloads
        if self.tabs.iter().any(|tab| tab.id() == tab_id) {
            let signal = match &event {
                BrowserEvent::DownloadOffered {
                    url,
                    mime_type,
                    posted,
                    referrer,
                } => Some(crate::DownloadSignal::Offered(crate::DownloadOffer {
                    url: url.clone(),
                    mime_type: mime_type.clone(),
                    posted: *posted,
                    referrer: referrer.clone(),
                })),
                BrowserEvent::DocumentRequestStarted {
                    url,
                    redirect,
                    from,
                    user_started,
                } => Some(crate::DownloadSignal::RequestStarted {
                    url: url.clone(),
                    redirect: *redirect,
                    from: from.clone(),
                    user_started: *user_started,
                }),
                _ => None,
            };
            if let Some(signal) = signal {
                self.download_signals.push((tab_id, signal));
                return true;
            }
        }
        // dialogs
        self.observe_event_for_page_requests(tab_id, &event);
        // site security
        self.observe_event_for_site_security(tab_id, &event);
        // controls
        let event = match self.route_control_request(tab_id, event) {
            Some(event) => event,
            None => return true,
        };
        // groups-core: what a background tab shows now, to compare after the event is applied
        let watch = self.watch_event(tab_id, &event);
        let Some(tab) = self.tab_mut(tab_id) else {
            return false;
        };
        // openurl: queued, not shown; nothing in the window changes.
        if let BrowserEvent::ExternalLinkRequested(request) = event {
            self.external_link_requests.push((tab_id, request));
            return false;
        }

        if event.shows_engine_ready() {
            tab.mark_engine_ready();
        }
        // media session
        let reapply_mute = tab.state().is_audio_muted()
            && (reaches_a_new_document(&event) || shows_unmuted_media(&event));
        tab.state_mut().apply(event);
        // find: an answer or a selection read can ask for the next search
        let find_follow_up = tab.state_mut().find_mut().take_follow_up();
        self.settle_event(tab_id, watch); // groups-core: unread and crashed flags
        if reapply_mute {
            self.queue_command_for(tab_id, BrowserCommand::SetAudioMuted { muted: true });
        }
        if let Some(command) = find_follow_up {
            self.queue_command_for(tab_id, command);
        }
        true
    }

    /// Names how the navigation just started in the tab was reached, overriding what the
    /// command that started it implied: call it right after `queue_navigation` or
    /// `open_tab_with` (for example `Bookmark`, or `Restored` for a restored tab, whose page is
    /// then not recorded). Returns false if the tab does not exist.
    pub fn set_navigation_origin(&mut self, tab_id: TabId, origin: NavigationOrigin) -> bool {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id() == tab_id) else {
            return false;
        };
        self.visits.rename_origin(tab_id, origin, tab.state().url());
        true
    }

    /// Titles that only the engine's own error pages carry; a visit whose first title is one
    /// of them is taken back.
    pub fn set_error_page_titles(&mut self, titles: impl IntoIterator<Item = String>) {
        self.visits
            .set_error_page_titles(titles.into_iter().collect());
    }

    /// The history updates since the last call, in order. See the `visits` module for the
    /// rules; the caller stamps them with the time and hands them to the history store.
    pub fn take_visit_updates(&mut self) -> Vec<VisitUpdate> {
        self.visits.take_updates()
    }

    /// Opens a tab from a saved session or the closed-tab list at `index` (clamped to the
    /// strip), in the background. It shows `restored`'s address, title and zoom at once, but its
    /// page is not created until the tab is first selected: see `is_load_deferred`. The address
    /// is resolved like `queue_navigation`; if it does not resolve, no tab is opened.
    pub fn open_restored_tab(
        &mut self,
        index: usize,
        restored: &RestoredTab,
    ) -> Result<TabId, AddressResolutionError> {
        let url = restored
            .url
            .as_deref()
            .map(|address| self.resolve_address(address).map(|url| url.to_string()))
            .transpose()?;
        let tab_id = TabId::new(self.next_tab_id);
        self.next_tab_id += 1;

        let mut tab = Tab::new(tab_id, BrowserState::default());
        tab.defer_load();
        if let Some(url) = url {
            // history: the restored page is not a new visit.
            self.visits
                .set_origin(tab_id, NavigationOrigin::Restored, Some(&url));
            tab.set_initial_url(url.clone());
            tab.state_mut().apply(BrowserEvent::AddressChanged { url });
        }
        // After the address, which shows a placeholder title for a new document.
        let state = tab.state_mut();
        state.apply(BrowserEvent::PageTitleChanged {
            title: Some(restored.title.clone()),
        });
        state.set_zoom_percent(
            restored
                .zoom_percent
                .clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1]),
        );
        // tabs-plus: a pinned tab opens inside the pinned group, any other after it
        tab.set_pinned(restored.pinned);
        self.insert_tab(index, tab);
        Ok(tab_id)
    }

    /// Whether the engine must leave this tab without a page for now: it was restored and has
    /// not been the active tab since. Such a tab is not engine-ready, so commands for it are
    /// held (and pointer input dropped) exactly as for any tab whose page does not exist yet.
    pub fn is_load_deferred(&self, tab_id: TabId) -> bool {
        tab_id != self.active_tab_id
            && self
                .tabs
                .iter()
                .any(|tab| tab.id() == tab_id && tab.is_load_deferred())
    }

    /// Whether the tab was restored and its page has not been created yet, selected or not.
    /// While the active tab is such a tab, the engine has work to do at its next pump.
    pub fn is_restored_without_page(&self, tab_id: TabId) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.id() == tab_id && tab.is_load_deferred())
    }

    /// downloads: the top-level requests and the pages offered as downloads since the last
    /// call, oldest first.
    pub fn take_download_signals(&mut self) -> Vec<(TabId, crate::DownloadSignal)> {
        std::mem::take(&mut self.download_signals)
    }

    /// Reserves a tab for a web view the engine is creating itself. The tab is inserted
    /// directly after `opener`, is selected if `activation` says so, has no initial URL, and is
    /// titled after the opener's host until its page reports. Like any other tab it becomes
    /// engine-ready on its first address or history event.
    /// Returns None if `opener` does not exist.
    pub fn adopt_tab(&mut self, opener: TabId, activation: TabActivation) -> Option<TabId> {
        let opener_index = self.tabs.iter().position(|tab| tab.id() == opener)?;
        let title = placeholder_title(self.tabs[opener_index].state().url());
        let tab_id = TabId::new(self.next_tab_id);
        self.next_tab_id += 1;
        let mut state = BrowserState::default();
        state.set_page_title(title);
        let mut tab = Tab::opened_by_page(tab_id, state);
        // groups-core: the new tab joins its opener's group (a pinned opener has none)
        let opener = &self.tabs[opener_index];
        tab.set_group(opener.group().filter(|_| !opener.is_pinned()));
        // tabs-plus: never among the pinned tabs
        self.insert_tab(opener_index + 1, tab);
        if activation == TabActivation::Foreground {
            self.activate(tab_id); // groups-core
        }
        Some(tab_id)
    }

    // controls
    /// Answers the active tab's pending control. Returns false, and sends nothing, unless `id`
    /// is that control: every request is answered at most once.
    pub fn answer_control(&mut self, id: ControlId, answer: ControlAnswer) -> bool {
        let tab_id = self.active_tab_id;
        let Some(tab) = self.tab_mut(tab_id) else {
            return false;
        };
        if tab
            .state()
            .pending_control()
            .map(|(pending_id, _)| pending_id)
            != Some(id)
        {
            return false;
        }
        tab.state_mut().take_pending_control();
        self.queue_command_for(tab_id, BrowserCommand::AnswerControl { id, answer })
    }

    /// Drops the tab's pending control, if any, with the engine's default answer. Called when
    /// the tab stops being shown: pickers are interactive, and a stale one makes no sense.
    pub fn dismiss_control(&mut self, tab_id: TabId) {
        let Some(id) = self
            .tab_mut(tab_id)
            .and_then(|tab| tab.state_mut().take_pending_control())
        else {
            return;
        };
        self.queue_command_for(
            tab_id,
            BrowserCommand::AnswerControl {
                id,
                answer: ControlAnswer::Dismiss,
            },
        );
    }

    /// A control for a background tab, or one asked for while the tab's file chooser is up, is
    /// dropped at once; otherwise one that replaces a pending control drops the older one. Returns the event to apply, or None if it was consumed.
    fn route_control_request(
        &mut self,
        tab_id: TabId,
        event: BrowserEvent,
    ) -> Option<BrowserEvent> {
        let BrowserEvent::ControlShown { id, .. } = &event else {
            return Some(event);
        };
        let id = *id;
        if !self.tabs.iter().any(|tab| tab.id() == tab_id) {
            return Some(event);
        }
        // The file chooser is a sheet on the whole window: while it is up, newer requests of
        // the tab are dropped rather than the chooser.
        let file_chooser_up = self.tabs.iter().any(|tab| {
            tab.id() == tab_id
                && matches!(
                    tab.state().pending_control(),
                    Some((_, ControlRequest::File { .. }))
                )
        });
        if tab_id != self.active_tab_id || file_chooser_up {
            self.queue_command_for(
                tab_id,
                BrowserCommand::AnswerControl {
                    id,
                    answer: ControlAnswer::Dismiss,
                },
            );
            return None;
        }
        self.dismiss_control(tab_id);
        Some(event)
    }

    // media session
    /// Flips the tab's mute choice and tells its page. Returns false if there is no such tab.
    pub fn toggle_audio_muted(&mut self, tab_id: TabId) -> bool {
        let Some(tab) = self.tab_mut(tab_id) else {
            return false;
        };
        let muted = !tab.state().is_audio_muted();
        tab.state_mut().set_audio_muted(muted);
        self.queue_command_for(tab_id, BrowserCommand::SetAudioMuted { muted })
    }

    // openurl
    /// The pages' requests to open links in other apps, oldest first, each returned once.
    pub fn take_external_link_requests(&mut self) -> Vec<(TabId, ExternalLinkRequest)> {
        std::mem::take(&mut self.external_link_requests)
    }

    // find
    /// Opens the active tab's find bar; false if it was already open.
    pub fn open_find(&mut self) -> bool {
        let command = self.active_find_mut().open();
        let opened = command.is_some();
        self.queue_find_command(command);
        opened
    }

    /// Records the text in the active tab's find field; `run_due_find` searches it.
    pub fn set_find_query(&mut self, query: &str) {
        self.active_find_mut().set_query(query);
    }

    /// Searches the active tab's query if it changed since its last search (the end of the
    /// typing debounce).
    pub fn run_due_find(&mut self) {
        let command = self.active_find_mut().due_search();
        self.queue_find_command(command);
    }

    pub fn find_next(&mut self) {
        let command = self.active_find_mut().step(true);
        self.queue_find_command(command);
    }

    pub fn find_previous(&mut self) {
        let command = self.active_find_mut().step(false);
        self.queue_find_command(command);
    }

    pub fn toggle_find_match_case(&mut self) {
        let command = self.active_find_mut().toggle_match_case();
        self.queue_find_command(command);
    }

    /// The page has not answered find request `request_id` in time (a desktop timer decides).
    pub fn find_answer_overdue(&mut self, tab_id: TabId, request_id: u64) {
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.state_mut().find_mut().answer_overdue(request_id);
        }
    }

    pub fn close_find(&mut self) {
        let command = self.active_find_mut().close();
        self.queue_find_command(command);
    }

    fn active_find_mut(&mut self) -> &mut crate::FindState {
        let tab_id = self.active_tab_id;
        self.tab_mut(tab_id)
            .expect("the controller always owns an active tab")
            .state_mut()
            .find_mut()
    }

    fn queue_find_command(&mut self, command: Option<BrowserCommand>) {
        if let Some(command) = command {
            self.queue_command(command);
        }
    }

    fn open_blank_tab(&mut self, activation: TabActivation) -> TabId {
        let tab_id = TabId::new(self.next_tab_id);
        self.next_tab_id += 1;
        let mut state = BrowserState::default();
        state.set_zoom_percent(self.default_zoom_percent);
        self.insert_tab(self.tabs.len(), Tab::new(tab_id, state)); // tabs-plus
        if activation == TabActivation::Foreground {
            self.activate(tab_id); // groups-core
        }
        tab_id
    }

    // settings

    /// The URL address-bar input stands for, with searches built by this controller's search
    /// builder (keywords included), without navigating anywhere. Until a builder is set, text
    /// that is not an address is refused rather than searched for anywhere.
    pub fn resolve_address(&self, input: &str) -> Result<url::Url, AddressResolutionError> {
        match &self.search {
            Some(search) => resolve_address(input, search),
            None => resolve_address(input, &SearchUrlBuilder::new(|_| Err(NO_SEARCH.to_owned()))),
        }
    }

    // history

    /// The results page for searching `text` as it is, even if it looks like an address (the
    /// suggestion list's search row), built by this controller's search builder. Refused, like
    /// any search, until a builder is set.
    pub fn search_url(&self, text: &str) -> Result<url::Url, AddressResolutionError> {
        let query = NavigationTarget::SearchQuery(text.trim().to_owned());
        match &self.search {
            Some(search) => query.into_url(search),
            None => query.into_url(&SearchUrlBuilder::new(|_| Err(NO_SEARCH.to_owned()))),
        }
    }

    // settings

    /// Searches typed in the address bar are built by `search` from now on.
    pub fn set_search_url_builder(&mut self, search: SearchUrlBuilder) {
        self.search = Some(search);
    }

    /// The zoom of a tab that has no zoom of its own.
    pub const fn default_zoom_percent(&self) -> u16 {
        self.default_zoom_percent
    }

    /// New tabs open at `percent`, and every tab still at the old default moves to it.
    pub fn set_default_zoom_percent(&mut self, percent: u16) {
        let old = std::mem::replace(&mut self.default_zoom_percent, percent);
        if old == percent {
            return;
        }
        let at_old_default = self
            .tabs
            .iter()
            .filter(|tab| tab.state().zoom_percent() == old)
            .map(Tab::id)
            .collect::<Vec<_>>();
        for tab_id in at_old_default {
            if let Some(tab) = self.tab_mut(tab_id) {
                tab.state_mut().set_zoom_percent(percent);
            }
            self.queue_command_for(tab_id, BrowserCommand::SetPageZoom { percent });
        }
    }

    /// Makes pages see `scheme` whatever the system's is, or follow the system again with
    /// `None`. Tabs are told only when what they see changes.
    pub fn set_forced_color_scheme(&mut self, scheme: Option<ColorScheme>) {
        self.forced_color_scheme = scheme;
        let effective = scheme.unwrap_or(self.system_color_scheme);
        if effective != self.color_scheme {
            self.send_color_scheme(effective);
        }
    }

    fn is_engine_ready(&self, tab_id: TabId) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.id() == tab_id && tab.is_engine_ready())
    }

    fn tab_mut(&mut self, tab_id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id() == tab_id)
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id() == self.active_tab_id)
    }

    fn is_dispatchable(&self, queued: &TabCommand) -> bool {
        self.is_engine_ready(queued.tab_id())
            || queued.command().is_dispatched_before_engine_ready()
    }
}

// dialogs
impl BrowserController {
    /// The request the tab shows now: the oldest one it has not answered.
    pub fn pending_page_request(&self, tab_id: TabId) -> Option<PendingPageRequest> {
        self.tabs
            .iter()
            .find(|tab| tab.id() == tab_id)?
            .page_requests()
            .front()
    }

    /// Answers one of the tab's requests and queues the answer for the engine. `now` is on the
    /// clock of `PageRequest::received_at`. Returns false if the tab has no such request (it was
    /// answered or discarded already).
    pub fn answer_page_request(
        &mut self,
        tab_id: TabId,
        id: PageRequestId,
        reply: PageRequestReply,
        now: Duration,
    ) -> bool {
        let Some(tab) = self.tab_mut(tab_id) else {
            return false;
        };
        let Some(Answered {
            answered,
            discarded,
            paired,
        }) = tab.page_requests_mut().answer(id, &reply, now)
        else {
            return false;
        };
        let remembered = std::iter::once(&answered)
            .chain(paired.as_ref())
            .filter_map(|pending| decision_to_remember(pending, &reply))
            .collect::<Vec<_>>();
        self.queue_page_request_answers(tab_id, &discarded, PageRequestAnswer::Discard);
        let answer = reply.into_answer();
        self.queue_command_for(
            tab_id,
            BrowserCommand::AnswerPageRequest {
                id,
                answer: answer.clone(),
            },
        );
        // media-capture: a camera and a microphone request shown together get one answer.
        if let Some(paired) = &paired {
            self.queue_page_request_answers(tab_id, &[paired.id()], answer);
        }

        for (origin, permission, decision) in remembered {
            self.permission_decisions
                .remember(&origin, permission, decision);
            let decided = self
                .tabs
                .iter_mut()
                .map(|tab| {
                    let ids = tab
                        .page_requests_mut()
                        .take_decided(&origin, permission, decision);
                    (tab.id(), ids)
                })
                .collect::<Vec<_>>();
            for (tab_id, ids) in decided {
                self.queue_page_request_answers(tab_id, &ids, decision.answer());
            }
        }
        true
    }

    /// Replaces where remembered permission decisions are kept (by default, for the session).
    pub fn set_permission_decisions(&mut self, decisions: Box<dyn PermissionDecisions>) {
        self.permission_decisions = decisions;
    }

    fn observe_event_for_page_requests(&mut self, tab_id: TabId, event: &BrowserEvent) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id() == tab_id) else {
            return;
        };
        let awaiting_replacement = tab.is_awaiting_replacement();
        let requests = tab.page_requests_mut();
        match event {
            // Sent by the web view that is being replaced: it never reaches the new page.
            BrowserEvent::PageRequested(request) if awaiting_replacement => {
                self.queue_page_request_answers(tab_id, &[request.id], PageRequestAnswer::Discard);
            }
            BrowserEvent::PageRequested(request) => {
                let id = request.id;
                if let Some(answer) =
                    requests.receive(request.clone(), self.permission_decisions.as_ref())
                {
                    // media-capture: the other kind of a call a remembered Block refused.
                    let refused = requests.take_refused_partners();
                    self.queue_page_request_answers(tab_id, &[id], answer);
                    self.queue_page_request_answers(tab_id, &refused, PageRequestAnswer::Decline);
                }
            }
            BrowserEvent::PageLoadStarted => {
                let discarded = requests.start_new_document();
                self.queue_page_request_answers(tab_id, &discarded, PageRequestAnswer::Discard);
            }
            // media-capture: a crashed page has no devices any more. A `Failed` load state alone
            // does not end access: the address bar reports addresses it cannot use that way,
            // while the page stays and may still be capturing.
            BrowserEvent::PageCrashed => requests.end_capture(),
            BrowserEvent::LoadStateChanged {
                state: LoadState::Failed { .. },
            } => {
                let discarded = requests.discard_all();
                self.queue_page_request_answers(tab_id, &discarded, PageRequestAnswer::Discard);
            }
            _ => {}
        }
    }

    fn observe_command_for_page_requests(&mut self, tab_id: TabId, command: &BrowserCommand) {
        let Some(tab) = self.tab_mut(tab_id) else {
            return;
        };
        match command {
            // The user is leaving the page: its questions no longer matter. Answering them first
            // also unblocks the page's script, which the navigation may need.
            command if command.targets_the_current_page() => {
                let reload = matches!(command, BrowserCommand::Reload);
                let discarded = tab.page_requests_mut().note_user_navigation(reload);
                self.queue_page_request_answers(tab_id, &discarded, PageRequestAnswer::Discard);
            }
            BrowserCommand::PointerButton {
                action: crate::PointerButtonAction::Down,
                ..
            } => tab.page_requests_mut().note_user_activation(),
            BrowserCommand::KeyboardInput(input) if input.state == crate::KeyState::Down => {
                tab.page_requests_mut().note_user_activation();
            }
            _ => {}
        }
    }

    fn queue_page_request_answers(
        &mut self,
        tab_id: TabId,
        ids: &[PageRequestId],
        answer: PageRequestAnswer,
    ) {
        for &id in ids {
            self.queue_command_for(
                tab_id,
                BrowserCommand::AnswerPageRequest {
                    id,
                    answer: answer.clone(),
                },
            );
        }
    }
}

// site security
impl BrowserController {
    /// What the address bar's security indicator shows for the tab.
    pub fn security_indicator(&self, tab_id: TabId) -> SecurityIndicator {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id() == tab_id) else {
            return SecurityIndicator::None;
        };
        let url = tab.state().url();
        let error_page = tab
            .site_security()
            .error_page
            .as_ref()
            .filter(|(_, error_url)| same_address(error_url, url))
            .map(|(kind, _)| *kind);
        let host = url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
        let exception = host.is_some_and(|host| self.certificate_exceptions.contains(&host));
        security_indicator(url, error_page, exception)
    }

    /// The Mercury error page the tab shows, if any.
    pub fn error_page(&self, tab_id: TabId) -> Option<ErrorPageKind> {
        let tab = self.tabs.iter().find(|tab| tab.id() == tab_id)?;
        let (kind, url) = tab.site_security().error_page.as_ref()?;
        same_address(url, tab.state().url()).then_some(*kind)
    }

    /// The HTTPS-only interstitial the tab shows, if any.
    pub fn https_only_interstitial(&self, tab_id: TabId) -> Option<&HttpsOnlyInterstitial> {
        self.tabs
            .iter()
            .find(|tab| tab.id() == tab_id)?
            .site_security()
            .interstitial
            .as_ref()
    }

    pub const fn https_policy(&self) -> &HttpsPolicy {
        &self.https_policy
    }

    /// HTTPS-only mode on or off. Turning it off removes every interstitial.
    pub fn set_https_only(&mut self, on: bool) {
        self.https_policy.https_only = on;
        if !on {
            for tab in &mut self.tabs {
                tab.site_security_mut().interstitial = None;
            }
        }
    }

    /// See `HttpsPolicy::upgrade_local_names`.
    pub fn set_upgrade_local_names(&mut self, on: bool) {
        self.https_policy.upgrade_local_names = on;
    }

    /// Lets HTTPS-only mode load the host over HTTP again (or not). Returns whether it changed.
    pub fn set_https_exception(&mut self, host: &str, allowed: bool) -> bool {
        if allowed {
            self.https_policy.add_exception(host)
        } else {
            self.https_policy.remove_exception(host)
        }
    }

    /// The user accepted an invalid certificate for `host` (ASCII form).
    pub fn add_certificate_exception(&mut self, host: &str) {
        self.certificate_exceptions
            .insert(host.to_ascii_lowercase());
    }

    pub fn has_certificate_exception(&self, host: &str) -> bool {
        self.certificate_exceptions
            .contains(&host.to_ascii_lowercase())
    }

    /// The interstitial's "Continue to the site": the host may load over HTTP for the rest of
    /// the session, and the tab loads the `http` address. Returns the host, or None if the tab
    /// shows no interstitial.
    pub fn continue_to_http_site(&mut self, tab_id: TabId) -> Option<String> {
        let interstitial = self
            .tab_mut(tab_id)?
            .site_security_mut()
            .interstitial
            .take()?;
        let host = interstitial.host();
        self.https_policy.add_exception(&host);
        self.queue_command_for(
            tab_id,
            BrowserCommand::Navigate {
                url: interstitial.http_url,
            },
        );
        Some(host)
    }

    /// The interstitial's "Go back": the previous page, or a new-tab page if there is none.
    pub fn leave_https_only_interstitial(&mut self, tab_id: TabId) -> bool {
        let Some(tab) = self.tab_mut(tab_id) else {
            return false;
        };
        if tab.site_security_mut().interstitial.take().is_none() {
            return false;
        }
        let command = if tab.state().can_go_back() {
            BrowserCommand::GoBack
        } else {
            BrowserCommand::Navigate {
                url: "about:blank".to_owned(),
            }
        };
        self.queue_command_for(tab_id, command)
    }

    /// The permissions remembered for `origin` (a serialised origin).
    pub fn site_permissions(&self, origin: &str) -> Vec<(PagePermission, PermissionDecision)> {
        self.permission_decisions.decisions_for(origin)
    }

    /// Forgets a remembered permission: the site asks again next time.
    pub fn forget_site_permission(&mut self, origin: &str, permission: PagePermission) {
        self.permission_decisions.forget(origin, permission);
    }

    /// The URL to navigate to for typed input, and the `http` URL it replaces if it is an
    /// HTTPS upgrade.
    fn typed_upgrade(&self, url: String) -> (String, Option<PendingUpgrade>) {
        let Some(upgraded) = url::Url::parse(&url)
            .ok()
            .and_then(|parsed| typed_https_upgrade(&self.https_policy, &parsed))
        else {
            return (url, None);
        };
        let https = upgraded.to_string();
        let upgrade = PendingUpgrade {
            https: https.clone(),
            http: url,
            strict: self.https_policy.https_only,
        };
        (https, Some(upgrade))
    }

    fn start_site_security_navigation(&mut self, tab_id: TabId, upgrade: Option<PendingUpgrade>) {
        if let Some(tab) = self.tab_mut(tab_id) {
            let security = tab.site_security_mut();
            security.interstitial = None;
            security.pending_upgrade = upgrade;
        }
    }

    fn observe_event_for_site_security(&mut self, tab_id: TabId, event: &BrowserEvent) {
        let Some(tab) = self.tab_mut(tab_id) else {
            return;
        };
        let security = tab.site_security_mut();
        match event {
            BrowserEvent::PageLoadStarted => security.error_page = None,
            BrowserEvent::AddressChanged { url } => {
                let involved = |address: &str| same_address(address, url);
                let stays = security
                    .pending_upgrade
                    .as_ref()
                    .is_some_and(|upgrade| involved(&upgrade.https) || involved(&upgrade.http))
                    || security.interstitial.as_ref().is_some_and(|interstitial| {
                        involved(&interstitial.http_url)
                            || interstitial
                                .failed_https_url
                                .as_deref()
                                .is_some_and(involved)
                    });
                if !stays {
                    security.pending_upgrade = None;
                    security.interstitial = None;
                }
            }
            BrowserEvent::HttpsUpgraded { from, to } => {
                security.interstitial = None;
                security.pending_upgrade = Some(PendingUpgrade {
                    https: to.clone(),
                    http: from.clone(),
                    strict: true,
                });
                // The engine cancelled the `http` load; the tab loads the HTTPS address. A tab
                // whose first page never committed gets a new web view for it.
                if tab.is_engine_ready() {
                    self.queue_command_for(tab_id, BrowserCommand::Navigate { url: to.clone() });
                } else {
                    tab.replace_initial_url(to.clone());
                    show_loading(tab, to.clone());
                }
            }
            BrowserEvent::HttpsOnlyBlocked { url } => {
                security.pending_upgrade = None;
                security.interstitial = Some(HttpsOnlyInterstitial {
                    http_url: url.clone(),
                    failed_https_url: None,
                });
            }
            BrowserEvent::ErrorPageShown { kind, url } => {
                security.error_page = Some((*kind, url.clone()));
                let failed_upgrade = security
                    .pending_upgrade
                    .take_if(|upgrade| same_address(&upgrade.https, url));
                match failed_upgrade {
                    Some(upgrade) if upgrade.strict => {
                        security.interstitial = Some(HttpsOnlyInterstitial {
                            http_url: upgrade.http,
                            failed_https_url: Some(upgrade.https),
                        });
                    }
                    Some(upgrade) => {
                        self.queue_command_for(
                            tab_id,
                            BrowserCommand::Navigate { url: upgrade.http },
                        );
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
}

// media session
/// Mute is applied by script, so each new document of a muted tab needs it again: when it
/// commits, when it has loaded, and whenever media starts playing (also catching elements the
/// page created or un-muted since).
fn reaches_a_new_document(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::AddressChanged { .. }
            | BrowserEvent::PageLoadStarted
            | BrowserEvent::LoadStateChanged {
                state: LoadState::Loaded
            }
            | BrowserEvent::MediaPlaybackChanged {
                state: crate::MediaPlaybackState::Playing
            }
    )
}

/// The page un-muted media of a muted tab (or added media the mute has not reached yet).
fn shows_unmuted_media(event: &BrowserEvent) -> bool {
    matches!(event, BrowserEvent::MediaProbed { probe } if probe.audible > 0)
}

fn set_tab_initial_url(tab: &mut Tab, url: String) -> bool {
    if !tab.set_initial_url(url.clone()) {
        return false;
    }

    show_loading(tab, url);
    true
}

fn show_loading(tab: &mut Tab, url: String) {
    let state = tab.state_mut();
    state.apply(BrowserEvent::AddressChanged { url });
    state.apply(BrowserEvent::LoadStateChanged {
        state: LoadState::Loading,
    });
}

impl Default for BrowserController {
    fn default() -> Self {
        Self::new(BrowserState::default())
    }
}

// media-capture
impl BrowserController {
    /// Which capture devices the tab's current page has been allowed to use (for the indicator).
    pub fn capture_access(&self, tab_id: TabId) -> CaptureAccess {
        self.tabs
            .iter()
            .find(|tab| tab.id() == tab_id)
            .map(|tab| tab.page_requests().capture_access())
            .unwrap_or_default()
    }

    /// The site (as shown to the user) that was allowed to capture in the tab, if any.
    pub fn capture_origin(&self, tab_id: TabId) -> Option<String> {
        self.tabs
            .iter()
            .find(|tab| tab.id() == tab_id)?
            .page_requests()
            .capture_origin()
            .map(|origin| origin.label().to_owned())
    }

    /// Whether the tab's page kept itself from being reloaded after Stop (the user chose Stay
    /// on its "Reload this page?" prompt): it still has its camera and microphone.
    pub fn capture_stop_refused(&self, tab_id: TabId) -> bool {
        self.tabs
            .iter()
            .find(|tab| tab.id() == tab_id)
            .is_some_and(|tab| tab.page_requests().capture_stop_refused())
    }

    /// Stop: the site's remembered camera and microphone decisions are forgotten and the page is
    /// reloaded; the engine releases the devices when the old document is unloaded, and the
    /// reloaded page has to ask again. The marks stay until the new document starts: the page
    /// can keep itself from being reloaded (its leave prompt, answered Stay), and then still has
    /// the devices. Returns false (and does nothing) when the tab's page has no access.
    pub fn stop_capture(&mut self, tab_id: TabId) -> bool {
        let Some(tab) = self.tab_mut(tab_id) else {
            return false;
        };
        let requests = tab.page_requests_mut();
        if requests.capture_access().is_empty() {
            return false;
        }
        let origin_key = requests
            .capture_origin()
            .and_then(|origin| origin.key().map(str::to_owned));
        requests.note_capture_stop();
        if let Some(origin_key) = origin_key {
            for permission in [PagePermission::Camera, PagePermission::Microphone] {
                self.permission_decisions.forget(&origin_key, permission);
            }
        }
        self.queue_command_for(tab_id, BrowserCommand::Reload);
        true
    }
}

// tabs-plus
/// What the close-tab shortcut did to a pinned tab, which it never closes.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinnedCloseOutcome {
    /// The tab is not pinned (or does not exist): close it as usual.
    NotPinned,
    /// The first unpinned tab is selected instead.
    SelectedUnpinned,
    /// Every tab is pinned, so a new tab was opened and selected.
    OpenedTab,
}

// tabs-plus
/// Pinned tabs come first in the strip, always; every method here keeps that true.
impl BrowserController {
    /// How many tabs are pinned; they are the first this many in `tabs()`.
    pub fn pinned_tab_count(&self) -> usize {
        let count = self.tabs.iter().take_while(|tab| tab.is_pinned()).count();
        debug_assert!(
            self.tabs[count..].iter().all(|tab| !tab.is_pinned()),
            "a pinned tab follows an unpinned one"
        );
        count
    }

    /// The one way a new tab enters the strip: `index` is clamped into the tab's group, so a
    /// pinned tab lands among the pinned tabs and any other after them.
    fn insert_tab(&mut self, index: usize, mut tab: Tab) {
        let pinned_count = self.pinned_tab_count();
        let index = if tab.is_pinned() {
            index.min(pinned_count)
        } else {
            let index = index.clamp(pinned_count, self.tabs.len());
            // groups-core: a grouped tab stays in its run, a loose one is snapped out of any (R1)
            self.place_among_groups(index, &mut tab)
        };
        self.tabs.insert(index, tab);
    }

    /// Pins a tab, moving it to the end of the pinned tabs, or unpins it, moving it to the
    /// start of the unpinned tabs. Returns false if nothing changed.
    pub fn set_tab_pinned(&mut self, tab_id: TabId, pinned: bool) -> bool {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id() == tab_id) else {
            return false;
        };
        if tab.is_pinned() == pinned {
            return false;
        }
        let index = if pinned { self.pinned_tab_count() } else { 0 };
        self.move_tab(tab_id, index, pinned)
    }

    /// Moves a tab so that it ends up at `index` in the strip, pinned or not. The index is
    /// clamped into the tab's group: a pinned tab lands among the pinned tabs, an unpinned one
    /// after them. Dragging a tab into or out of the pinned group pins or unpins it.
    /// The selection does not change. Returns false if nothing changed.
    pub fn move_tab(&mut self, tab_id: TabId, index: usize, pinned: bool) -> bool {
        let Some(from) = self.tabs.iter().position(|tab| tab.id() == tab_id) else {
            return false;
        };
        // groups-core: an unpinned tab in a group stays in its run
        let run = if pinned { None } else { self.run_around(from) };
        let mut tab = self.tabs.remove(from);
        let was_pinned = tab.is_pinned();
        tab.set_pinned(pinned);
        if pinned {
            // groups-core: a pinned tab is in no group; one left without tabs is deleted
            tab.set_group(None);
            self.drop_empty_groups();
        }
        let pinned_count = self.pinned_tab_count();
        let index = if pinned {
            index.min(pinned_count)
        } else if let Some((start, end)) = run {
            index.clamp(start, end) // groups-core: inside the run it came from
        } else {
            // groups-core: a loose tab is snapped out of any run (R1)
            self.snap_out_of_run(index.clamp(pinned_count, self.tabs.len()))
        };
        self.tabs.insert(index, tab);
        index != from || was_pinned != pinned
    }

    /// Moves a tab one place left (`-1`) or right (`1`) within its group, for the keyboard.
    /// Returns false at the edge of the group.
    pub fn move_tab_by(&mut self, tab_id: TabId, delta: isize) -> bool {
        let Some(from) = self.tabs.iter().position(|tab| tab.id() == tab_id) else {
            return false;
        };
        let pinned = self.tabs[from].is_pinned();
        let pinned_count = self.pinned_tab_count();
        let group = if pinned {
            0..pinned_count
        } else {
            pinned_count..self.tabs.len()
        };
        // groups-core: a grouped tab moves within its run only
        let group = self
            .run_around(from)
            .map_or(group, |(start, end)| start..end + 1);
        match from.checked_add_signed(delta) {
            // groups-core: a loose tab steps over a whole run, never into it
            Some(to) if group.contains(&to) => {
                let to = self.past_adjacent_run(from, to);
                self.move_tab(tab_id, to, pinned)
            }
            _ => false,
        }
    }

    /// Closes every unpinned tab except `keep`, and selects `keep`. Pinned tabs stay.
    /// Returns how many tabs were closed (none if `keep` does not exist); when none close, the
    /// selection stays as it was.
    pub fn close_other_tabs(&mut self, keep: TabId) -> usize {
        if !self.tabs.iter().any(|tab| tab.id() == keep) {
            return 0;
        }
        let closing = self
            .tabs
            .iter()
            .filter(|tab| tab.id() != keep && !tab.is_pinned())
            .map(Tab::id)
            .collect::<Vec<_>>();
        if closing.is_empty() {
            return 0;
        }
        self.tabs.retain(|tab| !closing.contains(&tab.id()));
        for tab_id in closing.iter().copied() {
            self.forget_closed_tab(tab_id);
        }
        self.activate(keep); // groups-core
        closing.len()
    }

    /// The close-tab shortcut never closes a pinned tab: it selects the first unpinned tab
    /// instead, or opens a new tab when every tab is pinned. Closing a pinned tab takes its
    /// own command (the tab's menu).
    pub fn step_off_pinned_tab(&mut self, tab_id: TabId) -> PinnedCloseOutcome {
        if !self
            .tabs
            .iter()
            .any(|tab| tab.id() == tab_id && tab.is_pinned())
        {
            return PinnedCloseOutcome::NotPinned;
        }
        match self.tabs.iter().find(|tab| !tab.is_pinned()).map(Tab::id) {
            Some(unpinned) => {
                self.activate(unpinned); // groups-core
                PinnedCloseOutcome::SelectedUnpinned
            }
            None => {
                self.open_tab();
                PinnedCloseOutcome::OpenedTab
            }
        }
    }

    /// The tabs whose title or address contains every word of `query`, in strip order; see
    /// `tab_matches_query`.
    pub fn matching_tabs(&self, query: &str) -> Vec<TabId> {
        self.tabs
            .iter()
            .filter(|tab| tab_matches_query(tab.state().page_title(), tab.state().url(), query))
            .map(Tab::id)
            .collect()
    }
}

/// Whether every whitespace-separated word of `query` occurs in the title or the address,
/// ignoring case. An empty query matches everything.
pub fn tab_matches_query(title: &str, url: &str, query: &str) -> bool {
    let title = title.to_lowercase();
    let url = url.to_lowercase();
    query.split_whitespace().all(|word| {
        let word = word.to_lowercase();
        title.contains(&word) || url.contains(&word)
    })
}
