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

On macOS, a page that asks to use your **camera** or **microphone** gets a prompt too, with **Block** and **Allow**; see [Camera and microphone](camera-and-microphone.md). On other systems those requests are refused.

Every other permission a page can ask for is refused without a prompt, because Mercury doesn't support the feature behind it yet: location, notifications and push messages, speakers and media device lists, MIDI, Bluetooth, game controllers, background sync and keeping the screen on. Location stays off until a Servo patch lets Mercury refuse requests from other sites' frames: Servo 0.8 doesn't say which frame asked. Pages get at the clipboard only through your own Cut, Copy and Paste: `navigator.clipboard` is off (see [Clipboard](#clipboard)).

## Site security

The icon before the address says how the page was loaded:

- **A padlock:** an HTTPS page whose certificate Mercury could verify.
- **A warning mark:** the connection is not safe. The heading of **site information** says which case it is. **Not secure** is plain HTTP to any site that is not on this computer, including a device on your network at a private address such as `192.168.1.2` or a name such as `printer.local` (`data:` pages are marked the same way). **Not secure: the certificate is not valid** is the certificate-error page itself, which shows the mark before you have chosen anything. **Not secure: you allowed an invalid certificate** is an HTTPS page you opened by continuing past that error.
- **A circled "i":** neither a padlock nor a warning. It marks **Page on this computer** (plain HTTP to `localhost` or another loopback address such as `127.0.0.1`), **File on this computer**, and **Mercury Browser page** (one of Mercury's own error pages).
- **Nothing:** a new tab.

Click the icon for **site information**: the permissions the site has been given (each with **Reset**), the cookies and site data it holds (with **Clear**), and any exception you made for the site.

### HTTPS-only mode

With **Settings → Privacy and security → Load pages over HTTPS only** on, Mercury opens the HTTPS address instead of the HTTP address of a page you open or follow a link to, except for your own computer and the local network (the first item below). If the site has no HTTPS version, Mercury shows **The secure version of this site is not available** with **Go back** and **Continue to the site (not secure)**; nothing else is loaded over HTTP until you choose to. Some things are outside it:

- Your own computer and the local network. Their addresses load over HTTP as usual, because devices at home seldom have a certificate a browser accepts, and the mode does not protect them: `localhost` and other loopback addresses, private and link-local addresses such as `192.168.1.1`, and names of one word (`router`) or ending in `.local`, `.localhost`, `.internal`, `.lan` or `.home.arpa` (`printer.local`). The address bar still marks such a page **Not secure**, or **Page on this computer** for `localhost` and loopback addresses.
- Only the page itself is upgraded. Images, scripts and frames a page loads over HTTP are loaded as the page asks.
- A page whose HTTPS address keeps redirecting to an HTTP address, so that no load ever finishes, is upgraded up to five times and then stopped, and Mercury shows the same **The secure version of this site is not available** page. A page that finishes loading and then sends you back to its HTTP address with a script or a meta refresh is upgraded again each time, for as long as it keeps doing that; you can leave it at any time.
- If you choose **Continue** after Mercury blocked a form that was being sent, the page is opened again with a plain request: the form's data is not sent a second time.
- A page that a link opened in a new tab may lose its connection to the page that opened it when it is upgraded.

Apart from that setting, and whether or not it is on, Mercury opens over HTTPS the sites that asked browsers in advance to be opened only that way. The list of them (the HSTS preload list) comes from the Chromium project and is updated before each Mercury release. It applies to an `http://` address you type or follow, to a redirect to one, and to the images, scripts and frames a page loads from such a site. If the certificate of such a site is not valid, Mercury does not let you continue past the error (see [Certificate errors](#certificate-errors)).

### Certificate errors

When a site's certificate is not valid, Mercury shows an error page instead of the site. Under **Details**, a **Continue anyway** button is offered only for your own computer and the local network (`localhost`, private addresses, and names such as `printer.local`), and only for a page of its own, not a frame inside another page. For every other site there is no way past the error. Mercury also refuses it for a site that has told browsers to use only secure connections (HSTS), whatever its name.

What **Continue anyway** accepts is narrow:

- **One site and one certificate.** Mercury accepts the certificate the error was about, for the name in the address (`printer.local`), and for nothing else. If the site later presents a different certificate under that name, Mercury shows the error page again and asks again. The same certificate under another name is not accepted: the same device reached by its address instead of its name (`192.168.1.20` for `printer.local`), another site on the same network, or a name above or below the one you accepted (`www.printer.local` for `printer.local`). Capital letters and a final dot do not make a different name.
- **The name, not the port.** An exception is for a site's name. Another port on the same name is accepted too, if it presents the same certificate.
- **Until Mercury quits.** Mercury does not save an exception. There is no **Remove** button yet: to forget an exception, quit Mercury. A button would not be reliable until Mercury can also close the connections that are already open, because they stay trusted after an exception is removed.
- **The warning stays.** While an exception lasts, the address bar keeps its warning mark for that name, and the site information says that you allowed an invalid certificate.
- **No HSTS from that name.** Many devices with their own certificate (routers, network storage) ask browsers to use only secure connections for their name. Mercury does not remember such a request from a name you continued on, for as long as Mercury runs, because that connection was trusted only on your say-so (RFC 6797, section 8.1). It also ignores the name's request on a later connection with a valid certificate in the same session, and learns it in the next one. So the name is not locked, and after a restart you can continue again. Other sites are not affected.

Continue only for devices you control. An exception means Mercury trusts whoever holds that certificate's key when you visit that name.

## What Mercury stores

| What | Where |
| --- | --- |
| Settings | `~/Library/Application Support/Mercury Browser/Default/settings.json` |
| History, bookmarks and the downloads list | `places.sqlite` in the same profile |
| Open and recently closed tabs (addresses and titles), for restoring them | `session.json` in the same profile, with `session.previous.json` and `session.recovery.json` |
| Updated filter lists (only after an update has downloaded a newer version) | the `filter-lists` folder in the same profile |
| Cookies and site data (local and session storage, IndexedDB, Cache Storage) | the `servo` folder in the same profile |
| The HTTP cache | `~/Library/Caches/io.github.thepigdestroyer.mercury-browser/Default/http-cache` |
| Copies of media while it plays | `~/Library/Caches/io.github.thepigdestroyer.mercury-browser/media-buffers/`, removed when Mercury quits ([details](media.md#how-the-bundled-gstreamer-is-kept-separate)) |

Cookies without an expiry date (session cookies) are removed when Mercury quits. Runs with `--screenshot` or `--background` use a temporary profile that is removed when they end, unless `--profile-dir` names another one.

Saved logins are not in the profile: they are in your login keychain (see [Saved passwords](#saved-passwords)).

### Clearing data when Mercury quits

In **Settings → Privacy and security → When Mercury Browser quits, clear** you can choose:

- **Browsing and download history**;
- **Cookies and site data**;
- **Cached files** (the HTTP cache).

They are cleared when Mercury quits, or at the next start if Mercury did not quit normally (for history, only if the quit had begun). The **Clear browsing data…** button (also in the **History** menu) clears now: choose a time range and what to clear. Cookies, site data and cached files are always cleared for all time, because the engine cannot clear them by date. Bookmarks, downloaded files and the saved tabs are kept.

## Outside services

Mercury sends nothing to any server of its own: no crash reports, no usage statistics, no update checks for the app, no search suggestions. The requests are the ones you cause (the pages you open, and your search engine when you search from the address bar) and the filter-list updates: with **Update filter lists automatically** on, Mercury asks easylist.to and GitHub (raw.githubusercontent.com) for newer versions of the filter lists, as described under [Content blocking](#content-blocking). Turn that setting off and Mercury contacts them only when you press **Update now**. **Help** opens this documentation site in a tab.

The setting for search suggestions is greyed out because that feature isn't there yet. Phishing protection, password breach checks, translation and update checks are planned; each will say which service it uses and can be turned off.

## Saved passwords

Mercury keeps saved logins in your login keychain. Open **Settings > Passwords** to search them, open one, copy its user name or password, edit or delete it, generate a new password, or import and export a CSV file (Chrome, Edge, Brave, Firefox and Safari exports are read; Chrome's format is written).

- **Showing, copying or exporting a password** asks for Touch ID or your Mac's login password first. Once you have confirmed, Mercury does not ask again for a minute while the Settings window stays open. Mercury asks macOS to keep a copied password on this Mac rather than send it to your other devices through Universal Clipboard, and to mark it as secret for clipboard managers that respect that. It is removed from the clipboard after a minute or when Mercury quits, unless you have copied something else since.
- **Filling a login into a page:** right-click (or Control-click) the page's password field and choose **Fill login…**, then the login. Mercury fills the password and the user-name field before it, only on the page's own site, and never into a frame embedded from another site. Nothing is filled on its own when a page loads.
- **Sites that ask through Mercury's sign-in sheet** (HTTP authentication): the sheet starts with your saved user name for that site; leave the password empty and press **Sign in** to use the saved password; if it cannot be read, the sheet stays open and asks you to type it. An empty password field always uses the saved password for that user name, so a saved login whose password is genuinely empty cannot be re-entered empty from the sheet; type any password to override. **Save the user name and password in Mercury Browser** saves what you type once the sign-in has worked; if it cannot be saved, Mercury says so under the toolbar and offers **Retry**. Nothing is filled in or saved for a proxy server's sign-in. **Offer to save passwords** and **Never save passwords for** in Settings control this.
- Saving what you type into a page's own sign-in form is not available yet: it needs a later version of the web engine.
- An exported file holds every password in clear text and is readable only by you. Delete it once you no longer need it, and do not open it in a spreadsheet.
- Runs with `--profile-dir`, `--screenshot` or `--background` keep logins only until Mercury quits and never use the keychain.

## Clipboard

Cut, Copy and Paste work in pages as usual: with their shortcuts, from the page's context menu, and from the Edit menu while the page has keyboard focus.

A page sees what is on your clipboard only when you paste into it, and it can change your clipboard only through your own Cut or Copy. Mercury does not yet offer pages the clipboard API (`navigator.clipboard`): a site's own **Copy** button, which uses it, does not work yet (select the text and copy it yourself), and no script can read or overwrite your clipboard on its own.

The reason is that Mercury cannot yet tell which frame of a page is asking. With the API on, a frame from another site inside the page (an advert, for example) would be treated like the page itself: it could put text on your clipboard shortly after you click, and it could read what you are pasting, such as a password you copied from the password manager. The API will be offered once the engine can tell the frames apart.

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
