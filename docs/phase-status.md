# Phase status (engineering & documentation)

High-level checklist for **infrastructure, engine, and repo documentation**. For **product/UI** milestones (menus, timeline, export UX, AI), see **`docs/phases-ui.md`** and **`docs/FEATURES.md`**.

**Maintenance:** Mark items when done; add rows for new initiatives. UI-visible changes should also update **`docs/FEATURES.md`**. Agents: keep **`docs/phases-ui.md`** aligned when U-scope work completes.

---

## Product UI phases (U1–U5)

Detailed **status**, **exit criteria**, **dependencies**, **sub-milestones** (U2-a … U5-c), **parking lot**, **logging standards**, **suggested next focus**, and the **v0 → Slint design reference** live in **`docs/phases-ui.md`**. The v0 mock design lives under **`assets/Knotreels.v0.ui/`**; the **Slint implementation prompt** is **`assets/Knotreels.v0.ui/CURSOR_IMPLEMENTATION_PROMPT.md`**. This file does not duplicate the product roadmap; it tracks **engineering** phases below.

---

## Phase 0 — Infrastructure & observability ✅

- [x] Cargo workspace: `reel-core`, `reel-app`, `reel-cli`
- [x] Makefile: `setup`, `build`, `lint`, `test`, `run`, `run-cli`, `fixtures`, `clean`, `ci`, `check-tools`
- [x] `uv`-managed Python sidecar (`sidecar/`, `facefusion_bridge.py`)
- [x] `tracing` + `REEL_LOG*` environment variables; **session log file** (`reels.session.*.log` under `{data_local_dir}/reel/logs/`) as **NDJSON** with module path + file:line and structured `tracing` fields
- [x] Child stdout/stderr piped into `tracing` (`spawn_logged_child` / sidecar)
- [x] GitHub Actions: `macos-14`, `make setup lint test`

**Logging coverage (ongoing — not a phase gate):** Phase 0 delivers **where** logs go and how filters work. **What** we log in application and library code—**error** / **warn** / **info** / **debug** / **trace** at the right places—is a **standing requirement** for new and changed behavior, documented in **`docs/phases-ui.md`** → **Logging standards (requirement)**. PRs should add or extend **`tracing`** calls when they ship user-visible flows or failure paths that would otherwise be silent in the session log.

---

## Phase 1 — Media engine & TDD ✅

- [x] `MediaProbe` + `FfmpegProbe` (ffmpeg-next 7.1; dev pins **ffmpeg@7**)
- [x] Unrecognized audio: warn + `audio_disabled`, do not fail video
- [x] Committed fixtures + `scripts/generate_fixtures.sh`
- [x] `Project` serde v2, migration, insta snapshot, extension maps
- [x] `ProjectStore` debounced atomic autosave (library; tests)
- [x] `reel-cli probe` JSON output
- [x] **Desktop status footer** — single-line strip (codecs · paths · save/dirty) driven by per-clip probe metadata at the playhead; *follow-on to Phase 1 probe/clip model, originally out of scope for the engine milestone* (see **`docs/FEATURES.md`**)

---

## Phase 2 — Player window ✅

- [x] Slint `AppWindow`, timeline stub, transport
- [x] Video thread: decode → RGBA → `SharedPixelBuffer`
- [x] Audio thread: decode → resample → ringbuf → cpal (**AudioClock** master)
- [x] Fast seek on scrub (drain, flush, keyframe seek, advance)
- [x] `rfd` Open dialog; `REEL_OPEN_PATH` startup load
- [x] UI gates: play/slider until `media-ready`
- [x] Panic containment around decode hot paths; safe RGBA conversion
- [x] Separate command channels to video and audio threads
- [ ] Formal manual QA checklist on diverse real-world files (informal testing ongoing)

---

## Phase 3 — Sidecar & bridge ✅ (MVP)

- [x] `SidecarClient` stdio JSON + tempfile RGBA; timeouts and crash handling
- [x] `spawn_child_with_logged_stderr` for IPC-capable children
- [x] `grab_frame` one-shot decode for CLI/effects
- [x] `facefusion_bridge.py`: `ping` / `swap` / `shutdown`, transform table + tests
- [x] `reel-cli swap` end-to-end; integration + pytest coverage
- [x] Desktop **Effects** menu (shared pipeline with CLI)
- [x] **`docs/EXTERNAL_AI.md`** — handoff model (JSON + files, extension via `params`)
- [ ] Production-grade FaceFusion / ONNX pipelines in the bridge (tracked under **Phase U5** in `phases-ui.md`)

---

## Phase U1 — Desktop shell & documentation ✅

*Aligned with **Phase U1** in `phases-ui.md` (menus, Help, timeline scrub). Product **U1** exit criteria there are **met**; follow-on shell items such as **File → Open Recent** live under **Product Phase U4** in `phases-ui.md`.*

- [x] Menu bar: File / Edit / Effects / Window / Help
- [x] File / Edit / Effects / Window behaviors (see `FEATURES.md`)
- [x] ffmpeg export integration tests (`target/reel-export-verify/`)
- [x] Bundled Help: multi-topic (`shell.rs` `HelpDoc`), `docs/README.md` index
- [x] Contributor docs: `CONTRIBUTING.md`, `DEVELOPERS.md`, `AGENTS.md`, `CLI.md`, `MEDIA_FORMATS.md`, `SUPPORTED_FORMATS.md`, `FEATURES.md`
- [x] `EXTERNAL_AI.md` + cross-links in `architecture.md`, `HELP.md`

---

## Phase UT — UI testing harness 🚧

Implementation notes live in **`docs/phases-ui-test.md`**. This row tracks engineering shipments.

- [x] **Strict version pin** — `i-slint-backend-testing = "=1.16.0"` in `[workspace.dependencies]`, consumed by `reel-app` as a dev-dep. Must be bumped in lockstep with `slint`.
- [x] **Headless harness** — `ui_test_support::init()` (`crates/reel-app/src/lib.rs`) wraps `init_integration_test_with_system_time()` behind a `Once`; first smoke test boots `AppWindow`, verifies startup defaults, and round-trips `export-preset-index` across the full 0..=6 range.
- [x] **Probe seam** (Phase 1b) — `MediaProbe` threaded through `project_from_media_path_with_probe` + `EditSession::{open_media,insert_clip_at_playhead,insert_audio_clip_at_playhead}_with_probe`; `FakeProbe` test helper in `session.rs` drives a no-ffmpeg Open → Insert scenario (`session::tests::open_and_insert_via_fake_probe_no_ffmpeg`).
- [ ] **Player engine trait** — abstract `crates/reel-app/src/player.rs` so tests can inject a mock engine (returns static frames, no ffmpeg). Deferred until a UI test actually needs rendered frames or timing behavior.
- [x] **`Open → Edit → Export` integration test** — three layers shipped: (a) headless UI smoke — `ui_smoke_tests::window_boots_and_round_trips_basic_properties` boots `AppWindow`, drives Open through `FakeProbe`, asserts Slint menu-gate properties, and invokes all three `file-export` preflight branches (sheet opens / status warns / silent); (b) preset-confirm seam — `prepare_export_job(&session, preset_index, &dyn SaveDialogProvider) -> ExportPreflight`, with `StubSaveDialog` covering invalid preset index, empty range, cancelled dialog, wrong extension, happy-path `Spawn`, marker range passthrough, and preset-index → `WebExportFormat` wiring for all four presets; (c) export pipeline — `export_payload_tests::payload_*` covers `export_timeline_payload` for single-clip, range-sliced, out-of-range, first-audio-lane cases. Still deferred: live click through `invoke_export_preset_confirm()` (needs wiring a `SaveDialogProvider` into `install_export_preset_confirm_callback` so tests can swap it) and `ElementHandle::find_by_accessible_label` usage (blocked on adding `accessible-label` to Slint buttons).
- [x] **Visual regression harness** — `crates/reel-app/tests/ui_visual_golden.rs` uses Slint’s headless `MinimalSoftwareWindow` + `renderer-software`, `take_snapshot()` on `AppWindow` at 400×320, compares RGBA to `tests/golden/app_window_default.png`. Update goldens: `UPDATE_UI_GOLDENS=1 cargo test -p reel-app --test ui_visual_golden`. Separate integration-test process (not `i-slint-backend-testing`). See **`docs/phases-ui-test.md` Phase 3**.
- [x] **Post-export validation for session-driven exports** — `export_payload_tests::roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux` opens the fixture through `FfmpegProbe`, runs the real `export_concat_timeline`, and re-probes the output (duration > 0, video stream present). Complements `crates/reel-core/tests/export_web_formats.rs` which covers each preset in isolation.

---

## Phase 4 — Distribution & long-form docs 📋

- [x] Core technical docs: `architecture.md`, `phases-ui.md`, `phase-status.md`, `HELP.md`
- [x] Reference set: features, formats, CLI, external AI, phases (this cycle)
- [x] `docs/SUPPORTED_FORMATS.md` — playback vs export matrix + prioritized roadmap (see **Format support roadmap** below)
- [ ] `docs/USER_GUIDE.md` — optional narrative end-user guide (non-Help)
- [ ] App bundle / notarization / Linux packaging (also **U4** polish in `phases-ui.md`)

---

## Format support roadmap (engineering)

Source matrix: **`docs/SUPPORTED_FORMATS.md`**. These items track **first-class** support beyond “whatever FFmpeg accepts today.”

- [x] **Export — H.264 + AAC MP4 transcode preset** — explicit encode preset **MP4 — H.264 + AAC** (`libx264 -preset medium -crf 20 -pix_fmt yuv420p`, AAC 160 kbps, `+faststart`); use when MP4 remux fails on codec mismatch or for fixed delivery targets
- [x] **Export — HEVC + AAC MP4 preset** — mobile-tier encode **MP4 — HEVC (H.265) + AAC** (`libx265 -preset medium -crf 24 -pix_fmt yuv420p -tag:v hvc1`, AAC 160 kbps, `+faststart`); smaller files at equal quality vs H.264, iOS-native playback
- [x] **Export — VP9 / AV1 WebM presets** — **WebM — VP9 + Opus** (`libvpx-vp9 -b:v 0 -crf 32 -row-mt 1`, Opus 96 kbps) and **WebM — AV1 + Opus** (`libaom-av1 -crf 30 -b:v 0 -cpu-used 6 -row-mt 1`, Opus 96 kbps); better compression than VP8 at the cost of encode time
- [ ] **Export — UX for remux failures** — clearer errors when MP4/MKV reject a stream (codec / licensing / mux constraints); link to transcode presets above
- [ ] **Export — MOV mux** and/or **ProRes / DNx** intermediate paths for pro handoff
- [ ] **Playback / export — Subtitles** — **WebVTT**, **SRT**, **TTML** (platform targets in `SUPPORTED_FORMATS.md`); **ASS/SSA** for advanced styling — not decoded, shown, or muxed today (see `MEDIA_FORMATS.md`)
- [ ] **Playback — Multi-audio** stream selection (today: first decodable audio only)

---

## UI initiative checklist (product — see `phases-ui.md`)

Implementation tracking for **menu- and timeline-visible** features described in **`docs/phases-ui.md`** (U2-d … U4, **U3** presets). Uncheck until shipped; update **`docs/FEATURES.md`** when done.

- [x] **File → Open Recent** (**U4**) — MRU **projects** and **media**; **Clear Recent**; persistence + prune on missing file (per-entry remove optional / not shipped)
- [x] **Edit** — **Rotate 90°** left/right, **flip** horizontal/vertical (**QuickTime-style**) — per-clip, **Ctrl+R** / **Ctrl+Shift+R**; preview post-scaler + ffmpeg `-vf` on export
- [x] **Trim Clip…** (**U2**) — **Edit → Trim Clip…**; per-clip begin/end in source seconds, inline validation, undoable (see **`docs/FEATURES.md`**, **`docs/KEYBOARD.md`**)
- [x] **Timeline** — **two markers** on the seek bar (in/out range) — **Edit → Set In/Out Point** + **Clear Range Markers**; keys **I** / **O** / **Alt+X**; cyan/magenta overlay with tinted range on the slider; ephemeral per session (clears on close / new project). **Range-scoped export** (both markers set → ffmpeg concat is sliced to the range on video and first-audio tracks, rebased to sequence 0) — **shipped**. **Follow-on:** on-timeline drag handles and any additional batch operations that should honor the range.
- [~] **Edit → Mute Clip Audio** (**U2-e**) — per-clip `audio_mute` toggle, undoable; export emits `-an` when every primary-track clip is muted (partial-mute case surfaces a status pointing at **U2-b**). **Replace** / **Overlay** audio still open — blocked on **U2-b** (multi-audio mix)
- [x] **Edit → Resize Video…** (**U2-f**) — per-clip scale percent (10–400%, 100% = identity); preset buttons (25/50/75/100/150/200%) + numeric entry; export-only (preview unchanged); composes with rotate/flip in a combined `-vf` chain; undoable
- [x] **View** — **Loop Playback** (primary-track sequence; **prefs** + **Ctrl+L** / **⌘L**)
- [x] **View** — **Show Video / Audio / Subtitle track rows** (each toggles timeline section; **prefs**; default all on)
- [x] **View** — **Zoom** (in / out / fit / actual size; **prefs** + **Ctrl+=** / **+-** / **0**); **Enter/Exit Fullscreen** (menu; **Esc**)
- [x] **Shell** — **Menubar** **Lucide** icons + shortcut annotations (**v0**-aligned; native OS bar may omit icons)
- [x] **Transport** — floating bar **Lucide** icons, **z-order** for click hit-testing, spacing vs **v0**
- [x] **File** — **New Subtitle Track** (**Ctrl+Shift+T**); **`TrackKind::Subtitle`** project lanes; timeline merge with container streams (**U2**)
- [ ] **View** (optional) — **Zoom to Video**; **pan** when zoomed; **fullscreen** on playback toolbar
- [~] **Export** — **preset catalog** aligned with **`docs/SUPPORTED_FORMATS.md`** (web + mobile tiers). **Shipped today:** MP4 remux, **MP4 H.264 + AAC** (web-tier transcode), **MP4 HEVC + AAC** (mobile tier), **WebM VP8 / VP9 / AV1 + Opus**, MKV remux. **Remaining:** MOV mux, ProRes / DNx intermediates for pro handoff.

---

## How to use this file

| Role | Use |
|------|-----|
| **Product / roadmap** | **`docs/phases-ui.md`** + **`docs/FEATURES.md`** |
| **Session logs & `tracing`** | **`docs/phases-ui.md`** → *Logging standards*; **`docs/architecture.md`** (paths, `REEL_LOG*`) |
| **Infra & engine completeness** | This file (Phases 0–3) |
| **Design detail** | **`docs/architecture.md`**, **`docs/EXTERNAL_AI.md`** |

When **Phase U1–U5** items in `phases-ui.md` ship, update **`FEATURES.md`**; update **this file** only if you also complete or add **engineering** deliverables (e.g. new CI job, new crate).
