---
title: Privacy and Security
description: Learn what Mercury blocks, what it stores, and why the current preview is not sandboxed.
---

# Privacy and security

Mercury has useful privacy foundations, but it does not yet have the process isolation expected from a production browser. Both facts matter.

## Content blocking

Mercury compiles a snapshot of Brave's default network filter lists into the browser. Requests matching known advertising and tracking rules are cancelled before their resources load.

Top-level page navigations are not blocked. Mercury does not yet have an interstitial that could explain a blocked main document, so cancelling one would produce an unexplained blank tab.

There are currently no per-site controls, exception lists, cosmetic filtering controls, or visible blocked-request counter.

## Local data

The current preview does not implement persistent sessions, history, bookmarks, settings, extensions, or private profiles. Do not interpret the absence of those features as a complete privacy mode: Servo and native media libraries may still use normal process and system resources during a session.

Pages are cached on disk as well as in memory: for the default profile in `~/Library/Caches/io.github.thepigdestroyer.mercury-browser/Default/http-cache` (macOS), for a `--profile-dir` profile in its `http-cache` folder. In Settings, **When Mercury Browser quits, clear > Cached files** removes it when Mercury quits (or at the next start, if Mercury did not quit normally).

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

## Sandboxing status

!!! danger "Do not use Mercury for sensitive browsing"
    Web content runs in separate Servo content processes, but those child processes currently have no operating-system sandbox restrictions. Process isolation improves fault containment, but it is not the hardened security boundary expected in a mature browser.

Mercury enables Servo 0.7.0 multiprocess mode: navigation, visible rendering, input, audio, and video cross the process boundary. The separate Linux `gaol` sandbox mode is still unreliable, so Mercury keeps `sandbox` disabled until it can enforce restrictions without breaking content processes.

Use Mercury with local test pages, known public sites, and non-sensitive data. Do not enter passwords, payment information, private documents, or account recovery details.

## Reporting a security issue

Avoid publishing exploit details in a general bug report. Contact the maintainer through the repository profile first so the issue can be handled privately and routed upstream when it belongs to Servo or a dependency.
