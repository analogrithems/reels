# Reel — features & roadmap

**Maintenance:** When you add or change user-visible editing behavior, update this file in the same PR. AI coding agents (Cursor, Claude) should treat this as the checklist for “what Reel does today.” UI phase status and exit criteria live in **`docs/phases-ui.md`** (see the mapping table there). New contributors: **`docs/CONTRIBUTING.md`**.

## Currently supported (desktop app)

### Playback & transport

- Open a media file or a saved **`.reel` / `.json` project** via **File → Open** (native dialog) or **Ctrl+O** (**⌘O** on macOS). The dialog lists **Video**, **Reel project**, and **All files** filters. The shortcut is handled before media is ready so you can open from an empty window after the main view has focus (click the window once if keys do nothing). **File → Open Recent** lists the last opened projects and media (stored under the OS app data directory; **Clear Recent** empties the list). Missing paths are removed when picked.
- **Play / Pause**; timeline **Slider** scrub (seeks video + audio) with a **two-way playhead** so scrubbing and playback stay aligned. **Volume** slider on the bottom bar (**0–100%**) applies to **preview audio only** (not exported files); the level is saved to **`prefs.json`** in the app data directory and restored on launch. **Space** toggles play/pause when the main view is focused (click the video/timeline area). **← / →** nudge the playhead by **±1 s** (clamped to the sequence) when the main view has focus — not the timeline thumb’s fine steps. **Home** / **End** jump to the **start** or **end** of the sequence. **Ctrl+Z** / **Ctrl+Shift+Z** invoke **Undo** / **Redo** when enabled (**⌘Z** / **⌘⇧Z** on macOS). **Ctrl+B** invokes **Split Clip at Playhead** when enabled (**⌘B** on macOS). **Ctrl+Shift+↓** / **Ctrl+Shift+↑** invoke **Move Clip to Track Below / Above** when those Edit actions are enabled (on **macOS**, Slint uses **⌘** for the `control` modifier, so **⌘⇧↓ / ⌘⇧↑**). A **shortcut table** is in **Help → Keyboard shortcuts** (bundled from `docs/KEYBOARD.md`). The timeline strip shows **playhead / duration** timecode (`M:SS.mmm`).
- **AudioClock**: audio drives timing; video follows (may drop frames when behind).
- **Close** clears the project and stops playback (**Ctrl+W** / **⌘W** when enabled).
- Startup: optional **`REEL_OPEN_PATH`** env var auto-opens one file (dev/testing).

### Viewport

- **View → Loop Playback** — when on, preview **restarts from the beginning** when playback reaches the **end of the primary-track sequence** (same concat as export’s primary video track). The setting is stored in **`prefs.json`** and restored on launch. Shortcut **Ctrl+L** (**⌘L** on macOS); works even before media is loaded.
- **View → Zoom In** / **Zoom Out** (25% steps, **25%–400%** of the **fit** size for the current **Window → Fit** or **Fill** mode), **Zoom to Fit** (contain + 100% scale — same reset as **Window → Fit**), and **Actual Size** (decoded frame drawn at **1:1** logical pixels). Zoom prefs are saved in **`prefs.json`**. Shortcuts **Ctrl+=** / **Ctrl+-** / **Ctrl+0** (**⌘** on macOS for the same keys); zoom shortcuts work without media. When zoomed in, overflow is **clipped** (no pan yet).
- **View → Enter Fullscreen** / **Exit Fullscreen** — toggles platform fullscreen; **Esc** exits fullscreen (and still dismisses export UI when not fullscreen — see **Keyboard shortcuts**).
- **Window → Fit / Fill / Center** (Slint `image-fit`: contain vs cover) — each action also resets preview zoom to **100%** and turns off **Actual Size**, matching **View → Zoom to Fit** semantics for scale.
- **Always on Top**.

### Status footer (when media is loaded)

Below the transport bar, a **single-line footer** shows three regions separated by thin vertical dividers: **video and audio codec** labels for the **primary-track clip at the playhead** (from probe metadata), the **full path** to that clip and the **project file** path (or **Not saved to disk** if there is no `.reel` path yet), and **All changes saved** vs **Unsaved changes** (matches edit dirty state). Content is filled from the **project** as soon as a file or project opens (it does **not** wait for the decoder to finish); the strip stays hidden until **media-ready**. If a **first audio track** is in use, the audio segment reflects the **dedicated** clip at the playhead when present, otherwise **embedded** audio from the video file; when the playhead is past the dedicated audio run, it indicates **silence**. While **playing**, the footer refreshes periodically so codec/path stay correct across clip boundaries. The transport **status** line only shows short messages (e.g. **Loading…**, **Ready**, export/effects feedback)—not paths or codecs (those are in the footer).

### Project & timeline (minimal)

- **One primary video track** in the project model for insert/split math. **Preview** plays the **concatenated** sequence on that track: the timeline slider spans the sum of clip lengths; scrub and play advance across clips (new file opens at each boundary). **File → New Video Track** (**Ctrl+Shift+N** / **⌘⇧N** when media ready) appends an extra empty **video** lane (not yet mixed into preview); **File → New Audio Track** (**Ctrl+Shift+A** / **⌘⇧A** when media ready) appends an empty **audio** lane. The timeline strip shows **one label per video lane** and **one label per audio lane** (clip count and summed duration per lane); multi-track **summary** and paths live in the **status footer** when loaded. **Playback sound** uses the **first audio track** when it has at least one clip (concatenated in sequence time, same clock as the primary video); otherwise sound comes from the **embedded audio** in each primary video clip’s file. If the dedicated audio ends before the video sequence, preview **pads silence** until the video ends. Insert/split for **video** still targets the **primary video** lane only. **Edit → Move Clip to Track Below** moves the clip under the playhead from the primary lane to the **next** video track (requires a second video track and the playhead on a clip, not in a gap). **Edit → Move Clip to Track Above** takes the **first** clip on the **second** video track and appends it to the **end** of the primary track (the lower lane is not in the preview timeline, so lane order is used instead of playhead-on-secondary). Undo/redo applies; if the primary lane becomes empty, preview stops until you add clips or undo.
- **Insert Video…** at playhead: probes the file, appends or inserts a clip on the **primary** (first) video track. If the playhead is **inside** an existing clip, that clip is **split** and the new clip is inserted between the two parts. **Ctrl+I** (**⌘I**) when **media ready**.
- **Insert Audio…** (**File** menu): probes the file and inserts on the **first audio track** at the playhead (same sequence-time rules as insert video). Requires **File → New Audio Track** first. **Ctrl+Shift+I** (**⌘⇧I**) when **Insert Audio** is enabled.
- **Split Clip at Playhead** (**Edit** menu, **Ctrl+B** / **⌘B** when enabled): cuts the primary-track clip at the playhead into two clips (same source, adjusted in/out). Only when the playhead lies **strictly inside** a clip—not in a gap or on a cut (same rule as insert-split).
- **Save…** writes the current `Project` as JSON (`.reel` or `.json` filter). **Ctrl+S** (**⌘S** on macOS) when **Save** is enabled (same as the menu).
- **Revert** restores the last explicit save baseline, or re-probes the original opened media file if never saved.
- **Undo / Redo** (document snapshots): insert and related edits; **explicit Save** clears undo/redo stacks.
- **Autosave**: after a project has been saved once (on-disk path set), edits trigger a **debounced** write to that path (~900 ms after activity). Autosave **does not** clear undo/redo. **Close** attempts a final autosave when a path exists.

### Export

- **Export…** (**Ctrl+E** / **⌘E** when **media ready** and no export is running) opens an **export preset** sheet: pick **MP4** (remux), **WebM** (VP8 + Opus re-encode), or **MKV** (remux), then **Next…** opens a save dialog filtered to that container. The flow remux/transcodes the **primary video track** (all clips in order, respecting each clip’s in/out points) via ffmpeg: one segment uses `-ss`/`-t`; multiple segments use a temporary **concat** list. When the **first audio** lane has clips, **`export_concat_with_audio`** muxes that concat with the video (`-map 0:v:0 -map 1:a:0`), capping duration to the **video** timeline; otherwise **`export_concat_timeline`** exports video only (embedded audio from each video file may still copy). Export runs **off the UI thread**; the bottom bar shows a **blue progress strip** plus status **Exporting… N%** (ffmpeg `-progress` vs timeline duration), then success or error. **File → Cancel Export** or **Esc** requests cancellation while encoding (ffmpeg is interrupted; status shows **Export cancelled.**); **Esc** also dismisses the preset sheet. Stream copy may fail if clips use incompatible codecs—try WebM (re-encode) or align sources.

### Effects (experimental)

- **Effects** menu: **Face Swap (FaceFusion)**, **Face Enhance**, **Remove Background (RVM-style)**.
- Each command decodes **one frame at the playhead**, runs the Python sidecar (`sidecar/facefusion_bridge.py`), and prompts for a **PNG** output path.
- Models include stubs and placeholders; see **`docs/EXTERNAL_AI.md`** for how handoff works (JSON + tempfiles, optional external CLIs). **`docs/CLI.md`** lists CLI flags; **Help → Media formats & tracks** covers decode limits.

### Help

- **Help** menu entries bundle markdown from `docs/` (overview, features, keyboard shortcuts, media formats, CLI, external AI & tools, developers, agents, UI phases). Long topics open in a **scrollable** secondary window. **F1** opens **Help → Overview**. **File** track shortcuts (**New Video Track**, **New Audio Track**, **Insert Audio**) are listed in **Keyboard shortcuts** and **Features**.

---

## Roadmap (not yet in the product)

Priorities shift; this list is indicative. For **phased planning** (U2 sub-milestones **U2-d** …, **U3** presets, **U4** view chrome), see **`docs/phases-ui.md`**.

### File & project

- **Per-entry remove from Open Recent** (optional) — today only **Clear Recent**; no single-row delete.

### Editing / timeline (QuickTime-style)

- **Edit → Rotate 90° Left** / **Rotate 90° Right**; **Flip Horizontal** / **Flip Vertical** (preview + project/export semantics TBD).
- **Seek bar clip range** — **two markers** (in/out) on the timeline scrub bar to define a **begin** and **end** for the working range (export, trim, or preview scope—see **`docs/phases-ui.md` U2-d**).
- **Double-click timeline** — opens **trim** UI: adjust begin/end, **Trim** (commit) or **Cancel** (QuickTime-like).
- **Edit → Resize Video…** — scale to target resolution / preset dimensions.
- **Multi-track** video (multiple `TrackKind::Video` lanes) and **separate audio tracks**: secondary **video** lanes are still not in the video preview; **mixing** multiple audio lanes, **gain**, and **J/L cuts** are open. **U2-b** preview now **switches** to the first audio lane when it has clips (see **Project & timeline** above).
- **Trim / ripple / roll** at playhead beyond the trim sheet; blade tool; slip/slide.
- **Subtitles / captions** import, edit, and burn-in export.
- **Keyframes** and motion/effect parameters per clip.

### Audio (Edit menu)

- **Remove audio** — drop embedded audio from the selected clip(s) or timeline selection (export muxing must match).
- **Replace audio** — substitute another audio file for the clip’s sound.
- **Overlay audio** — mix in an additional track with **independent volume** vs the source (requires **U2-b** mixer work).

### Export & effects

- **Export configuration presets** — named targets derived from **`docs/SUPPORTED_FORMATS.md`**: e.g. **web** (H.264 + AAC-LC MP4, VP9/AV1 WebM when implemented), **mobile-friendly** (HEVC + AAC MP4, etc.), **compatibility remux**, plus resolution/bitrate fields per preset.
- Richer determinate **progress** presentation in the window chrome (status **N%** + thin strip exist today).
- **Batch export**.
- **Real FaceFusion** frame pipeline in the sidecar (beyond import check / stubs).
- **Robust Video Matting (ONNX)** or equivalent for true matting (current `rvm_chroma` is chroma-style).

### View & playback

- **Zoom to Video** (optional semantic — e.g. match **Fill**); **pan** when zoomed past the viewport (today: clip only).
- **Fullscreen** — optional duplicate control on the **playback** toolbar (menu **View → Enter Fullscreen** exists).

### UX / platform

- **Keyboard shortcuts** (full menu parity with common editors): core transport + clip lane moves are partly covered (see **Playback** above).
- **Accessibility** pass, icons, density.
- **macOS app bundle** / notarization story.

### Project I/O

- Optional deeper integration of **`ProjectStore`** from `reel-core` with the app (library already has debounced atomic saves for `project.json`-style usage).

### AI / future

- **AI upsampling / super-resolution** — increase output resolution (export or intermediate) using a model or external tool; complements **Edit → Resize** and **export presets** (see **`docs/phases-ui.md` U5**).

---

## How to update this document

1. Ship a feature → move bullets from **Roadmap** to **Currently supported** (or add a bullet under the right heading).
2. Mention any new **menu paths** and **limitations** (e.g. “first video stream only”).
3. If behavior depends on ffmpeg or codecs, add a short note and link **`docs/MEDIA_FORMATS.md`**.
