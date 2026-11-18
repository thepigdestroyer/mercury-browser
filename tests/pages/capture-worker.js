// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// A worker of capture-frame-inner.html (media-capture lane): Mercury's engine refuses camera
// and microphone to workers without asking.
(async () => {
  const results = [];
  for (const step of ["query", "request"]) {
    try {
      results.push(`${step}=${(await navigator.permissions[step]({ name: "camera" })).state}`);
    } catch (error) {
      results.push(`${step}=error ${error.name}`);
    }
  }
  postMessage(results.join("; "));
})();
