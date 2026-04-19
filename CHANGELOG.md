# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3] - 2026-04-19

### Added

- **Subtitles** — TTML (`.ttml` / `.dfxp`) subtitle import. The loader now dispatches on extension between SRT, WebVTT, and TTML; cues are normalized to the same internal representation so live preview, trim, and burn-in on export all work identically.
- **Preview** — **pan when zoomed**. When the scaled image overflows the preview rect (zoom-in past fit, or **Actual Size** on a ≥ viewport-sized source), left-button drag on the preview shifts the frame via ephemeral `preview-pan-x` / `preview-pan-y` offsets. Cursor switches to `grab` / `grabbing` while pannable; offsets clamp to the image edge so the frame can't be dragged off-screen. Pan resets on **View → Zoom to Fit**, **Actual Size**, and all **Window → Fit / Fill / Center** entries.
- **Accessibility (U4)** — sheet **Confirm / Cancel** buttons on **Trim Clip…**, **Audio Lane Gain…**, and **Resize Video…** now carry explicit `accessible-label`s that name the action being confirmed (not just the bare button text). The six **Resize** preset percent buttons (25 / 50 / 75 / 100 / 150 / 200 %) also get contextual labels so screen-reader users hear what each `%` button does.

### Fixed

- **Playhead ↔ filmstrip alignment** — the preview frame and timeline playhead now track the same time cursor precisely during scrub and playback.

### Docs

- **macOS Gatekeeper dialog** — added a **First launch on macOS (unsigned build)** subsection to `README.md` explaining the *"Reel.app is damaged"* dialog that testers see after downloading a release zip in a browser. Documents the single-line `xattr -d com.apple.quarantine` fix and a `find | xargs` fallback for older macOS where `xattr` lacks `-r`.

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
- **Code signing / notarization:** Release zips are **unsigned** and **not notarized**. On modern macOS, Gatekeeper will refuse first launch with *"Reel.app is damaged and can't be opened"* — this is a Gatekeeper refusal, not actual damage. See **[README → First launch on macOS](README.md#first-launch-on-macos-unsigned-build)** for the one-line `xattr` workaround.
