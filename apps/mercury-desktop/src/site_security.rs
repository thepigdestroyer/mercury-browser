// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Site security in the window: the indicator before the address, the site-information
//! popover, the HTTPS-only interstitial, and keeping the engine's HTTPS policy and the
//! controller's certificate exceptions in step.

use std::cell::RefCell;
use std::rc::Rc;

use mercury_ui::{MainWindow, SiteInfo, SitePermissionRow, SiteSecurityIcon};
use slint::{ComponentHandle, ModelRc, VecModel};
use talaria_core::{
    BrowserController, PagePermission, PermissionDecision, SecurityIndicator, display_origin,
};
use talaria_servo::{ServoRuntime, SiteDataSummary};
use url::Url;

use crate::ui_sync::sync_ui;

/// For tests and diagnostics: upgrade local names and addresses too (they run on loopback).
const UPGRADE_LOCAL_NAMES_VARIABLE: &str = "MERCURY_HTTPS_UPGRADE_LOCAL_NAMES";

/// Called on every engine pump: the engine follows the controller's HTTPS policy, and the
/// controller learns of certificates the user accepted. Returns whether the UI must update.
pub(crate) fn sync_engine(runtime: &ServoRuntime, controller: &RefCell<BrowserController>) -> bool {
    let accepted = runtime.take_certificate_exceptions();
    let mut controller = controller.borrow_mut();
    for host in &accepted {
        controller.add_certificate_exception(host);
    }
    runtime.set_https_policy(controller.https_policy());
    !accepted.is_empty()
}

/// The window's site-security properties for the active tab.
pub(crate) fn sync_window(window: &MainWindow, controller: &BrowserController) {
    let tab_id = controller.active_tab_id();
    let indicator = controller.security_indicator(tab_id);
    window.set_security_icon(icon(indicator));
    window.set_security_label(indicator.label().into());
    let interstitial = controller.https_only_interstitial(tab_id);
    window.set_https_only_interstitial_shown(interstitial.is_some());
    if let Some(interstitial) = interstitial {
        window.set_https_only_interstitial_host(
            mercury_site_security::display_host(&interstitial.host()).into(),
        );
        window.set_https_only_interstitial_tried_https(interstitial.failed_https_url.is_some());
    }
}

/// The address as the address bar shows it (see `mercury_site_security::display_url`).
pub(crate) fn display_address(url: &str) -> String {
    mercury_site_security::display_url(url)
}

pub(crate) fn connect_site_security_callbacks(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
) {
    if std::env::var_os(UPGRADE_LOCAL_NAMES_VARIABLE).is_some_and(|value| value == "1") {
        controller.borrow_mut().set_upgrade_local_names(true);
    }

    let info_controller = Rc::clone(&controller);
    let info_window = window.as_weak();
    window.on_site_info_requested(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(window) = info_window.upgrade() {
            refresh_site_info(&window, &info_controller.borrow());
        }
    });

    let reset_controller = Rc::clone(&controller);
    let reset_window = window.as_weak();
    window.on_site_permission_reset(move |index| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let Some(window) = reset_window.upgrade() else {
            return;
        };
        let url = reset_controller.borrow().active_state().url().to_owned();
        if let Some(origin) = display_origin(&url) {
            let permissions = reset_controller.borrow().site_permissions(&origin);
            if let Some((permission, _)) = usize::try_from(index)
                .ok()
                .and_then(|index| permissions.get(index))
            {
                reset_controller
                    .borrow_mut()
                    .forget_site_permission(&origin, *permission);
            }
        }
        refresh_site_info(&window, &reset_controller.borrow());
    });

    let clear_controller = Rc::clone(&controller);
    let clear_window = window.as_weak();
    window.on_site_data_clear(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let Some(window) = clear_window.upgrade() else {
            return;
        };
        if let Ok(url) = Url::parse(clear_controller.borrow().active_state().url()) {
            crate::engine::with_runtime(|runtime| {
                runtime.clear_data_for_site(&url);
            });
        }
        refresh_site_info(&window, &clear_controller.borrow());
    });

    let exception_controller = Rc::clone(&controller);
    let exception_window = window.as_weak();
    window.on_site_https_exception_remove(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let Some(window) = exception_window.upgrade() else {
            return;
        };
        if let Some(host) = active_host(&exception_controller.borrow()) {
            exception_controller
                .borrow_mut()
                .set_https_exception(&host, false);
        }
        refresh_site_info(&window, &exception_controller.borrow());
    });

    // sh-pages: the popover's "Images" row turns images off or on for the active site.
    let images_controller = Rc::clone(&controller);
    let images_window = window.as_weak();
    window.on_site_images_toggle(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let Some(window) = images_window.upgrade() else {
            return;
        };
        let blocked = crate::site_images::active_site_blocks_images(&images_controller.borrow());
        crate::site_images::set_active_site_images(&window, &images_controller, !blocked);
    });

    let back_controller = Rc::clone(&controller);
    let back_window = window.as_weak();
    window.on_https_only_go_back(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let tab_id = back_controller.borrow().active_tab_id();
        back_controller
            .borrow_mut()
            .leave_https_only_interstitial(tab_id);
        if let Some(window) = back_window.upgrade() {
            sync_ui(&window, &back_controller.borrow());
            window.window().request_redraw();
        }
    });

    let continue_controller = Rc::clone(&controller);
    let continue_window = window.as_weak();
    window.on_https_only_continue(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let tab_id = continue_controller.borrow().active_tab_id();
        let host = continue_controller
            .borrow_mut()
            .continue_to_http_site(tab_id);
        if host.is_some() {
            // The exception must reach the engine before the navigation does.
            crate::engine::with_runtime(|runtime| {
                runtime.set_https_policy(continue_controller.borrow().https_policy());
            });
        }
        if let Some(window) = continue_window.upgrade() {
            sync_ui(&window, &continue_controller.borrow());
            window.window().request_redraw();
        }
    });
}

fn active_host(controller: &BrowserController) -> Option<String> {
    Url::parse(controller.active_state().url())
        .ok()?
        .host_str()
        .map(str::to_owned)
}

/// Fills the popover for the active tab.
fn refresh_site_info(window: &MainWindow, controller: &BrowserController) {
    let url = controller.active_state().url().to_owned();
    let mut summary = None;
    if let Ok(parsed) = Url::parse(&url) {
        crate::engine::with_runtime(|runtime| {
            summary = Some(runtime.site_data_summary(&parsed));
        });
    }
    window.set_site_info(site_info(controller, &url, summary));
    crate::site_images::sync_popover(window, controller); // sh-pages: the "Images" row
}

fn site_info(
    controller: &BrowserController,
    url: &str,
    site_data: Option<SiteDataSummary>,
) -> SiteInfo {
    let indicator = controller.security_indicator(controller.active_tab_id());
    let host = active_host(controller);
    let permissions = display_origin(url)
        .map(|origin| controller.site_permissions(&origin))
        .unwrap_or_default()
        .into_iter()
        .map(|(permission, decision)| permission_row(permission, decision))
        .collect::<Vec<_>>();
    let shown_host = host
        .as_deref()
        .map(mercury_site_security::display_host)
        .unwrap_or_else(|| display_address(url));
    let has_site_data = site_data.is_some_and(|summary| !summary.is_empty());
    SiteInfo {
        host: shown_host.into(),
        icon: icon(indicator),
        heading: indicator.label().into(),
        detail: connection_detail(indicator).into(),
        permissions: ModelRc::new(VecModel::from(permissions)),
        site_data: site_data_text(site_data).into(),
        can_clear_site_data: has_site_data,
        https_exception: host
            .as_deref()
            .is_some_and(|host| controller.https_policy().has_exception(host)),
        certificate_exception: indicator == SecurityIndicator::CertificateException,
    }
}

fn permission_row(permission: PagePermission, decision: PermissionDecision) -> SitePermissionRow {
    let description = permission.description();
    let mut name = description.to_owned();
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    SitePermissionRow {
        name: name.into(),
        state: match decision {
            PermissionDecision::Allow => "Allowed",
            PermissionDecision::Block => "Blocked",
        }
        .into(),
    }
}

pub(crate) fn icon(indicator: SecurityIndicator) -> SiteSecurityIcon {
    match indicator {
        SecurityIndicator::None => SiteSecurityIcon::None,
        SecurityIndicator::Secure => SiteSecurityIcon::Padlock,
        SecurityIndicator::CertificateException
        | SecurityIndicator::CertificateError
        | SecurityIndicator::NotSecure => SiteSecurityIcon::Warning,
        SecurityIndicator::LocalHost | SecurityIndicator::File | SecurityIndicator::Internal => {
            SiteSecurityIcon::Info
        }
    }
}

pub(crate) const fn connection_detail(indicator: SecurityIndicator) -> &'static str {
    match indicator {
        SecurityIndicator::None => "",
        SecurityIndicator::Secure => {
            "Information you send to this site cannot be read or changed by others on the network."
        }
        SecurityIndicator::CertificateException | SecurityIndicator::CertificateError => {
            "Mercury Browser could not confirm who runs this site. Others on the network may be able to read or change what you send to it."
        }
        SecurityIndicator::NotSecure => {
            "This site does not use HTTPS. Others on the network can read or change what you send to it, such as passwords."
        }
        SecurityIndicator::LocalHost => {
            "This page comes from a server on this computer. It does not use HTTPS."
        }
        SecurityIndicator::File => "This page is a file on this computer.",
        SecurityIndicator::Internal => "This page is part of Mercury Browser.",
    }
}

pub(crate) fn site_data_text(summary: Option<SiteDataSummary>) -> String {
    let Some(summary) = summary else {
        return "None".to_owned();
    };
    let mut parts = Vec::new();
    match summary.cookies {
        0 => {}
        1 => parts.push("1 cookie".to_owned()),
        count => parts.push(format!("{count} cookies")),
    }
    if summary.local_storage {
        parts.push("local storage".to_owned());
    }
    if summary.session_storage {
        parts.push("session storage".to_owned());
    }
    if parts.is_empty() {
        "None".to_owned()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicators_map_to_icons() {
        assert_eq!(icon(SecurityIndicator::None), SiteSecurityIcon::None);
        assert_eq!(icon(SecurityIndicator::Secure), SiteSecurityIcon::Padlock);
        assert_eq!(
            icon(SecurityIndicator::NotSecure),
            SiteSecurityIcon::Warning
        );
        assert_eq!(
            icon(SecurityIndicator::CertificateException),
            SiteSecurityIcon::Warning
        );
        assert_eq!(icon(SecurityIndicator::File), SiteSecurityIcon::Info);
        assert_eq!(connection_detail(SecurityIndicator::None), "");
    }

    #[test]
    fn site_data_is_summed_up_in_words() {
        assert_eq!(site_data_text(None), "None");
        assert_eq!(site_data_text(Some(SiteDataSummary::default())), "None");
        assert_eq!(
            site_data_text(Some(SiteDataSummary {
                cookies: 1,
                local_storage: false,
                session_storage: true,
            })),
            "1 cookie, session storage"
        );
        assert_eq!(
            site_data_text(Some(SiteDataSummary {
                cookies: 3,
                local_storage: true,
                session_storage: false,
            })),
            "3 cookies, local storage"
        );
    }

    #[test]
    fn permission_rows_read_as_sentences() {
        let row = permission_row(PagePermission::PersistentStorage, PermissionDecision::Block);
        assert_eq!(row.name, "Store data permanently on this device");
        assert_eq!(row.state, "Blocked");
    }

    #[test]
    fn the_address_bar_shows_safe_international_names() {
        assert_eq!(
            display_address("https://xn--mnchen-3ya.de/"),
            "https://münchen.de/"
        );
        assert_eq!(
            display_address("https://xn--80ak6aa92e.com/"),
            "https://xn--80ak6aa92e.com/"
        );
    }
}
