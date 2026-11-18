---
title: Mercury Browser Documentation
description: How to install, use, build, test and contribute to Mercury Browser, a desktop web browser written in Rust on the Servo engine.
hide:
  - navigation
  - toc
---

<div class="mercury-home">

<section class="mercury-hero">
  <div class="mercury-hero__copy">
    <div class="mercury-brand">
      <img src="assets/mercury-browser.svg" alt="" aria-hidden="true">
      <span>Mercury Browser</span>
    </div>
    <h1>A desktop browser in Rust, on the Servo engine.</h1>
    <p class="mercury-hero__lead">Servo renders the pages and Slint draws the windows, menus and dialogs around them. Mercury is built for macOS on Apple Silicon first. It is a preview: try it and work on it, but don't use it for anything sensitive yet.</p>
    <div class="mercury-actions">
      <a href="user-guide/installation/" class="md-button md-button--primary">Install Mercury</a>
      <a href="developer-guide/" class="md-button">Developer guide</a>
    </div>
  </div>
  <div class="mercury-hero__visual">
    <img src="screenshots/mercury-new-tab.png" alt="Mercury Browser's new tab page in Dark mode" loading="eager">
  </div>
</section>

<div class="mercury-proof">
  <div class="mercury-proof__item">
    <strong>Servo 0.8</strong>
    <span>The published engine, with a few small patches in third_party/</span>
  </div>
  <div class="mercury-proof__item">
    <strong>macOS on Apple Silicon</strong>
    <span>Metal rendering, a native menu bar, macOS file panels</span>
  </div>
  <div class="mercury-proof__item">
    <strong>GStreamer 1.28</strong>
    <span>Audio and video, bundled inside the app</span>
  </div>
</div>

<section class="mercury-section">
  <div class="mercury-section__intro">
    <p class="mercury-eyebrow">Two guides</p>
    <h2>Using Mercury, or working on it.</h2>
    <p>The user guide covers installing and using the browser and what it can't do yet. The developer guide covers building it, how the code is laid out, and how changes are tested.</p>
  </div>

  <div class="mercury-audience-grid">
    <article class="mercury-card">
      <span class="mercury-card__number">USER GUIDE</span>
      <h3>Install and browse</h3>
      <p>Install Mercury, use tabs, search and shortcuts, play media, read what Mercury blocks and stores, and fix common problems.</p>
      <a href="user-guide/">Open the user guide →</a>
    </article>
    <article class="mercury-card">
      <span class="mercury-card__number">DEVELOPER GUIDE</span>
      <h3>Build and change it</h3>
      <p>Set up a build, follow commands and frames through the crates, run the checks, and keep the Servo patches small.</p>
      <a href="developer-guide/">Open the developer guide →</a>
    </article>
  </div>
</section>

<section class="mercury-section">
  <div class="mercury-section__intro">
    <p class="mercury-eyebrow">On the mercury branch today</p>
    <h2>What works now.</h2>
  </div>

  <div class="mercury-feature-grid">
    <article class="mercury-card">
      <span class="mercury-card__number">01</span>
      <h3>Pages and tabs</h3>
      <p>GPU compositing through Metal, GPU 2D canvas and WebGL. Tabs, pages that open tabs, pop-up blocking, zoom and page full screen.</p>
    </article>
    <article class="mercury-card">
      <span class="mercury-card__number">02</span>
      <h3>Address bar and search</h3>
      <p>Seven search engines, DuckDuckGo by default, with keywords such as <code>@wikipedia</code>. You can add your own engines in Settings.</p>
    </article>
    <article class="mercury-card">
      <span class="mercury-card__number">03</span>
      <h3>Native macOS parts</h3>
      <p>A menu bar, an About window, the macOS file panel, a right-click menu, drop-down lists and a colour picker. Pickers open only after a click.</p>
    </article>
    <article class="mercury-card">
      <span class="mercury-card__number">04</span>
      <h3>Dialogs and prompts</h3>
      <p>Page alerts, confirmations, text prompts, HTTP sign-in and permission prompts. None of them take the keyboard away from you.</p>
    </article>
    <article class="mercury-card">
      <span class="mercury-card__number">05</span>
      <h3>Blocking and privacy</h3>
      <p>Ads and trackers blocked at the network level with EasyList, EasyPrivacy, uBlock Origin and Brave lists. Choose what is cleared when Mercury quits.</p>
    </article>
    <article class="mercury-card">
      <span class="mercury-card__number">06</span>
      <h3>Audio and video</h3>
      <p>MP4 (H.264, AAC), WebM (VP9, Opus), MP3 and WAV, decoded by the GStreamer copy inside the app. Sites that stream through Media Source Extensions may not play.</p>
    </article>
  </div>
</section>

<section class="mercury-showcase">
  <img src="screenshots/mercury-servo.png" alt="servo.org open in Mercury Browser in Dark mode" loading="lazy">
  <div class="mercury-showcase__copy">
    <p class="mercury-eyebrow">How it fits together</p>
    <h2>One Servo web view per tab.</h2>
    <p>The browser logic in <code>talaria-core</code> knows nothing about Servo. <code>talaria-servo</code> connects it to the engine, and the app draws Servo's frames inside its Slint window through Metal.</p>
    <a href="developer-guide/architecture/">Read the architecture →</a>
  </div>
</section>

<section class="mercury-section">
  <div class="mercury-section__intro">
    <p class="mercury-eyebrow">Origins</p>
    <h2>Where Mercury came from.</h2>
    <p>Mercury began as a fork of <a href="https://github.com/KaykCaputo/hermes-browser">Hermes Browser</a> by Kayk Caputo, a Linux-first browser shell around Servo 0.7. Some Hermes code is still here, credited and under its original MPL-2.0 licence. Most of Mercury has since been rewritten or added, and the interface it inherited is due to be replaced by a complete redesign. <a href="developer-guide/origins/">Origins and credits</a> has the details.</p>
  </div>
</section>

<aside class="mercury-preview">
  <img class="mercury-preview__mark" src="assets/mercury-browser.svg" alt="Mercury Browser logo">
  <div>
    <h2>Mercury is a preview.</h2>
    <p>Pages run in separate Servo content processes, but the operating system does not sandbox those processes yet. The app is signed ad hoc, not with an Apple Developer ID. Use known sites and data you don't mind losing.</p>
  </div>
  <a href="user-guide/privacy-and-security/">Read about privacy and security →</a>
</aside>

</div>
