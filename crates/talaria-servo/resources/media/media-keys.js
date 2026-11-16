// Mercury Browser: play or pause the tab's media from a media key when Servo's media session
// belongs to another tab (`__ACTION__` is "play" or "pause"). Reaches the top-level document
// only. Pause remembers what it paused, so the next play resumes exactly that.
(function (action) {
  "use strict";
  const key = Symbol.for("mercury.mediaKeys");
  if (!window[key]) Object.defineProperty(window, key, { value: { paused: [] } });
  const state = window[key];
  const media = Array.from(document.querySelectorAll("audio, video"));
  if (action === "pause") {
    state.paused = media.filter((element) => !element.paused);
    state.paused.forEach((element) => element.pause());
    return state.paused.length;
  }
  const resume = state.paused.length > 0
    ? state.paused
    : media.filter((element) => element.paused && !element.ended && element.currentTime > 0).slice(0, 1);
  state.paused = [];
  resume.forEach((element) => element.play().catch(() => {}));
  return resume.length;
})(__ACTION__)
