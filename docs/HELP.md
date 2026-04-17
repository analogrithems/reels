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
| `REEL_LOG`, `REEL_LOG_FORMAT`, `REEL_LOG_FILE` | `tracing` output (see `docs/architecture.md`). |
| `FACE_FUSION_ROOT` | Optional FaceFusion checkout for sidecar `facefusion` model. |

## More topics (Help menu)

Open **Help** in the menu bar for bundled copies of:

- **Features & roadmap** — what works today vs planned.
- **Keyboard shortcuts** — focus, playback, and multi-track clip moves (`docs/KEYBOARD.md`).
- **Media formats & tracks** — containers, codecs, subtitles (or lack thereof).
- **CLI** — `reel-cli probe` / `swap`.
- **External AI & tools** — decode → tempfile RGBA → JSON bridge to child processes (`docs/EXTERNAL_AI.md`).
- **Developers** — build, test, layout (humans).
- **Agent guide** — Cursor / Claude expectations and doc-update duties.
- **UI phases** — phased roadmap (U1–U5).

The canonical sources live under **`docs/`** in the repository (`docs/README.md` indexes them).
