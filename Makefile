.PHONY: setup build lint test run run-cli macos-app macos-app-release macos-app-release-bundled build-deps clean ci check-tools fixtures fmt

SHELL := /bin/bash

# Pin ffmpeg@7 for ffmpeg-next 7.1 compatibility.
FFMPEG7_PREFIX := $(shell brew --prefix ffmpeg@7 2>/dev/null)
export PKG_CONFIG_PATH := $(FFMPEG7_PREFIX)/lib/pkgconfig:$(PKG_CONFIG_PATH)

check-tools:
	@command -v rustup >/dev/null    || { echo "rustup missing: https://rustup.rs"; exit 1; }
	@command -v pkg-config >/dev/null || { echo "pkg-config missing: brew install pkg-config"; exit 1; }
	@command -v uv >/dev/null         || { echo "uv missing: brew install uv"; exit 1; }
	@brew list ffmpeg@7 >/dev/null 2>&1 || { echo "ffmpeg@7 missing: brew install ffmpeg@7"; exit 1; }
	@pkg-config --exists libavformat  || { echo "ffmpeg@7 pkg-config not found at $(FFMPEG7_PREFIX); check PKG_CONFIG_PATH"; exit 1; }
	@echo "tools OK ($(shell pkg-config --modversion libavformat) libavformat)"

setup: check-tools
	rustup show
	cargo fetch
	cd sidecar && uv sync

build:
	cargo build --workspace

fmt:
	cargo fmt --all

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cd sidecar && uv run ruff check .

test:
	cargo test --workspace --all-features
	cd sidecar && uv run pytest -q

# Session logs: reels.session.*.log next to where you invoked make (see docs/architecture.md).
# Optional: make run ARGS='path/to/file.mp4'  (same as: cargo run -p reel-app -- path/to/file.mp4)
run:
	REEL_LOG_SESSION_DIR="$(CURDIR)" cargo run -p reel-app -- $(ARGS)

run-cli:
	REEL_LOG_SESSION_DIR="$(CURDIR)" cargo run -p reel-cli -- $(ARGS)

# macOS: bare `target/*/reel` shows a generic executable icon in Finder. Build a .app with AppIcon.icns for the real Dock/Finder icon.
macos-app:
	./scripts/macos/build_app_bundle.sh debug

macos-app-release:
	./scripts/macos/build_app_bundle.sh release

# Build FFmpeg + GPL deps (pinned in build/deps.toml) into .build/deps/.
# First run is slow (~15-25 min); subsequent runs are no-ops when nothing
# in deps.toml changes.
build-deps:
	./scripts/macos/build_deps.sh

# Release-style bundle: same output shape the GitHub release workflow
# produces locally. Links against the bundled FFmpeg instead of Homebrew
# ffmpeg@7 and ships dylibs + licenses inside Reel.app.
macos-app-release-bundled: build-deps
	PKG_CONFIG_PATH=$(CURDIR)/.build/deps/out/lib/pkgconfig:$(PKG_CONFIG_PATH) \
		./scripts/macos/build_app_bundle.sh release
	./scripts/macos/bundle_deps_into_app.sh

fixtures:
	bash scripts/generate_fixtures.sh

clean:
	cargo clean
	rm -rf sidecar/.venv

ci: lint test
