// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//
// The reporter shim of the MSE window run (docs/developer-guide/mse-testing.md). serve-tests
// inserts it at the start of the <head> of an HTML page under tests/ when the page was opened with
// ?__mse_run=<token> (which only `mse-run` does). It does not change the page: it reports to
// /__report what the page already shows, and `mse-run` reads it there, because the browser has no
// command to print a page's title.
//
//   hello     the page started (sent once, at once)
//   progress  the PASS/FAIL/INFO lines so far (sent when their number changes)
//   page      the page is done: its title ends with "(done)"; sent once, with every line
//
// The pages (tests/pages/mse-*.html and mse-players/*.html) put each line in an element of class
// "line" and end their title with "(done)". Script errors and unhandled rejections are collected
// and sent with every message.
(function () {
  "use strict";
  var token = new URLSearchParams(location.search).get("__mse_run");
  if (!token) return;
  var errors = [];
  var finished = false;
  var lastCount = -1;

  function post(kind, extra) {
    var body = { token: token, kind: kind, url: location.href, title: document.title, errors: errors };
    for (var key in extra) body[key] = extra[key];
    try {
      var request = new XMLHttpRequest();
      request.open("POST", "/__report");
      request.setRequestHeader("Content-Type", "application/json");
      request.send(JSON.stringify(body));
    } catch (error) {
      // The runner's watchdog ends the test; there is nowhere else to report.
    }
  }

  function lines() {
    var found = [];
    var nodes = document.querySelectorAll(".line");
    for (var i = 0; i < nodes.length; i++) found.push(nodes[i].textContent);
    return found;
  }

  window.addEventListener("error", function (event) {
    errors.push(String(event.message) + " (" + event.filename + ":" + event.lineno + ")");
  });
  window.addEventListener("unhandledrejection", function (event) {
    errors.push("unhandled rejection: " + String(event.reason));
  });

  post("hello", {});
  var timer = setInterval(function () {
    if (finished) return;
    var found = lines();
    if (/\(done\)\s*$/.test(document.title)) {
      finished = true;
      clearInterval(timer);
      post("page", { lines: found });
    } else if (found.length !== lastCount) {
      lastCount = found.length;
      post("progress", { lines: found });
    }
  }, 250);
})();
