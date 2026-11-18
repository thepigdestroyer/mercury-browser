---
title: Install Mercury
description: Install Mercury Browser on macOS from the disk image, or build and run it on Linux with the required Rust, graphics, and GStreamer dependencies.
---

# Install Mercury

## macOS (Apple Silicon)

Mercury for macOS runs on Macs with Apple silicon (M1 or later) and macOS 13 Ventura or later. It is distributed as a disk image named like `mercury-browser-0.2.0-macos-arm64.dmg`.

### Install from the disk image

1. Optionally, check the download against the `.sha256` file published next to it:

    ```bash
    shasum -a 256 -c mercury-browser-0.2.0-macos-arm64.dmg.sha256
    ```

2. Double-click the `.dmg` to open it.
3. Drag **Mercury Browser** onto the **Applications** folder shown next to it.
4. Eject the disk image.

### The first launch: allowing Mercury past Gatekeeper

Current Mercury builds are not signed with an Apple Developer ID or notarised by Apple; they carry only an "ad-hoc" signature. macOS marks everything downloaded from the internet, and Gatekeeper blocks a marked app that Apple has not checked. The first time you open Mercury Browser, macOS refuses and offers only **Done** or **Move to Trash**. Choose **Done**, then allow it once:

1. Open **System Settings** and choose **Privacy & Security**.
2. Scroll down to the **Security** section. It shows a line saying that "Mercury Browser" was blocked to protect your Mac.
3. Click **Open Anyway** and confirm with your password or Touch ID.
4. Open Mercury Browser again and click **Open** in the dialog that follows.

macOS remembers the choice; later launches open normally. The button only appears for about an hour after the blocked attempt, so try opening the app again if it is missing.

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
