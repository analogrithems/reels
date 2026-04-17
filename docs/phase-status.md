# Phase status (engineering & documentation)

High-level checklist for **infrastructure, engine, and repo documentation**. For **product/UI** milestones (menus, timeline, export UX, AI), see **`docs/phases-ui.md`** and **`docs/FEATURES.md`**.

**Maintenance:** Mark items when done; add rows for new initiatives. UI-visible changes should also update **`docs/FEATURES.md`**. Agents: keep **`docs/phases-ui.md`** aligned when U-scope work completes.

---

## Product UI phases (U1–U5)

Detailed **status**, **exit criteria**, **dependencies**, **sub-milestones** (U2-a … U5-c), **parking lot**, and **suggested next focus** live in **`docs/phases-ui.md`**. This file does not duplicate that roadmap; it tracks **engineering** phases below.

---

## Phase 0 — Infrastructure & observability ✅

- [x] Cargo workspace: `reel-core`, `reel-app`, `reel-cli`
- [x] Makefile: `setup`, `build`, `lint`, `test`, `run`, `run-cli`, `fixtures`, `clean`, `ci`, `check-tools`
- [x] `uv`-managed Python sidecar (`sidecar/`, `facefusion_bridge.py`)
- [x] `tracing` + `REEL_LOG*` environment variables
- [x] Child stdout/stderr piped into `tracing` (`spawn_logged_child` / sidecar)
- [x] GitHub Actions: `macos-14`, `make setup lint test`

---

## Phase 1 — Media engine & TDD ✅

- [x] `MediaProbe` + `FfmpegProbe` (ffmpeg-next 7.1; dev pins **ffmpeg@7**)
- [x] Unrecognized audio: warn + `audio_disabled`, do not fail video
- [x] Committed fixtures + `scripts/generate_fixtures.sh`
- [x] `Project` serde v2, migration, insta snapshot, extension maps
- [x] `ProjectStore` debounced atomic autosave (library; tests)
- [x] `reel-cli probe` JSON output

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

*Aligned with **Phase U1** in `phases-ui.md` (menus, Help, timeline scrub).*

- [x] Menu bar: File / Edit / Effects / Window / Help
- [x] File / Edit / Effects / Window behaviors (see `FEATURES.md`)
- [x] ffmpeg export integration tests (`target/reel-export-verify/`)
- [x] Bundled Help: multi-topic (`shell.rs` `HelpDoc`), `docs/README.md` index
- [x] Contributor docs: `CONTRIBUTING.md`, `DEVELOPERS.md`, `AGENTS.md`, `CLI.md`, `MEDIA_FORMATS.md`, `SUPPORTED_FORMATS.md`, `FEATURES.md`
- [x] `EXTERNAL_AI.md` + cross-links in `architecture.md`, `HELP.md`

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

- [ ] **Export — H.264 + AAC MP4 transcode preset** (explicit encode when `-c copy` fails or for fixed delivery targets)
- [ ] **Export — VP9 and/or AV1 WebM** as user-selectable presets (today: **VP8 + Opus** only for `.webm`)
- [ ] **Export — UX for remux failures** — clearer errors when MP4/MKV reject a stream (codec / licensing / mux constraints); link to transcode presets above
- [ ] **Export — MOV mux** and/or **ProRes / DNx** intermediate paths for pro handoff
- [ ] **Playback / export — Subtitles** — **WebVTT**, **SRT**, **TTML** (platform targets in `SUPPORTED_FORMATS.md`); **ASS/SSA** for advanced styling — not decoded, shown, or muxed today (see `MEDIA_FORMATS.md`)
- [ ] **Playback — Multi-audio** stream selection (today: first decodable audio only)

---

## How to use this file

| Role | Use |
|------|-----|
| **Product / roadmap** | **`docs/phases-ui.md`** + **`docs/FEATURES.md`** |
| **Infra & engine completeness** | This file (Phases 0–3) |
| **Design detail** | **`docs/architecture.md`**, **`docs/EXTERNAL_AI.md`** |

When **Phase U1–U5** items in `phases-ui.md` ship, update **`FEATURES.md`**; update **this file** only if you also complete or add **engineering** deliverables (e.g. new CI job, new crate).
