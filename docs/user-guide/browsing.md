---
title: Tabs and Shortcuts
description: Keyboard shortcuts, navigation controls, tab behavior, and text input in Mercury Browser.
---

# Tabs and shortcuts

Mercury follows familiar desktop-browser shortcuts while keeping the interface intentionally compact.

## Keyboard reference

| Shortcut | Action |
| --- | --- |
| ++ctrl+l++ | Focus and select the address bar |
| ++ctrl+t++ | Open a new tab |
| ++ctrl+w++ | Close the active tab |
| ++ctrl+tab++ | Select the next tab |
| ++ctrl+shift+tab++ | Select the previous tab |
| ++ctrl+r++ or ++f5++ | Reload the current page |
| ++alt+left++ | Go back |
| ++alt+right++ | Go forward |
| ++ctrl+m++ (++cmd+shift+m++ on macOS) | Mute or unmute the active tab |

On macOS, muting the tab is ++cmd+shift+m++. Firefox uses ++ctrl+m++, but on macOS Mercury leaves
Control shortcuts to the page (++ctrl+m++ is a text-editing key there). ++cmd+m++ (Minimize) and
++option+cmd+m++ (Minimize All) belong to the system. Chrome's ++cmd+shift+m++ opens its profile
menu, which Mercury does not have. Elsewhere it is ++ctrl+m++, as in Firefox and Edge.

## Mouse and trackpad input

The active page receives pointer movement, primary and secondary button events, and pixel-based scrolling. Navigation controls remain native UI elements outside the web content surface.

## Text input and IME

Mercury forwards physical key information, logical characters, modifier state, key repeats, and input-method composition to Servo. This supports dead keys and composed text used by many keyboard layouts and languages.

If typing works in the address bar but not inside a page, first click the page to return focus to the WebView. If composition still fails, capture the affected language, input method, and site in a bug report.

## Background tabs

Background tabs are throttled to reduce needless work. They are not yet suspended or discarded, so every open tab still keeps its WebView and browser state in memory.

There is currently no session restore. Closing Mercury discards open tabs and navigation history.

