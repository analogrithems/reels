# UI & editor phases (Reel desktop)

This document is the **product / UI roadmap** for **`reel-app`** (Slint). Engineering/infrastructure phases (0–4) are in **`docs/phase-status.md`**.

**Maintenance:** When a milestone moves, update **this file** and **`docs/FEATURES.md`**. Agents: also touch **`docs/phase-status.md`** if the change reflects completed engineering or doc work described there. See **`docs/AGENTS.md`**.

---

## Executive summary

Reel’s UI work is grouped into **U1–U5**: shell & help (**largely complete**), **deep editing** (multi-track, trim—**in progress**), **export UX** (presets & progress—**not started**), **polish** (shortcuts, a11y—**partial**: transport + clip-move keys), and **AI/effects** (**MVP shipped**, full pipelines **open**). Phases **overlap in time**; the table below is the source of truth for **status**, not strict waterfall ordering.

---

## At a glance

| Phase | Theme | Status | Notes |
|-------|--------|--------|--------|
| **U1** | Shell, menus, timeline scrub, Help | **Done** (core) | Stretch: keyboard shortcuts → **U4** |
| **U2** | Project editing depth (tracks, trim) | **In progress** | Autosave + insert/split **done**; **U2-a** partial (multi-lane labels + **move clip to next track**); trim **open** |
| **U3** | Export UX (presets, progress) | **Not started** | Remux today is **File → Export** without presets UI |
| **U4** | Polish (a11y, shortcuts, icons) | **In progress** (partial) | Transport + **Ctrl+Shift+↓/↑** clip moves; a11y / icons **open** |
| **U5** | AI & effects in product | **In progress** (MVP) | Frame → sidecar → PNG; full pipelines **open** |

---

## Dependencies & sequencing (informal)

```text
  U1 (shell) ──► U2 (editing depth) ──┬──► U4 (shortcuts polish — easier when edit ops exist)
                  │                  │
                  ├──► U3 (export UX — can start once export paths are stable)
                  │
                  └──► U5 (AI — parallel; already MVP via Effects menu)

  Parking lot (not phase-numbered yet): subtitles, keyframes, motion — see FEATURES roadmap
```

- **U2** unlocks *meaningful* **U4** shortcuts (trim, blade, track ops) once those actions exist.
- **U3** can proceed in parallel with U2 if contributors avoid conflicting `Export` UI refactors.
- **U5** shares **`SidecarClient`** / **`grab_frame`** with the rest of the app; timeline-integrated effects depend on **U2** clip model richness.

---

## Mapping: phases ↔ `FEATURES.md`

| UI phase | Primary place in **`docs/FEATURES.md`** |
|----------|----------------------------------------|
| U1 | Currently supported: Playback, Viewport, Help |
| U2 | Currently supported: Project & timeline (partial) + Roadmap: Editing / timeline, Project I/O |
| U3 | Roadmap: Export & effects (presets, batch) |
| U4 | Roadmap: UX / platform (shortcuts, a11y, bundle) |
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
- **File:** Open, Close, Revert, New Window, Save (`.reel` JSON), Insert Video (playhead-aware, **split** when inside a clip), New Video Track (empty lane; **U2-a**), Export (ffmpeg **primary-track** concat / trim).
- **Edit:** Undo / redo (project snapshots).
- **Window:** Always on top; Fit / Fill / Center viewport.
- **Effects:** Menu hooks to sidecar (see **U5**).
- **Help:** Secondary window; topics bundled from `docs/` via `crates/reel-app/src/shell.rs` (overview, features, **keyboard shortcuts**, media formats, CLI, external AI & tools, developers, agents, UI phases).
- **Timeline:** `Slider` scrub → same seek as transport.
- **Tests:** Session, project I/O, shell, effects resolve path; reel-core export fixture tests.

**Explicitly deferred**

- Global **keyboard shortcuts** (menu accelerators) → **U4**.

---

## Phase U2 — Project & timeline depth 🚧

**Goal:** Editing that matches a “real” NLE more closely while keeping the data model honest.

**Exit criteria (not all met)**

- [x] Non-destructive **project file** workflow with undo/redo and autosave (path-backed).
- [ ] User can see and edit **more than one logical video/audio lane** in the UI (multi-track). *Progress:* **File → New Video Track** + timeline strip summary + **per-lane labels** (clip count / duration); **Move Clip to Track Below / Above** (Edit menu) move between primary and second video lane (below = playhead on primary clip; above = first clip on second lane to end of primary). Not a full per-lane visual editor (waveforms, drag) yet.
- [ ] User can **trim** or **split** clips for edit intent beyond insert+split-at-playhead (e.g. trim handles, ripple).

**Done**

- In-memory **`Project`** with at least one **video** track; open creates one clip + track.
- **Insert Video** with **split-at-playhead** when the playhead lies inside a clip.
- **Save**, **Revert**, **Undo / Redo** (with explicit Save clearing stacks).
- **Debounced autosave** to the on-disk project path (Slint timer; **preserves** undo vs explicit Save); flush on **Close** when possible.
- **U2-a (partial):** **New Video Track** appends an empty `TrackKind::Video` lane (undoable); timeline strip summarizes the project and lists **each video lane** (primary vs secondary, clip count, summed duration). **Move Clip to Track Below / Above** (Edit menu) shuffle clips between primary and second video lane (see **FEATURES** for exact rules). **Primary-track sequence preview** (concat timeline, play/scrub across clips, auto-advance at boundaries) is **implemented**; secondary lanes are still **not** in the decode graph. Remaining: richer per-lane visuals (waveforms, thumbnails), drag moves, trim.

**Suggested sub-milestones (order may vary)**

1. **U2-a — Multi-track preview:** ~~Sequence-across-clips on the primary track~~ **done** for core playback; **New Video Track** + summary + **per-lane labels** **done**; **move clip to next video track** (menu) **done**; remaining: richer per-lane visuals, draggable moves, preview/mix for secondary lanes.
2. **U2-b — Audio in timeline:** Expose **Audio** `TrackKind` in UI; mix or switch preview path.
3. **U2-c — Trim / ripple:** In/out handles or numeric trim; ripple optional.

**Not yet**

- **Multi-track** video: clips can be **moved to the next video track** via the Edit menu; there is still no **mix** or preview from secondary lanes, and audible **audio tracks** in the UI are **not** exposed yet.
- **Trim, ripple, roll, blade**, slip/slide; multi-cam.
- **Subtitles / captions** timeline (see **FEATURES** roadmap—may become **U6** or fold into U2/U3).
- Optional: adopt **`ProjectStore`** from `reel-core` inside the app (library already implements debounced atomic writes).

---

## Phase U3 — Export UX 📋

**Goal:** User-controlled export presets and feedback.

**Exit criteria (not all met)**

- [ ] User picks **preset** (resolution / bitrate / format family) from the app.
- [ ] Long exports show **determinate progress** (percent) and are **cancellable** without killing the whole app.

**Partial / shipped**

- **Export** runs **off the UI thread**; status line shows **Exporting…** then result (see **`docs/FEATURES.md`**).

**Not started (UI)**

- Resolution / bitrate **presets**, **determinate progress bar**, **cancel** button.
- **Batch** export.

*Today:* **File → Export…** uses ffmpeg concat / trim without a presets panel (see **`docs/FEATURES.md`**).

---

## Phase U4 — Polish 🚧

**Goal:** Production-quality feel on supported platforms.

**Exit criteria (draft)**

- [ ] Core actions reachable via **keyboard** (parity with common editors where feasible). *Progress:* **Ctrl+O** / **Ctrl+S** / **Ctrl+W** (open / save / close when enabled), **Ctrl+I** / **Ctrl+E** (insert / export when **media-ready**), **Space** (play/pause), **← / →** (±1 s seek), **Home** / **End** (sequence start/end), **Ctrl+Z** / **Ctrl+Shift+Z** (undo/redo when enabled), **Ctrl+Shift+↓/↑** (move clip between primary and second video lane when enabled; **⌘⇧↓/↑** on macOS). Transport and edit shortcuts expect the main view focused; **Open** works from an empty window; **Insert**/**Export** need decode ready.
- [ ] **Accessibility** audit pass on main window + dialogs (labels, focus order—scope TBD).

**Scope**

- Keyboard **shortcuts** (including menu parity).
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

---

## Parking lot (not assigned to U1–U5 yet)

Items live in **`docs/FEATURES.md`** until we carve **U6+**:

- **Keyframes**, motion paths, per-clip effect parameters.
- Rich **subtitle** authoring and burn-in beyond ffmpeg one-off.
- **Collaboration** / cloud project (no roadmap commitment).

---

## Suggested next focus (rolling)

Priorities change; this is **guidance for contributors**, not a commitment.

1. **U2-b** — Audio in timeline / richer multi-track UI (builds on **U2-a**).
2. **U5-a** — Bridge quality: one **non-stub** transform or clearly labeled experimental paths (pairs with **U5** exit criteria).
3. **U3** — Export preset + progress UI: high user visibility once ffmpeg export paths stay stable.

Revisit this list when **U2-b** starts or when export UI work starts in earnest.

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
