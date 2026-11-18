---
title: Privacy and Security
description: Learn what Mercury blocks, what it stores, and why the current preview is not sandboxed.
---

# Privacy and security

Mercury has useful privacy foundations, but it does not yet have the process isolation expected from a production browser. Both facts matter.

## Content blocking

Mercury blocks requests that match the network rules of five filter lists: EasyList and EasyPrivacy (by the EasyList authors, https://easylist.to/, used under CC BY-SA 3.0) and three Brave lists (MPL 2.0). Lists available only under the GPL, such as the uBlock Origin filters, are not included. Matching requests are cancelled before their resources load. Element-hiding (cosmetic) rules are not used.

The lists ship with Mercury as separate files, not inside the program. At start-up Mercury prepares them in the background; until that is done (usually a fraction of a second) nothing is blocked, so the very first page of a session may load a few trackers.

**Updates.** With **Settings → Privacy and security → Update filter lists automatically** on (the default), Mercury asks each list's publisher for a newer version about a minute after start-up and then at most once a day: easylist.to for EasyList and EasyPrivacy, GitHub for the Brave lists. Only the lists are downloaded, nothing about you is sent beyond an ordinary web request, and an unchanged list costs no download. A download that is damaged, looks like a web page, or has fewer than half the rules of the list in use is ignored, so a failed update never turns blocking off. **Update now** checks at once; the section shows when the lists were last updated and each list's version. Updated lists are kept in your profile.

Top-level page navigations are not blocked. Mercury does not yet have an interstitial that could explain a blocked main document, so cancelling one would produce an unexplained blank tab.

There are currently no per-site controls, exception lists, cosmetic filtering controls, or visible blocked-request counter.

## Local data

The current preview does not implement persistent sessions, history, bookmarks, settings, extensions, or private profiles. Do not interpret the absence of those features as a complete privacy mode: Servo and native media libraries may still use normal process and system resources during a session.

Pages are cached on disk as well as in memory: for the default profile in `~/Library/Caches/io.github.thepigdestroyer.mercury-browser/Default/http-cache` (macOS), for a `--profile-dir` profile in its `http-cache` folder. In Settings, **When Mercury Browser quits, clear > Cached files** removes it when Mercury quits (or at the next start, if Mercury did not quit normally).

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

## Sandboxing status

!!! danger "Do not use Mercury for sensitive browsing"
    Web content runs in separate Servo content processes, but those child processes currently have no operating-system sandbox restrictions. Process isolation improves fault containment, but it is not the hardened security boundary expected in a mature browser.

Mercury enables Servo 0.7.0 multiprocess mode: navigation, visible rendering, input, audio, and video cross the process boundary. The separate Linux `gaol` sandbox mode is still unreliable, so Mercury keeps `sandbox` disabled until it can enforce restrictions without breaking content processes.

Use Mercury with local test pages, known public sites, and non-sensitive data. Do not enter passwords, payment information, private documents, or account recovery details.

## Reporting a security issue

Avoid publishing exploit details in a general bug report. Contact the maintainer through the repository profile first so the issue can be handled privately and routed upstream when it belongs to Servo or a dependency.
