---
title: User Guide
description: Install Mercury Browser, browse with it, play media, and understand what the preview does and does not do yet.
---

# User Guide

Mercury Browser is a desktop web browser for macOS on Apple Silicon. Servo renders the pages; Mercury provides the window, tabs, address bar, menus, settings and dialogs around them. It is a preview: many everyday features work, some are not there yet, and a few depend on things Servo does not do yet.

## Start here

1. [Install Mercury](installation.md).
2. [Start browsing](getting-started.md): the address bar, search engines, tabs and settings.
3. Learn the [shortcuts and menus](browsing.md).
4. Read what Mercury [blocks, stores and asks](privacy-and-security.md).
5. If a page or a video doesn't work, see [Media playback](media.md) and [Troubleshooting](troubleshooting.md).

## Platforms

| Platform | Status |
| --- | --- |
| macOS 13 or later on Apple Silicon (M1 or later) | The platform Mercury is built for. Media libraries are bundled with the app. |
| Linux x86-64 | Builds from source and is tested in CI. It gets less attention than macOS, and some parts are macOS only, such as the menu bar and the native file panel. |
| Windows | Not built at the moment. |
| Intel Macs | Build and run from source, in the same way as on Apple Silicon. The disk image and the app bundle are Apple Silicon only. |

## What is ready and what isn't

On the `mercury` branch today:

- tabs, pages that open tabs, pop-up blocking (pop-ups need a click in the page), and pages closing their own tab;
- the address bar with seven built-in search engines and keywords such as `@wikipedia`;
- a settings window for search, appearance (Light, Dark, System), the home page and clearing data when Mercury quits;
- page dialogs, permission prompts and HTTP sign-in;
- drop-down lists, a colour picker, the macOS file panel and a right-click menu;
- the macOS menu bar and the About window;
- an orderly quit from the window, ⌘Q, the Dock, log-out or a terminal signal;
- audio and video through GStreamer 1.28, bundled with the app;
- ad and tracker blocking at the network level, with filter lists updated in the background;
- your system's proxy settings and an HTTP disk cache;
- session restore and reopening closed tabs, and pinned tabs, tab search and dragging tabs into order;
- the history window and address-bar suggestions, bookmarks, importing from other browsers, the downloads list, find in page, and opening links from other apps;
- the tab audio indicator and Now Playing;
- saved logins in the Keychain, managed in Settings, with a manual fill from the right-click menu;
- site information, error pages for network and certificate problems, and an HTTPS-only mode;
- camera and microphone on macOS;
- cut, copy and paste in pages;
- screen-reader labels for Mercury's own windows (the toolbar, tabs, address bar, menus, dialogs, Settings and the other windows), written into the code and covered by automated tests, but not yet tried with VoiceOver on a real Mac. Web pages are not readable by a screen reader yet: to VoiceOver a page is one item, "Page content".

Not in the app yet: location, which is built but stays off until a Servo patch tells Mercury which frame is asking; video calls (WebRTC); a page's own clipboard API (`navigator.clipboard`); and the download folder settings, which are stored but not used. Mercury doesn't fill in or save the logins you type into a page's own sign-in form on its own.

Planned: a complete redesign of the interface, accessibility of web pages (a screen reader reading what a page says), phishing protection, search suggestions, translation, update checks, and releases signed with an Apple Developer ID.

## What Servo can't do yet

These come from the engine, so Mercury can't offer them either:

- Media Source Extensions, which most large video sites use to stream. Their videos may not play; a plain MP4 or WebM file plays.
- DRM, passkeys, extensions, printing, spell-checking, picture-in-picture and WebRTC video calls.
- Some pages look or behave differently from other browsers. Site compatibility is whatever Servo implements.

## Is Mercury for you?

Good uses today:

- trying Servo on real websites in a native Mac app;
- following the project, testing builds and reporting problems;
- working on the browser itself.

Not yet:

- passwords, payments, banking or anything sensitive: content processes are not sandboxed by the operating system;
- your only browser: some sites need features Servo doesn't have.
