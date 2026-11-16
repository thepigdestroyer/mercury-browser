// Mercury Browser: what the tab's top-level document is playing. Answers
// [playing elements, playing elements neither muted nor at volume 0, fullscreen video].
// Media inside frames is out of reach; a fullscreen frame counts as video.
(function () {
  "use strict";
  const playing = Array.from(document.querySelectorAll("audio, video"))
    .filter((element) => !element.paused && !element.ended);
  const audible = playing.filter((element) => !element.muted && element.volume > 0);
  const fullscreen = document.fullscreenElement;
  const fullscreenVideo = !!fullscreen && (
    fullscreen.localName === "video" ||
    fullscreen.localName === "iframe" ||
    Array.from(fullscreen.querySelectorAll("video")).some((video) => !video.paused && !video.ended));
  return [playing.length, audible.length, fullscreenVideo];
})()
