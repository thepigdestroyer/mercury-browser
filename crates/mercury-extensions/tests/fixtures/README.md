# Test fixtures

## `video-bg-play-1.8.1.xpi`: a real Mozilla-signed add-on

Used by `tests/package_xpi_real.rs` to prove that the XPI signature check accepts a package that
Mozilla's production add-on signing service signed, against the bundled root in `data/`.

| | |
| --- | --- |
| Add-on | Video Background Play Fix 1.8.1 (`video-bg-play@timdream.org`), manifest version 2 |
| Source | https://addons.mozilla.org/firefox/downloads/file/4394140/video_background_play_fix-1.8.1.xpi |
| AMO page | https://addons.mozilla.org/en-US/firefox/addon/video-background-play-fix/ |
| Code repository | https://github.com/mozilla/video-bg-play |
| Fetched | 2026-11-22 (the tests pass `now` = 2026-11-22T12:00:00Z) |
| Size | 13,436 bytes |
| SHA-256 | `b04e7e489262b5c9bcd4dbf8219a4bd6fb665d0ff7cf9f4526eda0147025f550` (the value AMO's API reports) |
| Signatures | COSE (`cose.sig`) and PKCS#7 (`mozilla.rsa`), both present |

Licence: the add-on code is MIT (AMO's licence field is "MIT License" and the add-on's README says
"Add-on code: MIT"); the text, "Copyright (c) 2017 Mozilla Corporation", is in
`video-bg-play-LICENSE.txt`, copied from the repository above. The logo `icon.svg` inside the package is
"Glasses" by rahmatmasiv from the Noun Project, licensed CC BY 3.0 US (credited in the add-on's own
README, which is part of the package). The package is kept unmodified: changing a byte would break the
signature, which is the point of the fixture.

## Test PKI

There is no Mozilla signing key, so every other signature test builds its own root, intermediate and end
entity in `tests/package_common/pki.rs` and signs with those.
