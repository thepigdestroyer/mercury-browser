---
title: Privacy and Security
description: What Mercury Browser blocks, what it asks, what it stores and clears, which outside services it contacts, and why it is not yet safe for sensitive browsing.
---

# Privacy and security

Mercury blocks ads and trackers by default, contacts no outside service except the publishers of its filter lists (to update them, which you can turn off), and asks before pages get extra permissions. It is not yet a hardened browser: its content processes have no operating-system sandbox. Both facts matter.

## Do not use Mercury for sensitive browsing yet

!!! danger "Content processes are not sandboxed"
    Each page runs in a separate Servo content process, but the operating system does not restrict what those processes can do. A page that exploits a bug in the engine could act with your user account's permissions. Until that changes, don't use Mercury for passwords, payments, banking, private documents or account recovery.

Servo's option to confine content processes (`sandbox`) is off on every platform. Turning it on, and testing that navigation, rendering, input, media and quitting still work with it, is open work.

The app is also signed only ad hoc, not with an Apple Developer ID, and is not notarised by Apple, so macOS warns the first time you open it and there are no automatic updates. See [Install Mercury](installation.md).

## Content blocking

Mercury blocks ads and trackers at the network level, using the network rules of five filter lists:

- EasyList and EasyPrivacy (by the EasyList authors, https://easylist.to/, used under CC BY-SA 3.0);
- Brave's first-party, site-specific and unbreak lists (MPL 2.0).

Lists available only under the GPL, such as the uBlock Origin filters, are not included. Element-hiding (cosmetic) rules are not used.

A request from a page that matches a rule is cancelled before anything is fetched. The page itself (the document in the address bar) is never blocked, because Mercury has no page yet that could explain why a blocked page is empty.

The lists ship with Mercury as separate files, not inside the program. At start-up Mercury prepares them in the background; until that is done (usually a fraction of a second) nothing is blocked, so the very first page of a session may load a few trackers.

**Updates.** With **Settings → Privacy and security → Update filter lists automatically** on (the default), Mercury asks each list's publisher for a newer version about a minute after start-up and then at most once a day: easylist.to for EasyList and EasyPrivacy, GitHub for the Brave lists. If a check cannot reach a publisher (no network, a time-out or a server error), Mercury tries again sooner: after 1 hour, then after 2, 4, 8 and 16 hours, and then daily until a check gets through. Only the lists are downloaded, nothing about you is sent beyond an ordinary web request, and an unchanged list costs no download (Mercury sends back the list's `ETag` and date, which a publisher could in principle use to recognise repeat requests from the same profile). A download that is damaged, looks like a web page, or has fewer than half the rules of the list in use or of the copy that shipped with Mercury is ignored, so a failed update never turns blocking off. **Update now** checks at once; the section shows when the lists were last updated and each list's version. Updated lists are kept in your profile.

Turn blocking off and on in **Settings → Privacy and security → Block ads and trackers**. The change applies to everything loaded afterwards; reload a page to apply it there.

Not available yet: per-site exceptions, hiding page elements (cosmetic filtering) and a count of blocked requests.

## Pop-ups

A page may open a new tab or window only in response to a click or key press in that page. Otherwise Mercury blocks it and shows **Pop-up blocked** in the toolbar with a button to allow pop-ups from that site until Mercury quits. Turn blocking off in **Settings → Privacy and security → Block pop-up windows**.

The same rule applies to the colour picker and the file panel: a page cannot open them without a click or key press.

## Permissions

When a page asks to **store data permanently on this device**, a prompt appears under the address bar with **Block** and **Allow**. It never takes the keyboard from you; use the mouse, or ++tab++ to reach the buttons. The answer applies to that one request.

Every other permission a page can ask for is refused without a prompt, because Mercury doesn't support the feature behind it yet: location, notifications and push messages, camera, microphone, speakers and media device lists, MIDI, Bluetooth, game controllers, background sync and keeping the screen on. Camera and microphone support and location are being worked on. Location stays off until a Servo patch lets Mercury refuse requests from other sites' frames: Servo 0.8 doesn't say which frame asked.

## What Mercury stores

| What | Where |
| --- | --- |
| Settings | `~/Library/Application Support/Mercury Browser/Default/settings.json` |
| Updated filter lists (only after an update has downloaded a newer version) | the `filter-lists` folder in the same profile |
| Cookies and site data (local and session storage, IndexedDB, Cache Storage) | the `servo` folder in the same profile |
| The HTTP cache | `~/Library/Caches/io.github.thepigdestroyer.mercury-browser/Default/http-cache` |
| Copies of media while it plays | `~/Library/Caches/io.github.thepigdestroyer.mercury-browser/media-buffers/`, removed when Mercury quits ([details](media.md#how-the-bundled-gstreamer-is-kept-separate)) |

Cookies without an expiry date (session cookies) are removed when Mercury quits. Runs with `--screenshot` or `--background` use a temporary profile that is removed when they end, unless `--profile-dir` names another one.

Mercury doesn't keep a history of visited pages, bookmarks or saved passwords in this build. Those features are being merged.

### Clearing data when Mercury quits

In **Settings → Privacy and security → When Mercury Browser quits, clear** you can choose:

- **Cookies and site data**;
- **Cached files** (the HTTP cache).

They are cleared when Mercury quits, or at the next start if Mercury did not quit normally. **Browsing history** is in the list too but does nothing until the history feature is merged, and the **Clear browsing data…** button is greyed out for the same reason.

## Outside services

Mercury sends nothing to any server of its own: no crash reports, no usage statistics, no update checks for the app, no search suggestions. The requests are the ones you cause (the pages you open, and your search engine when you search from the address bar) and the filter-list updates: with **Update filter lists automatically** on, Mercury asks easylist.to and GitHub (raw.githubusercontent.com) for newer versions of the filter lists, as described under [Content blocking](#content-blocking). Turn that setting off and Mercury contacts them only when you press **Update now**. **Help** opens this documentation site in a tab.

The settings for search suggestions and HTTPS-only browsing are greyed out because those features aren't there yet. Phishing protection, password breach checks, translation and update checks are planned; each will say which service it uses and can be turned off.

## Saved passwords

Mercury keeps saved logins in your login keychain. Open **Settings > Passwords** to search them, open one, copy its user name or password, edit or delete it, generate a new password, or import and export a CSV file (Chrome, Edge, Brave, Firefox and Safari exports are read; Chrome's format is written).

- **Showing, copying or exporting a password** asks for Touch ID or your Mac's login password first. Once you have confirmed, Mercury does not ask again for a minute while the Settings window stays open. Mercury asks macOS to keep a copied password on this Mac rather than send it to your other devices through Universal Clipboard, and to mark it as secret for clipboard managers that respect that. It is removed from the clipboard after a minute or when Mercury quits, unless you have copied something else since.
- **Filling a login into a page:** right-click (or Control-click) the page's password field and choose **Fill login…**, then the login. Mercury fills the password and the user-name field before it, only on the page's own site, and never into a frame embedded from another site. Nothing is filled on its own when a page loads.
- **Sites that ask through Mercury's sign-in sheet** (HTTP authentication): the sheet starts with your saved user name for that site; leave the password empty and press **Sign in** to use the saved password; if it cannot be read, the sheet stays open and asks you to type it. An empty password field always uses the saved password for that user name, so a saved login whose password is genuinely empty cannot be re-entered empty from the sheet; type any password to override. **Save the user name and password in Mercury Browser** saves what you type once the sign-in has worked; if it cannot be saved, Mercury says so under the toolbar and offers **Retry**. Nothing is filled in or saved for a proxy server's sign-in. **Offer to save passwords** and **Never save passwords for** in Settings control this.
- Saving what you type into a page's own sign-in form is not available yet: it needs a later version of the web engine.
- An exported file holds every password in clear text and is readable only by you. Delete it once you no longer need it, and do not open it in a spreadsheet.
- Runs with `--profile-dir`, `--screenshot` or `--background` keep logins only until Mercury quits and never use the keychain.

## Proxy servers

On macOS, Mercury uses the proxies set in **System Settings > Network > (your network) > Details > Proxies**, read when Mercury starts; a change takes effect at the next start. The `http_proxy`, `https_proxy` and `no_proxy` environment variables, if set, are used instead (on Linux they are the only source).

Used:

- **Web proxy (HTTP)** and **Secure web proxy (HTTPS)**, with the list of hosts and domains that bypass them. `localhost` and `127.0.0.1` always bypass the proxy.

Not used:

- **Automatic proxy configuration** (a PAC file) and **Auto proxy discovery** (WPAD). If your network depends on them, set the web proxies by hand.
- **SOCKS proxy.** With only a SOCKS proxy set, Mercury connects **directly**, not through it.
- **Exclude simple hostnames**, and bypass entries with a wildcard in the middle (such as `www.*.example`).
- **Proxies that ask for a password.** Mercury cannot prompt for one; the page fails with a connection error. A user name and password in `http_proxy` (`http://user:password@host:port`) do work.

Mercury sends plain `http://` addresses through the proxy with `CONNECT` as well, so a proxy that allows `CONNECT` only to port 443 refuses them. Mercury writes a warning to its log at start when the system asks for something it does not use.

## Reporting a security problem

Don't put exploit details in a public issue. Contact the maintainer through the GitHub profile linked from the repository first, so the problem can be handled privately and passed on to Servo or a dependency when it belongs there.
