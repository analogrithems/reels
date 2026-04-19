# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-04-18

### Added

- **Timeline** — video scene thumbnails on primary-track chips; **track previews** for video lanes; **click-to-seek** on the filmstrip strip.
- **Playback** — A/V sync calibration, microsecond-accurate audio clock improvements, and an **audio waveform** scaffold on the timeline.

### Changed

- **Export** — extended export format options and related UI.
- **Controls** — broader keyboard shortcuts and A/V offset handling; timeline and floating control polish.

### Fixed

- Timeline UI cleanup and layout tweaks.

## [0.1.0] - 2026-04-18

First public release of **Reel**, an open-source Rust + Slint + FFmpeg desktop video editor (macOS-first).

### Added

- **Desktop app (`reel`)** — timeline editing, preview playback, multi-format export (MP4/WebM/MKV presets), `.reel` project JSON, autosave, undo/redo, trim/split/rotate, multi-audio-lane export mix, Effects menu (experimental sidecar).
- **CLI (`reel-cli`)** — `probe` and related helpers for headless workflows.
- **Open on launch** — optional single file path: `reel <path>` or `REEL_OPEN_PATH`; `make run ARGS='…'` for development.
- **macOS `.app` bundle** — `make macos-app` / `scripts/macos/build_app_bundle.sh` for Dock/Finder icon (see `docs/DEVELOPERS.md`).

### Notes

- **Platform:** CI and release artifacts target **macOS (Apple Silicon)**. Other platforms are not built in GitHub Actions yet.
- **FFmpeg:** The app expects **FFmpeg 7.x** on `PATH` or via `pkg-config` (e.g. `brew install ffmpeg@7`). It is not bundled inside the `.app`.
- **Code signing / notarization:** Release zips are **unsigned**; gatekeeper may require right-click → Open on first launch.
