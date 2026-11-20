---
title: Releases
description: Build and publish the Mercury Browser macOS disk image and Linux preview archive, and deploy the documentation site.
---

# Releases

Each GitHub Release carries a macOS (Apple Silicon) disk image holding `Mercury Browser.app`, built on a GitHub-hosted Apple Silicon runner and signed ad hoc (see [Package for macOS](#package-for-macos)), and a Linux preview archive built on Ubuntu 22.04, a portable test artifact rather than a signed installer. Next to them go the source of the bundled LGPL libraries ([LGPL sources and credits](#lgpl-sources-and-credits)), a signed update manifest ([The update manifest](#the-update-manifest)) and SHA-256 checksums with a build-provenance attestation ([Publish a tagged release](#publish-a-tagged-release)). Nothing is notarised yet, and the browser does not check for updates yet.

Before the first tag is pushed, the owner has one-time set-up to do: see [Before the first public release](#before-the-first-public-release).

## Build a Linux package locally

```bash
make package-linux
```

The target builds the optimized workspace, copies the executable and strips it, copies the README, the license, the third-party notices and the licence texts, installs the filter lists, then writes a compressed archive and SHA-256 checksum under `dist/`. The release workflow's archive has the same contents. Inside the archive's single top-level folder (`<package>/`):

```text
<package>/
  mercury-browser          the stripped executable
  README.md
  LICENSE
  THIRD-PARTY-NOTICES.md
  LICENSES/                the licence texts the notices refer to
  credits/credits.html     the licence text and copyright of every crate in the executable (generated)
  filter-lists/            the ad and tracker filter lists (data, read at start-up)
    manifest.json          each list's name, source URL, licence, version or date and SHA-256
    *.txt                  the five lists the manifest names
    NOTICE.md              each list's name, credit, licence and source
    LICENSES/              the licence texts of those lists (CC-BY-SA-3.0 and MPL-2.0)
```

`filter-lists/` is written by `mercury-filter-lists install <destination>` (the binary of `crates/mercury-filter-lists`, run from the repository root). It checks every list against its SHA-256 in `crates/talaria-servo/resources/adblock/manifest.json` and writes nothing if one does not match or a licence text is missing from `LICENSES/`. The lists are data files beside the program and are never compiled into it: after stripping, the target and the release workflow fail if the executable contains a fragment of a list (`advanced-ads-`, `scorecardresearch` or `Brave Firstparty specific filters`). The macOS bundle check (`scripts/ci/check-macos-bundle.sh`) looks for the same fragments.

The current package name follows this shape:

```text
mercury-browser-v0.2.0-linux-x86_64.tar.gz
mercury-browser-v0.2.0-linux-x86_64.tar.gz.sha256
```

Change `PROJECT_VERSION` when testing another version:

```bash
make package-linux PROJECT_VERSION=0.3.0
```

The archive on a GitHub Release is made by the release workflow, not by this target, and differs from it: it is named `mercury-browser-linux-x86_64.tar.gz` (the version is in the release's tag, not in the file name), and it also holds `NOTICE`, `THIRD-PARTY-NOTICES.md` and `LICENSES/`, where the local one holds only the README and `LICENSE`. Both carry `credits/credits.html` beside the executable, generated for `x86_64-unknown-linux-gnu` by `mercury-credits` (see [LGPL sources and credits](#lgpl-sources-and-credits)); a run that cannot produce the page fails.

## Package for macOS

Run these on an Apple Silicon Mac with the Xcode command-line tools installed (`xcode-select --install`); full Xcode is not needed. The bundle carries its GStreamer libraries, so the unpacked GStreamer must be present first ([Development setup](setup.md#macos-media-set-up-gstreamer)):

```bash
make gstreamer-macos
make app-macos
make package-macos
```

`make gstreamer-macos` downloads and unpacks GStreamer once (no `sudo`). `make app-macos` does a release build, then writes `dist/Mercury Browser.app`; `make package-macos` does the same and adds the disk image and its checksum. Both targets run `scripts/macos/bundle.sh`, which can also be called directly on an existing build:

```bash
scripts/macos/bundle.sh --gstreamer-prefix <dir> --credits <dir> [--binary target/release/mercury-browser] \
    [--out dist] [--identity -] [--dmg]
```

`--gstreamer-prefix` (or `MERCURY_GSTREAMER_PREFIX`, which `scripts/macos/gstreamer.sh env` exports and the Makefile targets pass on) is required: the executable cannot start without its media libraries, so the script refuses to build a bundle without them. So is `--credits` (or `MERCURY_CREDITS_DIR`): the directory that holds `credits.html`, the licence text and copyright of every crate in the executable, which `mercury-credits` generates ([Licence compliance](licence-compliance.md)). `make app-macos` and `make package-macos` generate it first into `target/release/credits` and pass it on. The script refuses to build a bundle without a usable page, because an app without its credits breaks the notice duties of the crates it contains.

The script writes only inside the output directory and can be re-run; it rebuilds the bundle from scratch each time and removes it again if any step fails. It produces:

```text
dist/Mercury Browser.app/Contents/
  Info.plist            from apps/mercury-desktop/resources/macos/Info.plist.in
  PkgInfo
  MacOS/mercury-browser the release executable (arm64 only)
  MacOS/lib/            the GStreamer libraries and plugins (see below); nothing else
  Resources/AppIcon.icns
  Resources/LICENSE
  Resources/NOTICE
  Resources/THIRD-PARTY-NOTICES.md
  Resources/LICENSES/   the licence texts the notices refer to (LGPL-2.1, MPL-2.0, CC-BY-SA-3.0, …)
  Resources/credits/credits.html  the licence text and copyright of every crate in the executable (About, Third-party notices)
  Resources/filter-lists/  the ad and tracker filter lists and their manifest (data, read at start-up)
  Resources/gstreamer-licenses/  each bundled library project's licence files, from the GStreamer packages
  _CodeSignature/
dist/Mercury-Browser-0.2.0-macos-arm64.dmg         with --dmg: the app, an Applications link, LICENSE and NOTICE
dist/Mercury-Browser-0.2.0-macos-arm64.dmg.sha256
```

The version comes from `[workspace.package]` in `Cargo.toml` and `LSMinimumSystemVersion` from `MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml`. The script refuses a binary that is not arm64 or that was linked for a different minimum macOS version than the configuration says.

### Info.plist in the executable

`apps/mercury-desktop/build.rs` fills the same `Info.plist.in` when building for macOS and `src/platform_macos.rs` places it in the executable's `__TEXT,__info_plist` section. A binary started from `target/release/` therefore has the application name and HiDPI behaviour of the bundle. Check it with:

```bash
otool -s __TEXT __info_plist target/release/mercury-browser | head -3
```

### Signing

Signing goes inside out and never uses `codesign --deep`: every Mach-O file in `Contents/MacOS/lib`, then the executable, then the bundle, all with the same identity. The hardened runtime (`--options runtime`) is on for every identity.

The default identity is `-`, an ad-hoc signature: valid for running on the machine that built it, but not trusted by Gatekeeper on other Macs. Set `--identity` or `MERCURY_SIGN_IDENTITY` to sign with a certificate from your keychain (`security find-identity -v -p codesigning` lists them). A real identity also gets a secure timestamp (`--timestamp`), and the disk image is signed too.

Besides ad hoc, only identities that carry a team identifier are supported: a Developer ID Application certificate for releases, or an Apple Development certificate for testing. A self-signed certificate has no team, so library validation would reject the bundled libraries at launch even though `codesign --verify` passes; the script refuses that combination after signing. The script also refuses identities that are empty, start with `-` (other than `-` itself) or have leading or trailing spaces, since those are almost always typing mistakes.

#### Entitlements

The identity decides which of two entitlement files in `apps/mercury-desktop/resources/macos/` is used, and the script prints which one:

| Identity | File | Grants |
|---|---|---|
| `-` (ad hoc, the default) | `entitlements-adhoc.plist` | `com.apple.security.cs.allow-jit`, `com.apple.security.cs.allow-unsigned-executable-memory`, `com.apple.security.cs.disable-library-validation`, `com.apple.security.device.camera`, `com.apple.security.device.audio-input` |
| anything else (Developer ID, Apple Development) | `entitlements.plist` | `com.apple.security.cs.allow-jit`, `com.apple.security.cs.allow-unsigned-executable-memory`, `com.apple.security.device.camera`, `com.apple.security.device.audio-input` |

The two JIT entitlements are needed by SpiderMonkey to run JavaScript under the hardened runtime; without them no page loads. The two device entitlements let websites the user allows use the camera and microphone (the hardened runtime refuses both without them); `Info.plist.in` carries the matching `NSCameraUsageDescription` and `NSMicrophoneUsageDescription`, without which macOS ends a process that opens either device.

`disable-library-validation` exists only because of ad-hoc signing. The hardened runtime normally lets a process load only libraries signed by Apple or by its own team, and an ad-hoc signature has no team identifier, so an ad-hoc build could never load the libraries it bundles in `Contents/MacOS/lib`: it would stop at launch with "mapping process and mapped file (non-platform) have different Team IDs". The ad-hoc file turns that check off.

The trade-off is real: with library validation off, any process that can write into the application bundle can make Mercury load a library of its own. That is acceptable for a build you made and run yourself. Until releases are signed with a Developer ID, the builds published for download are ad hoc too, so they also run with library validation off. A Developer ID build closes the gap: every bundled library is signed with the same team identifier as the executable, validation stays on, and `entitlements-adhoc.plist` must never be used with a Developer ID.

#### Checks after signing

`codesign --verify` checks neither the hardened runtime nor the entitlements, so after verifying the bundle the script reads both back from the executable and from the bundle. It stops if the `runtime` flag is missing or if the granted entitlements differ from the chosen file.

#### Testing the script

`make test-bundle-macos` (or `scripts/macos/test-bundle.sh`) runs without a build or a signing identity. It checks which entitlement file each identity gets (`scripts/macos/bundle.sh --identity <id> --print-entitlements` prints it without building), the refused identities, what each file grants, and the helpers in `scripts/macos/lib.sh` that parse `otool` and `codesign` output. That includes a load-command listing far larger than a pipe buffer, which a parser that stops reading early would turn into a silent exit.

Check a bundle with:

```bash
codesign --verify --deep --strict --verbose=2 "dist/Mercury Browser.app"
codesign -dv "dist/Mercury Browser.app"                    # flags=…(runtime)
codesign -d --entitlements - "dist/Mercury Browser.app"
spctl --assess --type execute -vv "dist/Mercury Browser.app" # "rejected" for ad hoc
"dist/Mercury Browser.app/Contents/MacOS/mercury-browser" --screenshot /path/to/js.png \
    "file://$PWD/tests/pages/js.html"                        # every line ends in PASS
```

### Media libraries

Servo loads its GStreamer libraries from `Contents/MacOS/lib`, found through the executable's `@executable_path/lib/` rpath (the script adds it when the build did not). Before signing, the script runs `scripts/macos/collect-gstreamer.sh <prefix> "Contents/MacOS/lib" <executable>`, which copies the libraries the executable links, the plugins `scripts/macos/gstreamer-plugins.sh` names (Servo's list minus the excluded plugins; Ogg is excluded for now) and their dependencies, thinned to arm64. Inside a bundle the collector writes no marker file: a signed bundle may hold only code in `Contents/MacOS`. The script then checks that every plugin from `gstreamer-plugins.sh` is present and no excluded one is, and copies `LICENSES/` and the bundled projects' licence files into `Contents/Resources`.

Only libraries and loadable plugins belong in `Contents/MacOS/lib`. They are signed without entitlements, so an executable placed there (a plugin scanner, for example) would run with library validation on and, in an ad-hoc build, fail to load the libraries next to it; the script stops if it finds one, and it stops too on any file there that is not code, which `codesign` would refuse. The collected libraries are signed with the same identity as the app. Ad-hoc builds can load them because of the ad-hoc entitlement set described under [Entitlements](#entitlements).

### Developer ID and notarisation

To publish a build that opens without warnings, a release manager with an Apple Developer ID Application certificate in the keychain signs with it:

```bash
make package-macos MERCURY_SIGN_IDENTITY="Developer ID Application: NAME (TEAMID)"
```

The bundle then carries the Developer ID signature, the hardened runtime, the entitlements and a secure timestamp, which is what notarisation requires. Submit the disk image and staple the ticket to it (credentials are stored once with `xcrun notarytool store-credentials mercury-notary`):

```bash
xcrun notarytool submit dist/Mercury-Browser-0.2.0-macos-arm64.dmg --keychain-profile mercury-notary --wait
xcrun stapler staple dist/Mercury-Browser-0.2.0-macos-arm64.dmg
```

Stapling changes the disk image, so write the checksum again afterwards:

```bash
(cd dist && shasum -a 256 Mercury-Browser-0.2.0-macos-arm64.dmg > Mercury-Browser-0.2.0-macos-arm64.dmg.sha256)
```

Notarisation has not been run for Mercury yet; current macOS builds are ad hoc.

### Application icon

`AppIcon.icns` and the PNGs in `apps/mercury-desktop/resources/macos/AppIcon.iconset/` are generated from two SVG masters in the same folder and committed, so packaging does not need to render them:

- `AppIcon.svg`, the full icon drawn on Apple's macOS icon grid (an 824 px rounded plate on a 1024 px canvas), for 64 px and up;
- `AppIcon-small.svg`, a simplified version (no halo, a larger Sun and a heavier M) for 16 and 32 px, where the full icon's detail is lost.

After changing either, regenerate the iconset and `.icns` with:

```bash
scripts/macos/make-icns.sh
```

It runs the workspace tool `mercury-iconset` (`crates/mercury-iconset`, built on the resvg SVG renderer) to render the ten iconset sizes, checks each size with `sips`, then runs `iconutil`.

The other brand files live in `docs/assets/`:

- `mercury-browser.svg` (the mark, also used in the app);
- `favicon.svg`;
- `readme-banner.svg`;
- `brand/`, which holds the one-colour mark, the light and dark lockups, and the GitHub social preview (`social-preview.svg` and the `social-preview.png` rendered from it).

## Continuous integration

The Linux quality job installs the same native GStreamer families used by development, verifies required plugins, runs formatting and Clippy, and executes workspace and media tests.

The release workflow (`.github/workflows/release.yml`) builds the Linux archive on Ubuntu 22.04 to preserve the glibc 2.35 compatibility baseline, and the macOS disk image on a GitHub-hosted Apple Silicon runner (`macos-26`). Windows release packaging remains deferred until the native build and GStreamer runtime can be validated together. Besides "Check the version" and the builds (Linux, and macOS in two jobs: build, then sign and package) it has four more jobs: "LGPL library sources" collects the source of the bundled LGPL libraries, "Update-manifest signer (build)" builds and tests the manifest signer with no secrets, "Sign the update manifest" writes and signs `update.json`, and "Publish GitHub Release" checks everything and publishes. Each job is described in [Continuous integration](continuous-integration.md#releaseyml-tags-v-manual-runs).

The macOS build job uses the same steps as a local build: it frees disk space, installs the pinned toolchain, restores the cached GStreamer packages (only the `.pkg` files, about 0.9 GB, keyed on `scripts/macos/gstreamer.sh`, as in CI), then runs `make gstreamer-macos`, the release build, `make package-macos` and `make test-bundle-macos`, checks the bundle and the mounted disk image with `scripts/ci/check-macos-bundle.sh --signature adhoc --require-media`, and uploads the ad-hoc `Mercury-Browser-<version>-macos-arm64.dmg` and its `.sha256` to the run as the artefact `adhoc-macos-arm64-dmg`. `make package-macos` first generates the credits page, and the job records its SHA-256 as the job output `credits-sha256` and uploads it as the artefact `credits-macos-arm64`. For a tag, the signing job then rebuilds the bundle from the same executable on a fresh runner (with a Developer ID if the `release` environment has one, otherwise ad hoc again) and that disk image is the one published. The signing job has no cargo, so it downloads `credits-macos-arm64` by exact name, checks its digest against `credits-sha256`, passes it to `bundle.sh --credits`, and checks that the page in the finished disk image has that digest. The checks are two functions of `scripts/macos/lib.sh`, `credits_artefact_problem` (one regular file, nothing else in the artefact, the recorded digest) and `credits_digest_problem` (the digest alone, for the disk image), which `scripts/macos/test-bundle.sh` runs against good and bad samples; the workflow only calls them.

## Publish a tagged release

A tag `v<version>` triggers GitHub Release publication. The tag must equal `[workspace.package] version` in `Cargo.toml`; the first job of the workflow fails otherwise, before anything builds. The version must be a plain `MAJOR.MINOR.PATCH`: the LGPL-sources script and the update-manifest tool both refuse anything else, so a tag for a pre-release version such as `0.3.0-rc.1` fails before anything is published.

```bash
git tag v0.2.0
git push origin v0.2.0
```

Before pushing a tag:

1. Check that the one-time set-up in [Before the first public release](#before-the-first-public-release) is done. Without the committed public key the release fails in its first minute; without the two secrets it fails after both builds have finished.
2. Run `make qa`.
3. Run `make docs`.
4. Run `make package-macos` and `make test-bundle-macos` on an Apple Silicon Mac, and check the disk image mounts and holds the app (`hdiutil attach -readonly -nobrowse -mountpoint <dir> dist/Mercury-Browser-<version>-macos-arm64.dmg`, then `hdiutil detach <dir>`).
5. Test the optimized browser on Linux.
6. Test direct media playback when the release changes GStreamer or graphics code.
7. Confirm the version and expected artifact names.

The release gets these files:

```text
mercury-browser-linux-x86_64.tar.gz          LICENSE, NOTICE, LICENSES/, THIRD-PARTY-NOTICES.md and credits/credits.html inside
mercury-browser-linux-x86_64.tar.gz.sha256
Mercury-Browser-0.2.0-macos-arm64.dmg        LICENSE and NOTICE next to the app and in its Resources, and the credits page
Mercury-Browser-0.2.0-macos-arm64.dmg.sha256
Mercury-Browser-0.2.0-lgpl-sources.tar       source of the bundled LGPL libraries
Mercury-Browser-0.2.0-lgpl-sources.tar.sha256
update.json                                  the update manifest
update.json.minisig                          its minisign signature
SHA256SUMS                                   the two downloads, the sources and update.json
```

Each `.sha256` file holds one line for its own file; `SHA256SUMS` holds four, for the Linux archive, the disk image, the LGPL sources and `update.json`. It does not list the `.sha256` files, `update.json.minisig` or itself. Users check downloads as described in [Verify a download](../user-guide/installation.md#verify-a-download).

The release notes start with the licence ("source-available, free for non-commercial use") and the names of the two downloads. For macOS they give a short version of the first-launch steps, with a link to [Install Mercury](../user-guide/installation.md#the-first-launch-allowing-mercury-past-gatekeeper), when the disk image is not notarised (and say it is signed with a Developer ID and notarised when it is). They then say that `update.json` and its signature are for an update check that is not built yet, give the LGPL source offer and the Independent JPEG Group credit, and are followed by GitHub's generated notes.

The publishing job writes `SHA256SUMS` only after it has checked that exactly the expected files arrived and that each matches the digest recorded by the job that made it. It then attests build provenance (`actions/attest`, `subject-checksums: dist/SHA256SUMS`), so the four files `SHA256SUMS` lists are attested subjects, and `gh attestation verify` can check them. The `.sha256` files, `update.json.minisig` and `SHA256SUMS` itself are not. Attestations need a public repository or GitHub Enterprise Cloud ([Public versus private](continuous-integration.md#the-release-environment)). The release is created as a draft, every file is uploaded, and only then is it published; a release that is already published is refused, not changed.

Two jobs run in the `release` environment: the macOS signing job ("macOS Apple Silicon (sign and package)") and the update-manifest signing job ("Sign the update manifest"). If the owner has set required reviewers there, each waits for an approval in the run's page. Without Developer ID secrets the disk image is signed ad hoc and not notarised; the job summary says so, and [Install Mercury](../user-guide/installation.md#the-first-launch-allowing-mercury-past-gatekeeper) tells users how to open it the first time.

Without the update key's public half committed, "Check the version" fails the release in its first minute, before anything builds; it also fails it if a tracked file looks like a minisign secret key (it prints file names, never the matching lines; see [The update-key checks](continuous-integration.md#the-update-key-checks)). Without the two secrets in the `release` environment, "Sign the update manifest" fails the release, and because that job waits for both builds, the failure shows up only after they have finished, hours into the run. Nothing is published in either case. Check the set-up before pushing the tag.

Manual workflow runs (`workflow_dispatch`) publish nothing. They keep the Linux archive (`mercury-browser-linux-x86_64`), the ad-hoc macOS disk image (`adhoc-macos-arm64-dmg`), the LGPL sources (`mercury-browser-lgpl-sources`), the unsigned executable and the signer as Actions artefacts for 14 days. The two jobs in the `release` environment run only for `v*` tags, so a manual run from a branch makes no signed disk image and no `update.json`.

### Before the first public release

The owner does these once. Nothing here can be done by an agent or by CI, and until the first two items are done, pushing a `v*` tag fails (see above): at once without the public key, after the builds without the secrets.

1. **The update signing key.** Generate the key pair, commit `apps/mercury-desktop/resources/update-key.pub` (and list it in `REUSE.toml`), and set `MINISIGN_SECRET_KEY` and `MINISIGN_PASSWORD` as secrets of the `release` environment. The steps are in [The update signing key](#the-update-signing-key).
2. **The `release` environment** exists, with a tag rule for `v*` and required reviewers set (and a tag ruleset on `v*`), as in [The release environment](continuous-integration.md#the-release-environment). Without reviewers the signing jobs run without anyone approving them.
3. **GitHub Pages** is set to build from GitHub Actions: **Settings → Pages → Build and deployment → Source**. Until it is, the documentation deploy fails ([Documentation site](#documentation-site)).
4. **Developer ID secrets, optional.** Without them the disk image is signed ad hoc and not notarised. To change that, add the certificate and notarisation secrets listed under [Release secrets](continuous-integration.md#release-secrets) to the `release` environment, and run one real sign, notarise and staple before relying on it ([Developer ID and notarisation](#developer-id-and-notarisation)).
5. **The About window and the credits page are right.** Open **About** in the build you are about to release and check four things: the licence line names the PolyForm Noncommercial License 1.0.0 and says that the Hermes-derived and Servo-derived files are MPL-2.0; **Third-party notices** opens the credits page bundled with that build (`Contents/Resources/credits/credits.html`; a build without the page opens the notices on GitHub instead, which is a failure for a release); **Source code** opens the repository at the tag of this version, `tree/v<version>`, the address the credits page's source section gives too (MPL-2.0 asks the binary to say where the source of the version it ships is). The tag is made by the release itself, so before tagging GitHub says the page is not found; check that the address ends in this version's number, and open it again after the tag exists; and the `AboutSlint` widget is visible (Slint's Royalty-free licence requires it). The Independent JPEG Group credit is not in the window. It is in `NOTICE`, `THIRD-PARTY-NOTICES.md`, the release notes and the installation guide ([LGPL sources and credits](#lgpl-sources-and-credits)). Then open the credits page itself and look at the index, the MPL-2.0 source section and one `NOTICE` block. [Licence compliance](licence-compliance.md) lists every duty and where it is met.
6. **The certificate-exception patch has landed (done; nothing for the owner to do).** Servo 0.8 kept a certificate exception per certificate, not per host: a certificate that a user accepted for one site was accepted for any host that presented it, for the rest of the session. The `cert-exceptions` lane (engine item `EC27`, the release blocker that the site-security lane found; its `feat(servo-net)` commit `b0e5fb6` on `lane/cert-exceptions`) replaced that with a `servo-net` patch that accepts an exception only for the pair of host and certificate, and only for the certificate that host failed with, and that fails closed everywhere else ([The servo-net certificate-exception patch](vendored-patches.md#the-servo-net-certificate-exception-patch)). The user guide no longer carries a warning about it: [Certificate errors](../user-guide/privacy-and-security.md#certificate-errors) says what an exception covers. What was checked, all in Rust ([Testing](testing.md#certificate-exceptions)):
    - `servo-net`'s own tests: the same certificate under another name, another certificate under the same name, a subdomain and a parent name are all refused; an exception is refused for an HSTS host (learned and preloaded), for a name Mercury cannot parse, for a host with no recorded failure and for a certificate other than the one the host failed with; the record of failures is bounded; and, against a loopback TLS server with a self-signed certificate, a load fails, succeeds under the accepted name, still fails under the server's other name, and fails again on a new connection after a revoke. The test for the other name was written first and failed against the unpatched crate (the second name loaded). An IPv6 literal, which got no certificate error before, now gets one and can be accepted.
    - `talaria-core` and `mercury-site-security`: one form of the host name for the exception set, the address-bar mark, the request parser and the handler; the handler answers a refusal with a status and a code, and the certificate page has a line for every code.
    - HSTS: a response over a connection that only the user's exception made valid does not teach the HSTS list (a self-signed device that sends `Strict-Transport-Security` would otherwise be refused for good in the next session, because Mercury keeps the list, and the holder of its key could plant a persistent entry). Tested with the loopback server answering the header, with a control that shows the list does learn from a connection that needed no exception, and with a scan of the engine sources for any other update of the list.
    - `scripts/macos/test-certificate-exceptions.sh`, the end-to-end check on the release build, which ends with `all checks passed` only when an image from the accepted name loads and the same certificate under another name does not, in a run for each of the server's two names, and when a run without the test switch accepts nothing. It passed (all 18 checks) on an Intel Mac on 20 November 2026; the first run on a heavily loaded machine once missed the browser's 30-second page-load limit, and the script now repeats a run that times out once.

    The engine tests above run in every full gate and in CI: `make test` runs `make test-servo-net`, and the CI job "Vendored servo-net tests" runs it on its own (servo-net is not a workspace member, so `cargo test --workspace` alone never ran them).

    What is left, and not part of this item:

    - **A user-visible Remove button (`EC27b`).** The engine can forget an exception, but connections that were opened under it (HTTP keep-alive, HTTP/2) stay authenticated until they close, so a reload after a Remove could still show the page with a padlock. A Remove button needs control of the connection pool (poison or rebuild the client). Until then exceptions last until Mercury quits and the user guide says so.
    - **Public sites still have no way past a certificate error.** The engine fix removes the exposure that made the local-names-only rule necessary, but lifting the rule is a product decision (it widens what one click can do, and Mercury has no way to add a company's own certificate authority yet). `talaria_core::certificate_exception_allowed` and the certificate page's `isLocalName` are the two places to change.
    - A host that becomes an HSTS host after an exception was made for it keeps that exception for that exact certificate; the refusal happens when an exception is made.

### The update manifest

Every release carries `update.json` and its signature `update.json.minisig`, made for Mercury's update check. **That check is not built yet.** The browser does not fetch, read or verify either file, no public key is compiled into it, and the README lists update checks as planned. What exists is the signing side: the manifest format, the `mercury-release` tool and the release job that signs the manifest and checks its own signature against the committed public key. The check is planned to fetch both files from `https://github.com/thepigdestroyer/mercury-browser/releases/latest/download/`, verify them with `mercury_release::verify_update` against the public key built into the browser (`apps/mercury-desktop/resources/update-key.pub`) and the browser's own version, and ignore the manifest on any error. `published` is the UTC date on which the signing job ran. The manifest looks like this:

```json
{
  "product": "mercury-browser",
  "version": "0.3.0",
  "published": "2026-12-01",
  "platforms": {
    "macos-arm64": {
      "asset": "Mercury-Browser-0.3.0-macos-arm64.dmg",
      "sha256": "…",
      "size": 123456789
    },
    "linux-x86_64": {
      "asset": "mercury-browser-linux-x86_64.tar.gz",
      "sha256": "…",
      "size": 45678901
    }
  }
}
```

The signature's trusted comment, which the signature covers, is `mercury-browser update <version>`. Verification fails closed: `verify_signed_manifest` accepts only a current (prehashed) minisign signature by the update key whose trusted comment is exactly `mercury-browser update <MAJOR.MINOR.PATCH>` and whose manifest names the same product and version, and returns that version; anything else is an error. It does not know which version is installed, so `verify_update` also requires the signed version to be newer than the running browser's, which refuses a replayed older manifest; an update check calls that one. `sign` signs only the exact manifest that `update-manifest` writes for the version it is asked to sign, and refuses anything else. Pre-release versions (`0.3.0-rc.1`) are refused when the manifest is written, so they are never offered.

The manifest is written and signed by `mercury-release`, a small Rust tool in `crates/mercury-release` built on the `minisign` crate (MIT):

```bash
mercury-release update-manifest --version V --published YYYY-MM-DD \
    --platform macos-arm64=FILE:SHA256:SIZE --platform linux-x86_64=FILE:SHA256:SIZE --out update.json
mercury-release sign --manifest update.json --version V --out update.json.minisig
mercury-release verify --manifest update.json --signature update.json.minisig \
    --public-key apps/mercury-desktop/resources/update-key.pub --version V
mercury-release keygen --public-key FILE --secret-key FILE
```

`--platform` may be repeated, once per platform; `update-manifest`, `sign` and `keygen` refuse to overwrite an existing output file. `sign` reads the secret key from `MINISIGN_SECRET_KEY` and its password from `MINISIGN_PASSWORD`, never from the command line, and refuses to run without a password, so the key must be password-protected. `keygen` asks for the password at the terminal. Build the tool with `cargo build --release -p mercury-release`; it is then `target/release/mercury-release`.

In the workflow, the "Update-manifest signer (build)" job builds and tests the tool with no secrets. "Sign the update manifest" runs in the `release` environment on a fresh runner: it checks that the public key is committed, checks the tool against the build's digest, writes the manifest from the digests and sizes the packaging jobs recorded, signs it, and verifies the signature with the committed public key. The signing step is the only one that receives the key.

### The update signing key

The owner does this once, by hand, on their own Mac (Apple Silicon or Intel) with a clone of the repository on the `mercury` branch, signed in to GitHub with `gh auth login` if they use the GitHub CLI. **Agents and CI never handle the secret key.** No agent generates it, reads it or sets the secrets; the only place a workflow ever receives it is the signing step of "Sign the update manifest", from the `release` environment.

Until `apps/mercury-desktop/resources/update-key.pub` is committed and both secrets exist in the `release` environment, pushing a `v*` tag fails. That is intentional: a release without a signed manifest is never published, and nothing is published by a failed run. Without the public key, "Check the version" fails in the first minute. Without the secrets, "Sign the update manifest" fails, and it runs only after both builds have finished. The release workflow also fails if a tracked file looks like a minisign secret key, so a key committed by mistake stops the release instead of going out; if one is ever committed, treat the key as leaked (see below).

What the key does today: it signs `update.json` for each release, and the release job and `mercury-release verify` check that signature. The browser verifies nothing yet, so no installed copy depends on the key.

1. **Build the signer** once, and run that file for every step below instead of `cargo run`, so that cargo never runs with the secret key in its environment (a build script of a dependency could read it). It needs only the Rust toolchain that `rust-toolchain.toml` pins, not Servo or GStreamer:

    ```bash
    cargo build --release -p mercury-release
    ```

    The tool is then `target/release/mercury-release`.

2. **Generate the key pair.** The public key goes into the clone; the secret key goes into a folder that only you can read and that is not synced (not `~/Documents` or `~/Desktop`, which iCloud Drive may sync). The tool asks for the password twice; choose a long one and keep it in your password manager:

    ```bash
    mkdir -m 700 ~/mercury-release-keys
    target/release/mercury-release keygen \
        --public-key apps/mercury-desktop/resources/update-key.pub \
        --secret-key ~/mercury-release-keys/update-key.key
    ```

    The secret key is written with mode 600 outside the repository, and the tool refuses to overwrite an existing file. Both files are two lines of text. If a key file ever ends up inside the clone, `.gitignore` keeps `*.key` files out of a commit.

3. **Back up the secret key and its password offline**, for example on an encrypted USB stick kept apart from the Mac, plus the password in your password manager. Without them no future release can be signed with this key.

4. **Add the two secrets to the `release` environment**, not as repository secrets: a repository secret is visible to every workflow, the environment's secrets only to jobs that declare `environment: release`. GitHub creates the environment by itself only when a job first uses it, so create it first, with the rules in [The release environment](continuous-integration.md#the-release-environment), under **Settings → Environments → New environment**. Then, on GitHub, **Settings → Environments → release → Environment secrets → Add environment secret**:

    - `MINISIGN_SECRET_KEY`: the whole content of `update-key.key`, both lines;
    - `MINISIGN_PASSWORD`: the password.

    Or with the GitHub CLI, from the clone:

    ```bash
    gh secret set MINISIGN_SECRET_KEY --env release --repo thepigdestroyer/mercury-browser < ~/mercury-release-keys/update-key.key
    gh secret set MINISIGN_PASSWORD --env release --repo thepigdestroyer/mercury-browser
    ```

    `--env release` is what makes them environment secrets; without it `gh secret set` makes repository secrets. The second command asks for the password at a prompt, where it is not shown; do not pipe it in with `echo`, which would add a newline to the password.

5. **Commit the public key.** A `.pub` file cannot hold a licence comment, so it must be listed in `REUSE.toml`: add the line `  "apps/mercury-desktop/resources/update-key.pub",` (two spaces, double quotes, trailing comma) to the first `path = [ … ]` list, the one followed by `SPDX-License-Identifier = "PolyForm-Noncommercial-1.0.0"`, in alphabetical order after `apps/mercury-desktop/resources/macos/entitlements-adhoc.plist`. The check, `scripts/ci/check-license-headers.sh` (part of `make lint-scripts` and of CI), reads that list line by line, so keep that exact form. It fails for a file without a licence and for a listed file that does not exist, so list the key only after it has been generated. Then:

    ```bash
    scripts/ci/check-license-headers.sh
    git add apps/mercury-desktop/resources/update-key.pub REUSE.toml
    git commit -m "chore(release): add the update signing public key"
    git push origin mercury
    ```

    The check prints `licence headers: every file has one`. Tag a commit that contains the file: "Check the version" and "Sign the update manifest" both check that it is in the tagged checkout.

6. **Check the pair** before relying on it, without the secret leaving the Mac. This is for zsh, macOS's default shell (`read -rs "NAME?prompt"` is zsh syntax; in bash it is `read -rsp 'Password: ' NAME`). Run it from the clone, one block at a time, so that the password you type at the prompt is not mixed up with pasted lines. First a test manifest:

    ```bash
    tool="$PWD/target/release/mercury-release"
    scratch="$(mktemp -d)"
    "$tool" update-manifest --version 0.0.1 --published 2026-01-01 \
        --platform test=test.bin:0000000000000000000000000000000000000000000000000000000000000000:1 \
        --out "$scratch/update.json"
    ```

    Then the password, typed at the prompt (it is not shown):

    ```bash
    read -rs "MINISIGN_PASSWORD?Password: " && export MINISIGN_PASSWORD
    ```

    Then sign, forget the password, verify and clean up:

    ```bash
    MINISIGN_SECRET_KEY="$(cat ~/mercury-release-keys/update-key.key)" \
        "$tool" sign --manifest "$scratch/update.json" --version 0.0.1 --out "$scratch/update.json.minisig"
    unset MINISIGN_PASSWORD
    "$tool" verify --manifest "$scratch/update.json" --signature "$scratch/update.json.minisig" \
        --public-key apps/mercury-desktop/resources/update-key.pub --version 0.0.1
    rm -r "$scratch"
    ```

    The `verify` command prints `update manifest for 0.0.1: signature good`. If the password is wrong, `sign` says so ("the password is wrong, or the key is corrupt") and writes nothing. Do not add `#` comments to the ends of these lines: zsh in a terminal does not treat them as comments, so they would be passed to the commands as arguments.

**Rotating the key.** Today nothing in the browser verifies a signature, so replacing the key costs nothing: generate a new pair (steps 2 to 6, after deleting the old `update-key.pub` and choosing a new name for the secret key file, since the tool refuses to overwrite), commit the new public key and replace the two secrets. Once the browser has an update check, every installed copy trusts the public key built into it, and a new key reaches it only through a release signed with the old one. A safe rotation then needs a browser that trusts both public keys: ship a release that trusts both (signed with the old key), and switch the secrets to the new key only from the release after that, so that every installed copy can verify the manifest it receives. That two-key support is not built; it has to come with the update check.

**If the key is lost or leaked.** Revoke it by deleting `MINISIGN_SECRET_KEY` and `MINISIGN_PASSWORD` from the `release` environment (**Settings → Environments → release**), so that no run can sign with it. Then generate a new pair, commit the new public key and set the new secrets (steps 2 to 6, as for a rotation); until then a tag fails in "Sign the update manifest". The key file is password-protected, so a leak of the file alone is not enough to sign, but replace the key anyway if the password may have gone with it. Installed browsers that trust the old public key cannot be told remotely to stop trusting it: nothing reaches them except a release, and a release is accepted only if a key they trust signed it. That costs nothing today, because no browser trusts any key yet. It will matter once the update check exists: copies already installed would keep accepting manifests signed with the old key, so the update check must not ship before it can trust more than one key (see the rotation above), and a key known to be leaked must be replaced before it does.

### LGPL sources and credits

The macOS app bundles GStreamer, FFmpeg, GLib, proxy-libintl and libnice as separate dynamic libraries. Each release publishes their corresponding source as `Mercury-Browser-<version>-lgpl-sources.tar` (and its `.sha256`), collected by `scripts/ci/lgpl-sources.sh <version> <output directory>`, which the "LGPL library sources" job runs as `scripts/ci/lgpl-sources.sh "${VERSION}" dist`. It needs `curl`, `git` and network access, and takes a plain `MAJOR.MINOR.PATCH` version. The archive holds each project's release tarball, checked against a pinned SHA-256, and the whole of cerbero, GStreamer's build system, at the commit that built the bundled binaries (its recipes name every source, option and patch). A `README.txt` inside explains it, says which patches the cerbero recipes apply (2 to FFmpeg, 14 to GLib, 1 to proxy-libintl; none to libnice or the five GStreamer tarballs), lists `SHA256SUMS`, and repeats the written offer from `THIRD-PARTY-NOTICES.md`. When `scripts/macos/gstreamer.sh` moves to a new GStreamer, update the versions, URLs, checksums and cerbero commit in `lgpl-sources.sh` too; the script refuses to run while the GStreamer versions differ.

`NOTICE`, which the macOS and Linux downloads carry, the release notes and `THIRD-PARTY-NOTICES.md` include the credit libjpeg-turbo's IJG licence requires: "This software is based in part on the work of the Independent JPEG Group."

The licence text and copyright line of every Rust crate built into the application, with the `NOTICE` files of the Apache-2.0 crates and the source pointers that the MPL-2.0 asks for, are in one generated page, `credits.html`: `Contents/Resources/credits/credits.html` in the macOS app and `credits/credits.html` in the Linux archive. `mercury-credits` writes it at package time from `Cargo.lock` (it is never committed), `bundle.sh` refuses to build without it, and **About, Third-party notices** opens it. [Licence compliance](licence-compliance.md#the-credits-page-and-its-tool) explains the tool, how to read a failure and which duties the page meets; the release checklist's item about the About window checks that the link works.

## Documentation site

`.github/workflows/pages.yml` builds this site with `make docs` (MkDocs in strict mode) and deploys it to GitHub Pages, <https://thepigdestroyer.github.io/mercury-browser/>, with `actions/upload-pages-artifact` and `actions/deploy-pages`. It runs on pushes to `master` that change `docs/`, `mkdocs.yml` or the workflow itself, and on manual runs. Only the deploying job gets `pages: write` and `id-token: write`. The repository's **Settings → Pages → Build and deployment → Source** must be set to **GitHub Actions** once; until then the deployment fails.

