---
title: Camera and microphone
description: How websites ask to use your camera and microphone in Mercury Browser, how to stop them, and what to do when macOS refuses.
---

# Camera and microphone

On macOS, websites can ask to use your camera and microphone, for example a webcam test or a voice recorder. Mercury Browser never lets a site use either without asking you first. On other systems websites cannot use them at all for now.

Video calls (WebRTC) are not supported. Video-calling sites tell you that your browser is not supported.

## When a site asks

A bubble under the address bar names the site and what it wants: "example.com wants to use your camera and microphone".

- **Allow** lets this page use the devices named. Nothing is opened before you answer.
- **Block** refuses. The site is told that permission was denied.
- **Remember for this site until you quit**: if you tick it, Mercury gives the same answer to that site's later requests for the same device, until you quit Mercury. "Site" means the exact address scheme, name and port, so `https://example.com` and `https://example.com:8443` are different sites. Without the tick, your answer lasts only as long as the page stays open: after a reload or another page, the site asks again.

A frame from another site embedded in a page cannot ask for the camera or microphone. It is refused without a prompt, so every prompt names the site you are actually on.

## Seeing and stopping access

While a page may use your camera or microphone:

- its tab shows a red camera or microphone mark;
- a red button with the same marks appears to the right of the address bar.

The marks show that the page has *permission*. They appear when you allow it and stay until the page is closed or reloaded, or until you leave it. A page that has stopped using the camera may still show them. macOS's own indicator shows whether a device is actually in use: a green dot (camera) or orange dot (microphone) in the menu bar, and Control Center lists Mercury Browser.

To stop a page, click the red button and then **Stop**. Mercury:

1. forgets any "remember" answer for that site;
2. reloads the page, which closes the camera and microphone.

The page has to ask again. Closing the tab or going to another page also closes the devices.

## When macOS refuses

The first time a site you allowed opens the camera or microphone, macOS asks whether Mercury Browser may use it. If you refused there, or your administrator restricts it, the bubble says so: "macOS does not allow Mercury Browser to use the camera". Its **Open Privacy Settings** button opens System Settings, Privacy & Security, at Camera or Microphone, where you can turn Mercury Browser on. The site is refused until you do.
