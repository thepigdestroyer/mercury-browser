// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//
// Shared by the player pages (dashjs.html, shaka.html, hlsjs.html): the PASS/FAIL line reporting of
// the other tests/pages/mse-*.html pages, and the one script every player follows: play, seek, switch
// the quality, play to the end, read getVideoPlaybackQuality(). Each page only says how its library is
// loaded and how it switches the rendition. See docs/developer-guide/mse-testing.md.
//
// A line is an element of class "line" that starts with PASS, FAIL or INFO, in #out; the result is
// document.title, which ends with "(done)" when the page has finished. A page that stops (an
// exception, or no end within its own watchdog) shows a FAIL line, never a hang.
"use strict";
(function () {
  const out = document.getElementById("out");
  const summary = document.getElementById("sum");
  let passed = 0, failed = 0, finished = false, title = "", timer = null;

  function line(cls, text) {
    const d = document.createElement("div");
    d.className = "line " + cls;
    d.textContent = text;
    out.appendChild(d);
  }
  function check(name, ok, detail) {
    if (ok) { passed++; line("pass", "PASS " + name); }
    else { failed++; line("fail", "FAIL " + name + (detail !== undefined && detail !== "" ? ": " + detail : "")); }
    return ok;
  }
  function info(text) { line("info", "INFO " + text); }
  function finish(error) {
    if (finished) return;
    finished = true;
    clearTimeout(timer);
    if (error) { failed++; line("fail", "FAIL the page stopped: " + (error && error.stack || error)); }
    const text = passed + " PASS, " + failed + " FAIL";
    summary.textContent = text;
    document.title = title + ": " + text + " (done)";
  }
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  function once(target, name, ms) {
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => { target.removeEventListener(name, handler); reject(new Error("timeout waiting for " + name)); }, ms || 10000);
      const handler = event => { clearTimeout(t); resolve(event); };
      target.addEventListener(name, handler, { once: true });
    });
  }
  async function until(predicate, ms, what) {
    const end = performance.now() + (ms || 10000);
    while (!predicate()) {
      if (performance.now() > end) throw new Error("timeout waiting for " + what);
      await sleep(50);
    }
  }
  // Like until, but a timeout is a FAIL line and the script goes on (so one slow step does not hide the rest).
  async function expect(name, predicate, ms) {
    try { await until(predicate, ms, name); return check(name, true); }
    catch (error) { return check(name, false, error.message); }
  }
  function loadScript(src) {
    return new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = src;
      script.onload = resolve;
      script.onerror = () => reject(new Error("cannot load " + src + " (run scripts/dev/fetch-mse-tools.sh; the server serves it under /mse-players/)"));
      document.head.appendChild(script);
    });
  }

  // The rendition the player is showing, by the video's frame width: the low rendition is 320 wide, the high one 640.
  const LOW = 320, HIGH = 640;

  // The script of every player page. `hooks`: video, errors (an array the page fills with fatal player errors),
  // switchTo(width) (select the rendition of that width, flushing what is buffered; resolves when asked),
  // and optionally extra(): more checks after the end.
  async function standardRun(hooks) {
    const video = hooks.video;
    await expect("the video reaches HAVE_FUTURE_DATA", () => video.readyState >= 3, 20000);
    check("the duration is 8 s", Math.abs(video.duration - 8) < 0.5, video.duration);
    check("videoWidth is a rendition of the stream (320 or 640)", video.videoWidth === LOW || video.videoWidth === HIGH, video.videoWidth + "x" + video.videoHeight);
    info("first frame size " + video.videoWidth + "x" + video.videoHeight + ", readyState " + video.readyState);
    try { await video.play(); } catch (error) { info("play() rejected: " + error); }
    await expect("currentTime advances past 1 s", () => video.currentTime > 1, 15000);

    // One seek: forward to 4 s, into data that may not be buffered yet.
    video.currentTime = 4;
    try {
      await once(video, "seeked", 15000);
      check("seeked fires after a seek to 4 s", true);
    } catch (error) { check("seeked fires after a seek to 4 s", false, error.message); }
    check("currentTime is near 4 s after the seek", Math.abs(video.currentTime - 4) < 1.5, video.currentTime);
    await expect("playback goes on after the seek", () => video.currentTime > 4.4, 15000);

    // One forced quality switch, to the other rendition, with the buffer flushed.
    const from = video.videoWidth;
    const to = from === HIGH ? LOW : HIGH;
    try {
      await hooks.switchTo(to);
      await expect("the frame size changes to " + to + " after the switch", () => video.videoWidth === to, 15000);
    } catch (error) { check("the quality switch to " + to + " is accepted", false, error.message); }

    await expect("the video ends", () => video.ended, 30000);
    check("currentTime is at the end", Math.abs(video.currentTime - 8) < 0.5, video.currentTime);

    check("getVideoPlaybackQuality exists", typeof video.getVideoPlaybackQuality === "function");
    if (typeof video.getVideoPlaybackQuality === "function") {
      const q = video.getVideoPlaybackQuality();
      info("getVideoPlaybackQuality: " + JSON.stringify({ total: q.totalVideoFrames, dropped: q.droppedVideoFrames, corrupted: q.corruptedVideoFrames }));
      check("getVideoPlaybackQuality counts frames", q.totalVideoFrames > 0, q.totalVideoFrames);
      check("dropped frames are not above the total", q.droppedVideoFrames <= q.totalVideoFrames);
    }
    if (hooks.extra) await hooks.extra();
    check("the element has no error", video.error === null, video.error && video.error.code);
    check("the player raised no fatal error", hooks.errors.length === 0, hooks.errors.join(" | "));
  }

  // Runs `body` and finishes the page: with the summary, or with the exception it threw. `watchdog` (ms)
  // ends the page with a FAIL line when the body never finishes; keep it under the runner's 60 s.
  function run(pageTitle, body, watchdog) {
    title = pageTitle;
    timer = setTimeout(() => finish(new Error("the page did not finish within " + (watchdog || 48000) + " ms")), watchdog || 48000);
    if (typeof MediaSource === "undefined") {
      check("MediaSource exists (start the browser with MERCURY_DEV_MSE=1)", false);
      finish();
      return;
    }
    body().then(() => finish(), error => finish(error));
  }

  window.kit = { line, check, info, finish, sleep, once, until, expect, loadScript, standardRun, run, LOW, HIGH };
})();
