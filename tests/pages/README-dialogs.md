# Page-request test pages

Pages that ask the browser something as soon as they have loaded, so that `--screenshot` shows the
browser's answer UI without any input. Serve `tests/` with `scripts/serve-tests.sh` (loopback; its
`/private` route asks for HTTP Basic authentication and accepts only `alice` / `secret`). Loopback
is a secure context, which permission requests need.

| Page | Shows |
|---|---|
| `dialog-alert.html`, `dialog-confirm.html`, `dialog-prompt.html` | the dialog sheet |
| `dialog-spoof.html` | an alert whose message has direction overrides, a control character, CR LF and a long text |
| `dialog-twice.html` | two alerts waiting at once (the page and a frame on `localhost`), with "Don't allow this page to show more dialogs" |
| `permission.html` | `navigator.storage.persist()`: the permission prompt under the address bar |
| `permission-camera.html` | `navigator.permissions.request({name: "camera"})`: on macOS the permission bubble under the address bar, with the page saying "Not answered yet." until it is answered (Block makes it say "camera permission: denied."; see [the hands-on checklist](../../docs/developer-guide/hands-on-checklist.md)); on other systems denied at once, without a prompt |
| `clipboard.html` | `navigator.clipboard` is missing while Mercury does not offer the clipboard API (`ASYNC_CLIPBOARD_OFFERED`): the page says so. If the API is ever offered, the page calls `writeText()` then `readText()` on load, without a gesture: both refused (Servo resolves the write anyway), the read resolves with `""`; run with `MERCURY_TEST_CLIPBOARD_TEXT=seed` to give the run's in-memory clipboard a text the page must not see. By hand, in a normal run: a text box whose `paste` handler shows the text the user's paste gave the page (and, with the API offered, buttons for a script's copy and read) |
| `http-auth.html` | a frame loading `/private`: the sign-in sheet (Servo asks only for navigations, not for `fetch()`) |
| `navigate-then-head-alert.html?port=N` | navigates (by script) to `head-alert.html` on port `N`, another origin, whose `<head>` alerts; run with `RUST_LOG=talaria_servo=debug` to see whether the new address or the new document's dialog reaches the browser first. Needs a second `scripts/serve-tests.sh --port N`. The run ends with exit 3, because the head alert keeps the page from loading. |

```sh
scripts/serve-tests.sh >serve.port &   # builds the Rust server the first time
until [ -s serve.port ]; do sleep 0.2; done; port="$(head -n 1 serve.port)"
./target/release/mercury-browser --background --window-size 1100x700 --screenshot alert.png \
  --screenshot-delay 2000 "http://127.0.0.1:$port/pages/dialog-alert.html"
kill %1; rm serve.port
```

## Password fill pages (passwords-ui)

`login-form.html` focuses its password field on load and reports what was filled (the user name and
the password's length, never the password). `login-iframe.html` shows the same form in a frame on
`localhost`, another origin. With a build that has the `passwords-diagnostics` feature,
`MERCURY_PASSWORDS_DIAG_FILL="http://127.0.0.1:$port alice@example.com fake-pw"` fills the focused field
each time a page finishes loading, as if the user had chosen the login from "Fill login…"
(docs/developer-guide/passwords.md):

```sh
MERCURY_PASSWORDS_DIAG_FILL="http://127.0.0.1:$port alice@example.com fake-pw" RUST_LOG=info \
  ./target/release/mercury-browser --background --window-size 1100x700 --screenshot fill.png \
  --screenshot-delay 3000 "http://127.0.0.1:$port/pages/login-form.html"
```
