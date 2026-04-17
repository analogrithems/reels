# Reel — overview

**Reel** is an open-source video editor (Rust + Slint + FFmpeg). This file is **Help → Overview** in the app.

## Quick start

1. **File → Open** to load a video.
2. Use **Play / Pause** and drag the **timeline** to scrub.
3. **File → Save…** stores your project as `.reel` JSON. After the first save, **autosave** periodically writes the same path (see **Help → Features & roadmap**).
4. **Effects** runs one-frame AI/sidecar experiments and saves a **PNG** (experimental).

## Environment (optional)

| Variable | Purpose |
|----------|---------|
| `REEL_OPEN_PATH` | If set to a media file path, opens it on launch (dev/testing). |
| `REEL_SIDECAR_DIR` | Override path to `sidecar/` for the desktop app’s Effects pipeline. |
| `REEL_LOG`, `REEL_LOG_FORMAT`, `REEL_LOG_FILE`, `REEL_LOG_SESSION_DIR`, `REEL_LOG_STDOUT` | `tracing`: session log is **always JSON** (NDJSON); `REEL_LOG_FORMAT` only affects **stdout** when mirrored (see `docs/architecture.md`). |
| `FACE_FUSION_ROOT` | Optional FaceFusion checkout for sidecar `facefusion` model. |

## More topics (Help menu)

**F1** opens **Overview** when the main window has keyboard focus (same as **Help → Overview**).

Open **Help** in the menu bar for bundled copies of:

- **About Reel…** — version line and short credits (the **Knot Reels** logo appears at the top of every Help window and as the window icon).
- **Features & roadmap** — what works today vs planned.
- **Keyboard shortcuts** — focus, playback, and multi-track clip moves (`docs/KEYBOARD.md`).
- **Media formats & tracks** — containers, codecs, subtitles (or lack thereof).
- **Supported formats (playback vs export)** — matrix and roadmap (`docs/SUPPORTED_FORMATS.md`).
- **CLI** — `reel-cli probe` / `swap`.
- **External AI & tools** — decode → tempfile RGBA → JSON bridge to child processes (`docs/EXTERNAL_AI.md`).
- **Developers** — build, test, layout (humans).
- **Agent guide** — Cursor / Claude expectations and doc-update duties.
- **UI phases** — phased roadmap (U1–U5).

The canonical sources live under **`docs/`** in the repository (`docs/README.md` indexes them).
