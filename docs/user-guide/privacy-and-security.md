---
title: Privacy and Security
description: What Mercury Browser blocks, what it asks, what it stores and clears, which outside services it contacts, and why it is not yet safe for sensitive browsing.
---

# Privacy and security

Mercury blocks ads and trackers by default, contacts no outside service of its own, and asks before pages get extra permissions. It is not yet a hardened browser: its content processes have no operating-system sandbox. Both facts matter.

## Do not use Mercury for sensitive browsing yet

!!! danger "Content processes are not sandboxed"
    Each page runs in a separate Servo content process, but the operating system does not restrict what those processes can do. A page that exploits a bug in the engine could act with your user account's permissions. Until that changes, don't use Mercury for passwords, payments, banking, private documents or account recovery.

Servo's option to confine content processes (`sandbox`) is off on every platform. Turning it on, and testing that navigation, rendering, input, media and quitting still work with it, is open work.

The app is also signed only ad hoc, not with an Apple Developer ID, and is not notarised by Apple, so macOS warns the first time you open it and there are no automatic updates. See [Install Mercury](installation.md).

## Content blocking

Mercury blocks ads and trackers at the network level. Its blocker is built into the app from snapshots of these filter lists:

- EasyList and EasyPrivacy;
- uBlock Origin's filters;
- Brave's first-party, site-specific and unbreak lists.

A request from a page that matches a rule is cancelled before anything is fetched. The page itself (the document in the address bar) is never blocked, because Mercury has no page yet that could explain why a blocked page is empty.

Turn blocking off and on in **Settings → Privacy and security → Block ads and trackers**. The change applies to everything loaded afterwards; reload a page to apply it there.

Not available yet: per-site exceptions, hiding page elements (cosmetic filtering), a count of blocked requests, and updating the lists without a new version of Mercury. Filter lists that update in the background are being worked on.

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

Mercury itself sends nothing to any server: no crash reports, no usage statistics, no update checks, no search suggestions. The only requests are the ones you cause: the pages you open, and your search engine when you search from the address bar. **Help** opens this documentation site in a tab.

The settings for search suggestions and HTTPS-only browsing are greyed out because those features aren't there yet. Phishing protection, password breach checks, translation and update checks are planned; each will say which service it uses and can be turned off.

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
