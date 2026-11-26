---
title: Tabs and Shortcuts
description: Keyboard shortcuts, the macOS menu bar, the browser menu, the right-click menu, page dialogs and form controls in Mercury Browser.
---

# Tabs and shortcuts

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| ++cmd+t++ / ++cmd+w++ | New tab / close tab |
| ++shift+cmd+t++ | Reopen the last closed tab |
| ++shift+cmd+a++ / ++alt+cmd+w++ | Search tabs / close other tabs |
| ++cmd+l++ | Go to the address bar and select it |
| ++cmd+r++ | Reload |
| ++cmd+bracket-left++ or ++cmd+left++ / ++cmd+bracket-right++ or ++cmd+right++ | Back / forward |
| ++ctrl+tab++ or ++shift+cmd+bracket-right++ | Next tab |
| ++ctrl+shift+tab++ or ++shift+cmd+bracket-left++ | Previous tab |
| ++cmd+1++ … ++cmd+8++ | Go to that tab |
| ++cmd+9++ | Go to the last tab |
| ++cmd+equal++ / ++cmd+minus++ / ++cmd+0++ | Zoom in / zoom out / actual size |
| ++cmd+f++ | Find in the page |
| ++cmd+g++ / ++shift+cmd+g++ | Next / previous match |
| ++cmd+d++ / ++shift+cmd+b++ / ++alt+cmd+b++ | Bookmark this page / show or hide the bookmarks bar / bookmarks manager |
| ++cmd+y++ / ++shift+cmd+backspace++ | History / clear browsing data |
| ++shift+cmd+m++ | Mute or unmute the active tab |
| ++cmd+s++ | Save the page |
| ++alt+cmd+u++ | View the page's source in a new tab (++ctrl+u++ on Linux) |
| ++shift+cmd+j++ | Downloads |
| ++ctrl+cmd+b++ | Shrink the group dock to its small band, or open it again |
| ++shift+cmd+e++ | Show or hide the tab dial |
| ++ctrl+cmd+f++ | Enter or leave full screen |
| ++escape++ | Leave a page's full screen |
| ++cmd+comma++ | Settings |
| ++cmd+h++ / ++alt+cmd+h++ | Hide Mercury / hide other apps |
| ++cmd+m++ | Minimise the window |
| ++cmd+q++ | Quit |

Muting the tab is ++shift+cmd+m++ because ++cmd+m++ (Minimise) and ++alt+cmd+m++ (Minimise All)
belong to the system, and Mercury leaves Control shortcuts to the page on macOS (++ctrl+m++ is a
text-editing key there). Chrome's ++shift+cmd+m++ opens its profile menu, which Mercury does not
have. On Linux it is ++ctrl+m++, as in Firefox and Edge.

++cmd+left++ and ++cmd+right++ go back and forward only when no text field has the keyboard; in a text field they move to the start or end of the line as usual. The editing keys (++cmd+z++, ++shift+cmd+z++, ++cmd+x++, ++cmd+c++, ++cmd+v++, ++cmd+a++) work in the address bar and in the page. Holding a zoom or tab-switching shortcut, ++cmd+g++ or ++shift+cmd+g++ repeats it; other shortcuts run once per press.

On Linux, ++ctrl++ takes the place of ++cmd++, ++alt+left++ and ++alt+right++ go back and forward, ++f5++ reloads, ++f3++ and ++shift+f3++ go to the next and previous match (as do ++ctrl+g++ and ++ctrl+shift+g++), ++alt+d++ goes to the address bar, ++ctrl+page-down++ and ++ctrl+page-up++ switch tabs, ++ctrl+h++ opens the history, ++ctrl+shift+delete++ clears browsing data, ++ctrl+shift+o++ opens the bookmarks manager, ++ctrl+alt+b++ shrinks or opens the group dock, ++ctrl+shift+e++ shows or hides the tab dial, and Close Other Tabs has no key. On keyboard layouts that use ++ctrl+alt++ as AltGr, ++ctrl+alt+b++ may type a character instead; use **View → Shrink Dock** there.

**View → Show Tab Ring** has no key; it opens the ring of tab groups from the hub button, the same as clicking it.

## The menu bar

| Menu | Items |
| --- | --- |
| Mercury Browser | About Mercury Browser, Settings…, Services, Hide, Hide Others, Show All, Quit |
| File | New Tab, Duplicate Tab, Open Location…, Close Tab, Close Other Tabs, Reopen Closed Tab, Import… |
| Edit | Undo, Redo, Cut, Copy, Paste, Select All, Find…, Find Next, Find Previous |
| View | Reload Page, Stop, Actual Size, Zoom In, Zoom Out, View Source, Block Images on This Site, Show Images on This Site, Show Tab Ring, Shrink Dock (Show Dock while the dock is shrunk), Show Tab Dial, Enter Full Screen |
| History | Back, Forward, Show All History, Clear Browsing Data…, Restore Previous Session |
| Bookmarks | Bookmark This Page…, Show Bookmarks Bar, Bookmarks Manager, then one item per bookmark or folder on the bookmarks bar |
| Window | Minimize, Zoom, Show Next Tab, Show Previous Tab, Search Tabs…, Move Tab Left, Move Tab Right, Pin Tab, Unpin Tab, then one item per open tab |
| Help | Mercury Browser Help (opens this site in a new tab) |

Some items are greyed out:

- **Stop**: Servo 0.8 has no way for Mercury to stop a load.
- **Reopen Closed Tab** until you have closed a tab, and **Restore Previous Session** unless the last run's tabs are still waiting to be restored.
- **Undo**, **Redo** and **Select All** while the page has the keyboard: Servo offers only cut, copy and paste to Mercury.
- **View Source** unless the tab shows a web page (`http` or `https`) that has loaded: a new tab, a file, one of Mercury's own pages, an error page and the source page itself have no source to view.
- **Show Tab Ring**, **Shrink Dock** (**Show Dock**) and **Show Tab Dial** unless the window has what they act on: the hub button, the group dock and the corner dial. They stay greyed out while the tab-group interface is not switched on, and in a window whose layout and settings leave that control off (for example, Classic has no dock unless you turn it on). Their keys do nothing then either.
- **Block Images on This Site** and **Show Images on This Site**: exactly one of them is available at a time, and neither on a page that is not a web page. See [Images on one site](privacy-and-security.md#images-on-one-site).

**About Mercury Browser** shows the version, the engine and media versions, the copyright and licence, and three links: **Licence**, **Third-party notices** (the page packaged with the app, which lists the licence text and copyright of every library built into it and works offline; a build run straight from the source tree has none and opens the notices on GitHub instead) and **Source code** (the repository at the tag of your version, where the source of the MPL-2.0 parts is; a build of a version that is not released yet has no tag, and GitHub says the page is not found).

## The browser menu

The ⋮ button at the end of the toolbar opens the browser menu: New Tab, Search Tabs…, Duplicate Tab, Pin Tab (Unpin Tab while the tab is pinned), Close Other Tabs, a zoom row with the current zoom, Find…, Settings, About Mercury Browser, Help and Quit. The arrow keys move through it, ++return++ chooses and ++escape++ closes it.

## The right-click menu

Right-click (or Control-click) in a page:

- on a link: **Open link in new tab** (in the background) and **Copy link address**;
- on an image: **Open image in new tab** and **Copy image address**;
- on a password field, when logins are saved for the site: **Fill login…** at the top (see [Saved passwords](privacy-and-security.md#saved-passwords));
- below those, and anywhere else in the page: Cut, Copy, Paste, Select all, Back, Forward and Reload, each greyed out when it doesn't apply there;
- after Reload, in a menu for the page itself (not on a link or an image), when the page is a web page: **View page source**.

`javascript:` and `blob:` addresses are neither opened nor copied, and `data:` addresses can be copied but not opened.

## View source

**View → View Source** (++alt+cmd+u++, ++ctrl+u++ on Linux) and **View page source** in the right-click menu open the source of the page in a new tab right after the current one, and switch to it. You can also type or paste an address that starts with `view-source:`, such as `view-source:https://example.com/`. Only `http` and `https` pages have a source to view: `view-source:file:…`, `view-source:data:…` and the like are refused. A page cannot open a `view-source:` address itself, by a link or by a script; only you can.

The source page shows the text with line numbers and colours: HTML (with the JavaScript in `<script>` and the CSS in `<style>` coloured as such), JavaScript files, CSS files, and XML and SVG, which are coloured like HTML. JSON and any other text are shown without colours. The **Wrap long lines** tick box at the top turns wrapping off and on (it starts on, and is not remembered). In an HTML source, the addresses in `href` and `src` attributes are links that open the source of that address. The colours follow the light or dark appearance of the system. The page has no script of its own.

What to know:

- **It is a fresh copy.** Mercury fetches the address again; it does not show the exact bytes the page was drawn from. A page that came from a form (a POST), that changes at every request, that was built with a one-time token, or that the engine showed from its cache can differ from its source. The page says so at the top. Mercury sends your cookies for the site, but it cannot send what only the engine holds: a certificate you accepted with **Continue anyway** (the source page then says "The secure connection to the server failed."), a name and password you typed into a sign-in sheet, and the copy in the HTTP cache. It stores no cookie the server sets. The cookies are the ones the engine would send for a top-level request you typed to that address, under your cookie settings (the network policy: cookies it blocks are not sent, and a cookie marked `SameSite=Strict` is not sent after a redirect from another site). If Global Privacy Control is on, the request carries it, as every request of the browser does.
- **It is a plain GET, and so is every link.** Mercury only ever reads: it sends a GET with your cookies for the site, never a form. Following a link in the source page fetches that address the same way, so a one-time link (a sign-in or an unsubscribe link) is used up by looking at its source.
- **The same rules as for a page.** The address is checked as a page would be: a site that asked browsers to use only HTTPS (HSTS) is fetched over HTTPS even if you typed `http://`, **Load pages over HTTPS only** upgrades a public `http` address (and leaves your own computer and local network alone), and the ports browsers refuse (such as 22 and 25) are refused. The same checks are made for every redirect. A refused address shows "This address is refused by the same rules that apply to pages (HTTPS-only mode, blocked ports)." The address bar keeps the address you typed, without a user name or password, even when the source of an upgraded address is shown.
- **It follows your proxy.** The fetch goes through the same proxy settings as the pages (see [Proxy servers](privacy-and-security.md#proxy-servers)), and when a proxy applies to the address and cannot fetch it, the source page says "The proxy could not fetch the page." instead of connecting directly.
- **Limits.** At most 2 MiB of the source is fetched and shown, at most 50,000 lines are shown, and a source over 512 KiB is not coloured; the page says when the text was cut. A file that is not text (an image, a PDF, a font) is not shown: the page gives its size.
- **Nothing is kept.** A source tab is not recorded in the history and is not restored the next time Mercury starts.

## Dialogs and prompts from pages

- **Alerts, confirmations and text prompts** appear as a sheet headed "*site* says" over the dimmed page. They don't put the cursor in a text field until you click it or press ++tab++. If a page keeps showing dialogs, the second one offers **Don't allow this page to show more dialogs**.
- **HTTP sign-in** shows a sheet asking for a user name and password for the site. The address bar never shows them afterwards.
- **"Leave this page?"** prompts appear when a page asks for them on a reload or a navigation. The sheet is headed "Leave this page?" (buttons **Leave** and **Stay**) and says "Changes you made may not be saved."; when your own navigation was a reload, it is headed "Reload this page?" (buttons **Reload** and **Stay**). Closing a tab skips them.
- **Permission prompts** appear under the address bar; see [Privacy and security](privacy-and-security.md#permissions).

None of these take the keyboard from the address bar or the page while you type.

## Form controls

- **Drop-down lists** open under the control. The arrow keys, ++home++, ++end++ and typing a letter move the highlight; ++return++ chooses and ++escape++ leaves the choice unchanged.
- **Colour inputs** open a colour picker with swatches and a hex field.
- **File inputs** open the macOS open panel as a sheet on the window.

A page can open the colour picker or the file panel only in response to a click or key press in it. One requested from a timer is ignored.

## Text input

Mercury passes key presses, modifier keys, repeats and input-method composition to Servo, so dead keys and input methods for other languages work in pages. If typing works in the address bar but not in the page, click the page first.
