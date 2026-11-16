// Mercury Browser: mutes or unmutes the media of one tab from script (`__MUTED__` is replaced by
// `true` or `false`). Servo has no per-web-view mute, so this is an interim measure that a page
// can undo. Installed once per document; later runs only switch it.
(function (muted) {
  "use strict";
  const key = Symbol.for("mercury.tabMute");
  const weak = (value) =>
    typeof WeakRef === "function" ? new WeakRef(value) : { deref: () => value };
  let state = window[key];
  if (!state) {
    state = { muted: false, elements: [], contexts: [], suspended: [] };
    Object.defineProperty(window, key, { value: state });
    // Mutes an element the page left unmuted, and remembers it so unmuting restores only those.
    state.silence = (element) => {
      if (state.muted && !element.muted) {
        element.muted = true;
        state.elements.push(weak(element));
      }
    };
    state.suspend = (context) => {
      if (context.state !== "closed") {
        state.suspended.push(weak(context));
        context.suspend().catch(() => {});
      }
    };
    // Elements that are never put in the document (`new Audio()`) are caught when they play.
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () {
      state.silence(this);
      return play.apply(this, arguments);
    };
    document.addEventListener("play", (event) => {
      if (event.target instanceof HTMLMediaElement) state.silence(event.target);
    }, true);
    new MutationObserver((records) => {
      if (!state.muted) return;
      for (const record of records) {
        for (const node of record.addedNodes) {
          if (node instanceof HTMLMediaElement) state.silence(node);
          else if (node.querySelectorAll) node.querySelectorAll("audio, video").forEach(state.silence);
        }
      }
    }).observe(document, { childList: true, subtree: true });
    // Only contexts created from now on can be reached.
    if (typeof AudioContext === "function") {
      window.AudioContext = new Proxy(AudioContext, {
        construct(target, args, newTarget) {
          const context = Reflect.construct(target, args, newTarget);
          state.contexts.push(weak(context));
          if (state.muted) state.suspend(context);
          return context;
        },
      });
    }
  }
  state.muted = muted;
  if (muted) {
    document.querySelectorAll("audio, video").forEach(state.silence);
    for (const reference of state.contexts) {
      const context = reference.deref();
      if (context && context.state === "running") state.suspend(context);
    }
  } else {
    for (const reference of state.elements.splice(0)) {
      const element = reference.deref();
      if (element) element.muted = false;
    }
    for (const reference of state.suspended.splice(0)) {
      const context = reference.deref();
      if (context && context.state === "suspended") context.resume().catch(() => {});
    }
  }
  return muted;
})(__MUTED__)
