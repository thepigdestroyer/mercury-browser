// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//
// Mercury's replacement for web-platform-tests' resources/testharnessreport.js, served by
// serve-tests in its place (docs/developer-guide/mse-testing.md). Where the original shows the
// results in the page for a person, this posts them to /__report for `mse-run`:
//
//   hello     the test page started (sent once, at once)
//   progress  the subtests finished so far (every 2 s, when there are new ones)
//   wpt       the file is complete: the harness status and every subtest
//
// Statuses are testharness.js numbers: subtests PASS 0, FAIL 1, TIMEOUT 2, NOTRUN 3,
// PRECONDITION_FAILED 4; harness OK 0, ERROR 1, TIMEOUT 2, PRECONDITION_FAILED 3.
(function () {
  "use strict";
  var token = new URLSearchParams(location.search).get("__mse_run");
  var finished = [];
  var sent = 0;
  var done = false;

  function post(kind, extra) {
    if (!token) return;
    var body = { token: token, kind: kind, url: location.href, title: document.title };
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

  function describe(test) {
    return { name: String(test.name), status: test.status, message: test.message === null || test.message === undefined ? null : String(test.message) };
  }

  if (typeof add_result_callback !== "function" || typeof add_completion_callback !== "function") return;
  post("hello", {});
  add_result_callback(function (test) {
    finished.push(describe(test));
  });
  setInterval(function () {
    if (!done && finished.length !== sent) {
      sent = finished.length;
      post("progress", { tests: finished });
    }
  }, 2000);
  add_completion_callback(function (tests, harnessStatus) {
    done = true;
    var all = [];
    for (var i = 0; i < tests.length; i++) all.push(describe(tests[i]));
    post("wpt", {
      harness: { status: harnessStatus.status, message: harnessStatus.message === null || harnessStatus.message === undefined ? null : String(harnessStatus.message) },
      tests: all,
    });
  });
})();
