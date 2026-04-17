# Phase status

## Phase 0 — Infrastructure & observability ✅

- [x] Cargo workspace: `reel-core`, `reel-app`, `reel-cli`
- [x] Makefile targets: `setup`, `build`, `lint`, `test`, `run`, `run-cli`, `fixtures`, `clean`, `ci`
- [x] `uv`-managed Python sidecar with stub `facefusion_bridge.py`
- [x] `tracing` + `tracing-subscriber` initialization (`REEL_LOG*` env vars)
- [x] Child-process stdout/stderr piped into `tracing` via `spawn_logged_child`
- [x] GitHub Actions CI: `macos-14`, runs `make setup lint test`

## Phase 1 — Media engine & TDD ✅

- [x] `MediaProbe` trait + `FfmpegProbe` real impl (ffmpeg-next 7.1, pinned ffmpeg@7)
- [x] Unrecognized-audio handling: `WARN` + `audio_disabled: true`, never propagate
- [x] Three committed fixtures under `crates/reel-core/tests/fixtures/` with `scripts/generate_fixtures.sh`
- [x] `Project` serde v2 + round-trip + insta snapshot + `serde(flatten)` extension maps (project / clip / track) + v1→v2 migration
- [x] `ProjectStore` debounced atomic autosave (`.tmp → rename`) with tests
- [x] `reel-cli probe <path>` emits metadata JSON (verified end-to-end)

## Phase 2 — Player window ✅

- [x] Slint `AppWindow`: viewport + timeline stub + transport bar
- [x] Video decoder thread: packet read → decoder → RGBA scaler → `SharedPixelBuffer` → UI
- [x] Audio decoder thread: packet read → decoder → f32 stereo resample → ringbuf → cpal output
- [x] `AudioClock` master clock; video thread sleep/drop policy
- [x] Fast-seek on slider drag (drain channel → flush → nearest keyframe → advance to target)
- [x] `rfd::FileDialog` native Open panel; `REEL_OPEN_PATH` retained as one-shot startup auto-load
- [x] UI gates: Play + Slider disabled until `media-ready`; Play ignored server-side when no source loaded
- [x] Crash-proofing: `try_open_video` / `try_open_audio` catch panics; `frame_to_rgba` is bounds-safe on bad strides/scaler errors
- [x] Separate crossbeam channels for video and audio threads (fixes the prior fan-out bug where Open/Play were split across threads)
- [ ] End-to-end manual verification on a real video (pending developer run of `make run`)

## Phase 3 — FaceFusion bridge ✅ (placeholder transforms)

- [x] `reel_core::sidecar::SidecarClient` — long-lived Python child (`uv run python facefusion_bridge.py` from `sidecar/`), multiplexed by request `id`, reader thread, timeout + crash surfacing
- [x] `reel_core::logging::spawn_child_with_logged_stderr` — variant that leaves stdin/stdout open for IPC
- [x] `reel_core::media::grab_frame` — single-frame decode to tightly-packed RGBA8
- [x] `sidecar/facefusion_bridge.py` — real line loop with `ping` / `swap` / `shutdown`, `identity` + `invert` transforms, `sleep_ms`/`crash` test hooks
- [x] `reel-cli swap <path> --out <png> [--model identity|invert]` — end-to-end pipeline
- [x] Integration tests: ping, identity/invert round-trip, timeout, crash, unknown-model, local length validation (7 total)
- [x] Python pytest: transform unit tests + stdio protocol round-trip (5 total)
- [x] `uv run pytest` wired into `make test`
- [ ] Real FaceFusion model install + new `TRANSFORMS["swap_face"]` entry (deferred; out of scope for this iteration)
- [ ] UI surface (AI panel) for invoking swap (deferred; CLI-only this phase)

## Phase U1 — Menus, timeline scrub, export tests (see `docs/phases-ui.md`) 🚧

- [x] Slint `MenuBar`: File / Edit / Window / Help (native bar on macOS)
- [x] File: Open, Close, Revert, New Window, Save (.reel), Insert Video (queued path), Export (ffmpeg)
- [x] Edit: Undo / redo (session stack; timeline wiring later)
- [x] Window: Always on top, Fit / Fill / Center (viewport `image-fit`)
- [x] Help: secondary `HelpWindow` with bundled `docs/HELP.md` text
- [x] Timeline strip: `Slider` scrubbing → seek
- [x] `reel-core` ffmpeg CLI export + `target/reel-export-verify/` integration test
- [x] `reel-app` unit tests: `session`, `shell`, `project_io`

## Phase 4 — Documentation & polish (not started)

- `docs/architecture.md` ✅ (covers Phases 0–3)
- `docs/phases-ui.md` ✅ (revised UI roadmap)
- `docs/HELP.md` ✅ (in-app help source)
- `docs/USER_GUIDE.md` (pending)
- Slint UI density pass, icons, app bundle targets (pending)
