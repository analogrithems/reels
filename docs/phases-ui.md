# UI & editor phases (Reel desktop)

This document is the **product / UI roadmap** for **`reel-app`** (Slint). Engineering/infrastructure phases (0–4) are in **`docs/phase-status.md`**.

**Maintenance:** When a milestone moves, update **this file** and **`docs/FEATURES.md`**. Agents: also touch **`docs/phase-status.md`** if the change reflects completed engineering or doc work described there. See **`docs/AGENTS.md`**.

---

## Executive summary

Reel’s UI work is grouped into **U1–U5**: shell & help (**done** for core scope), **deep editing** (multi-track, **per-clip rotate/flip** **shipped**; **Trim Clip…** numeric sheet **shipped**; seek-bar **in/out markers** + **range-scoped export** **shipped**; **subtitle** project lanes **shipped** (`TrackKind::Subtitle`, **File → New Subtitle Track**, lane add/remove + timeline merge with container streams); **ripple** and **on-timeline trim handles** still **open**), **export UX** (cancel + **%** + **strip** + **3-container preset sheet** shipped; **rich transcode catalog** (H.264/VP9/AV1 tiers)—**open**), **polish** (**File → Open Recent** **done**; **v0-aligned** **menubar** Lucide icons + shortcuts, **View** show/hide **video / audio / subtitle** track rows in prefs, floating transport **z-order** + **Lucide** icons + spacing—**partial**; shortcuts, a11y—**partial**), and **AI/effects** (**MVP shipped**; **upscaling** on roadmap). Phases **overlap in time**; the table below is the source of truth for **status**, not strict waterfall ordering.

A **v0 UI mock** (Next.js reference + a **Slint implementation guide**) defines a **target layout and component split**—dark **Theme** tokens, **floating QuickTime-style** preview controls (auto-hide, optional drag), **multi-row timeline** with per-track actions, **export progress** placement, **status footer** pattern, and an optional **full-window trim mode**. That spec is **not** a commitment to ship every control in one release; it is the **north star** for evolving **`reel-app`** through **U2–U4**. See **Design reference (v0 mock → Slint)** below.

---

## Design reference (v0 mock → Slint)

**Design bundle (repo):** `assets/Knotreels.v0.ui/` — v0 mock UI (reference implementation, shared components, public assets). **Slint guide (entry point for implementation):** `assets/Knotreels.v0.ui/CURSOR_IMPLEMENTATION_PROMPT.md`.

**What it specifies (summary):**

- **Visual system:** Dark, macOS-adjacent **Theme** global (backgrounds, text, borders, accents including **yellow** trim handles, per-track **clip** colors for video/audio/subtitle).
- **Shell layout:** **Export progress** region at the top when exporting; **video preview** uses remaining height; **floating** transport/scrub/volume/speed strip **over** the preview (~**60%** width, **auto-hide** after idle, **draggable** position); **timeline** below with **padded** track rows, **+ Video** / **+ Audio** / **+ Subtitles**; **View → Video / Audio / Subtitle Tracks** toggles whether each **row group** is shown (persisted in **`prefs.json`**—default **on**); **status footer** (codec summary · project path · saved/dirty).
- **Component boundaries (Slint):** Suggested modules such as `VideoPreview` (with embedded floating controls), `TransportBar`, multi-track `Timeline`, `StatusFooter`, `ExportProgress`, and optional **`TrimMode`** (dedicated trim UI vs. main editor) plus Rust callbacks (`seek`, track add, clip select, trim apply, export, etc.).
- **Interaction details:** Track labels with **dismiss** on hover, **playhead** line with glow, **J/K/L**-style shuttle hooks in the prompt—map to **U4** shortcuts where we adopt them.

**Phase mapping (where convergence is tracked):**

| v0 area | Primary UI phase | Notes |
|--------|-------------------|--------|
| Theme tokens, typography, icon density | **U4** | Aligns with “icons, density, typography pass”; tokens can land incrementally. |
| Floating preview controls (width, auto-hide, drag, scrub row) | **U4** (with **U2** seek/playhead) | Partially shipped today; v0 doc is the **target** chrome. |
| Multi-row timeline, **+** track affordances, filmstrip/waveform richness | **U2** | Extends current lanes/labels toward full **NLE** row model. **Subtitle** project lanes + **View** visibility toggles **shipped**; **.srt** insert / burn-in still **roadmap**. |
| Export progress **placement** (top bar vs. strip-only) | **U3** | Determinate **%** + strip **shipped**; optional **layout** match to v0. |
| **Trim mode** full-window flow vs. **Trim Clip…** sheet | **U2** | Complements numeric trim and future **on-timeline** handles. |
| **Menubar** icons + shortcut column (Lucide, v0 parity) | **U4** | **Shipped** for Slint-drawn / in-app menus; native OS menubar may omit icons. |
| Floating transport icons + hit-testing | **U4** | **Lucide** SVGs (`icons/lucide/`); overlay **`z`** above viewport wake **TouchArea** so play/step/skip receive clicks. |

**Maintenance:** If the v0 bundle moves or is regenerated, keep **this path** and the **summary** in sync; do not paste the full prompt into `phases-ui.md`.

---

## At a glance

| Phase | Theme | Status | Notes |
|-------|--------|--------|--------|
| **U1** | Shell, menus, timeline scrub, Help | **Done** | Core exit criteria met; stretch shortcuts → **U4** |
| **U2** | Project editing depth (tracks, trim, transforms) | **In progress** | Insert/split/blade **done**; **rotate/flip** **shipped** (per-clip, preview + export); **Trim Clip…** sheet **shipped** (**Edit** menu); **seek-bar range markers** **shipped** (I/O keys + Edit menu); **range-scoped export** **shipped** (markers slice ffmpeg concat); **U2-f Resize Video…** **shipped** (per-clip scale %, composed with rotate/flip in a combined `-vf`); **U2-e Mute Clip Audio** **shipped** (per-clip `audio_mute`; export emits `-an` when all primary-track clips muted — partial case pending **U2-b**); multi-lane **partial**; **draggable seek-bar In/Out handles** **shipped**; **planned:** **Replace/Overlay audio** + per-lane gain (**U2-b**), **per-clip timeline trim handles** (drag clip-chip edges) + ripple, **.srt** import; **visual target:** multi-row timeline + track **+** affordances per **v0 Slint guide** |
| **U3** | Export UX (presets, progress) | **Done (core)** | Cancel + **N%** + **strip** + **7-row preset sheet** **shipped** — MP4 remux / H.264+AAC / HEVC+AAC / WebM VP8 / VP9 / AV1 / MKV remux; optional **layout** match to v0 **top** export bar — **open**; **roadmap:** MOV mux, ProRes/DNx intermediates (see **`SUPPORTED_FORMATS.md`**). |
| **U4** | Polish (a11y, shortcuts, file & view chrome) | **In progress** (partial) | **File → Open Recent** + **Clear Recent** **done**; **menubar** **Lucide** icons + shortcuts aligned to v0 where Slint supports them **done**; **View** — **Loop**, **show Video/Audio/Subtitle track rows** (prefs), **Show Status**, **Always Show Controls**, **zoom** ladder + **Actual Size**, **Enter/Exit Fullscreen** (**Esc** exits) **done**; optional **Zoom to Video** / pan when zoomed / fullscreen on playback chrome **open**; transport + clip-move keys; **QuickTime-style** floating bar over **video** (Lucide icons, **z-order** for clicks, wider step/play spacing) — **converge** with **v0** (**Theme**, **~60%** bar, optional **drag**) — see **U4** body |
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

- **U2** unlocks *meaningful* **U4** shortcuts (blade, **Trim Clip…**, rotate, track moves, **Resize Video…**, **Mute Clip Audio**) once those actions exist; further **U2** work adds more shortcut targets (e.g. ripple trim, **U2-b** audio mix).
- **U3** can proceed in parallel with U2 if contributors avoid conflicting `Export` UI refactors.
- **U5** shares **`SidecarClient`** / **`grab_frame`** with the rest of the app; timeline-integrated effects depend on **U2** clip model richness.

---

## Mapping: phases ↔ `FEATURES.md`

| UI phase | Primary place in **`docs/FEATURES.md`** |
|----------|----------------------------------------|
| U1 | Currently supported: Playback, Viewport, Help |
| U2 | Project & timeline (partial) + **Currently supported:** seek-bar **in/out markers** + **range-scoped export**; **QuickTime-style** **rotate/flip** and **Trim Clip…**; **subtitle** project lanes (**New Subtitle Track**, strip merge); **Roadmap:** **audio** remove/replace/overlay, **resize**, **ripple**, on-timeline trim handles, **caption** import/burn-in; **v0 mock** optional **trim mode** (see **`assets/Knotreels.v0.ui/CURSOR_IMPLEMENTATION_PROMPT.md`**) |
| U3 | Shipped: **Export preset sheet** (7 rows — MP4 remux/H.264+AAC/HEVC+AAC, WebM VP8/VP9/AV1, MKV remux), cancellable progress with **N%** + strip; **v0** optional **top** progress bar vs. today’s strip |
| U4 | **Currently supported:** **File → Open Recent**; **menubar** Lucide icons + shortcuts (v0-style); **View** (loop, **track-row visibility**, show status, always-show controls, zoom, fullscreen) + partial shortcuts — see **Viewport** / **Playback** in **`FEATURES.md`**. **Roadmap:** a11y, bundle, optional zoom pan / toolbar fullscreen; optional **drag** on floating bar |
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
- **File:** Open (media or **`.reel` / `.json` project**), **Open Recent** (MRU + **Clear Recent**), Close, Revert, New Window, Save (`.reel` JSON), Insert Video (playhead-aware, **split** when inside a clip), Insert Audio (first audio lane; **U2-b**), New Video Track (empty lane; **U2-a**), New Audio Track (empty lane; **U2-b**), **New Subtitle Track** (empty `TrackKind::Subtitle` lane; **U2**), **Export…**, **Cancel Export** when encoding; menu items use **Lucide** icons + shortcuts where applicable (**v0**-aligned).
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

**Design target:** The **v0 Slint implementation guide** (`assets/Knotreels.v0.ui/CURSOR_IMPLEMENTATION_PROMPT.md`) describes a **multi-row timeline** (video / audio / subtitle lanes), per-track **add** actions, **filmstrip**-style clips and **waveforms**, **yellow** in/out trim handles, and an optional **full-window Trim mode**—use it when extending lanes and trim UX beyond what ships today.

**Deferred (explicit):** Timeline **audio waveform** / peak visualization (drawn under or inside audio lanes) ships **after** name-based filmstrip chips, footer alignment, and other v0 shell work—it needs peak samples or a decode pass and is tracked as **last among** timeline polish items (see `crates/reel-app/ui/app.slint` near the AUDIO lanes).

### U2 — progress snapshot (rolling)

| Area | Shipped | Still open |
|------|---------|------------|
| **Tracks / lanes** | Primary-track video concat **preview**; **New Video / Audio / Subtitle Track**; **Subtitle** rows: project lanes + container streams (single-file) merged in the strip; per-lane **filmstrip** chips + delete on project subtitle lanes; **Move Clip** between primary ↔ second **video** lane; **multi-audio-lane export mix** (ffmpeg `amix` on ≥2 audio lanes; single-lane remains dual-mux for stream-copy) | Preview/mix from **secondary video** lanes; **preview-side** multi-audio mix (realtime mixer still first-lane-only); **waveforms** (deferred); **.srt** (or insert-on-subtitle-lane) **open**; draggable moves |
| **Audio** | **First** audio lane **concat** drives preview when it has clips (else embedded video audio); silence-pad after audio ends; **Edit → Mute Clip Audio** toggle (per-clip `audio_mute`; export emits `-an` when every primary-track clip is muted); **export** mixes **all** dedicated audio lanes via ffmpeg `amix` (`duration=longest`, `normalize=0`) — amix output always transcodes (aac / libopus) since stream-copy is incompatible with a filter-graph stream | **Preview-side** multi-lane mix (realtime audio thread rewrite — follow-up); **U2-e** replace/overlay + per-lane **gain**; partial-mute silence substitution |
| **Trim** | **Blade**; **Trim Clip…** numeric sheet (**Edit** menu + **double-click** video lane strip); seek-bar **In/Out markers** + **range-scoped export**; **draggable** seek-bar In/Out handles (`edit-drag-in-marker-ms` / `edit-drag-out-marker-ms`); **per-clip timeline trim handles** (drag left / right edges of any filmstrip chip; ripple is automatic — sequential clip model has no absolute positions so downstream clips shift on their own) | — |
| **Transform** | Per-clip **rotate / flip** (preview + export **`-vf`**) | — |
| **Resize** | **U2-f** **Edit → Resize Video…** per-clip scale % (10–400%); preset buttons + numeric entry; composes with rotate/flip into a combined `-vf` chain; export-only (preview unchanged) | — |

**Exit criteria (not all met)**

- [x] Non-destructive **project file** workflow with undo/redo and autosave (path-backed).
- [ ] User can see and edit **more than one logical video/audio lane** in the UI (multi-track). *Progress:* **File → New Video Track** / **New Audio Track** + **Move Clip to Track Below / Above** (Edit menu) move between primary and second **video** lane (below = playhead on primary clip; above = first clip on second lane to end of primary). **First** audio lane drives **preview sound** when it has clips (else embedded primary-track audio); additional audio lanes have **no** mix yet. Secondary **video** lanes are still **not** in the preview decode graph. Not a full per-lane visual editor (waveforms, drag) yet.
- [~] User can **trim** in the NLE sense (handles, ripple, range on the scrub bar). *Shipped:* **blade** (**Split at Playhead** / **Ctrl+B**); **per-clip numeric trim** (**Trim Clip…** — **`docs/FEATURES.md`**); **seek-bar In/Out markers** (**I** / **O** / **Alt+X**, cyan/magenta overlay) **with range-scoped export** (markers slice the ffmpeg concat on both video and audio tracks, rebased to start at 0); **draggable seek-bar In/Out handles** (yellow QuickTime-style grips, clamped so In ≤ Out − 10ms). *Still open:* **per-clip timeline trim handles** (drag clip edges on the lane chips) and **ripple**.

**Done**

- In-memory **`Project`** with at least one **video** track; open creates one clip + track.
- **Insert Video** with **split-at-playhead** when the playhead lies inside a clip.
- **Split Clip at Playhead** (blade): two clips from one at the playhead; undoable.
- **Save**, **Revert**, **Undo / Redo** (with explicit Save clearing stacks).
- **Debounced autosave** to the on-disk project path (Slint timer; **preserves** undo vs explicit Save); **Close Window** prompts to save when dirty.
- **Per-clip orientation** (**Edit → Rotate 90°** / **Flip**): stored in the project; preview after the scaler; export uses ffmpeg **`-vf`** when any primary-track clip is non-identity (mixed orientations in one export remain a limitation—see **`docs/FEATURES.md`**).
- **Trim Clip…** (**U2-c / U2-d**): sheet with begin/end in **source-file seconds**, inline validation, undoable; see **`docs/FEATURES.md`** and **`docs/KEYBOARD.md`**.
- **U2-a (partial):** **New Video Track** appends an empty `TrackKind::Video` lane (undoable). **Move Clip to Track Below / Above** (Edit menu) shuffle clips between primary and second video lane (see **FEATURES** for exact rules). **Primary-track sequence preview** (concat timeline, play/scrub across clips, auto-advance at boundaries) is **implemented**; secondary lanes are still **not** in the decode graph. Remaining: richer per-lane visuals (waveforms, thumbnails), draggable moves, **on-timeline** trim handles (numeric **Trim Clip…** shipped).
- **U2-b (partial):** **New Audio Track** appends an empty `TrackKind::Audio` lane (undoable); **Insert Audio…** places clips on that lane. When the **first** audio lane has clips, **playback uses that concat** for sound (else embedded audio from primary video files); silence after audio if video runs longer. **Export** mixes **every** dedicated audio lane via ffmpeg `-filter_complex amix=inputs=N:duration=longest:normalize=0[aout]` (single-lane still uses the faster dual-mux stream-copy path; 2+ lanes route through amix and force a container-appropriate audio encoder — aac for MP4/MOV/MKV, libopus for WebM — because amix output is filter-graph PCM). **Not yet:** the **preview-side** realtime mixer still drives sound from the **first** audio lane only; merging the rest needs a separate audio-thread rewrite.
- **Subtitle lanes (partial):** **`TrackKind::Subtitle`** in **`reel-core`**; **File → New Subtitle Track** (**Ctrl+Shift+T** / **⌘⇧T** when media-ready) appends an empty subtitle lane (undoable). Timeline shows up to **four** subtitle rows: **project** lanes first, then **container subtitle streams** (single-media) as synthetic chips—same merge rule as video/audio display vs project counts. **Remove** a project subtitle lane via the lane trash when enabled. **View → Subtitle Tracks** hides or shows the whole subtitle block (prefs). **Not shipped:** inserting captions onto subtitle lanes, preview burn-in, or export of subtitle tracks.

**Suggested sub-milestones (order may vary)**

1. **U2-a — Multi-track preview:** ~~Sequence-across-clips on the primary track~~ **done** for core playback; **New Video Track** + summary + **per-lane labels** **done**; **move clip to next video track** (menu) **done**; remaining: richer per-lane visuals, draggable moves, preview/mix for secondary lanes.
2. **U2-b — Audio in timeline:** **Partial:** **New Audio Track**, **Insert Audio…**, first-lane **concat preview** (switch from embedded video audio when the lane has clips; silence-pad if audio ends early); **export-side** multi-lane **mix shipped** (`export_concat_with_audio_lanes_oriented` dispatches 0/1/N lanes → mute / dual-mux / `amix` filter_complex; aac for MP4/MOV/MKV, libopus for WebM; `normalize=0` keeps unit gain so callers can attenuate upstream). **Open:** **preview-side** mixer (realtime audio thread still reads first lane only), **per-lane gain** / levels, **trim-on-lane**.
3. **U2-c — Trim / ripple:** **Numeric trim** via **Trim Clip…** **shipped**; **range-scoped export** (markers limit ffmpeg concat spans) **shipped**; **draggable seek-bar In/Out handles** **shipped** (yellow grips on the scrub slider, clamped to a 10ms min gap); **per-clip timeline trim handles** **shipped** — 6-px TouchArea zones on the left / right edges of each filmstrip chip emit a fractional drag delta that Rust applies against the clip's current in/out and delegates to `session::trim_clip` (invariants: `begin >= 0`, `begin < end`, `duration >= 50 ms`, `end <= source_duration`). **Ripple is automatic** because the project has no absolute timeline positions — shortening a clip pulls downstream clips forward by the same delta without extra bookkeeping.
4. **U2-d — QuickTime-style Edit menu:** **Rotate 90° Left** / **Rotate 90° Right** / **Flip Horizontal** / **Flip Vertical** — **shipped** (per-clip orientation persists in the project; applied in preview post-scaler and in ffmpeg export via `-vf`). **Trim Clip…** sheet — **shipped** (**Edit → Trim Clip…**; numeric begin/end in source-file seconds, inline validation, undoable). **Seek-bar range markers** — **shipped** (**Edit → Set In/Out Point** + **Clear Range Markers**; keys **I** / **O** / **Alt+X**; cyan/magenta overlay lines with a tinted range on the slider; ephemeral session state that clears on close / new project open). **Range-scoped export** — **shipped**: when both markers are set, `export_timeline_payload` slices the primary-video and first-audio concat inputs to the In/Out range (rebased to sequence 0) before handing them to `export_concat_with_audio_oriented`; empty slices refuse with a clear status message, and the run status reads **Exporting range In–Out s…**.
5. **U2-e — Audio (Edit menu):** **Remove audio** **shipped** as per-clip **Mute Clip Audio** toggle. Export emits `-an` when every primary-track clip is muted and there's no dedicated audio lane; **partial-clip mute** now **shipped** via **silence substitution** on the export thread (`reel_core::generate_silence_wav` + `build_mute_substitution_lane` synthesize a dedicated audio lane that keeps each unmuted clip's embedded audio and swaps silence in for muted spans, muxed via the existing N-lane dispatch). **Overlay Audio…** menu entry **shipped** — appends a fresh `TrackKind::Audio` lane per invocation so the export mix layers every overlay in parallel via `amix`. **Replace Audio…** (mute primary + insert audio in one atomic step) and **per-lane gain** remain open. Preview-side mix is still first-lane-only until the audio-thread rewrite lands.
6. **U2-f — Resize:** **Edit → Resize Video…** **shipped** — per-clip scale percent (10–400%) with preset buttons (25/50/75/100/150/200%) + numeric entry; composes with rotate/flip into a combined `-vf` chain; export-only (preview is unchanged). **AI-assisted upsampling** to higher resolution is **not** in this milestone—see **U5** / parking lot.

**Not yet**

- **Multi-track** video: clips can be **moved to the next video track** via the Edit menu; there is still no **mix** or preview from secondary **video** lanes. **Audio:** the **first** lane can drive **preview sound** when it has clips; additional audio lanes have **no** mix yet.
- **Roll**, slip/slide; multi-cam (blade **without** new media is **Split Clip at Playhead**). **U2-d**'s **double-click trim sheet**, **seek-bar range markers**, **range-scoped export**, **draggable seek-bar In/Out handles**, and **per-clip timeline trim handles** (drag the left / right edge of any filmstrip chip; ripple is automatic) are all **shipped**.
- **Subtitles / captions** — **project lanes + UI** **shipped** (see **U2** subtitle bullet); **import / edit / burn-in export** remain **roadmap** (**FEATURES**).
- Optional: adopt **`ProjectStore`** from `reel-core` inside the app (library already implements debounced atomic writes).

---

## Phase U3 — Export UX ✅

**Goal:** User-controlled export presets and feedback.

**Exit criteria**

- [x] User picks **named export configurations** aligned with **`docs/SUPPORTED_FORMATS.md`** — **shipped tiers:** MP4 remux, **MP4 — H.264 + AAC** (web-tier transcode), **MP4 — HEVC + AAC** (mobile tier, `libx265 -tag:v hvc1`), **WebM — VP8 + Opus**, **WebM — VP9 + Opus** (`libvpx-vp9`), **WebM — AV1 + Opus** (`libaom-av1`), MKV remux.
- [x] User picks **preset** (format family) from the app — seven named presets in the sheet; resolution/bitrate knobs intentionally deferred (per-preset CRF defaults cover the common tiers).
- [x] Long exports show **cancellable** ffmpeg work without killing the whole app (**Esc** / **Cancel export** on the progress modal).
- [x] **Determinate export feedback** in the main window: status **%** plus a thin **progress strip** above the transport row (ffmpeg `out_time_ms` vs timeline duration).

**Shipped**

- **Export** runs **off the UI thread**; **File → Export…** opens a **7-row preset sheet** (MP4 remux / H.264+AAC / HEVC+AAC / WebM VP8 / VP9 / AV1 / MKV remux), then a filtered save dialog; status line shows **Exporting…**, then **Exporting… N%**, with the **strip** filling left→right, then result (see **`docs/FEATURES.md`**). The **MP4 H.264+AAC** preset is the guaranteed MP4-side fallback when remux `-c copy` rejects a mix of codecs; **MP4 HEVC+AAC** targets iOS-native (hvc1) and smaller files at equal quality.

**Optional chrome (v0 alignment)**

- The v0 mock adds a **dedicated export progress bar** at the **top** of the window while exporting. Today’s **determinate %** + **thin strip** already meet core **U3** exit criteria; a **top bar** is **optional** polish if we want pixel-parity with the mock.

**Not started (roadmap)**

- **MOV mux**, **ProRes / DNx** intermediate paths (pro handoff), explicit resolution/bitrate fields.

*Today:* preset sheet maps 1:1 to `WebExportFormat` variants (Mp4Remux, Mp4H264Aac, Mp4H265Aac, WebmVp8Opus, WebmVp9Opus, WebmAv1Opus, MkvRemux); further advanced transcodes remain **roadmap** work (see **`docs/SUPPORTED_FORMATS.md`**).

---

## Phase U4 — Polish 🚧

**Goal:** Production-quality feel on supported platforms.

**Exit criteria (draft)**

- [x] **File → Open Recent** — MRU list of **recent projects** (`.reel` / `.json`) and **recent media** (same kinds as **File → Open**); **Clear Recent**; missing files pruned on pick. *Optional:* per-entry remove only (not shipped).
- [ ] Core actions reachable via **keyboard** (parity with common editors where feasible). *Progress:* **F1** (Help overview), **Ctrl+O** / **Ctrl+S** / **Ctrl+W** (open / save / close when enabled), **Ctrl+I** / **Ctrl+Shift+I** / **Ctrl+E** / **Ctrl+Shift+N** / **Ctrl+Shift+A** / **Ctrl+Shift+T** (insert video / insert audio when enabled / export / new video / audio / **subtitle** track when **media-ready**), **Ctrl+B** (split at playhead when enabled), **Space** (play/pause), **Ctrl+L** (toggle **View → Loop Playback**; works without media), **Ctrl+=** / **Ctrl+-** / **Ctrl+0** (zoom in / out / zoom to fit — work without media), **← / →** (±1 s seek), **Home** / **End** (sequence start/end), **Ctrl+Z** / **Ctrl+Shift+Z** (undo/redo when enabled), **Ctrl+Shift+↓/↑** (move clip between primary and second video lane when enabled; **⌘⇧↓/↑** on macOS). Transport and edit shortcuts expect the main view focused; **Open** works from an empty window; **Insert**/**Export**/**New … Track** need decode ready where noted.
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

Treat these as **nice-to-have polish** overlapping **U4** (transport feel) that does **not** close remaining **U4** items (a11y audit, optional zoom pan / toolbar fullscreen, etc.). The **v0** guide expands this into a **two-row** floating panel (transport + scrub), **draggable** placement, and optional **tools** chevron—implement incrementally; see **Design reference (v0 mock → Slint)**.

**Scope**

- **File** menu: **Open Recent** (MRU persistence — `recent.json` under the OS data-local dir).
- Keyboard **shortcuts** (including menu parity).
- **View** chrome: loop, zoom ladder, fullscreen.
- **Accessibility** review.
- **Icons**, density, typography pass; introduce or align **`Theme`** globals and **system-font** sizing with **`assets/Knotreels.v0.ui/CURSOR_IMPLEMENTATION_PROMPT.md`** (dark palette, monospace timecode).
- **Floating preview bar:** converge on the v0 spec where practical (**~60%** width, **auto-hide**, **Lucide** transport icons, **z-order** so controls receive clicks, optional **drag** handle and tools affordance)—without blocking a11y or shortcuts work.
- **Native menubar:** **Lucide** icons on items and **shortcut** annotations where supported; OS-native menu bars may still omit custom icons.
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
- Rich **subtitle** authoring, **.srt** import on subtitle lanes, and **burn-in** export. **Shipped today:** **View** toggles for **video / audio / subtitle** track **rows**, **New Subtitle Track**, and **project** subtitle lanes in the timeline (see **U2**).
- **Collaboration** / cloud project (no roadmap commitment).
- **AI video upscale** (distinct from **U2-f** pixel resize—see **U5** “Not yet”).

---

## Suggested next focus (rolling)

Priorities change; this is **guidance for contributors**, not a commitment.

1. **U2-e** — **Remove audio** **shipped** (per-clip **Mute Clip Audio** toggle; export `-an` when all primary clips muted). **Replace** / **Overlay** audio with independent volume still open — both depend on **U2-b** for mix/export, which also unblocks partial-mute silence substitution. **Rotate/flip**, **Trim Clip…** sheet, **Resize Video…** sheet, **seek-bar range markers**, and **range-scoped export** are **shipped** (see **FEATURES.md**). Remaining under **U2-d**: **timeline in/out handles** so the markers also live on the scrub bar as drag handles.
2. **U3** — **Export preset catalog** per **`SUPPORTED_FORMATS.md`**. **Shipped:** MP4 remux, **MP4 H.264+AAC** (web tier), **MP4 HEVC+AAC** (mobile tier, hvc1), WebM **VP8+Opus** / **VP9+Opus** / **AV1+Opus**, MKV remux (7-row preset sheet). **Roadmap:** MOV mux, ProRes / DNx intermediates, explicit resolution/bitrate fields. Determinate **%** + strip already shipped — further **chrome** polish optional.
3. **U4** — **a11y** audit; optional **View** polish (pan when zoomed, fullscreen on playback chrome). (**Open Recent**, **View** loop/zoom/fullscreen, timeline scrub reliability — shipped.)
4. **U5-a** — Bridge quality: one **non-stub** transform or clearly labeled experimental paths (pairs with **U5** exit criteria).

Revisit when **on-timeline trim handles**, **ripple trim**, or major **export preset** catalog additions land.

---

## How these phases relate to other docs

| Document | Audience | Contents |
|----------|----------|----------|
| **`phases-ui.md`** (this file) | Product / UI roadmap | U1–U5, status, exit criteria, sequencing, **logging standards**, **v0 design reference** |
| **`assets/Knotreels.v0.ui/CURSOR_IMPLEMENTATION_PROMPT.md`** | Slint UI **target** spec (from v0) | Theme, layout diagram, suggested components/callbacks—**not** a phase gate on its own |
| **`phase-status.md`** | Engineering history | Phases 0–4 (infra, player, sidecar), doc milestones; **Phase 1** includes completed **status-footer** follow-on (probe metadata in UI) |
| **`FEATURES.md`** | What ships today + backlog | Actionable feature bullets; keep in sync when U* moves |
| **`CONTRIBUTING.md`** | New contributors | Workflow, links here and to **Suggested next focus** |

When you close out a **UI phase** item, update **`FEATURES.md`**, this file, and **`phase-status.md`** when the behavior is user-visible; adjust **`CONTRIBUTING.md`** if contributor workflow or entry points change.

---

*Living document; sub-milestones (U2-a … U5-c) and “Suggested next focus” should be revised when scope shifts.*
