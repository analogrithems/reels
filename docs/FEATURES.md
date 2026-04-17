# Reel — features & roadmap

**Maintenance:** When you add or change user-visible editing behavior, update this file in the same PR. AI coding agents (Cursor, Claude) should treat this as the checklist for “what Reel does today.”

## Currently supported (desktop app)

### Playback & transport

- Open a media file via **File → Open** (native dialog).
- **Play / Pause**; timeline **Slider** scrub (seeks video + audio).
- **AudioClock**: audio drives timing; video follows (may drop frames when behind).
- **Close** clears the project and stops playback.
- Startup: optional **`REEL_OPEN_PATH`** env var auto-opens one file (dev/testing).

### Viewport

- **Window → Fit / Fill / Center** (Slint `image-fit`: contain vs cover).
- **Always on Top**.

### Project & timeline (minimal)

- **One primary video track** in the project model; playback uses the **first clip** on that track.
- **Insert Video…** at playhead: probes the file, appends or inserts a clip. If the playhead is **inside** an existing clip, that clip is **split** and the new clip is inserted between the two parts.
- **Save…** writes the current `Project` as JSON (`.reel` or `.json` filter).
- **Revert** restores the last explicit save baseline, or re-probes the original opened media file if never saved.
- **Undo / Redo** (document snapshots): insert and related edits; **explicit Save** clears undo/redo stacks.
- **Autosave**: after a project has been saved once (on-disk path set), edits trigger a **debounced** write to that path (~900 ms after activity). Autosave **does not** clear undo/redo. **Close** attempts a final autosave when a path exists.

### Export

- **Export…** remux/transcode the **current primary media** to MP4 / WebM / MKV via ffmpeg (see `reel_core::export`).

### Effects (experimental)

- **Effects** menu: **Face Swap (FaceFusion)**, **Face Enhance**, **Remove Background (RVM-style)**.
- Each command decodes **one frame at the playhead**, runs the Python sidecar (`sidecar/facefusion_bridge.py`), and prompts for a **PNG** output path.
- Models include stubs and placeholders; see **`docs/EXTERNAL_AI.md`** for how handoff works (JSON + tempfiles, optional external CLIs). **`docs/CLI.md`** lists CLI flags; **Help → Media formats & tracks** covers decode limits.

### Help

- **Help** menu entries bundle markdown from `docs/` (overview, features, media formats, CLI, external AI & tools, developers, agents, UI phases).

---

## Roadmap (not yet in the product)

Priorities shift; this list is indicative.

### Editing / timeline

- **Multi-track** video and **separate audio tracks** in the UI (schema has `TrackKind`; app still assumes a single main video track for preview).
- **Trim / ripple / roll** at playhead; blade tool; slip/slide.
- **Subtitles / captions** import, edit, and burn-in export.
- **Keyframes** and motion/effect parameters per clip.

### Export & effects

- Export **presets** (resolution, bitrate), **progress**, **cancel**.
- **Batch export**.
- **Real FaceFusion** frame pipeline in the sidecar (beyond import check / stubs).
- **Robust Video Matting (ONNX)** or equivalent for true matting (current `rvm_chroma` is chroma-style).

### UX / platform

- **Keyboard shortcuts** (menu parity with common editors).
- **Accessibility** pass, icons, density.
- **macOS app bundle** / notarization story.

### Project I/O

- Optional deeper integration of **`ProjectStore`** from `reel-core` with the app (library already has debounced atomic saves for `project.json`-style usage).

---

## How to update this document

1. Ship a feature → move bullets from **Roadmap** to **Currently supported** (or add a bullet under the right heading).
2. Mention any new **menu paths** and **limitations** (e.g. “first video stream only”).
3. If behavior depends on ffmpeg or codecs, add a short note and link **`docs/MEDIA_FORMATS.md`**.
