// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Protocol handlers: web apps registered for `mailto:`, `web+...` and the other schemes.
//!
//! A handler's URL template is checked on the way in: it must be `https` (or `http` to a
//! loopback host), it must hold a `%s` for the protocol URL, and it must carry no user info.
//! Only the schemes on the HTML Standard's safelist, and `web+` schemes, can be registered.

use std::collections::BTreeMap;

use mercury_store::{ProtocolHandlerRow, SiteSettingChange};
use url::Url;

use crate::error::Error;
use crate::origin::Origin;
use crate::settings::SiteSettings;
use crate::{MAX_HANDLERS, UnixMillis};

/// The longest URL template a handler may hold, in bytes.
pub(crate) const MAX_TEMPLATE_BYTES: usize = 2048;
/// The longest title a handler may hold, in characters.
pub(crate) const MAX_TITLE_CHARS: usize = 128;

/// A web app registered to open one protocol scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolHandler {
    /// The protocol scheme, for example `"mailto"`.
    pub scheme: String,
    /// The origin of the web app.
    pub origin: Origin,
    /// The URL to open, with `%s` where the protocol URL goes.
    pub url_template: String,
    /// The name shown to the user.
    pub title: String,
    /// When it was registered, in milliseconds since the Unix epoch.
    pub added_at: UnixMillis,
    /// When it last handled a link.
    pub last_used_at: Option<UnixMillis>,
    /// Whether it is the handler opened for its scheme.
    pub is_default: bool,
}

/// The stored handlers, keyed by scheme and origin.
#[derive(Clone, Debug, Default)]
pub(crate) struct HandlerTable {
    handlers: BTreeMap<(String, Origin), ProtocolHandler>,
}

impl HandlerTable {
    /// The table for `rows`, and how many rows were ignored because they no longer validate.
    pub(crate) fn from_rows(rows: Vec<ProtocolHandlerRow>) -> (Self, usize) {
        let mut table = Self::default();
        let mut ignored = 0;
        for row in rows {
            let Some(handler) = handler_from_row(row) else {
                ignored += 1;
                continue;
            };
            table
                .handlers
                .insert((handler.scheme.clone(), handler.origin.clone()), handler);
        }
        (table, ignored)
    }

    /// The rows for every handler, in key order.
    pub(crate) fn to_rows(&self) -> Vec<ProtocolHandlerRow> {
        self.handlers.values().map(row_from_handler).collect()
    }

    fn len(&self) -> usize {
        self.handlers.len()
    }
}

impl SiteSettings {
    /// Registers `origin`'s web app for `scheme`, or updates its template and title when it is
    /// already registered (keeping its `added_at` and default flag). The first handler for a
    /// scheme becomes its default.
    pub fn register_handler(
        &mut self,
        scheme: &str,
        origin: &Origin,
        url_template: &str,
        title: &str,
        now: UnixMillis,
    ) -> Result<(), Error> {
        if !is_registrable_scheme(scheme) {
            return Err(Error::SchemeNotAllowed);
        }
        if !template_is_valid(url_template, origin) {
            return Err(Error::TemplateNotAllowed);
        }
        if title.chars().count() > MAX_TITLE_CHARS {
            return Err(Error::TemplateNotAllowed);
        }
        let key = (scheme.to_owned(), origin.clone());
        let existing = self.handlers.handlers.get(&key).cloned();
        if existing.is_none() && self.handlers.len() >= MAX_HANDLERS {
            return Err(Error::TooManyHandlers);
        }
        let has_default = self
            .handlers
            .handlers
            .values()
            .any(|h| h.scheme == scheme && h.is_default);
        let handler = match existing {
            Some(mut kept) => {
                kept.url_template = url_template.to_owned();
                kept.title = title.to_owned();
                kept
            }
            None => ProtocolHandler {
                scheme: scheme.to_owned(),
                origin: origin.clone(),
                url_template: url_template.to_owned(),
                title: title.to_owned(),
                added_at: now,
                last_used_at: None,
                is_default: !has_default,
            },
        };
        self.journal
            .push(SiteSettingChange::PutHandler(row_from_handler(&handler)));
        self.handlers.handlers.insert(key, handler);
        Ok(())
    }

    /// The handlers for `scheme`: the default first, then the rest by `added_at`.
    pub fn handlers_for(&self, scheme: &str) -> Vec<ProtocolHandler> {
        let mut found: Vec<ProtocolHandler> = self
            .handlers
            .handlers
            .values()
            .filter(|h| h.scheme == scheme)
            .cloned()
            .collect();
        found.sort_by_key(|h| (!h.is_default, h.added_at));
        found
    }

    /// The default handler for `scheme`, if the user has one.
    pub fn default_handler(&self, scheme: &str) -> Option<ProtocolHandler> {
        self.handlers
            .handlers
            .values()
            .find(|h| h.scheme == scheme && h.is_default)
            .cloned()
    }

    /// Makes `origin`'s handler the default for `scheme`. Returns `false` when it is not
    /// registered. Journals one `PutHandler` per row whose default flag changed.
    pub fn set_default_handler(&mut self, scheme: &str, origin: &Origin) -> bool {
        let key = (scheme.to_owned(), origin.clone());
        if !self.handlers.handlers.contains_key(&key) {
            return false;
        }
        for (row_key, handler) in self.handlers.handlers.iter_mut() {
            if row_key.0 != scheme {
                continue;
            }
            let wanted = row_key == &key;
            if handler.is_default != wanted {
                handler.is_default = wanted;
                self.journal
                    .push(SiteSettingChange::PutHandler(row_from_handler(handler)));
            }
        }
        true
    }

    /// Removes `origin`'s handler for `scheme`. Removing the default leaves no default: the
    /// user picks the next one. Returns `false` when it was not registered.
    pub fn remove_handler(&mut self, scheme: &str, origin: &Origin) -> bool {
        let key = (scheme.to_owned(), origin.clone());
        if self.handlers.handlers.remove(&key).is_none() {
            return false;
        }
        self.journal.push(SiteSettingChange::RemoveHandler {
            scheme: scheme.to_owned(),
            origin: origin.to_string(),
        });
        true
    }

    /// Records that `origin`'s handler for `scheme` handled a link at `now`. Journals the row.
    pub fn record_handler_use(&mut self, scheme: &str, origin: &Origin, now: UnixMillis) {
        let key = (scheme.to_owned(), origin.clone());
        let Some(handler) = self.handlers.handlers.get_mut(&key) else {
            return;
        };
        handler.last_used_at = Some(now);
        self.journal
            .push(SiteSettingChange::PutHandler(row_from_handler(handler)));
    }
}

/// Whether `scheme` may be registered as a protocol handler: a scheme on the HTML Standard's
/// safelist, or a `web+` scheme of lower-case ASCII letters.
pub fn is_registrable_scheme(scheme: &str) -> bool {
    const SAFELIST: &[&str] = &[
        "bitcoin",
        "ftp",
        "ftps",
        "geo",
        "im",
        "irc",
        "ircs",
        "magnet",
        "mailto",
        "matrix",
        "mms",
        "news",
        "nntp",
        "openpgp4fpr",
        "sftp",
        "sip",
        "sms",
        "smsto",
        "ssh",
        "tel",
        "urn",
        "webcal",
        "wtai",
        "xmpp",
    ];
    if SAFELIST.contains(&scheme) {
        return true;
    }
    scheme
        .strip_prefix("web+")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase()))
}

/// Whether `template` is a valid URL template for `origin`'s app.
fn template_is_valid(template: &str, origin: &Origin) -> bool {
    if template.len() > MAX_TEMPLATE_BYTES || !template.contains("%s") {
        return false;
    }
    let Ok(url) = Url::parse(&template.replace("%s", "x")) else {
        return false;
    };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    match url.scheme() {
        "https" => host == origin.host() || loopback,
        "http" => loopback,
        _ => false,
    }
}

fn handler_from_row(row: ProtocolHandlerRow) -> Option<ProtocolHandler> {
    if !is_registrable_scheme(&row.scheme) {
        return None;
    }
    let origin = Origin::parse(&row.origin).ok()?;
    if !template_is_valid(&row.url_template, &origin) {
        return None;
    }
    if row.title.chars().count() > MAX_TITLE_CHARS {
        return None;
    }
    Some(ProtocolHandler {
        scheme: row.scheme,
        origin,
        url_template: row.url_template,
        title: row.title,
        added_at: row.added_at,
        last_used_at: row.last_used_at,
        is_default: row.is_default,
    })
}

fn row_from_handler(handler: &ProtocolHandler) -> ProtocolHandlerRow {
    ProtocolHandlerRow {
        scheme: handler.scheme.clone(),
        origin: handler.origin.to_string(),
        url_template: handler.url_template.clone(),
        title: handler.title.clone(),
        added_at: handler.added_at,
        last_used_at: handler.last_used_at,
        is_default: handler.is_default,
    }
}
