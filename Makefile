# SPDX-License-Identifier: MPL-2.0
SHELL := /bin/sh

.DEFAULT_GOAL := help

CARGO ?= cargo
MKDOCS ?= mkdocs
# Used by "make docs-install" only.
PYTHON ?= python3
RUSTUP ?= rustup
STRIP ?= strip
APP_PACKAGE ?= mercury-desktop
ARGS ?=
RUST_LOG ?= talaria_servo=info,servo=warn
DIST_DIR ?= dist
PROJECT_VERSION ?= 0.2.0
LINUX_PACKAGE_NAME := mercury-browser-v$(PROJECT_VERSION)-linux-x86_64
LINUX_ARCHIVE := $(LINUX_PACKAGE_NAME).tar.gz
CI_TOOLS_DIR ?= target/ci-tools
# The pinned CI tools from `make ci-tools` are found before any other copy.
CI_TOOLS_PATH = PATH="$(abspath $(CI_TOOLS_DIR))/bin:$$PATH"

# macOS builds use a private, unpacked copy of the official GStreamer framework
# (scripts/macos/gstreamer.sh; `make gstreamer-macos` prepares it). Recipes that
# compile or run Servo load its environment first; the output of `gstreamer.sh env`
# is captured before it is evaluated, so a missing or broken prefix stops the recipe
# instead of letting pkg-config find another GStreamer. Executables get their
# GStreamer libraries copied into target/<profile>/lib, and test binaries, which
# have no rpath to those libraries, find that directory through
# DYLD_FALLBACK_LIBRARY_PATH. It is set on the cargo command line itself because
# macOS strips DYLD_* variables when the SIP-protected make and /bin/sh start.
# CARGO_FLAGS adds flags to check, lint and test (for example --release).
CARGO_FLAGS ?=
PROFILE_DIR = $(if $(filter --release,$(CARGO_FLAGS)),release,debug)
ifeq ($(shell uname -s),Darwin)
GSTREAMER_ENV = gst_env="$$(scripts/macos/gstreamer.sh env)" && eval "$$gst_env" &&
COLLECT_GSTREAMER = scripts/macos/collect-gstreamer.sh "$$MERCURY_GSTREAMER_PREFIX"
TEST_LIBRARY_PATH = DYLD_FALLBACK_LIBRARY_PATH="$(CURDIR)/target/$(PROFILE_DIR)/lib"
else
GSTREAMER_ENV =
COLLECT_GSTREAMER =
TEST_LIBRARY_PATH =
endif

# blocklists: the filter lists are data files beside the executable, not compiled into it
# (the mercury-filter-lists tool checks their SHA-256 against the manifest and copies them).
INSTALL_FILTER_LISTS = "$(CARGO)" run --quiet --package mercury-filter-lists --bin mercury-filter-lists
INSTALL_FILTER_LISTS_RELEASE = $(INSTALL_FILTER_LISTS) --release

# The credits page: the licence text, copyright line and notices of every crate built into the
# application (crates/mercury-credits). It is generated at package time from Cargo.lock and never
# committed. It needs the crate sources (`cargo fetch --locked`, or any build) but compiles no
# engine and needs no GStreamer. The products are the macOS app (aarch64-apple-darwin) and the
# Linux archive (x86_64-unknown-linux-gnu); CREDITS_TARGET is the one of this host, for the
# `credits` and `credits-check` targets.
CREDITS = "$(CARGO)" run --quiet --locked $(CARGO_FLAGS) --package mercury-credits --bin mercury-credits --
ifeq ($(shell uname -s),Darwin)
CREDITS_TARGET = aarch64-apple-darwin
else
CREDITS_TARGET = x86_64-unknown-linux-gnu
endif

.PHONY: \
	help \
	doctor \
	setup \
	install-linux-deps \
	gstreamer-macos \
	pdfium-macos \
	test-media-macos \
	fetch \
	check \
	fmt \
	fmt-check \
	lint \
	test \
	qa \
	qa-fast \
	ci-tools \
	lint-scripts \
	servo-patches-check \
	hsts-preload-refresh \
	hsts-preload-check \
	vendored-tests \
	deny \
	credits \
	credits-check \
	build \
	build-release \
	package-linux \
	app-macos \
	package-macos \
	test-bundle-macos \
	test-capture-macos \
	test-certificate-exceptions-macos \
	test-servo-net \
	run \
	run-debug \
	run-release \
	docs-install \
	docs \
	docs-serve \
	rust-docs \
	rust-docs-open \
	tree \
	clean

help:
	@printf '%-22s %s\n' \
		'help' 'Show the available commands.' \
		'doctor' 'Check the required development tools.' \
		'setup' 'Install Rust components and fetch dependencies.' \
		'install-linux-deps' 'Install Linux dependencies for the current Slint backend.' \
		'gstreamer-macos' 'Download and unpack GStreamer for macOS (no sudo).' \
		'pdfium-macos' 'Download and unpack PDFium (the PDF viewer library) for macOS (no sudo).' \
		'test-media-macos' 'Check the macOS GStreamer scripts (needs gstreamer-macos for all checks).' \
		'fetch' 'Download Cargo dependencies without building.' \
		'check' 'Type-check every workspace target and feature.' \
		'fmt' 'Format all Rust source files.' \
		'fmt-check' 'Check Rust formatting without changing files.' \
		'lint' 'Run Clippy and reject every warning.' \
		'test' 'Run all workspace tests, the gaol shared-memory test and the vendored servo-net tests.' \
		'test-servo-net' 'Run the vendored servo-net tests (certificate exceptions among them; no engine build, no window).' \
		'qa' 'Run formatting, linting, and tests.' \
		'qa-fast' 'Check formatting, lint and test the crates that need no engine.' \
		'ci-tools' 'Download the pinned shellcheck, actionlint and cargo-deny.' \
		'lint-scripts' 'Lint scripts/ (shellcheck), the workflows (actionlint), licence headers and vendored crates.' \
		'servo-patches-check' 'Check that each crate in third_party/ is the published crate plus its exported patch.' \
		'hsts-preload-refresh' 'Rebuild the HSTS preload list from Chromium (needs the network; a release-time step).' \
		'hsts-preload-check' 'Check that the shipped HSTS preload list is fit to ship (HSTS_MAX_AGE_DAYS=N limits its age).' \
		'vendored-tests' 'Run the unit tests of the vendored Servo crates that need no engine (preferences, message types).' \
		'deny' 'Check dependencies against the policy in deny.toml.' \
		'credits' 'Write the credits page (the licence text of every crate in the app) to target/<profile>/credits.' \
		'credits-check' 'Check that every crate in the app can be credited (needs the crate sources: make fetch).' \
		'build' 'Build the complete workspace in debug mode.' \
		'build-release' 'Build the complete workspace in release mode.' \
		'package-linux' 'Build and package the Linux x86-64 release.' \
		'app-macos' 'Build the release and assemble the signed Mercury Browser.app.' \
		'package-macos' 'Build Mercury Browser.app and its .dmg for macOS arm64.' \
		'test-bundle-macos' 'Check the macOS bundle script helpers (no build needed).' \
		'test-capture-macos' 'Check camera and microphone capture with fake devices (needs build-release).' \
		'test-certificate-exceptions-macos' 'Check that a certificate exception is bound to its host (needs build-release).' \
		'run' 'Run the optimized Mercury Desktop build.' \
		'run-debug' 'Run Mercury Desktop with development debug information.' \
		'run-release' 'Alias for the optimized run target.' \
		'docs-install' 'Install the MkDocs documentation dependencies.' \
		'docs' 'Build the MkDocs site with strict validation.' \
		'docs-serve' 'Serve the MkDocs site with live reload.' \
		'rust-docs' 'Build the Rust API documentation.' \
		'rust-docs-open' 'Build and open the Rust API documentation.' \
		'tree' 'Display the dependency tree.' \
		'clean' 'Remove Cargo build artifacts.'

doctor:
	@command -v "$(CARGO)" >/dev/null 2>&1 || { \
		printf '%s\n' 'error: Cargo was not found.' >&2; \
		exit 1; \
	}
	@command -v "$(RUSTUP)" >/dev/null 2>&1 || { \
		printf '%s\n' 'error: rustup was not found.' >&2; \
		exit 1; \
	}
	@command -v rustc >/dev/null 2>&1 || { \
		printf '%s\n' 'error: rustc was not found.' >&2; \
		exit 1; \
	}
	@printf '%s\n' 'Development tools:'
	@rustc --version
	@"$(CARGO)" --version
	@"$(RUSTUP)" show active-toolchain
	@if [ "$$(uname -s)" = Darwin ]; then \
		scripts/macos/gstreamer.sh check || { \
			printf '%s\n' 'error: GStreamer for macOS is not set up; run: make gstreamer-macos' >&2; \
			exit 1; \
		}; \
	fi

# On macOS GStreamer is fetched first: doctor checks it, and nothing builds without it.
ifeq ($(shell uname -s),Darwin)
SETUP_FIRST = gstreamer-macos
else
SETUP_FIRST =
endif

setup: $(SETUP_FIRST) doctor
	"$(RUSTUP)" component add rustfmt clippy
	"$(CARGO)" fetch --locked

install-linux-deps:
	@command -v apt-get >/dev/null 2>&1 || { \
		printf '%s\n' 'error: this target currently supports apt-based distributions only.' >&2; \
		exit 1; \
	}
	@printf '%s\n' 'Installing the same packages CI uses (.github/workflows/ci.yml). The' \
		'clang pinning follows the runner (clang-14/libclang-14-dev on Ubuntu 22.04);' \
		'releases that ship a different clang need the matching package names here.'
	sudo apt-get update
	sudo apt-get install --yes \
		build-essential \
		clang-14 \
		cmake \
		gstreamer1.0-libav \
		gstreamer1.0-plugins-bad \
		gstreamer1.0-plugins-base \
		gstreamer1.0-plugins-good \
		gstreamer1.0-tools \
		gstreamer1.0-plugins-ugly \
		gstreamer1.0-pulseaudio \
		libclang-14-dev \
		libfontconfig1-dev \
		libgstreamer1.0-dev \
		libgstreamer-plugins-base1.0-dev \
		libgstreamer-plugins-bad1.0-dev \
		libunwind-dev \
		libx11-dev \
		libx11-xcb-dev \
		libxcursor-dev \
		libxkbcommon-x11-dev \
		ninja-build \
		pkg-config \
		xinput

gstreamer-macos:
	scripts/macos/gstreamer.sh fetch
	scripts/macos/gstreamer.sh unpack
	scripts/macos/gstreamer.sh check

# PDFium for the PDF viewer (scripts/macos/pdfium.sh): this Mac's architecture, or arm64 for the
# application bundle (below). The viewer finds the library in this cache while it runs from the
# build tree, and in Contents/MacOS/lib inside the bundle.
PDFIUM_ARCH_ARGS = $(if $(PDFIUM_ARCH),--arch $(PDFIUM_ARCH))
pdfium-macos:
	scripts/macos/pdfium.sh $(PDFIUM_ARCH_ARGS) fetch
	scripts/macos/pdfium.sh $(PDFIUM_ARCH_ARGS) unpack
	scripts/macos/pdfium.sh $(PDFIUM_ARCH_ARGS) check

test-media-macos:
	scripts/macos/test-media.sh

fetch:
	"$(CARGO)" fetch --locked

check:
	$(GSTREAMER_ENV) "$(CARGO)" check $(CARGO_FLAGS) --workspace --all-targets --all-features

fmt:
	"$(CARGO)" fmt --all

fmt-check:
	"$(CARGO)" fmt --all -- --check

lint:
	$(GSTREAMER_ENV) "$(CARGO)" clippy $(CARGO_FLAGS) \
		--workspace \
		--all-targets \
		--all-features \
		-- \
		-D warnings

test:
	$(if $(COLLECT_GSTREAMER),$(GSTREAMER_ENV) "$(CARGO)" build $(CARGO_FLAGS) --package "$(APP_PACKAGE)")
	$(if $(COLLECT_GSTREAMER),$(GSTREAMER_ENV) $(COLLECT_GSTREAMER) target/$(PROFILE_DIR)/lib)
	$(GSTREAMER_ENV) $(TEST_LIBRARY_PATH) "$(CARGO)" test $(CARGO_FLAGS) --workspace --all-features $(if $(TEST_ARGS),-- $(TEST_ARGS))
	"$(CARGO)" test --target-dir target --manifest-path third_party/gaol/Cargo.toml --test shared-memory
	"$(MAKE)" test-servo-net

# servo-net's own tests (third_party/servo-net/tests/main.rs, 420 tests run, the certificate-exception ones among
# them), part of `make test` and run by their own CI job. The crate is vendored, has dev-dependencies and is
# excluded from the workspace (Cargo.toml), so cargo runs it on its own manifest and its own Cargo.lock,
# like gaol: against the published sibling crates (only servo-net is patched), not Mercury's vendored
# copies. Two tests fail without any Mercury change and are skipped by name: `fetch::test_file` reads a path
# that exists only in Servo's repository, and `filemanager_thread::test_filemanager` panics on the
# process-wide async runtime when it runs with the others. No GStreamer and no window; the first build takes
# about 11 minutes on 12 threads, the tests a few seconds. Set RUST_TEST_THREADS=2 on a shared machine.
test-servo-net:
	"$(CARGO)" test --locked $(CARGO_FLAGS) --target-dir target \
		--manifest-path third_party/servo-net/Cargo.toml \
		--features test-util,devtools --test main -- \
		--exact --skip fetch::test_file --skip filemanager_thread::test_filemanager

qa:
	"$(MAKE)" fmt-check
	"$(MAKE)" lint
	"$(MAKE)" test

qa-fast:
	"$(MAKE)" fmt-check
	@packages="$$(scripts/ci/light-crates.sh)" || exit 1; \
	printf 'engine-free crates: %s\n' "$$packages"; \
	"$(CARGO)" clippy --locked --release $$packages --all-targets -- -D warnings && \
	"$(CARGO)" test --locked --release $$packages

ci-tools:
	scripts/ci/install-tools.sh --dir "$(CI_TOOLS_DIR)"

lint-scripts:
	@$(CI_TOOLS_PATH); export PATH; \
	missing=; \
	for tool in shellcheck actionlint; do \
		command -v "$$tool" >/dev/null 2>&1 || missing="$$missing $$tool"; \
	done; \
	if [ -n "$$missing" ]; then \
		printf 'error: not found:%s\n' "$$missing" >&2; \
		printf '%s\n' \
			'Run "make ci-tools" to download the pinned versions into $(CI_TOOLS_DIR)/bin' \
			'(their checksums are verified), or install shellcheck and actionlint yourself' \
			'at the versions in scripts/ci/install-tools.sh.' >&2; \
		exit 1; \
	fi; \
	printf 'shellcheck %s (%s), actionlint %s (%s)\n' \
		"$$(shellcheck --version | sed -n 's/^version: //p')" "$$(command -v shellcheck)" \
		"$$(actionlint -version | sed -n 1p)" "$$(command -v actionlint)"; \
	find scripts -type f -name '*.sh' -print0 | xargs -0 shellcheck -x && \
	actionlint && \
	scripts/ci/check-license-headers.sh && \
	"$(MAKE)" servo-patches-check && \
	printf '%s\n' 'scripts, workflows, licence headers and vendored crates: no findings'

# Engine-free: builds only the small mercury-servo-patches tool. It downloads a published
# crate from crates.io when Cargo's cache lacks it (vendored crates never are in it).
servo-patches-check:
	"$(CARGO)" run --locked --quiet $(CARGO_FLAGS) --package mercury-servo-patches -- check

# The HSTS preload list (crates/talaria-servo/resources/hsts_preload.fstmap and its manifest).
# hsts-preload-refresh downloads Chromium's list, so it needs the network: it is a release-time
# maintainer step that CI never runs, and it leaves the changes uncommitted. HSTS_CHROMIUM_REF is a
# Chromium branch, tag or commit (a commit hash pins the refresh). hsts-preload-check is
# engine-free and reads the file only; HSTS_MAX_AGE_DAYS=N makes a list retrieved longer ago fail.
HSTS_CHROMIUM_REF ?= main

hsts-preload-refresh:
	scripts/dev/update-hsts-preload.sh "$(HSTS_CHROMIUM_REF)"

hsts-preload-check:
	"$(CARGO)" run --locked --quiet $(CARGO_FLAGS) --package mercury-devtools --bin hsts-preload -- check crates/talaria-servo/resources/hsts_preload.fstmap$(if $(HSTS_MAX_AGE_DAYS), --max-age-days $(HSTS_MAX_AGE_DAYS))

# The vendored crates' own unit tests that need neither GStreamer nor a window, so that this also
# runs on the Linux CI runner: Servo's preferences, the Stylo preference bridge and the message
# types the browser and the engine exchange. They are not part of `make test` (a vendored crate is
# not a workspace member). The tests of servo-script and servo-webgl (`cargo test -p servo-script --lib`,
# `-p servo-webgl --lib`) build the engine and are run on a Servo upgrade, not here; servo has
# dev-dependencies, which cargo refuses outside its own workspace (servo-net's own tests are
# `make test-servo-net`, since it is excluded from the workspace like gaol). `--lib`: the
# doc comments of servo-embedder-traits are examples written for the whole of Servo and do not
# compile as doc tests on their own (as published), so only the unit tests run.
vendored-tests:
	"$(CARGO)" test --locked --release --lib \
		-p servo-config \
		-p stylo_static_prefs \
		-p servo-embedder-traits \
		-p servo-constellation-traits

deny:
	@$(CI_TOOLS_PATH); export PATH; \
	if ! command -v cargo-deny >/dev/null 2>&1; then \
		printf '%s\n' \
			'error: cargo-deny not found.' \
			'Run "make ci-tools" to download the pinned version into $(CI_TOOLS_DIR)/bin' \
			'(its checksum is verified), or install cargo-deny yourself at the version in' \
			'scripts/ci/install-tools.sh.' >&2; \
		exit 1; \
	fi; \
	cargo-deny --locked check --show-stats

# Engine-free: builds only the small mercury-credits tool. `credits-check` stops at the first
# crate in the app that has no licence text it may carry; `credits` also writes the page.
# Neither runs in build, run or test, so the development loop stays fast.
credits-check:
	$(CREDITS) check --package "$(APP_PACKAGE)" --target "$(CREDITS_TARGET)"

credits:
	$(CREDITS) generate --package "$(APP_PACKAGE)" --target "$(CREDITS_TARGET)" --out "target/$(PROFILE_DIR)/credits"

build:
	$(GSTREAMER_ENV) "$(CARGO)" build --workspace
	$(if $(COLLECT_GSTREAMER),$(GSTREAMER_ENV) $(COLLECT_GSTREAMER) target/debug/lib)
	$(INSTALL_FILTER_LISTS) -- install target/debug/filter-lists

build-release:
	$(GSTREAMER_ENV) "$(CARGO)" build --workspace --release
	$(if $(COLLECT_GSTREAMER),$(GSTREAMER_ENV) $(COLLECT_GSTREAMER) target/release/lib)
	$(INSTALL_FILTER_LISTS_RELEASE) -- install target/release/filter-lists

package-linux: build-release
	mkdir -p "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)"
	cp "target/release/mercury-browser" "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/mercury-browser"
	"$(STRIP)" --strip-unneeded "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/mercury-browser"
# blocklists: the filter lists are data files beside the program, never compiled into it.
	@if LC_ALL=C grep -q -a -F -e "advanced-ads-" -e "scorecardresearch" -e "Brave Firstparty specific filters" "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/mercury-browser"; then \
		echo "error: mercury-browser contains filter-list text" >&2; exit 1; \
	fi
	cp README.md "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/README.md"
	cp LICENSE "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/LICENSE"
	cp THIRD-PARTY-NOTICES.md "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/THIRD-PARTY-NOTICES.md"
	rm -rf "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/LICENSES"
	cp -R LICENSES "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/LICENSES"
# The credits page (licence text and notices of every crate in the executable), beside it as
# credits/credits.html. Checked with the rule bundle.sh uses (credits_problem in scripts/macos/lib.sh),
# which also fails when the page is missing.
	$(CREDITS) generate --package "$(APP_PACKAGE)" --target x86_64-unknown-linux-gnu --out "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/credits"
	@bash -c '. scripts/macos/lib.sh; problem="$$(credits_problem "$$1")"; [ -z "$$problem" ] || { printf "error: %s\n" "$$problem" >&2; exit 1; }' credits-page "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/credits/credits.html"
	$(INSTALL_FILTER_LISTS_RELEASE) -- install "$(DIST_DIR)/$(LINUX_PACKAGE_NAME)/filter-lists"
	tar -C "$(DIST_DIR)" -czf "$(DIST_DIR)/$(LINUX_ARCHIVE)" "$(LINUX_PACKAGE_NAME)"
	cd "$(DIST_DIR)" && sha256sum "$(LINUX_ARCHIVE)" > "$(LINUX_ARCHIVE).sha256"

REQUIRE_MACOS = @[ "$$(uname -s)" = Darwin ] || { \
		printf '%s\n' 'error: $@ needs macOS; this host is not macOS.' >&2; \
		exit 1; \
	}

# The bundle needs its media libraries (bundle.sh refuses to build without a GStreamer
# prefix); both steps run with the GStreamer environment, and the prefix it names is passed on.
# It needs the credits page too (bundle.sh refuses without it): generated here for the app's
# own target, whatever this host is, into target/release/credits, which is passed on. Needs the
# crate sources, which the build has just fetched. The signing job of the release workflow has no
# cargo and takes the page from the build job instead.
# The bundle is arm64 only and carries the arm64 PDFium, whatever Mac builds it (pdf-viewer).
app-macos package-macos: PDFIUM_ARCH = arm64
app-macos package-macos: pdfium-macos
app-macos:
	$(REQUIRE_MACOS)
	$(GSTREAMER_ENV) "$(CARGO)" build --release --package "$(APP_PACKAGE)"
	$(CREDITS) generate --package "$(APP_PACKAGE)" --target aarch64-apple-darwin --out target/release/credits
	$(GSTREAMER_ENV) scripts/macos/bundle.sh --out "$(DIST_DIR)" --gstreamer-prefix "$$MERCURY_GSTREAMER_PREFIX" --credits target/release/credits

package-macos:
	$(REQUIRE_MACOS)
	$(GSTREAMER_ENV) "$(CARGO)" build --release --package "$(APP_PACKAGE)"
	$(CREDITS) generate --package "$(APP_PACKAGE)" --target aarch64-apple-darwin --out target/release/credits
	$(GSTREAMER_ENV) scripts/macos/bundle.sh --out "$(DIST_DIR)" --gstreamer-prefix "$$MERCURY_GSTREAMER_PREFIX" --credits target/release/credits --dmg

test-bundle-macos:
	$(REQUIRE_MACOS)
	scripts/macos/test-bundle.sh

# media-capture: fake devices only, from a copy whose lib lacks the real capture plugins.
test-capture-macos:
	$(REQUIRE_MACOS)
	scripts/macos/test-capture.sh

# cert-exceptions: loopback only; the page must load from the accepted name and fail from the other.
test-certificate-exceptions-macos:
	$(REQUIRE_MACOS)
	scripts/macos/test-certificate-exceptions.sh

run:
	$(if $(COLLECT_GSTREAMER),$(GSTREAMER_ENV) "$(CARGO)" build --package "$(APP_PACKAGE)" --release)
	$(if $(COLLECT_GSTREAMER),$(GSTREAMER_ENV) $(COLLECT_GSTREAMER) target/release/lib)
	$(INSTALL_FILTER_LISTS_RELEASE) -- install target/release/filter-lists
	$(GSTREAMER_ENV) RUST_LOG="$(RUST_LOG)" "$(CARGO)" run --package "$(APP_PACKAGE)" --release -- $(ARGS)

run-debug:
	$(if $(COLLECT_GSTREAMER),$(GSTREAMER_ENV) "$(CARGO)" build --package "$(APP_PACKAGE)")
	$(if $(COLLECT_GSTREAMER),$(GSTREAMER_ENV) $(COLLECT_GSTREAMER) target/debug/lib)
	$(INSTALL_FILTER_LISTS) -- install target/debug/filter-lists
	$(GSTREAMER_ENV) RUST_LOG="$(RUST_LOG)" "$(CARGO)" run --package "$(APP_PACKAGE)" -- $(ARGS)

run-release: run

docs-install:
	"$(PYTHON)" -m pip install -r docs/requirements.txt

docs:
	"$(MKDOCS)" build --strict

docs-serve:
	"$(MKDOCS)" serve

rust-docs:
	$(GSTREAMER_ENV) "$(CARGO)" doc --workspace --no-deps

rust-docs-open:
	$(GSTREAMER_ENV) "$(CARGO)" doc --workspace --no-deps --open

tree:
	"$(CARGO)" tree --workspace

clean:
	"$(CARGO)" clean
