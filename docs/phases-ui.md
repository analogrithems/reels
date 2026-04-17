# UI & editor phases (Reel desktop)

This document is the **product / UI roadmap** for **`reel-app`** (Slint). Engineering/infrastructure phases (0–4) are in **`docs/phase-status.md`**.

**Maintenance:** When a milestone moves, update **this file** and **`docs/FEATURES.md`**. Agents: also touch **`docs/phase-status.md`** if the change reflects completed engineering or doc work described there. See **`docs/AGENTS.md`**.

---

## At a glance

| Phase | Theme | Status | Notes |
|-------|--------|--------|--------|
| **U1** | Shell, menus, timeline scrub, Help | **Done** (core) | Stretch: keyboard shortcuts → **U4** |
| **U2** | Project editing depth (tracks, trim) | **In progress** | Autosave + insert/split **done**; multi-track / trim **open** |
| **U3** | Export UX (presets, progress) | **Not started** | Remux today is **File → Export** without presets UI |
| **U4** | Polish (a11y, shortcuts, icons) | **Not started** | Depends partly on U2 stability |
| **U5** | AI & effects in product | **In progress** (MVP) | Frame → sidecar → PNG; full pipelines **open** |

---

## Phase U1 — Shell, menus, timeline scrub, Help ✅

**Goal:** Usable window with native menus, transport, scrub, and discoverable documentation.

**Delivered**

- **MenuBar:** File, Edit, Effects, Window, Help.
- **File:** Open, Close, Revert, New Window, Save (`.reel` JSON), Insert Video (playhead-aware, **split** when inside a clip), Export (ffmpeg).
- **Edit:** Undo / redo (project snapshots).
- **Window:** Always on top; Fit / Fill / Center viewport.
- **Effects:** Menu hooks to sidecar (see **U5**).
- **Help:** Secondary window; topics bundled from `docs/` via `crates/reel-app/src/shell.rs` (overview, features, media formats, CLI, external AI & tools, developers, agents, UI phases).
- **Timeline:** `Slider` scrub → same seek as transport.
- **Tests:** Session, project I/O, shell, effects resolve path; reel-core export fixture tests.

**Explicitly deferred**

- Global **keyboard shortcuts** (menu accelerators) → **U4**.

---

## Phase U2 — Project & timeline depth 🚧

**Goal:** Editing that matches a “real” NLE more closely while keeping the data model honest.

**Done**

- In-memory **`Project`** with at least one **video** track; open creates one clip + track.
- **Insert Video** with **split-at-playhead** when the playhead lies inside a clip.
- **Save**, **Revert**, **Undo / Redo** (with explicit Save clearing stacks).
- **Debounced autosave** to the on-disk project path (Slint timer; **preserves** undo vs explicit Save); flush on **Close** when possible.

**Not yet**

- **Multi-track** video and audible **audio tracks** in the UI (schema has `TrackKind`; player still keys off the first video clip for preview).
- **Trim, ripple, roll, blade**, slip/slide; multi-cam.
- Optional: adopt **`ProjectStore`** from `reel-core` inside the app (library already implements debounced atomic writes).

---

## Phase U3 — Export UX 📋

**Goal:** User-controlled export presets and feedback.

**Not started (UI)**

- Resolution / bitrate **presets**, **progress**, **cancel**.
- **Batch** export.

*Today:* **File → Export…** calls ffmpeg helpers without a presets panel (see **`docs/FEATURES.md`**).

---

## Phase U4 — Polish 📋

**Goal:** Production-quality feel on supported platforms.

- Keyboard **shortcuts** (including menu parity).
- **Accessibility** review.
- **Icons**, density, typography pass.
- Optional: **macOS app bundle**, notarization, Linux packaging stories.

---

## Phase U5 — AI & external tools 🚧

**Goal:** Fast iteration on AI/ML without locking to one vendor API.

**Done (MVP)**

- **Effects** menu: decode **one frame** at playhead → **`SidecarClient`** → save **PNG**.
- **`reel-cli swap`** shares the same pipeline.
- Documented handoff: **`docs/EXTERNAL_AI.md`**, **`docs/CLI.md`**.

**Not yet**

- Full **FaceFusion** (or other) **inference** wired in the bridge beyond stubs/import checks.
- **ONNX RVM** (or similar) for true matting vs chroma stub.
- **Timeline effect clips** or live preview of processed video in the player.

---

## How these phases relate to `phase-status.md`

| Document | Audience | Contents |
|----------|----------|----------|
| **`phases-ui.md`** (this file) | Product / UI roadmap | U1–U5, user-visible scope |
| **`phase-status.md`** | Engineering history | Phases 0–4 (infra, player, sidecar), plus doc milestones |
| **`FEATURES.md`** | What ships today + backlog | Actionable feature bullets; keep in sync when U* moves |

When you close out a **UI phase** item, update all three when the behavior is user-visible.
