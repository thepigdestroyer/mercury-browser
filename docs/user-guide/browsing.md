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
| ++shift+cmd+m++ | Mute or unmute the active tab |
| ++cmd+s++ | Save the page |
| ++shift+cmd+j++ | Downloads |
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

++cmd+left++ and ++cmd+right++ go back and forward only when no text field has the keyboard; in a text field they move to the start or end of the line as usual. The editing keys (++cmd+z++, ++shift+cmd+z++, ++cmd+x++, ++cmd+c++, ++cmd+v++, ++cmd+a++) work in the address bar and in the page. Holding a zoom or tab-switching shortcut repeats it; other shortcuts run once per press.

On Linux, ++ctrl++ takes the place of ++cmd++, ++alt+left++ and ++alt+right++ go back and forward, ++f5++ reloads, ++alt+d++ goes to the address bar, and ++ctrl+page-down++ and ++ctrl+page-up++ switch tabs.

## The menu bar

| Menu | Items |
| --- | --- |
| Mercury Browser | About Mercury Browser, Settings…, Services, Hide, Hide Others, Show All, Quit |
| File | New Tab, Open Location…, Close Tab, Reopen Closed Tab |
| Edit | Undo, Redo, Cut, Copy, Paste, Select All, Find… |
| View | Reload Page, Stop, Actual Size, Zoom In, Zoom Out, Enter Full Screen |
| History | Back, Forward, Show All History, Clear Browsing Data…, Restore Previous Session |
| Bookmarks | Bookmark This Page…, Show Bookmarks Bar, Bookmarks Manager |
| Window | Minimize, Zoom, Show Next Tab, Show Previous Tab, then one item per open tab |
| Help | Mercury Browser Help (opens this site in a new tab) |

Some items are greyed out:

- **Find…** and **Stop**: Servo 0.8 has no way for Mercury to search a page or stop a load.
- **Reopen Closed Tab**, the History items after Forward, and the whole **Bookmarks** menu: these features are being merged and will work in later builds.
- **Undo**, **Redo** and **Select All** while the page has the keyboard: Servo offers only cut, copy and paste to Mercury.

**About Mercury Browser** shows the version, the engine and media versions, the copyright and licence, and links to the licence and the third-party notices.

## The browser menu

The ⋮ button at the end of the toolbar opens the browser menu: New Tab, a zoom row with the current zoom, Find… (greyed out), Settings, About Mercury Browser, Help and Quit. The arrow keys move through it, ++return++ chooses and ++escape++ closes it.

## The right-click menu

Right-click (or Control-click) in a page:

- on a link: **Open link in new tab** (in the background) and **Copy link address**;
- on an image: **Open image in new tab** and **Copy image address**;
- below those, and anywhere else in the page: Cut, Copy, Paste, Select all, Back, Forward and Reload, each greyed out when it doesn't apply there.

`javascript:` and `blob:` addresses are neither opened nor copied, and `data:` addresses can be copied but not opened.

## Dialogs and prompts from pages

- **Alerts, confirmations and text prompts** appear as a sheet headed "*site* says" over the dimmed page. They don't put the cursor in a text field until you click it or press ++tab++. If a page keeps showing dialogs, the second one offers **Don't allow this page to show more dialogs**.
- **HTTP sign-in** shows a sheet asking for a user name and password for the site. The address bar never shows them afterwards.
- **"Leave this page?"** prompts appear when a page asks for them on a reload or a navigation. Closing a tab skips them.
- **Permission prompts** appear under the address bar; see [Privacy and security](privacy-and-security.md#permissions).

None of these take the keyboard from the address bar or the page while you type.

## Form controls

- **Drop-down lists** open under the control. The arrow keys, ++home++, ++end++ and typing a letter move the highlight; ++return++ chooses and ++escape++ leaves the choice unchanged.
- **Colour inputs** open a colour picker with swatches and a hex field.
- **File inputs** open the macOS open panel as a sheet on the window.

A page can open the colour picker or the file panel only in response to a click or key press in it. One requested from a timer is ignored.

## Text input

Mercury passes key presses, modifier keys, repeats and input-method composition to Servo, so dead keys and input methods for other languages work in pages. If typing works in the address bar but not in the page, click the page first.
