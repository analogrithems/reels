# UI & editor phases (Reel desktop)

This document is the **product / UI roadmap** for **`reel-app`** (Slint). Engineering/infrastructure phases (0–4) are in **`docs/phase-status.md`**.

**Maintenance:** When a milestone moves, update **this file** and **`docs/FEATURES.md`**. Agents: also touch **`docs/phase-status.md`** if the change reflects completed engineering or doc work described there. See **`docs/AGENTS.md`**.

---

## Executive summary

Reel’s UI work is grouped into **U1–U5**: shell & help (**done** for core scope), **deep editing** (multi-track, **per-clip rotate/flip** **shipped**; **Trim Clip…** numeric sheet **shipped**; seek-bar **in/out markers**, **ripple**, and **on-timeline trim handles** still **open**), **export UX** (cancel + **%** + **strip** + **3-container preset sheet** shipped; **rich transcode catalog** (H.264/VP9/AV1 tiers)—**open**), **polish** (**File → Open Recent** **done**; shortcuts, a11y, **View** chrome—**partial**), and **AI/effects** (**MVP shipped**; **upscaling** on roadmap). Phases **overlap in time**; the table below is the source of truth for **status**, not strict waterfall ordering.

---

## At a glance

| Phase | Theme | Status | Notes |
|-------|--------|--------|--------|
| **U1** | Shell, menus, timeline scrub, Help | **Done** | Core exit criteria met; stretch shortcuts → **U4** |
| **U2** | Project editing depth (tracks, trim, transforms) | **In progress** | Insert/split/blade **done**; **rotate/flip** **shipped** (per-clip, preview + export); **Trim Clip…** sheet **shipped** (**Edit** menu); multi-lane **partial**; **planned:** clip markers, **audio** remove/replace/overlay+gain, **resize** |
| **U3** | Export UX (presets, progress) | **In progress** | Cancel + **N%** + **strip** + **MP4 / WebM / MKV preset sheet** **done**; **planned:** H.264/AAC transcode, VP9/AV1 WebM, resolution/bitrate (see **`SUPPORTED_FORMATS.md` roadmap**) |
| **U4** | Polish (a11y, shortcuts, file & view chrome) | **In progress** (partial) | **File → Open Recent** + **Clear Recent** **done**; **View** — **Loop** (prefs + **Ctrl+L** / **⌘L**), **zoom** ladder + **Actual Size**, **Enter/Exit Fullscreen** (**Esc** exits) **done** (prefs for zoom); optional **Zoom to Video** / pan when zoomed / fullscreen on playback chrome **open**; transport + clip-move keys; **QuickTime-style** floating **volume + transport + scrub + times** over the **video** (auto-hide ~5 s idle); stepped speeds — see **U4** body |
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

- **U2** unlocks *meaningful* **U4** shortcuts (blade, **Trim Clip…**, rotate, track moves) once those actions exist; further **U2** work adds more shortcut targets (e.g. ripple trim, **U2-e**).
- **U3** can proceed in parallel with U2 if contributors avoid conflicting `Export` UI refactors.
- **U5** shares **`SidecarClient`** / **`grab_frame`** with the rest of the app; timeline-integrated effects depend on **U2** clip model richness.

---

## Mapping: phases ↔ `FEATURES.md`

| UI phase | Primary place in **`docs/FEATURES.md`** |
|----------|----------------------------------------|
| U1 | Currently supported: Playback, Viewport, Help |
| U2 | Project & timeline (partial) + Roadmap: **QuickTime-style** Edit — **rotate/flip** and **Trim Clip…** in **Currently supported**; seek-bar **in/out markers**, **audio** remove/replace/overlay, **resize**, **ripple** still roadmap |
| U3 | Roadmap: **Export presets** (web/mobile from `SUPPORTED_FORMATS.md`), batch, progress UI |
| U4 | **Currently supported:** **File → Open Recent**; **View** (loop, zoom, fullscreen) + partial shortcuts — see **Viewport** / **Playback** in **`FEATURES.md`**. **Roadmap:** a11y, bundle, optional zoom pan / toolbar fullscreen |
| U5 | Currently supported: Effects + Roadmap: Export & effects (real models) |

When you ship something, **move or add bullets in `FEATURES.md`** and adjust the **Status** column in the table above.

---

## Logging standards (requirement)

**Infrastructure** (session NDJSON file, `REEL_LOG` / `RUST_LOG`, optional stdout mirror) is documented in **`docs/architecture.md`** and delivered under **engineering Phase 0** in **`docs/phase-status.md`**. This section is the **product/engineering requirement going forward**: new and changed code should make the **session log** useful for support and debugging—not only on crashes.

### Levels (`tracing`)

Use the **`tracing`** crate only (no ad-hoc `println!` in libraries). `tracing` does **not** define a **critical** level; use **`error!`** for failures that are fatal or must surface immediately.

| Level | Use for |
|-------|---------|
| **error** | Unrecoverable or user-visible failure; broken invariants; operations that cannot complete |
| **warn** | Recoverable problems, degraded mode, unexpected but handled conditions, child **stderr** (existing helpers) |
| **info** | Process lifecycle, **user-visible actions** (open/save/export start and outcome), long-running work boundaries |
| **debug** | Detailed diagnostics (per-operation detail, cancellation, state transitions) when **`REEL_LOG`** enables it (defaults often filter this) |
| **trace** | Extremely verbose paths (e.g. per-tick)—**rare**; prefer **debug** unless noise would drown real issues |

### What to log

- **Failures:** Any path that returns `Err` to the user or leaves the app in a bad state should emit **warn** or **error** with context (`error = %e`, paths, ids)—not only `unwrap` sites.
- **User flows:** Menu actions and dialogs that represent real work (**open**, **save**, **export**, **insert**, **effects**) should log **info** at start and on **success or failure** (export already has high-level hooks; extend **core** paths such as **`reel_core::media::export`** and **session** logic as those areas evolve).
- **Hot loops:** Playback decode threads should **not** spam **info**; use **debug**/**trace** or sample if needed. **warn**/**error** remain appropriate for real failures.

### Review expectation

PRs that add or materially change **behavior** in **`reel-app`**, **`reel-cli`**, or **`reel-core`** should include **appropriate `tracing` calls** so a default **info** session log shows **what happened** and **what failed**. Pure refactors and mechanical fixes may omit new logs unless they touch error paths.

---

## Phase U1 — Shell, menus, timeline scrub, Help ✅

**Goal:** Usable window with native menus, transport, scrub, and discoverable documentation.

**Exit criteria (met for core scope)**

- User can open media, play/pause, scrub, adjust viewport, use File/Edit/Window menus without crashes on supported files.
- Help opens and shows bundled docs from `docs/`.

**Delivered**

- **MenuBar:** File, Edit, Effects, View, Window, Help.
- **File:** Open (media or **`.reel` / `.json` project**), **Open Recent** (MRU + **Clear Recent**), Close Window, Revert, New Window, Save (`.reel` JSON), Insert Video (playhead-aware, **split** when inside a clip), Insert Audio (first audio lane; **U2-b**), New Video Track (empty lane; **U2-a**), New Audio Track (empty lane; **U2-b**), Export (ffmpeg **primary-track** concat / trim).
- **Edit:** Undo / redo (project snapshots).
- **Window:** Always on top; Fit / Fill / Center viewport.
- **Effects:** Menu hooks to sidecar (see **U5**).
- **Help:** Secondary window; topics bundled from `docs/` via `crates/reel-app/src/shell.rs` (overview, features, **keyboard shortcuts**, media formats, **supported formats (playback vs export)**, CLI, external AI & tools, developers, agents, UI phases).
- **Timeline:** `Slider` scrub → same seek as transport.
- **Footer:** single-line strip (**codecs** · **paths** · **saved / unsaved**) for the clip at the playhead, using per-clip probe metadata; recorded as completed **follow-on** work under **engineering Phase 1** in **`docs/phase-status.md`** (out of original engine scope).
- **Logging:** Every run writes **`reels.session.<timestamp>.log`** under the OS data directory (`reel/logs/`) as **NDJSON** (structured fields + module **target**, **file**, **line**); optional terminal mirror when stdout is a TTY (**pretty** or **json** per `REEL_LOG_FORMAT`). See **`docs/architecture.md`**. **Going forward**, new work must follow **`Logging standards (requirement)`** above—not only session file plumbing, but **coverage** at the right **levels** for user flows and failures.
- **Tests:** Session, project I/O, shell, effects resolve path; reel-core export fixture tests.

**Explicitly deferred** (relative to original U1 scope)

- Global **keyboard shortcuts** — **partially delivered** under **U4** (see **U4** exit criteria and **`docs/KEYBOARD.md`**); full menu parity + a11y remains **U4** / polish.

---

## Phase U2 — Project & timeline depth 🚧

**Goal:** Editing that matches a “real” NLE more closely while keeping the data model honest.

**Cross-links:** User-visible behavior and limits → **`docs/FEATURES.md`** (**Project & timeline**). Edit shortcuts (including **Trim Clip…** — menu / double-click only) → **`docs/KEYBOARD.md`**.

### U2 — progress snapshot (rolling)

| Area | Shipped | Still open |
|------|---------|------------|
| **Tracks / lanes** | Primary-track video concat **preview**; **New Video / Audio Track**; per-lane **strip labels**; **Move Clip** between primary ↔ second **video** lane | Preview/mix from **secondary video** lanes; **multiple audio lanes** mixed; draggable moves |
| **Audio** | **First** audio lane **concat** drives preview when it has clips (else embedded video audio); silence-pad after audio ends | **U2-e** remove/replace/overlay + per-lane **gain**; multi-lane **mix** |
| **Trim** | **Blade**; **Trim Clip…** numeric sheet (**Edit** menu + **double-click** video lane strip) | Timeline **in/out handles**, **ripple**, seek-bar **in/out markers** for working range |
| **Transform** | Per-clip **rotate / flip** (preview + export **`-vf`**) | — |
| **Resize** | — | **U2-f** **Edit → Resize Video…** |

**Exit criteria (not all met)**

- [x] Non-destructive **project file** workflow with undo/redo and autosave (path-backed).
- [ ] User can see and edit **more than one logical video/audio lane** in the UI (multi-track). *Progress:* **File → New Video Track** / **New Audio Track** + **Move Clip to Track Below / Above** (Edit menu) move between primary and second **video** lane (below = playhead on primary clip; above = first clip on second lane to end of primary). **First** audio lane drives **preview sound** when it has clips (else embedded primary-track audio); additional audio lanes have **no** mix yet. Secondary **video** lanes are still **not** in the preview decode graph. Not a full per-lane visual editor (waveforms, drag) yet.
- [ ] User can **trim** in the NLE sense (handles, ripple, range on the scrub bar). *Shipped:* **blade** (**Split at Playhead** / **Ctrl+B**); **per-clip numeric trim** (**Trim Clip…** — **`docs/FEATURES.md`**). *Still open:* timeline **in/out handles**, **ripple**, seek-bar **in/out markers** for export/trim scope.

**Done**

- In-memory **`Project`** with at least one **video** track; open creates one clip + track.
- **Insert Video** with **split-at-playhead** when the playhead lies inside a clip.
- **Split Clip at Playhead** (blade): two clips from one at the playhead; undoable.
- **Save**, **Revert**, **Undo / Redo** (with explicit Save clearing stacks).
- **Debounced autosave** to the on-disk project path (Slint timer; **preserves** undo vs explicit Save); **Close Window** prompts to save when dirty.
- **Per-clip orientation** (**Edit → Rotate 90°** / **Flip**): stored in the project; preview after the scaler; export uses ffmpeg **`-vf`** when any primary-track clip is non-identity (mixed orientations in one export remain a limitation—see **`docs/FEATURES.md`**).
- **Trim Clip…** (**U2-c / U2-d**): sheet with begin/end in **source-file seconds**, inline validation, undoable; see **`docs/FEATURES.md`** and **`docs/KEYBOARD.md`**.
- **U2-a (partial):** **New Video Track** appends an empty `TrackKind::Video` lane (undoable). **Move Clip to Track Below / Above** (Edit menu) shuffle clips between primary and second video lane (see **FEATURES** for exact rules). **Primary-track sequence preview** (concat timeline, play/scrub across clips, auto-advance at boundaries) is **implemented**; secondary lanes are still **not** in the decode graph. Remaining: richer per-lane visuals (waveforms, thumbnails), draggable moves, **on-timeline** trim handles (numeric **Trim Clip…** shipped).
- **U2-b (partial):** **New Audio Track** appends an empty `TrackKind::Audio` lane (undoable); **Insert Audio…** places clips on that lane. When the **first** audio lane has clips, **playback uses that concat** for sound (else embedded audio from primary video files); silence after audio if video runs longer.

**Suggested sub-milestones (order may vary)**

1. **U2-a — Multi-track preview:** ~~Sequence-across-clips on the primary track~~ **done** for core playback; **New Video Track** + summary + **per-lane labels** **done**; **move clip to next video track** (menu) **done**; remaining: richer per-lane visuals, draggable moves, preview/mix for secondary lanes.
2. **U2-b — Audio in timeline:** **Partial:** **New Audio Track**, **Insert Audio…**, first-lane **concat preview** (switch from embedded video audio when the lane has clips; silence-pad if audio ends early); **open:** multiple audio lanes, mix, levels, trim-on-lane.
3. **U2-c — Trim / ripple:** **Numeric trim** via **Trim Clip…** **shipped**; still open: **in/out handles** on the timeline, **ripple**, seek-bar **range markers** tied to trim/export scope.
4. **U2-d — QuickTime-style Edit menu:** **Rotate 90° Left** / **Rotate 90° Right** / **Flip Horizontal** / **Flip Vertical** — **shipped** (per-clip orientation persists in the project; applied in preview post-scaler and in ffmpeg export via `-vf`). **Trim Clip…** sheet — **shipped** (**Edit → Trim Clip…**; numeric begin/end in source-file seconds, inline validation, undoable). Still open: **two seek-bar markers** (in/out) to define a **clip range** for operations and export (same spirit as QuickTime’s selection).
5. **U2-e — Audio (Edit menu):** **Remove** embedded audio from the current clip/selection; **Replace** with an audio file; **Overlay** additional audio with **independent volume** vs the source (builds on **U2-b** mixing/export).
6. **U2-f — Resize:** **Edit → Resize Video…** (target resolution / scale). **AI-assisted upsampling** to higher resolution is **not** in this milestone—see **U5** / parking lot.

**Not yet**

- **Multi-track** video: clips can be **moved to the next video track** via the Edit menu; there is still no **mix** or preview from secondary **video** lanes. **Audio:** the **first** lane can drive **preview sound** when it has clips; additional audio lanes have **no** mix yet.
- **Ripple, roll**, slip/slide; multi-cam (blade **without** new media is **Split Clip at Playhead**). **U2-d**'s **double-click trim sheet** is **shipped** as the first pass; the **marker-based** range is still open.
- **Subtitles / captions** timeline (see **FEATURES** roadmap—may become **U6** or fold into U2/U3).
- Optional: adopt **`ProjectStore`** from `reel-core` inside the app (library already implements debounced atomic writes).

---

## Phase U3 — Export UX 📋

**Goal:** User-controlled export presets and feedback.

**Exit criteria (not all met)**

- [ ] User picks **named export configurations** aligned with **`docs/SUPPORTED_FORMATS.md`** — at minimum: **web** targets (e.g. **H.264 + AAC-LC MP4**, **VP9 or AV1 WebM** when codecs exist), **mobile-friendly** tiers (e.g. **HEVC + AAC** MP4 where supported), plus existing **remux** / **compatibility** paths; exact list ships with the preset picker.
- [ ] User picks **preset** (resolution / bitrate / format family) from the app (may merge with the row above).
- [x] Long exports show **cancellable** ffmpeg work without killing the whole app (**Esc** / **Cancel export** on the progress modal).
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
- [ ] Core actions reachable via **keyboard** (parity with common editors where feasible). *Progress:* **F1** (Help overview), **Ctrl+O** / **Ctrl+S** / **Ctrl+W** (open / save / close when enabled), **Ctrl+I** / **Ctrl+Shift+I** / **Ctrl+E** / **Ctrl+Shift+N** / **Ctrl+Shift+A** (insert video / insert audio when enabled / export / new video track / new audio track when **media-ready**), **Ctrl+B** (split at playhead when enabled), **Space** (play/pause), **Ctrl+L** (toggle **View → Loop Playback**; works without media), **Ctrl+=** / **Ctrl+-** / **Ctrl+0** (zoom in / out / zoom to fit — work without media), **← / →** (±1 s seek), **Home** / **End** (sequence start/end), **Ctrl+Z** / **Ctrl+Shift+Z** (undo/redo when enabled), **Ctrl+Shift+↓/↑** (move clip between primary and second video lane when enabled; **⌘⇧↓/↑** on macOS). Transport and edit shortcuts expect the main view focused; **Open** works from an empty window; **Insert**/**Export**/**New Video Track**/**New Audio Track** need decode ready.
- [x] **View** menu: **Loop Playback** — when enabled, playback **seeks to the start** and continues at the **end of the primary-track sequence** (same scope as export’s primary video concat). State is saved in **`prefs.json`**; shortcut **Ctrl+L** (**⌘L** on macOS).
- [x] **View** menu — **Zoom In** / **Zoom Out** (25% steps, 25%–400% of **fit** size), **Zoom to Fit** (contain + 100%; aligns with **Window → Fit**), **Actual Size** (1:1 logical pixels for the decoded frame). **Window → Fit / Fill / Center** also resets zoom to **100%** non-actual. Zoom state is saved in **`prefs.json`**. Shortcuts **Ctrl+=** / **Ctrl+-** / **Ctrl+0** (**⌘** on macOS).
- [x] **View → Enter Fullscreen** / **Exit Fullscreen** — platform window fullscreen (**Esc** exits). Optional: duplicate control on the **playback** chrome — not shipped.
- [ ] **View** (optional / polish): **Zoom to Video** (semantic TBD); **pan** when zoomed past the viewport (today overflow is **clipped**).
- [ ] **Accessibility** audit pass on main window + dialogs (labels, focus order—scope TBD).

**Reliability (shipped — keep in sync with code)**

- **Timeline scrub:** `playhead-ms` is **in-out** with the timeline **`Slider`** (two-way bind); **`step: 0`** on that slider so **← / →** stay on the main **FocusScope** (**±1 s** nudge), not the slider’s own 1 ms steps. **`timecode::clamp_playhead_ms`** clamps player + UI to **[0, duration]** (unit tests in `crates/reel-app/src/timecode.rs`).
- **Help window:** **ScrollView** fills the window so long bundled topics **scroll** correctly.

**Opportunistic / ahead of formal U4 chrome (not original exit criteria)**

These were **not** planned as named U4 deliverables; they improve everyday preview without blocking other milestones:

- **Preview playback speed (0.25×–2× forward and reverse):** **Volume**, **rewind**, **play/pause**, and **fast-forward** in a **floating bar** over the **bottom of the video**; **elapsed** / **total** (**tenths of a second**) flank the **scrub bar** on the second row of that bar. The bar **hides after ~5 s** without pointer movement over the **viewport**; movement or click **shows** it again. **Rewind** / **fast-forward** increase speed on **repeated clicks**; **export** is unchanged.
- **Transport:** Icon-style **play** / **pause** with **rewind** and **fast-forward** adjacent; status text lives in the **thin bar** below the timeline (not duplicated under the video).

Treat these as **nice-to-have polish** overlapping **U4** (transport feel) that does **not** close remaining **U4** items (a11y audit, optional zoom pan / toolbar fullscreen, etc.).

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

1. **U2-d / U2-e** — Seek-bar **in/out markers** (remaining U2-d work); **audio** remove/replace/overlay (depends on **U2-b** for mix/export). **Rotate/flip** and **Trim Clip…** sheet are **shipped** (see **FEATURES.md**).
2. **U3** — **Export preset catalog** (H.264/AAC, VP9/AV1 tiers, etc.) from **`SUPPORTED_FORMATS.md`**; determinate **%** + strip already shipped — further **chrome** polish optional.
3. **U4** — **a11y** audit; optional **View** polish (pan when zoomed, fullscreen on playback chrome). (**Open Recent**, **View** loop/zoom/fullscreen, timeline scrub reliability — shipped.)
4. **U5-a** — Bridge quality: one **non-stub** transform or clearly labeled experimental paths (pairs with **U5** exit criteria).

Revisit when **seek-bar range markers**, **ripple trim**, or **export presets** land.

---

## How these phases relate to other docs

| Document | Audience | Contents |
|----------|----------|----------|
| **`phases-ui.md`** (this file) | Product / UI roadmap | U1–U5, status, exit criteria, sequencing, **logging standards** |
| **`phase-status.md`** | Engineering history | Phases 0–4 (infra, player, sidecar), doc milestones; **Phase 1** includes completed **status-footer** follow-on (probe metadata in UI) |
| **`FEATURES.md`** | What ships today + backlog | Actionable feature bullets; keep in sync when U* moves |
| **`CONTRIBUTING.md`** | New contributors | Workflow, links here and to **Suggested next focus** |

When you close out a **UI phase** item, update **`FEATURES.md`**, this file, and **`phase-status.md`** when the behavior is user-visible; adjust **`CONTRIBUTING.md`** if contributor workflow or entry points change.

---

*Living document; sub-milestones (U2-a … U5-c) and “Suggested next focus” should be revised when scope shifts.*
