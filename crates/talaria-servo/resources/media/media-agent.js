// Mercury Browser: the media helper of one document, evaluated by the browser with
// `evaluate_javascript`. `__TOKEN__` is a random name chosen per browser run and `__CALL__` the
// command: ["mute", true|false], ["probe"] or ["key", "play"|"pause"].
//
// Installed once per document as a frozen function in a non-writable, non-configurable,
// non-enumerable property whose name a page cannot guess in advance; its state lives in the
// closure, so a page can neither read nor change it (it can still call it, or undo the mute on
// its own elements). Servo has no per-web-view mute, so muting here is best effort.
(function (token, call) {
  "use strict";
  const existing = Object.getOwnPropertyDescriptor(window, token);
  let agent = existing && existing.value;
  if (typeof agent !== "function") {
    agent = install();
    Object.defineProperty(window, token, { value: agent });
  }
  return agent(call[0], call[1]);

  function install() {
    const weak = (value) =>
      typeof WeakRef === "function" ? new WeakRef(value) : { deref: () => value };
    const FRAMES = new Set(["iframe", "frame", "object", "embed"]);
    let muted = false;
    let muteInstalled = false;
    let mutedByUs = [];
    let suspendedByUs = [];
    const contexts = [];
    // Elements that played since installation, including ones never put in the document.
    const played = [];
    const playedSet = new WeakSet();
    let pausedByKey = [];

    function remember(element) {
      if (!playedSet.has(element)) {
        playedSet.add(element);
        played.push(weak(element));
      }
    }

    function silence(element) {
      if (muted && !element.muted) {
        element.muted = true;
        mutedByUs.push(weak(element));
      }
    }

    // Media elements of the document and its open shadow roots, plus elements that played;
    // `frames` counts places that cannot be looked into.
    function survey() {
      const media = new Set();
      let frames = 0;
      const visit = (root) => {
        for (const element of root.querySelectorAll("*")) {
          if (element instanceof HTMLMediaElement) media.add(element);
          else if (FRAMES.has(element.localName)) frames++;
          if (element.shadowRoot) visit(element.shadowRoot);
        }
      };
      visit(document);
      for (const reference of played) {
        const element = reference.deref();
        if (element) media.add(element);
      }
      return { media: Array.from(media), frames };
    }

    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () {
      remember(this);
      silence(this);
      return play.apply(this, arguments);
    };
    document.addEventListener("play", (event) => {
      if (event.target instanceof HTMLMediaElement) {
        remember(event.target);
        silence(event.target);
      }
    }, true);

    // Only when a mute is first asked for: watch for new elements, and track audio contexts
    // created from then on (earlier ones cannot be reached).
    function installMute() {
      muteInstalled = true;
      new MutationObserver((records) => {
        if (!muted) return;
        for (const record of records) {
          for (const node of record.addedNodes) {
            if (node instanceof HTMLMediaElement) silence(node);
            else if (node.querySelectorAll) node.querySelectorAll("audio, video").forEach(silence);
          }
        }
      }).observe(document, { childList: true, subtree: true });
      if (typeof AudioContext === "function") {
        window.AudioContext = new Proxy(AudioContext, {
          construct(target, args, newTarget) {
            const context = Reflect.construct(target, args, newTarget);
            contexts.push(weak(context));
            if (muted) suspend(context);
            return context;
          },
        });
      }
    }

    function suspend(context) {
      if (context.state !== "closed") {
        suspendedByUs.push(weak(context));
        context.suspend().catch(() => {});
      }
    }

    function setMuted(value) {
      if (!muteInstalled) installMute();
      muted = value;
      if (muted) {
        survey().media.forEach(silence);
        for (const reference of contexts) {
          const context = reference.deref();
          if (context && context.state === "running") suspend(context);
        }
      } else {
        for (const reference of mutedByUs.splice(0)) {
          const element = reference.deref();
          if (element) element.muted = false;
        }
        for (const reference of suspendedByUs.splice(0)) {
          const context = reference.deref();
          if (context && context.state === "suspended") context.resume().catch(() => {});
        }
      }
      return muted;
    }

    function probe() {
      const { media, frames } = survey();
      const playing = media.filter((element) => !element.paused && !element.ended);
      const audible = playing.filter((element) => !element.muted && element.volume > 0);
      const fullscreen = document.fullscreenElement;
      const inside = (element) =>
        fullscreen.contains(element) || (!!fullscreen.shadowRoot && fullscreen.shadowRoot.contains(element));
      const fullscreenVideo = !!fullscreen && (
        fullscreen.localName === "video" ||
        FRAMES.has(fullscreen.localName) ||
        playing.some((element) => element.localName === "video" && inside(element)));
      return [playing.length, audible.length, frames > 0, fullscreenVideo];
    }

    // A media key for a tab that does not hold Servo's media session. Pause remembers what it
    // paused, so the next play resumes exactly that.
    function key(action) {
      const media = survey().media;
      if (action === "pause") {
        pausedByKey = media.filter((element) => !element.paused);
        pausedByKey.forEach((element) => element.pause());
        return pausedByKey.length;
      }
      const resume = pausedByKey.length > 0
        ? pausedByKey
        : media.filter((element) => element.paused && !element.ended && element.currentTime > 0).slice(0, 1);
      pausedByKey = [];
      resume.forEach((element) => element.play().catch(() => {}));
      return resume.length;
    }

    return Object.freeze(function (command, argument) {
      if (command === "mute") return setMuted(argument === true);
      if (command === "probe") return probe();
      if (command === "key") return key(argument);
      return null;
    });
  }
})("__TOKEN__", __CALL__)
