---
title: Releases
description: Build and publish the Mercury Browser macOS disk image and Linux preview archive, and deploy the documentation site.
---

# Releases

Each GitHub Release carries a macOS (Apple Silicon) disk image holding `Mercury Browser.app`, built on a GitHub-hosted Apple Silicon runner and signed ad hoc (see [Package for macOS](#package-for-macos)), and a Linux preview archive built on Ubuntu 22.04, a portable test artifact rather than a signed installer.

## Build a Linux package locally

```bash
make package-linux
```

The target creates an optimized executable, strips it, copies the README and license, then writes a compressed archive and SHA-256 checksum under `dist/`.

The current package name follows this shape:

```text
mercury-browser-v0.2.0-linux-x86_64.tar.gz
mercury-browser-v0.2.0-linux-x86_64.tar.gz.sha256
```

Change `PROJECT_VERSION` when testing another version:

```bash
make package-linux PROJECT_VERSION=0.3.0
```

## Package for macOS

Run these on an Apple Silicon Mac with the Xcode command-line tools installed (`xcode-select --install`); full Xcode is not needed. The bundle carries its GStreamer libraries, so the unpacked GStreamer must be present first ([Development setup](setup.md#macos-media-set-up-gstreamer)):

```bash
make gstreamer-macos # once: download and unpack GStreamer (no sudo)
make app-macos       # release build, then dist/Mercury Browser.app
make package-macos   # the same, plus the disk image and its checksum
```

Both targets run `scripts/macos/bundle.sh`, which can also be called directly on an existing build:

```bash
scripts/macos/bundle.sh --gstreamer-prefix <dir> [--binary target/release/mercury-browser] \
    [--out dist] [--identity -] [--dmg]
```

`--gstreamer-prefix` (or `MERCURY_GSTREAMER_PREFIX`, which `scripts/macos/gstreamer.sh env` exports and the Makefile targets pass on) is required: the executable cannot start without its media libraries, so the script refuses to build a bundle without them.

The script writes only inside the output directory and can be re-run; it rebuilds the bundle from scratch each time and removes it again if any step fails. It produces:

```text
dist/Mercury Browser.app/Contents/
  Info.plist            from apps/mercury-desktop/resources/macos/Info.plist.in
  PkgInfo
  MacOS/mercury-browser the release executable (arm64 only)
  MacOS/lib/            the GStreamer libraries and plugins (see below); nothing else
  Resources/AppIcon.icns
  Resources/LICENSE
  Resources/THIRD-PARTY-NOTICES.md
  Resources/LICENSES/   the licence texts the notices refer to (LGPL-2.1, GPL-3.0, …)
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
| `-` (ad hoc, the default) | `entitlements-adhoc.plist` | `com.apple.security.cs.allow-jit`, `com.apple.security.cs.allow-unsigned-executable-memory`, `com.apple.security.cs.disable-library-validation` |
| anything else (Developer ID, Apple Development) | `entitlements.plist` | `com.apple.security.cs.allow-jit`, `com.apple.security.cs.allow-unsigned-executable-memory` |

The two JIT entitlements are needed by SpiderMonkey to run JavaScript under the hardened runtime; without them no page loads.

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

`AppIcon.icns` and the PNGs in `apps/mercury-desktop/resources/macos/AppIcon.iconset/` are generated from `apps/mercury-desktop/resources/mercury-browser.svg` and committed, so packaging does not need to render them. After changing the logo, regenerate both with:

```bash
scripts/macos/make-icns.sh
```

It compiles `scripts/macos/render-icon.swift`, which draws the logo onto a rounded square following Apple's macOS icon grid at each of the ten iconset sizes, then runs `iconutil`.

## Continuous integration

The Linux quality job installs the same native GStreamer families used by development, verifies required plugins, runs formatting and Clippy, and executes workspace and media tests.

The release workflow (`.github/workflows/release.yml`) builds the Linux archive on Ubuntu 22.04 to preserve the glibc 2.35 compatibility baseline, and the macOS disk image on a GitHub-hosted Apple Silicon runner (`macos-26`). Windows release packaging remains deferred until the native build and GStreamer runtime can be validated together. Each job is described in [Continuous integration](continuous-integration.md#releaseyml-tags-v-manual-runs).

The macOS build job uses the same steps as a local build: it frees disk space, installs the pinned toolchain, restores the cached GStreamer packages (only the `.pkg` files, about 0.9 GB, keyed on `scripts/macos/gstreamer.sh`, as in CI), then runs `make gstreamer-macos`, the release build, `make package-macos` and `make test-bundle-macos`, checks the bundle and the mounted disk image with `scripts/ci/check-macos-bundle.sh --signature adhoc --require-media`, and uploads the ad-hoc `Mercury-Browser-<version>-macos-arm64.dmg` and its `.sha256` to the run as the artefact `adhoc-macos-arm64-dmg`. For a tag, the signing job then rebuilds the bundle from the same executable on a fresh runner (with a Developer ID if the `release` environment has one, otherwise ad hoc again) and that disk image is the one published.

## Publish a tagged release

A tag `v<version>` triggers GitHub Release publication. The tag must equal `[workspace.package] version` in `Cargo.toml`; the first job of the workflow fails otherwise, before anything builds.

```bash
git tag v0.2.0
git push origin v0.2.0
```

Before pushing a tag:

1. Run `make qa`.
2. Run `make docs`.
3. Run `make package-macos` and `make test-bundle-macos` on an Apple Silicon Mac, and check the disk image mounts and holds the app (`hdiutil attach -readonly -nobrowse -mountpoint <dir> dist/Mercury-Browser-<version>-macos-arm64.dmg`, then `hdiutil detach <dir>`).
4. Test the optimized browser on Linux.
5. Test direct media playback when the release changes GStreamer or graphics code.
6. Confirm the version and expected artifact names.

The release gets these files:

```text
mercury-browser-linux-x86_64.tar.gz          LICENSE, NOTICE, LICENSES/ and THIRD-PARTY-NOTICES.md inside
mercury-browser-linux-x86_64.tar.gz.sha256
Mercury-Browser-0.2.0-macos-arm64.dmg        LICENSE and NOTICE next to the app
Mercury-Browser-0.2.0-macos-arm64.dmg.sha256
Mercury-Browser-0.2.0-lgpl-sources.tar       source of the bundled LGPL libraries
Mercury-Browser-0.2.0-lgpl-sources.tar.sha256
update.json                                  the update manifest
update.json.minisig                          its minisign signature
SHA256SUMS                                   the downloads, the sources and update.json
```

The release notes start with the licence ("source-available, free for non-commercial use"), the first-launch steps, the LGPL source offer and the Independent JPEG Group credit, followed by GitHub's generated notes.

Two jobs run in the `release` environment: the macOS signing job and the update-manifest signing job. If the owner has set required reviewers there, each waits for an approval in the run's page. Without Developer ID secrets the disk image is signed ad hoc and not notarised; the job summary says so, and [Install Mercury](../user-guide/installation.md#the-first-launch-allowing-mercury-past-gatekeeper) tells users how to open it the first time. Without the update signing key, or without its public half committed, the release fails before anything is published.

Manual workflow runs (`workflow_dispatch`) publish nothing. They keep the Linux archive (`mercury-browser-linux-x86_64`), the ad-hoc macOS disk image (`adhoc-macos-arm64-dmg`), the LGPL sources (`mercury-browser-lgpl-sources`), the unsigned executable and the signer as Actions artefacts for 14 days.

### The update manifest

Mercury checks once a day whether a newer version exists. It fetches `update.json` and `update.json.minisig` from `https://github.com/thepigdestroyer/mercury-browser/releases/latest/download/`, verifies the signature against the public key compiled into the browser (`apps/mercury-desktop/resources/update-key.pub`) and compares the versions itself. The manifest looks like this:

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

The signature's trusted comment, which the signature covers, is `mercury-browser update <version>`; a verifier refuses a manifest whose comment names another version. Pre-release versions (`0.3.0-rc.1`) are refused when the manifest is written, so they are never offered.

The manifest is written and signed by `mercury-release`, a small Rust tool in `crates/mercury-release` built on the `minisign` crate (MIT):

```bash
mercury-release update-manifest --version V --published YYYY-MM-DD \
    --platform macos-arm64=FILE:SHA256:SIZE --platform linux-x86_64=FILE:SHA256:SIZE --out update.json
mercury-release sign --manifest update.json --version V --out update.json.minisig
mercury-release verify --manifest update.json --signature update.json.minisig \
    --public-key apps/mercury-desktop/resources/update-key.pub --version V
mercury-release keygen --public-key FILE --secret-key FILE
```

`sign` reads the secret key from `MINISIGN_SECRET_KEY` and its password from `MINISIGN_PASSWORD`, never from the command line, and refuses a key without a password. In the workflow, the "Update-manifest signer (build)" job builds and tests the tool with no secrets; "Sign the update manifest" runs in the `release` environment on a fresh runner, checks the tool against the build's digest, writes the manifest from the digests and sizes the packaging jobs recorded, signs it, and verifies the signature with the committed public key. The signing step is the only one that receives the key.

### The update signing key

The owner does this once, before the first release with update checks. Use an Apple Silicon Mac with a clone of the repository on the `mercury` branch.

1. **Generate the key pair** with Mercury's own signer (it asks for the password twice; choose a long one and keep it in your password manager):

    ```bash
    cargo run --release -p mercury-release -- keygen \
        --public-key apps/mercury-desktop/resources/update-key.pub \
        --secret-key ~/Documents/mercury-update-key.key
    ```

    The secret key is written with mode 600 outside the repository; the tool refuses to overwrite an existing key. (`minisign -G -p … -s …` from the minisign command-line tool makes compatible keys, if you prefer it.)

2. **Back up the secret key and its password offline**, for example on an encrypted USB stick kept apart from the Mac, plus the password in your password manager. Without them no future release can be signed with this key, and browsers already installed would stop accepting update manifests.

3. **Add the two secrets to the `release` environment** (not as repository secrets): on GitHub, **Settings → Environments → release → Environment secrets → Add environment secret**:
    - `MINISIGN_SECRET_KEY`: the whole content of `mercury-update-key.key`, both lines;
    - `MINISIGN_PASSWORD`: the password.

    Or with the GitHub CLI, from the clone:

    ```bash
    gh secret set MINISIGN_SECRET_KEY --env release < ~/Documents/mercury-update-key.key
    gh secret set MINISIGN_PASSWORD --env release    # pastes the password at the prompt
    ```

4. **Commit the public key.** Add `"apps/mercury-desktop/resources/update-key.pub",` to the PolyForm list in `REUSE.toml` (a `.pub` file cannot hold a licence comment; `make lint-scripts` checks this), then:

    ```bash
    git add apps/mercury-desktop/resources/update-key.pub REUSE.toml
    git commit -m "chore(release): add the update signing public key"
    git push origin mercury
    ```

5. **Check the pair** before relying on it, without the secret leaving the Mac:

    ```bash
    cargo run --release -p mercury-release -- update-manifest --version 0.0.1 --published 2026-01-01 \
        --platform test=test.bin:0000000000000000000000000000000000000000000000000000000000000000:1 \
        --out /tmp/update-test.json
    read -rs "MINISIGN_PASSWORD?Password: " && export MINISIGN_PASSWORD   # zsh; not echoed
    MINISIGN_SECRET_KEY="$(cat ~/Documents/mercury-update-key.key)" \
        cargo run --release -p mercury-release -- sign --manifest /tmp/update-test.json \
        --version 0.0.1 --out /tmp/update-test.json.minisig
    unset MINISIGN_PASSWORD
    cargo run --release -p mercury-release -- verify --manifest /tmp/update-test.json \
        --signature /tmp/update-test.json.minisig \
        --public-key apps/mercury-desktop/resources/update-key.pub --version 0.0.1
    rm /tmp/update-test.json /tmp/update-test.json.minisig
    ```

    The last command prints `update manifest for 0.0.1: signature good`.

**Rotating the key** later: generate a new pair, ship a release whose browser trusts both public keys, and switch the secrets to the new key only from the release after that, so that every installed copy can verify the manifest it receives.

### LGPL sources and credits

The macOS app bundles GStreamer, FFmpeg, GLib, proxy-libintl and libnice as separate dynamic libraries. Each release publishes their corresponding source as `Mercury-Browser-<version>-lgpl-sources.tar`, collected by `scripts/ci/lgpl-sources.sh`: each project's release tarball, checked against a pinned SHA-256, and the whole of cerbero, GStreamer's build system, at the commit that built the bundled binaries (its recipes name every source, option and patch). A `README.txt` inside explains it and repeats the written offer from `THIRD-PARTY-NOTICES.md`. When `scripts/macos/gstreamer.sh` moves to a new GStreamer, update the versions, URLs, checksums and cerbero commit in `lgpl-sources.sh` too; the script refuses to run while the GStreamer versions differ.

`NOTICE`, which every download carries, the release notes and `THIRD-PARTY-NOTICES.md` include the credit libjpeg-turbo's IJG licence requires: "This software is based in part on the work of the Independent JPEG Group."

## Documentation site

`.github/workflows/pages.yml` builds this site with `make docs` (MkDocs in strict mode) and deploys it to GitHub Pages, <https://thepigdestroyer.github.io/mercury-browser/>, with `actions/upload-pages-artifact` and `actions/deploy-pages`. It runs on pushes to `master` that change `docs/`, `mkdocs.yml` or the workflow itself, and on manual runs. Only the deploying job gets `pages: write` and `id-token: write`. The repository's **Settings → Pages → Build and deployment → Source** must be set to **GitHub Actions** once; until then the deployment fails.

