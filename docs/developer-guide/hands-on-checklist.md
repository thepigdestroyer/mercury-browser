---
description: Things to try by hand on each build, because automated runs cannot click, type, log out or use real devices.
---

# Hands-on checklist

Automated checks cannot click, press keys, log out or use real devices, so these steps cover what only
a person can see. Each step says what should happen. Note anything that differs (with a screenshot if
you can) and the commit you tried (`git log --oneline -1`).

This list covers what is on the `mercury` branch. Menus, tabs opened by pages, session restore,
history, bookmarks, downloads, find in page, links from other apps, camera and microphone are still
being merged; their sections are added as they land.

## Before you start

```sh
make app-macos                       # builds the app (signed ad hoc) into dist/
open "dist/Mercury Browser.app"      # a normal launch, with your own Mercury profile
python3 scripts/serve-tests.py --port 8765 &   # the repository's test pages, for some steps below
```

The test pages are then at `http://127.0.0.1:8765/pages/<name>.html`. Stop the server with
`kill %1` (or `fg` then Ctrl-C) when you are done.

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
5. `permission-camera.html`: **no** prompt; the page reports the request was denied.
6. `http-auth.html`: a sign-in sheet for the test server (user `alice`, password `secret`). The cursor
   is not in a field until you click one or press Tab. After signing in, the address bar does **not**
   show `alice:secret@`.

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

1. Close the window, Cmd+Q, and Dock → right-click → Quit: each time Mercury quits within a second or two.
2. **Log out with Mercury open** (save your other work first): log-out must **not** be held up or
   cancelled by Mercury. Log back in.
3. **Log out with a file panel open in Mercury** (step 3 of the previous section): macOS should say
   Mercury cancelled the log-out, and Mercury quits by itself. This is expected; log out again.
4. In Terminal: start `"dist/Mercury Browser.app/Contents/MacOS/mercury-browser"`, then press Ctrl-C:
   it quits cleanly (no crash report).

## Media, graphics and pages

1. Open `media.html` (server above): every line says PASS within about 20 s, and the videos move.
   It plays without sound by design.
2. A real page with video (for example a Wikipedia article with a video): it plays, with sound.
3. Canvas and WebGL: `canvas.html`, `webgl-multi.html`: drawings appear, nothing flickers.
4. With macOS in Dark mode, a plain page (`unstyled.html`) is black text on white.
5. A news site: noticeably fewer ads and trackers than in a browser without blocking.

## Reporting

For each step that differs, note the step number, what you saw, and the commit. Crash reports, if any,
are in Console → Crash Reports (`mercury-browser`).
