---
title: Architecture
description: Follow state, commands, input, page requests and rendered frames through the Mercury Browser crates.
---

# Architecture

Mercury keeps browser logic apart from the engine and from the interface. That boundary lets most behaviour run in fast Rust tests without starting Servo, Slint, WGPU or an event loop.

## Layers

```text
apps/mercury-desktop
  the window, macOS menu bar, shortcuts, file panels, settings glue,
  rendering bridge, start-up and the orderly quit
          │
          │ Slint callbacks and presentation properties
          ▼
crates/mercury-ui
  Slint components: tab strip, address bar, new tab page, settings window,
  dialogs, pickers, browser menu, About window
          │
          │ browser commands and events (from mercury-desktop)
          ▼
crates/talaria-core
  tabs, navigation, address resolution, page requests, zoom, the WebView trait
          │
          │ engine-neutral WebView calls
          ▼
crates/talaria-servo
  Servo runtime, web views, delegates, content blocking, page requests,
  media, HTTP cache, proxies, clearing data at exit
          │
          ▼
Servo 0.8 (with the patches in third_party/), WebRender, Surfman, GStreamer
```

`crates/mercury-settings` sits beside the core: the desktop loads `settings.json` from the profile at start-up, applies the settings to the controller and to `talaria-servo`, and saves changes from the settings window. The data crates (`mercury-store`, `mercury-passwords`, `mercury-downloads`, `mercury-import`) are not connected to the app yet.

## Talaria Core

`talaria-core` holds the browser concepts that should survive a change of engine or interface:

- `BrowserController` owns the tabs, the active tab and the command queue.
- `BrowserState` applies engine events such as URL, title, icon, history, load-state and zoom changes.
- `BrowserCommand` and `TabCommand` describe navigation and input without Servo types.
- `WebView` is the trait an engine adapter implements.
- Address resolution tells addresses from searches. Search URLs come only from a `SearchUrlBuilder` the application supplies from its search-engine settings; the core never builds one itself.
- Page requests (dialogs, sign-in, permissions, "leave this page?") are modelled and answered here, including which permissions may be asked about at all.

The controller merges adjacent pointer movement, scrolling and focus commands for the same tab, so fast input doesn't grow an unbounded queue between frames. The crate depends on `url` only, does no I/O and never reads the clock.

## Desktop composition

`mercury-desktop` creates the Slint window, the WGPU device, a `ServoRuntime` and one `ServoWebView` per tab.

During each turn of the event loop it:

1. reads pending Servo events and applies them to the matching tab;
2. makes sure every tab has a web view and removes closed ones;
3. shows and resumes the active web view, and hides and throttles the previous one;
4. updates the viewport size and scale factor;
5. dispatches a bounded number of queued browser commands;
6. paints only when Servo has announced a new frame;
7. brings the Slint interface and the menu bar up to date from the active tab's state.

The bounded command drain keeps a burst of input from taking over a frame.

## Command flow

```text
Slint callback, winit key or pointer event, or a menu item
  → BrowserController queue
  → TabCommand with a stable TabId
  → ServoWebView adapter
  → Servo WebView API
```

Commands carry a `TabId`, so switching tabs never redirects work already queued for another page. Key presses go through `shortcuts.rs` first; the macOS menu bar and the browser menu are built from one table in `menus/model.rs`, and its tests check that every key has exactly one owner.

## Event flow

```text
Servo WebViewDelegate
  → TabEvent over the runtime channel
  → BrowserController::handle_event
  → BrowserState::apply
  → Slint properties and the menu bar
```

The same `TabId` routes title, icon, load, history and page-request events back to the tab that caused them.

## Rendering

Slint owns the window and the event loop. Servo renders into a separate offscreen Surfman OpenGL context. On macOS, each finished frame's IOSurface becomes a Metal texture on Slint's own WGPU device, with one small render pass and no copy through the CPU; on Linux, the frame is copied into Vulkan external memory and imported into Slint's WGPU device. The two renderers never share OpenGL state or the window's framebuffer. A frame-ready signal that is consumed on use keeps Mercury from repainting when Servo has produced nothing new. [Servo integration](servo-integration.md#frame-delivery) has the details.

## Profiles

A normal launch locks and uses `~/Library/Application Support/Mercury Browser/Default` (`~/.local/share/mercury-browser/Default` on Linux): `settings.json` at its top, Servo's cookies and site data in its `servo` folder. The HTTP cache lives in the platform's cache folder. `--screenshot` and `--background` runs use a temporary profile that is removed at exit, unless `--profile-dir` names one; a directory is accepted as a profile only if it is new, empty or carries the marker file `.mercury-browser-profile`. A second Mercury on the same profile exits with status 4.
