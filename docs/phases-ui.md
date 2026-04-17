# UI & editor phases (Reel desktop)

This document is the **product / UI roadmap** for **`reel-app`** (Slint). Engineering/infrastructure phases (0–4) are in **`docs/phase-status.md`**.

**Maintenance:** When a milestone moves, update **this file** and **`docs/FEATURES.md`**. Agents: also touch **`docs/phase-status.md`** if the change reflects completed engineering or doc work described there. See **`docs/AGENTS.md`**.

---

## Executive summary

Reel’s UI work is grouped into **U1–U5**: shell & help (**done** for core scope), **deep editing** (multi-track, trim, **QuickTime-style** transforms & clip UI—**in progress / expanding**), **export UX** (cancel + **%** + **strip** + **3-container preset sheet** shipped; **rich transcode catalog** (H.264/VP9/AV1 tiers)—**open**), **polish** (**File → Open Recent** **done**; shortcuts, a11y, **View** chrome—**partial**), and **AI/effects** (**MVP shipped**; **upscaling** on roadmap). Phases **overlap in time**; the table below is the source of truth for **status**, not strict waterfall ordering.

---

## At a glance

| Phase | Theme | Status | Notes |
|-------|--------|--------|--------|
| **U1** | Shell, menus, timeline scrub, Help | **Done** | Core exit criteria met; stretch shortcuts → **U4** |
| **U2** | Project editing depth (tracks, trim, transforms) | **In progress** | Insert/split/blade **done**; multi-lane **partial**; **planned:** rotate/flip, clip markers, trim-on-double-click, **audio** remove/replace/overlay+gain, **resize** |
| **U3** | Export UX (presets, progress) | **In progress** | Cancel + **N%** + **strip** + **MP4 / WebM / MKV preset sheet** **done**; **planned:** H.264/AAC transcode, VP9/AV1 WebM, resolution/bitrate (see **`SUPPORTED_FORMATS.md` roadmap**) |
| **U4** | Polish (a11y, shortcuts, file & view chrome) | **In progress** (partial) | **File → Open Recent** + **Clear Recent** **done**; **View:** **loop**, **zoom**, **fullscreen** **open**; transport + clip-move keys |
| **U5** | AI & effects in product | **In progress** (MVP) | Frame → sidecar → PNG; **roadmap:** **AI upscale** / super-resolution |

---

## Dependencies & sequencing (informal)

```text
  U1 (shell) ──► U2 (editing depth) ──┬──► U4 (shortcuts polish — easier when edit ops exist)
                  │                  │
                  ├──► U3 (export UX — can start once export paths are stable)
                  │
                  └──► U5 (AI — parallel; already MVP via Effects menu)

  Parking lot (not phase-numbered yet): subtitles, keyframes, motion, **AI video upscale** — see FEATURES roadmap
```

- **U2** unlocks *meaningful* **U4** shortcuts (trim, blade, track ops) once those actions exist.
- **U3** can proceed in parallel with U2 if contributors avoid conflicting `Export` UI refactors.
- **U5** shares **`SidecarClient`** / **`grab_frame`** with the rest of the app; timeline-integrated effects depend on **U2** clip model richness.

---

## Mapping: phases ↔ `FEATURES.md`

| UI phase | Primary place in **`docs/FEATURES.md`** |
|----------|----------------------------------------|
| U1 | Currently supported: Playback, Viewport, Help |
| U2 | Project & timeline (partial) + Roadmap: **QuickTime-style** Edit (rotate, flip, markers, trim sheet, audio, resize) |
| U3 | Roadmap: **Export presets** (web/mobile from `SUPPORTED_FORMATS.md`), batch, progress UI |
| U4 | Roadmap: **File → Open Recent**; **View** (loop, zoom, fullscreen); shortcuts, a11y, bundle |
| U5 | Currently supported: Effects + Roadmap: Export & effects (real models) |

When you ship something, **move or add bullets in `FEATURES.md`** and adjust the **Status** column in the table above.

---

## Phase U1 — Shell, menus, timeline scrub, Help ✅

**Goal:** Usable window with native menus, transport, scrub, and discoverable documentation.

**Exit criteria (met for core scope)**

- User can open media, play/pause, scrub, adjust viewport, use File/Edit/Window menus without crashes on supported files.
- Help opens and shows bundled docs from `docs/`.

**Delivered**

- **MenuBar:** File, Edit, Effects, Window, Help.
- **File:** Open (media or **`.reel` / `.json` project**), **Open Recent** (MRU + **Clear Recent**), Close, Revert, New Window, Save (`.reel` JSON), Insert Video (playhead-aware, **split** when inside a clip), Insert Audio (first audio lane; **U2-b**), New Video Track (empty lane; **U2-a**), New Audio Track (empty lane; **U2-b**), Export (ffmpeg **primary-track** concat / trim).
- **Edit:** Undo / redo (project snapshots).
- **Window:** Always on top; Fit / Fill / Center viewport.
- **Effects:** Menu hooks to sidecar (see **U5**).
- **Help:** Secondary window; topics bundled from `docs/` via `crates/reel-app/src/shell.rs` (overview, features, **keyboard shortcuts**, media formats, **supported formats (playback vs export)**, CLI, external AI & tools, developers, agents, UI phases).
- **Timeline:** `Slider` scrub → same seek as transport.
- **Tests:** Session, project I/O, shell, effects resolve path; reel-core export fixture tests.

**Explicitly deferred**

- Global **keyboard shortcuts** (menu accelerators) → **U4**.

---

## Phase U2 — Project & timeline depth 🚧

**Goal:** Editing that matches a “real” NLE more closely while keeping the data model honest.

**Exit criteria (not all met)**

- [x] Non-destructive **project file** workflow with undo/redo and autosave (path-backed).
- [ ] User can see and edit **more than one logical video/audio lane** in the UI (multi-track). *Progress:* **File → New Video Track** / **New Audio Track** + timeline strip summary + **per-lane labels** for video and audio (clip count / duration); **Move Clip to Track Below / Above** (Edit menu) move between primary and second **video** lane (below = playhead on primary clip; above = first clip on second lane to end of primary). Audio lanes are **not** in preview yet. Not a full per-lane visual editor (waveforms, drag) yet.
- [ ] User can **trim** clips (in/out handles, ripple, etc.). *Progress:* **split without importing** via **Edit → Split Clip at Playhead** / **Ctrl+B** (blade at playhead on the primary track).

**Done**

- In-memory **`Project`** with at least one **video** track; open creates one clip + track.
- **Insert Video** with **split-at-playhead** when the playhead lies inside a clip.
- **Split Clip at Playhead** (blade): two clips from one at the playhead; undoable.
- **Save**, **Revert**, **Undo / Redo** (with explicit Save clearing stacks).
- **Debounced autosave** to the on-disk project path (Slint timer; **preserves** undo vs explicit Save); flush on **Close** when possible.
- **U2-a (partial):** **New Video Track** appends an empty `TrackKind::Video` lane (undoable); timeline strip summarizes the project and lists **each video lane** (primary vs secondary, clip count, summed duration). **Move Clip to Track Below / Above** (Edit menu) shuffle clips between primary and second video lane (see **FEATURES** for exact rules). **Primary-track sequence preview** (concat timeline, play/scrub across clips, auto-advance at boundaries) is **implemented**; secondary lanes are still **not** in the decode graph. Remaining: richer per-lane visuals (waveforms, thumbnails), drag moves, trim.
- **U2-b (partial):** **New Audio Track** appends an empty `TrackKind::Audio` lane (undoable); **Insert Audio…** places clips on that lane; strip lists **each audio lane** (clip count, summed duration). When the **first** audio lane has clips, **playback uses that concat** for sound (else embedded audio from primary video files); silence after audio if video runs longer.

**Suggested sub-milestones (order may vary)**

1. **U2-a — Multi-track preview:** ~~Sequence-across-clips on the primary track~~ **done** for core playback; **New Video Track** + summary + **per-lane labels** **done**; **move clip to next video track** (menu) **done**; remaining: richer per-lane visuals, draggable moves, preview/mix for secondary lanes.
2. **U2-b — Audio in timeline:** **Partial:** **New Audio Track**, **Insert Audio…**, first-lane **concat preview** (switch from embedded video audio when the lane has clips; silence-pad if audio ends early); **open:** multiple audio lanes, mix, levels, trim-on-lane.
3. **U2-c — Trim / ripple:** In/out handles or numeric trim; ripple optional.
4. **U2-d — QuickTime-style Edit menu:** **Rotate 90° Left**, **Rotate 90° Right**, **Flip Horizontal**, **Flip Vertical** (preview + persist in project/export pipeline—implementation TBD). **Seek bar:** **two markers** (in/out) to define a **clip range** for operations and export (same spirit as QuickTime’s selection). **Double-click timeline** opens **trim controls** (begin/end, **Trim** / **Cancel**).
5. **U2-e — Audio (Edit menu):** **Remove** embedded audio from the current clip/selection; **Replace** with an audio file; **Overlay** additional audio with **independent volume** vs the source (builds on **U2-b** mixing/export).
6. **U2-f — Resize:** **Edit → Resize Video…** (target resolution / scale). **AI-assisted upsampling** to higher resolution is **not** in this milestone—see **U5** / parking lot.

**Not yet**

- **Multi-track** video: clips can be **moved to the next video track** via the Edit menu; there is still no **mix** or preview from secondary **video** lanes. **Audio:** the **first** lane can drive **preview sound** when it has clips; additional audio lanes have **no** mix yet.
- **Trim, ripple, roll**, slip/slide; multi-cam (blade **without** new media is **Split Clip at Playhead**). **U2-d** adds **marker-based** range + **double-click trim sheet** as a first pass.
- **Subtitles / captions** timeline (see **FEATURES** roadmap—may become **U6** or fold into U2/U3).
- Optional: adopt **`ProjectStore`** from `reel-core` inside the app (library already implements debounced atomic writes).

---

## Phase U3 — Export UX 📋

**Goal:** User-controlled export presets and feedback.

**Exit criteria (not all met)**

- [ ] User picks **named export configurations** aligned with **`docs/SUPPORTED_FORMATS.md`** — at minimum: **web** targets (e.g. **H.264 + AAC-LC MP4**, **VP9 or AV1 WebM** when codecs exist), **mobile-friendly** tiers (e.g. **HEVC + AAC** MP4 where supported), plus existing **remux** / **compatibility** paths; exact list ships with the preset picker.
- [ ] User picks **preset** (resolution / bitrate / format family) from the app (may merge with the row above).
- [x] Long exports show **cancellable** ffmpeg work without killing the whole app (**Esc** / **File → Cancel Export**).
- [x] **Determinate export feedback** in the main window: status **%** plus a thin **progress strip** above the transport row (ffmpeg `out_time_ms` vs timeline duration).

**Partial / shipped**

- **Export** runs **off the UI thread**; **File → Export…** opens a **preset sheet** (MP4 remux, WebM VP8+Opus, MKV remux), then a filtered save dialog; status line shows **Exporting…**, then **Exporting… N%**, with the **strip** filling left→right, then result (see **`docs/FEATURES.md`**).

**Not started (UI)**

- **Rich preset catalog**: named tiers (e.g. **H.264 + AAC-LC** MP4 transcode, **VP9/AV1** WebM) + resolution / bitrate fields per **`SUPPORTED_FORMATS.md`** roadmap.
- **Batch** export.

*Today:* preset sheet maps 1:1 to existing `WebExportFormat` variants; advanced transcodes remain **ffmpeg CLI / roadmap** work (see **`docs/SUPPORTED_FORMATS.md`**).

---

## Phase U4 — Polish 🚧

**Goal:** Production-quality feel on supported platforms.

**Exit criteria (draft)**

- [x] **File → Open Recent** — MRU list of **recent projects** (`.reel` / `.json`) and **recent media** (same kinds as **File → Open**); **Clear Recent**; missing files pruned on pick. *Optional:* per-entry remove only (not shipped).
- [ ] Core actions reachable via **keyboard** (parity with common editors where feasible). *Progress:* **F1** (Help overview), **Ctrl+O** / **Ctrl+S** / **Ctrl+W** (open / save / close when enabled), **Ctrl+I** / **Ctrl+Shift+I** / **Ctrl+E** / **Ctrl+Shift+N** / **Ctrl+Shift+A** (insert video / insert audio when enabled / export / new video track / new audio track when **media-ready**), **Ctrl+B** (split at playhead when enabled), **Space** (play/pause), **← / →** (±1 s seek), **Home** / **End** (sequence start/end), **Ctrl+Z** / **Ctrl+Shift+Z** (undo/redo when enabled), **Ctrl+Shift+↓/↑** (move clip between primary and second video lane when enabled; **⌘⇧↓/↑** on macOS). Transport and edit shortcuts expect the main view focused; **Open** works from an empty window; **Insert**/**Export**/**New Video Track**/**New Audio Track** need decode ready.
- [ ] **View** menu (new or extended): **Loop Playback** — boolean; when enabled, **loop** the current playback scope (sequence or clip—product decision). **Zoom:** **Zoom In**, **Zoom Out** (enabled when zoomed), **Zoom to Fit** (align with current **Window → Fit** behavior), **Actual Size** (1:1 pixels); optional **Zoom to Video** (semantic TBD—may match **Fill** or max dimension). *Today:* **Window → Fit / Fill / Center** exists; roadmap may consolidate viewport zoom under **View**.
- [ ] **Fullscreen** — a **fullscreen** control on the **playback** chrome (and/or **View → Enter Fullscreen**) using platform-appropriate behavior.
- [ ] **Accessibility** audit pass on main window + dialogs (labels, focus order—scope TBD).

**Scope**

- **File** menu: **Open Recent** (MRU persistence — `recent.json` under the OS data-local dir).
- Keyboard **shortcuts** (including menu parity).
- **View** chrome: loop, zoom ladder, fullscreen.
- **Accessibility** review.
- **Icons**, density, typography pass.
- Optional: **macOS app bundle**, notarization, Linux packaging stories.

---

## Phase U5 — AI & external tools 🚧

**Goal:** Fast iteration on AI/ML without locking to one vendor API.

**Exit criteria**

- [x] **MVP:** One documented path from menu → frame → external process → saved asset (PNG).
- [ ] **Production:** At least one **non-stub** transform users can rely on (e.g. real matting or face pipeline), or clear “experimental” labeling across UI/docs.

**Done (MVP)**

- **Effects** menu: decode **one frame** at playhead → **`SidecarClient`** → save **PNG**.
- **`reel-cli swap`** shares the same pipeline.
- Documented handoff: **`docs/EXTERNAL_AI.md`**, **`docs/CLI.md`**.

**Suggested sub-milestones**

1. **U5-a — Bridge quality:** Harden transforms; reduce passthrough stubs where feasible.
2. **U5-b — Clip or range export:** Export a **segment** to temp file for heavier models (still out-of-process).
3. **U5-c — Timeline:** Effect regions or replacement clips referencing processed media.

**Not yet**

- Full **FaceFusion** (or other) **inference** wired in the bridge beyond stubs/import checks.
- **ONNX RVM** (or similar) for true matting vs chroma stub.
- **Timeline effect clips** or live preview of processed video in the player.
- **AI upsampling / super-resolution** — raise resolution for export (or preview proxy) via external model or service; pairs with **Edit → Resize** and **U3** export presets (see **`docs/FEATURES.md`**).

---

## Parking lot (not assigned to U1–U5 yet)

Items live in **`docs/FEATURES.md`** until we carve **U6+**:

- **Keyframes**, motion paths, per-clip effect parameters.
- Rich **subtitle** authoring and burn-in beyond ffmpeg one-off.
- **Collaboration** / cloud project (no roadmap commitment).
- **AI video upscale** (distinct from **U2-f** pixel resize—see **U5** “Not yet”).

---

## Suggested next focus (rolling)

Priorities change; this is **guidance for contributors**, not a commitment.

1. **U2-d / U2-e** — **QuickTime-style** Edit (rotate, flip, markers, trim sheet) + **audio** remove/replace/overlay (depends on **U2-b** for mix/export).
2. **U3** — **Export preset catalog** from **`SUPPORTED_FORMATS.md`** + determinate **progress bar**.
3. **U4** — **View:** loop, **zoom** ladder, **fullscreen** (Open Recent MRU is shipped).
4. **U5-a** — Bridge quality: one **non-stub** transform or clearly labeled experimental paths (pairs with **U5** exit criteria).

Revisit when **trim UI** or **export presets** land.

---

## How these phases relate to other docs

| Document | Audience | Contents |
|----------|----------|----------|
| **`phases-ui.md`** (this file) | Product / UI roadmap | U1–U5, status, exit criteria, sequencing |
| **`phase-status.md`** | Engineering history | Phases 0–4 (infra, player, sidecar), doc milestones |
| **`FEATURES.md`** | What ships today + backlog | Actionable feature bullets; keep in sync when U* moves |
| **`CONTRIBUTING.md`** | New contributors | Workflow, links here and to **Suggested next focus** |

When you close out a **UI phase** item, update **`FEATURES.md`**, this file, and **`phase-status.md`** when the behavior is user-visible; adjust **`CONTRIBUTING.md`** if contributor workflow or entry points change.

---

*Living document; sub-milestones (U2-a … U5-c) and “Suggested next focus” should be revised when scope shifts.*
