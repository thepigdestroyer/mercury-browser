---
description: Things to try by hand on each build, because automated runs cannot click, type, log out or use real devices.
---

# Hands-on checklist

Automated checks cannot click, press keys, log out or use real devices, so these steps cover what only
a person can see. Each step says what should happen. Note anything that differs (with a screenshot if
you can) and the commit you tried (`git log --oneline -1`).

This list covers what is on the `mercury` branch. Session restore, history, bookmarks, importing,
downloads, find in page, links from other apps, the tab audio indicator, camera and microphone have
been merged; their sections are still to be added.

## Before you start

```sh
make app-macos                       # builds the app (signed ad hoc) into dist/
open "dist/Mercury Browser.app"      # a normal launch, with your own Mercury profile
scripts/serve-tests.sh --port 8765 & # the repository's test pages, for some steps below
```

The first run builds the server before it starts listening. The test pages are then at
`http://127.0.0.1:8765/pages/<name>.html`. Stop the server with `kill %1` (or `fg` then Ctrl-C)
when you are done.

## Settings, theme and search

1. Cmd+, opens the settings window. Its sections can be reached by Tab, and Escape or Cmd+W closes it.
2. **Search:** in General, choose Startpage. An empty address bar now reads "Search with Startpage or
   enter an address", and typing `rust book` + Return opens Startpage's results. Type `@wikipedia rust` + Return → Wikipedia's search.
3. **Theme:** in Appearance, try Light, Dark and System. The tab strip, toolbar and new-tab page follow at
   once. With System, switch macOS between Light and Dark in System Settings: Mercury follows.
4. **Home button:** turn it on, set a home page, press it.
5. **Typed text survives:** type a home page, then close the settings window with its red close button
   without pressing Return. Reopen: the text is still there.
6. **Clear on exit:** turn on "Cookies and site data", sign in to a site, quit, start again: you are
   signed out. Turn it off again afterwards.

## Page dialogs and permission prompts

Use the test pages (server above), then a real site if you like.

1. `dialog-alert.html`: a sheet headed "127.0.0.1:8765 says" over a dimmed page; Return or OK closes it.
2. `dialog-prompt.html`: the sheet appears **without** a blinking cursor in its field. Press Tab, type a
   name, Return: the page shows what you typed. Escape cancels.
3. Open the alert page and reload it twice quickly: the second sheet offers "Don't allow this page to
   show more dialogs". Choose it, reload: no more sheets. Type another address: dialogs work again there.
4. `permission.html`: a bubble under the address bar ("wants to store data permanently"). It never takes
   the keyboard: keep typing in the address bar while it is shown. Block and Allow work with the mouse;
   Tab reaches them too.
5. `permission-camera.html`: on macOS a bubble under the address bar asks to use the camera (**Block** makes the page report the request was denied; see [Media capture](media-capture.md) for the rest of the camera checks); on other systems **no** prompt, and the page reports the request was denied.
6. `http-auth.html`: a sign-in sheet for the test server (user `alice`, password `secret`). The cursor
   is not in a field until you click one or press Tab. The sheet may also offer to save the login:
   leave that unticked. After signing in, the address bar does **not** show `alice:secret@`.

## Form controls and the context menu

Open `controls.html`.

1. **Select:** click a drop-down. The list opens under it, inside the page area. Arrows, Home/End and
   typing a letter move the highlight; Return chooses; Escape leaves the choice unchanged. A long list
   scrolls smoothly.
2. **Colour:** click a colour input; pick a swatch or type a hex value; Done or Return sets it.
3. **File:** click a file input. The macOS open panel slides down from the window as a sheet. Choose a
   file: the page shows its name. Try Cmd+V in the panel's "Go to folder" field (Cmd+Shift+G).
4. **No panel without a click:** open `controls.html#timer-click`. The page tries to open the file and
   colour pickers from a timer, without a click; its title changes to "Clicked from a timer". Nothing
   should open.
5. **Right-click** a link: "Open link in new tab" opens it in a background tab; "Copy link address"
   copies it. Control-click does the same. Right-click an image: open and copy its address.

## Menus and the About window

1. Open each menu in the menu bar. Every item that has a shortcut shows it. Stop is greyed out, and
   so are Reopen Closed Tab until you close a tab and Restore Previous Session unless the last run
   quit in order and its tabs were not reopened at start-up. Find…, Show All History, Clear Browsing
   Data… and the Bookmarks menu are enabled.
2. With three tabs open, the Window menu lists them; choosing one switches to it.
3. Press each shortcut once with the page focused, then with the address bar focused: each runs
   exactly once (for example ⌘T opens one tab, not two).
4. **Mercury Browser → About Mercury Browser**: the version, Servo 0.8.0, the GStreamer version, the
   copyright line and "Began as a fork of Hermes Browser by Kayk Caputo." Both links open in a tab.
5. The ⋮ menu: the arrow keys move through it, Return chooses, Escape closes it.

## Tabs opened by pages and pop-ups

1. `opener.html`: its `window.open` from a timer, 300 ms after load, is blocked and the toolbar shows
   **Pop-up blocked** naming `127.0.0.1:8765`; the page says "window.open returned null." Clicking
   the page's link opens `opened.html` in a new tab.
2. Choose **Allow pop-ups from 127.0.0.1:8765**, then open `two-pop-ups.html`: both pop-ups open,
   and the blue one is shown under its own address.
3. `opener.html?close`, after allowing pop-ups: the opened page closes its own tab again.

## Clipboard

Open `clipboard.html`. Mercury does not offer pages `navigator.clipboard` yet
(`ASYNC_CLIPBOARD_OFFERED` in `talaria-core`), so what must keep working is your own Cut, Copy and
Paste. Every one of them goes through Mercury's clipboard gate, which refuses a Paste it was not told
was yours (the page then gets empty text), so these steps are the only check of the real engine, the
Edit menu and the system clipboard together. The units drive the same code with a recorder in place of
the engine.

1. The page says `navigator.clipboard: missing`, and **Copy** and **Read the clipboard** both say
   `navigator.clipboard is missing.`
2. Copy a word in another app, then reload the page and wait a few seconds: you still have your word
   on the clipboard (nothing the page does changes it).
3. Your own Copy: select some page text, Edit → Copy, then again with Cmd+C and with the context
   menu's Copy: paste in another app each time: you get the text. Right-click a link and an image:
   "Copy link address" and "Copy image address" put the address on the clipboard.
4. Your own Paste shows the page what it pasted: copy a word in another app, click in the text box,
   and paste with Cmd+V, then with Edit → Paste, then with the context menu's Paste. Each time the
   word appears in the box and the line under it reads `paste event: "<your word>"` (a Paste that
   Mercury did not take as yours shows an empty box and `paste event: ""`). No prompt appears.
5. Your own Cut: type something in the box, select it and press Cmd+X: it leaves the box, and you can
   paste it in another app.

When `ASYNC_CLIPBOARD_OFFERED` is turned on (see its doc comment), add:

- Reload the page: on load it tries to write and read without a click and reports `writeText
  resolved` (Servo says so even when Mercury refuses the write) and `readText resolved with ""`. Your
  clipboard still holds your word, not "written by the page".
- Click **Copy** (an ordinary click: a script's write is allowed only within a second of the press):
  the page says "Copied."; paste in another app: you get "Mercury copy button".
- Click **Read the clipboard**: no prompt appears and the page says `Read ""`, whatever the
  clipboard holds. A script's read stays off until the engine can tell which frame asks
  (`CLIPBOARD_READ_FRAME_GATE`); then a bubble will ask whether "127.0.0.1:8765 wants to see text
  you copied".

## Quitting

1. Close the window, Cmd+Q, and Dock → right-click → Quit: each time Mercury quits within a second or
   two (with a download running, it first asks "Cancel 1 download and quit?").
2. **Log out with Mercury open** (save your other work first): log-out must **not** be held up or
   cancelled by Mercury. Log back in.
3. **Log out with a file panel open in Mercury** (step 3 of Form controls and the context menu):
   macOS should say Mercury cancelled the log-out, and Mercury quits by itself. This is expected;
   log out again.
4. In Terminal: start `"dist/Mercury Browser.app/Contents/MacOS/mercury-browser"`, then press Ctrl-C:
   it quits cleanly (no crash report).

## Media, graphics and pages

1. Open `media.html` (server above): every line says PASS within about 20 s, and the videos move.
   It plays without sound by design.
2. A real page with video (for example a Wikipedia article with a video): it plays, with sound.
3. Canvas and WebGL: `canvas.html`, `webgl-multi.html`: drawings appear, nothing flickers.
4. With macOS in Dark mode, a plain page (`unstyled.html`) is black text on white.
5. A news site: noticeably fewer ads and trackers than in a browser without blocking.

## PDF viewer

Do this section once on an **Apple Silicon** Mac with the application bundle (`make app-macos`): the
arm64 PDFium library has only been checked statically on the Intel development Mac, so this is its
first real run. The test PDFs are at `http://127.0.0.1:8765/pages/pdf/three-pages.pdf` and
`.../rotated.pdf` (server above); any PDF from the web will do for steps 4 and 5.

1. Open `three-pages.pdf`: the viewer appears in the tab with page 1 drawn crisp within a second,
   the toolbar says "1 / 3", and Activity Monitor shows a second `mercury-browser` process while the
   tab is open (the helper), which is gone after you close the tab.
2. Scroll with the trackpad: it follows your fingers with no stutter; pages that come into view are
   drawn at once or after a brief blur. Pinch in and out: the zoom changes in steps and the pages
   stay sharp.
3. Cmd+F, type a word of the file, press Return: "1 of n", the matches are marked, the next one is
   scrolled into view. Select text by dragging, press Cmd+C, paste it into TextEdit.
4. A long PDF (hundreds of pages): the page number is correct while dragging the scroll bar, and
   the helper's memory stays under about 200 MB.
5. A PDF with a password: the viewer asks, a wrong password says so, the right one opens it.
6. Settings, Downloads, PDF files: "Download" saves the file as before; "Open in the system's PDF
   app" opens Preview after a link click but not in a background tab.
7. In Terminal run `pkill -KILL -f pdf-worker` while a PDF is open: the viewer says it stopped
   working on this file and offers Try again; the browser itself keeps working.

## VoiceOver

A person's check: Mercury's own windows are labelled in code and covered by automated tests (see
[Accessibility of the interface](testing.md#accessibility-of-the-interface)), but nobody has yet tried
them with VoiceOver on a real Mac, and no automated run may turn VoiceOver on. This section is the
first try. Web pages are **not** expected to read yet: only Mercury's own interface.

1. Turn VoiceOver on with ⌘F5 (Fn ⌘F5 on a keyboard with Touch ID), then start Mercury. It must start
   and keep running with VoiceOver on, and also when VoiceOver is turned on while it runs. Open and
   close the About and Settings windows too: none of this may crash or hang.
2. **The toolbar.** In a new tab, move the VoiceOver cursor through the window with Control-Option-Right
   Arrow. You should hear, in this order: the tab strip, "New tab" (button), "Toolbar" (navigation),
   "Back" and "Forward" (buttons, dimmed), "Reload page", "Address bar" (search), "Address and search"
   (edit text), "Bookmark this page" (dimmed), "Browser menu", the bookmarks bar, "New tab page" with its
   captions, "Search or enter address" (edit text) and "Search" (button), and "Page content". Nothing
   should be read as a symbol ("left single quotation mark", "vertical ellipsis"), and nothing twice.
3. **Pressing.** With the VoiceOver cursor on a toolbar button, press it with Control-Option-Space. New
   tab opens a tab; Back, Forward and Reload do what they say once a page makes them available (a
   dimmed button does nothing); Home page (turn it on in Settings) opens the home page; "Search" in
   the new tab page opens a search for what you typed in "Search or enter address". Tab does not reach
   these buttons yet, by design: only the VoiceOver cursor does. Report a button the cursor cannot reach.
4. **While a page loads,** the VoiceOver cursor on "Reload page" may add "The page is loading" (VoiceOver
   usually says this help text after a pause).
5. **Tabs.** Open three tabs. The VoiceOver cursor on a tab should say its title, "tab", and whether it
   is selected; macOS may also say "n of m" (it is only passed on if VoiceOver asks for it, so say
   whether you hear it). Pressing a tab switches to it; a tab's close button says "Close tab".
6. **The address field.** The cursor on "Address and search" says "edit text". Interact with it
   (Control-Option-Shift-Down Arrow), type, and VoiceOver reads back what you type; Return opens it.
7. **The browser menu.** Press "Browser menu": its rows are read with their names (and their shortcuts
   as help text), the arrow keys move, Return chooses and Escape closes it.
8. **A page dialog.** Open `dialog-alert.html` (server above): VoiceOver reads the sheet with its
   heading naming the page ("127.0.0.1:8765 says") and its message, and can press OK. A permission
   bubble (`permission.html`) names the page as well.
9. **The Settings window** (⌘,): the sections read as tabs, each control by its name, and each can be
   pressed or changed; the same for the About window.
10. **A page.** With a page loaded, the VoiceOver cursor reaches "Page content" (main), whose help text
    says that Mercury doesn't pass web page content to assistive technology yet. Nothing inside the
    page is expected to read. Don't report that; do report a page area that cannot be reached at all.
11. **A long list** (the known limit). Open History with more than a screenful of visits. The VoiceOver
    cursor should reach the visits on screen; it cannot reach the ones below, because the list builds
    only the rows on screen and VoiceOver has no way to scroll it (keyboard scrolling is a follow-up).
    Scroll with the trackpad and check that the rows that come into view can be reached, and that
    typing in "Search history" narrows the list to rows the cursor reaches. Do report any other
    list that stops short, and whether "item n of m" is spoken for a visit.
12. **A password being typed.** In Settings > Passwords, open a login, choose Edit and type into "New
    password". VoiceOver must say "secure edit text" and must not read what was typed, as it does
    not for the sign-in sheet's password field. "Show the new password" shows the characters on
    screen only. Report it if VoiceOver reads the password aloud in either field.
13. **A PDF.** Open `three-pages.pdf` (the PDF viewer section above has the address). The window must
    offer one "Page content" (main) and no other main area: the VoiceOver cursor reaches the "PDF
    toolbar" (navigation) with the document's title, "Page number", "of 3 pages", "Zoom out", "Zoom
    100 percent", "Zoom in", "Fit width", "Fit page", "Rotate clockwise", "Find in document", "Save a
    copy" and "Open in Preview", and then "PDF document, page 1 of 3" (region). "Page content" must not say
    that Mercury passes no page content on while a PDF is open. Press a button with Control-Option-Space
    and check that it does what a click does. The text of the PDF's pages is not expected to read (the
    known limit); do report a toolbar button the cursor cannot reach, and, for a PDF with a password and one
    that cannot be shown, the panel's title, the "Password" field (which must say "secure edit text") and its buttons.

Report what reads badly: for each, the element, what VoiceOver said, what it should have said, the macOS
version and the commit. Wording that can be better is a finding as much as a crash.

## Reporting

For each step that differs, note the step number, what you saw, and the commit. Crash reports, if any,
are in Console → Crash Reports (`mercury-browser`).
