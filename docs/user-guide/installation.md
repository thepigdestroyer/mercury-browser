---
title: Install Mercury
description: Install Mercury Browser on macOS from the disk image, or build and run it on Linux with the required Rust, graphics, and GStreamer dependencies.
---

# Install Mercury

## macOS (Apple Silicon)

Mercury for macOS runs on Macs with Apple silicon (M1 or later) and macOS 13 Ventura or later. It is distributed as a disk image named like `Mercury-Browser-0.2.0-macos-arm64.dmg`, attached to each release on the [GitHub Releases page](https://github.com/thepigdestroyer/mercury-browser/releases). Mercury is source-available, free for non-commercial use; the disk image carries its `LICENSE` and `NOTICE` next to the app, and the app carries them, with the third-party notices, in its own `Contents/Resources`. The app bundles GStreamer, PDFium (which draws PDF files) and the libraries they need; the source of the LGPL ones is attached to the same release as `Mercury-Browser-<version>-lgpl-sources.tar`. This software is based in part on the work of the Independent JPEG Group. In the app, **About Mercury Browser**, then **Third-party notices**, lists every library it contains and its licence.

### Install from the disk image

1. Download `Mercury-Browser-<version>-macos-arm64.dmg` and, optionally, the `.sha256` file next to it from the release. Check the download against it (more checks are in [Verify a download](#verify-a-download)):

    ```bash
    shasum -a 256 -c Mercury-Browser-0.2.0-macos-arm64.dmg.sha256
    ```

2. Double-click the `.dmg` to open it.
3. Drag **Mercury Browser** onto the **Applications** folder shown next to it.
4. Eject the disk image.

### The first launch: allowing Mercury past Gatekeeper

**Mercury is not notarised.** The project has no Apple Developer ID yet, so the app is signed only ad hoc and Apple has never checked it. macOS marks everything downloaded from the internet, and Gatekeeper blocks a marked app that Apple has not checked. The first time you open Mercury Browser, macOS refuses (it says Apple could not verify that "Mercury Browser" is free of malware) and offers only **Done** or **Move to Trash**. Choose **Done**, then allow it once in one of these two ways.

**Right-click → Open** (macOS 13 Ventura and 14 Sonoma only):

1. In the **Applications** folder, hold Control and click **Mercury Browser** (or right-click it).
2. Choose **Open** from the menu.
3. Click **Open** in the dialog that follows.

From macOS 15 Sequoia on, this route no longer offers **Open**; use System Settings instead.

**System Settings → Privacy & Security → Open Anyway** (every supported macOS):

1. Try to open Mercury Browser once and choose **Done** when macOS refuses.
2. Open **System Settings** and choose **Privacy & Security**.
3. Scroll down to the **Security** section. It shows a line saying that "Mercury Browser" was blocked to protect your Mac.
4. Click **Open Anyway** and confirm with your password or Touch ID.
5. Open Mercury Browser again and click **Open Anyway** (or **Open**) in the dialog that follows.

macOS remembers the choice; later launches open normally. The **Open Anyway** button only appears for about an hour after the blocked attempt, so try opening the app again if it is missing. Each new version you download has to be allowed again.

If macOS instead reports that the app "is damaged and can't be opened", it has blocked it without offering **Open Anyway**. The app is not damaged. If you trust where you downloaded it from, remove the download mark in Terminal and open it again:

```bash
xattr -dr com.apple.quarantine "/Applications/Mercury Browser.app"
```

Only do either of these for a copy you downloaded from the project's own release page.

### Uninstall

Quit Mercury Browser and move `/Applications/Mercury Browser.app` to the Trash. To remove your data too, delete `~/Library/Application Support/Mercury Browser` (settings, cookies and site data) and `~/Library/Caches/io.github.thepigdestroyer.mercury-browser` (the HTTP cache and media files).

### Build the app yourself

You need a Mac with Apple silicon, the Xcode command-line tools (`xcode-select --install`) and [rustup](https://rustup.rs/). The Rust version is pinned in `rust-toolchain.toml` and installs itself on the first build. That build compiles all of Servo and needs 15–20 GB of free disk space while it runs.

```bash
git clone https://github.com/thepigdestroyer/mercury-browser.git
cd mercury-browser
git checkout mercury
make gstreamer-macos   # once: downloads GStreamer 1.28.9 (about 0.9 GB) into your user cache, no sudo
make app-macos         # builds dist/Mercury Browser.app
```

Drag `dist/Mercury Browser.app` to **Applications**, or run it where it is. An app you build yourself is not marked as downloaded, so Gatekeeper does not block it. `make run` builds and starts the browser without making the app. [Development setup](../developer-guide/setup.md) explains the GStreamer step and the other commands.

## Verify a download

Checking a download is optional, and takes a minute. Every release lists these files:

| File | What it is |
| --- | --- |
| `Mercury-Browser-<version>-macos-arm64.dmg` | The macOS disk image |
| `mercury-browser-linux-x86_64.tar.gz` | A Linux x86-64 preview archive, for testing; on Linux the way to run Mercury is still the source build below |
| `Mercury-Browser-<version>-lgpl-sources.tar` | The source of the LGPL libraries bundled in the macOS app |
| `update.json` and `update.json.minisig` | A manifest of the release and its signature, for a future update check |
| `SHA256SUMS`, and a `.sha256` file next to each download | Checksums |

**Check the checksum.** Put the file and its `.sha256` file in one folder and run the command for your system. It prints `OK` when the file is the one that was uploaded:

```bash
shasum -a 256 -c Mercury-Browser-0.2.0-macos-arm64.dmg.sha256
```

```bash
sha256sum -c mercury-browser-linux-x86_64.tar.gz.sha256
```

`SHA256SUMS` lists the same digests for the two downloads, the LGPL sources and `update.json` in one file. To check one file against it, pick out its line (here on macOS; on Linux use `sha256sum -c -`):

```bash
grep ' Mercury-Browser-0.2.0-macos-arm64.dmg$' SHA256SUMS | shasum -a 256 -c -
```

A checksum only shows that the file was not damaged on the way: whoever could change the file could change the checksum next to it. The next check does not have that weakness.

**Check where it was built.** The release workflow attests the build provenance of each file listed in `SHA256SUMS` (the downloads, the LGPL sources and `update.json`) with GitHub's artifact attestations. With the [GitHub CLI](https://cli.github.com/) installed and signed in (`gh auth login`), this checks that GitHub signed a record saying the file you hold was built by a workflow of this repository:

```bash
gh attestation verify Mercury-Browser-0.2.0-macos-arm64.dmg --repo thepigdestroyer/mercury-browser
```

It prints the verification result and exits with an error if there is no matching attestation for the file. The `.sha256` files, `update.json.minisig` and `SHA256SUMS` itself are not attested, and you need none of them to install Mercury. An attestation says where a file came from, not that the software is free of faults or that the repository is trustworthy.

**About `update.json.minisig`.** It is the minisign signature of `update.json`, made with a key the project keeps for its future update check. **Mercury does not check for updates yet**: the browser reads neither file, so you update by downloading the new version yourself and, if you like, verifying it as above. You do not need either file to install Mercury.

## Linux

Linux builds from source and is tested in CI, but it gets less attention than macOS. On Debian or Ubuntu:

```bash
git clone https://github.com/thepigdestroyer/mercury-browser.git
cd mercury-browser
git checkout mercury
make install-linux-deps   # installs the packages CI uses (needs sudo)
make setup
make run
```

`make install-linux-deps` installs a C and C++ toolchain, Clang, CMake, `pkg-config`, Fontconfig, X11 and XKB libraries, and GStreamer with its development headers and the Base, Good, Bad, Ugly, libav and PulseAudio plugin sets. On another distribution, install the equivalent packages; once `pkg-config` finds GStreamer, continue with `make setup` and `make run`.

Media uses the system's GStreamer. If the browser starts but media doesn't play, check the plugins:

```bash
gst-inspect-1.0 playbin3
gst-inspect-1.0 autoaudiosink
gst-inspect-1.0 avdec_h264
```

Each should describe a plugin rather than report that it is missing.

## Windows

Windows is not built at the moment.
