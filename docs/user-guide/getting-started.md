---
title: Start Browsing
description: Open pages and search from the address bar, choose a search engine, work with tabs, and use the settings window in Mercury Browser.
---

# Start browsing

Mercury opens with a new tab. Its search field and the address bar at the top both take either a web address or words to search for.

![Mercury's new tab page in Dark mode, with a search field in the middle](../screenshots/mercury-new-tab.png)

To open a particular page at start-up, give its address on the command line; each address opens in its own tab:

```bash
"/Applications/Mercury Browser.app/Contents/MacOS/mercury-browser" https://servo.org/
```

Links and files that other apps open with Mercury, and the addresses of a second launch while Mercury is already running, open in new tabs of the running Mercury. Mercury declares that it can open http and https links, so macOS may offer it as your default browser in System Settings; this has not been checked on a real Mac yet.

## Open a website

Type any of these into the address bar and press ++return++:

```text
https://servo.org/
servo.org
localhost:8080
127.0.0.1:3000
file:///Users/you/page.html
```

Mercury opens `http`, `https`, `file` and `about` addresses. Text without a scheme is treated as an address when it looks like a host name (it contains a dot, or is `localhost` or an IP address), and `https://` is added in front. Anything else is a search.

A typed `http://` address is tried over HTTPS first, on the default port. Mercury opens the `http://` address only if the HTTPS one fails with one of its own error pages, and it never upgrades local names such as `localhost`, private addresses and `.local`. **Settings → Privacy and security → Load pages over HTTPS only** goes further: Mercury loads the HTTPS version of every `http` page, and when there is none it shows a notice in place of the page that lets you go back or continue to that site for the rest of the session.

The mark in the address bar shows what Mercury knows about the page: a padlock for HTTPS, a warning for plain HTTP, for a page with an invalid certificate and for a certificate exception you accepted, and an information mark for `localhost`, files and Mercury's own error pages (a new tab shows no mark). Click it for the site information: the permissions you gave the site (with **Reset**), its cookies and site data (with **Clear**), and any HTTPS-only exception (with **Remove**). Servo does not give Mercury a certificate's details yet, so the mark comes from the address and Mercury's own records, and the details are not shown. A name that mixes look-alike scripts is shown in its `xn--` form.

A certificate that isn't valid, a site that can't be reached and a crashed page get Mercury's own pages instead of Servo's. For a public site the certificate page offers only **Go back**. For a local name, such as a test server on your own network, **Details** lets you continue until you quit; Servo records that choice for the certificate, not the site, so continue only for a certificate you know.

## Search the web

Type words that don't look like an address:

```text
how does the Servo web engine work
```

Mercury sends them to your search engine, DuckDuckGo unless you choose another. To use a different engine for one search, start with its keyword:

```text
@wikipedia Servo (software)
```

| Engine | Keyword |
| --- | --- |
| DuckDuckGo (the default) | `@duckduckgo` |
| Bing | `@bing` |
| Brave Search | `@brave` |
| Ecosia | `@ecosia` |
| Google | `@google` |
| Startpage | `@startpage` |
| Wikipedia (en) | `@wikipedia` |

You can change the default engine, and add, edit or remove your own engines (a name, an optional keyword and a URL containing `{searchTerms}`), in **Settings → General**. The keyword must be the first word, followed by the search.

## Work with tabs

- **New tab:** the **+** button or ++cmd+t++.
- **Switch:** click a tab, use ++ctrl+tab++ and ++ctrl+shift+tab++, or ++cmd+1++ to ++cmd+8++ for the first eight tabs and ++cmd+9++ for the last.
- **Close:** the tab's **×** or ++cmd+w++. Closing the last tab quits Mercury. A page that closes its own last tab is replaced by a blank tab instead.
- **Reopen:** ++shift+cmd+t++ (**File → Reopen Closed Tab**) brings back the last tab you closed, at its old place in the strip.
- **Pin:** right-click (or Control-click) a tab and choose **Pin tab**. Pinned tabs sit first in the strip as small icon-only tabs with no close button, and ++cmd+w++ doesn't close one: it selects the first unpinned tab instead (or opens a new tab when every tab is pinned). The tab menu's **Close tab** does close it; **Unpin tab** (or **Window → Unpin Tab**) undoes the pin.
- **Duplicate and close others:** the same menu has **Duplicate tab**, a copy just after the tab, and **Close other tabs**, which closes every unpinned tab except this one (++alt+cmd+w++). Pinned tabs stay.
- **Reorder:** drag a tab. Dropping it among the pinned tabs pins it, and dragging a pinned tab out unpins it. **Window → Move Tab Left** and **Move Tab Right** reorder a tab within its group from the keyboard.
- **Search:** ++shift+cmd+a++ (**Window → Search Tabs…**) lists your tabs. Type part of a title or address, and press ++return++ to switch to the tab. With many tabs the strip shrinks them and then scrolls, and a button at its end opens the same list.
- **Sound:** a tab that is playing sound shows a speaker. Click it, or press ++shift+cmd+m++, to mute that tab; the page can undo a mute, because Mercury does it from inside the page. macOS Now Playing and the media keys follow the playing tab.
- **Links:** right-click a link and choose **Open link in new tab**. A page can open a tab of its own (`target="_blank"`, `window.open`) only in response to a click or key press in that page. Otherwise Mercury blocks it and shows **Pop-up blocked** in the toolbar, naming the site, with a button to allow pop-ups from it.

Every tab has its own Servo web view. Tabs you aren't looking at are throttled, but they stay in memory.

Mercury saves your open tabs as you browse. To get them back after you quit, set **Settings → General → On start-up, open** to **The tabs from last time** (the window's size and place come back too), or choose **History → Restore Previous Session**. If Mercury didn't close properly, a bar under the toolbar offers to restore your tabs. Restored tabs load when you first select them.

## History and bookmarks

**History → Show All History** (++cmd+y++) opens your history over the page. Search it, and delete a visit or a whole site. **Clear Browsing Data…** (++shift+cmd+backspace++) clears the last hour, today, the last 7 days or everything; Servo can't clear cookies, site data and cached files by date, so those are always cleared in full. While you type in the address bar, Mercury suggests pages from your history and bookmarks, and can complete a host you have typed before. The suggestions come only from your own Mac.

++cmd+d++ bookmarks the page, and the star in the address bar does the same. ++shift+cmd+b++ shows or hides the bookmarks bar, and ++alt+cmd+b++ opens the bookmarks manager. **File → Import…** copies bookmarks and history from Chromium-based browsers and Firefox on this Mac.

## Find in a page

++cmd+f++ opens the find bar under the toolbar, with the page's selected text in it if that is short. Type to search; the bar says "3 of 7". ++return++ and ++cmd+g++ go to the next match, ++shift+return++ and ++shift+cmd+g++ to the previous one, **Aa** matches the case, and ++escape++ closes the bar. Each tab keeps its own search.

## Downloads

++cmd+s++ saves the page. A link to a file the engine can't show downloads straight away when you click it or type its address; if a page starts a download on its own, a bar asks first. The ↓ button in the toolbar (it appears after your first download), or ++shift+cmd+j++, opens the list, where you can open a file, show it in Finder, cancel, resume or remove it. [Downloads](downloads.md) has the details.

## Zoom and full screen

++cmd+equal++, ++cmd+minus++ and ++cmd+0++ zoom the page in, out and back to 100 %. The ⋮ browser menu shows the current zoom. **Settings → General → Default page zoom** sets the zoom new pages start at.

**View → Enter Full Screen** (++ctrl+cmd+f++) makes the window full screen. When a page asks to go full screen (a video player, for example), ++escape++ leaves it.

## Copy and paste

Cut, copy and paste work in pages and in the address bar with ++cmd+x++, ++cmd+c++ and ++cmd+v++, from the right-click menu and from the Edit menu. A page sees your clipboard only when you paste into it, and it changes your clipboard only through your own cut or copy. Mercury doesn't offer pages the clipboard API (`navigator.clipboard`) yet, so a site's own **Copy** button may do nothing: select the text and copy it yourself. [Privacy and security](privacy-and-security.md#clipboard) says why.

## Passwords, camera and microphone

**Settings → Passwords** is the list of your saved logins, kept in your login keychain: search them, open one, copy it, edit or delete it, generate a password, and import or export a CSV file. Showing, copying and exporting ask for Touch ID or your Mac's password first. On a page, right-click its password field and choose **Fill login…**; nothing is filled on its own. When a site asks you to sign in through Mercury's own sign-in sheet, the sheet can fill and save the login. Saving what you type into a page's own sign-in form is not possible yet.

On macOS a site can ask to use your camera and microphone. Nothing opens before you answer, a red mark on the tab and a red button next to the address bar show while the page may use them, and that button's **Stop** ends it. Video calls (WebRTC) don't work yet, and location is off for now. [Camera and microphone](camera-and-microphone.md) has the details.

## Settings

Open the settings window with ++cmd+comma++, from **Mercury Browser → Settings…**, or from the ⋮ menu. Changes are saved as you make them; a text field is saved when you press ++return++ or leave it. ++escape++ or ++cmd+w++ closes the window.

| Section | What you can set |
| --- | --- |
| General | What opens at start-up (a new tab, the home page or the tabs from last time), the home page, the default page zoom, the search engine and your own engines |
| Appearance | Theme (Follow the system, Light or Dark) for Mercury's own windows and the colour scheme pages are told to use; the home button; the bookmarks bar |
| Privacy and security | Blocking ads and trackers, and whether the filter lists update themselves (**Update now** checks at once); blocking pop-up windows; loading pages over HTTPS only; what to clear when Mercury quits (browsing and download history, cookies and site data, cached files), and the **Clear browsing data…** button |
| Downloads | A folder and a question about where to save; neither does anything yet (see below) |
| Passwords | Whether Mercury offers to save passwords, the sites it never offers for, and the saved logins themselves |
| Advanced | Experimental web features and drawing canvases on the GPU; both take effect at the next start |

Some settings are already in the window but do nothing yet, because the feature behind them is not there yet. Their notes say so:

- the **Downloads** section, **Save downloads in** and **Ask where to save each download**: files go to your Downloads folder without asking;
- **Show search suggestions while typing**, which is greyed out;
- **Language**: Mercury is only in English for now.

## Where your data is kept

Your profile is `~/Library/Application Support/Mercury Browser/Default`: settings, your history, bookmarks and downloads list (`places.sqlite`), your open tabs for restoring (`session.json`), updated filter lists, cookies and site data. Saved logins are not in the profile; they are in your login keychain. The HTTP cache is in `~/Library/Caches/io.github.thepigdestroyer.mercury-browser/Default/http-cache`. Only one copy of Mercury can use a profile at a time.
