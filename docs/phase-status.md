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
- [x] `Project` serde + round-trip + insta snapshot + `deny_unknown_fields` test
- [x] `ProjectStore` debounced atomic autosave (`.tmp → rename`) with tests
- [x] `reel-cli probe <path>` emits metadata JSON (verified end-to-end)

## Phase 2 — Player window ⏳

- [x] Slint `AppWindow`: viewport + timeline stub + transport bar
- [x] Video decoder thread: packet read → decoder → RGBA scaler → `SharedPixelBuffer` → UI
- [x] Audio decoder thread: packet read → decoder → f32 stereo resample → ringbuf → cpal output
- [x] `AudioClock` master clock; video thread sleep/drop policy
- [x] Fast-seek on slider drag (drain channel → flush → nearest keyframe → advance to target)
- [x] `Open…` button reads path from `REEL_OPEN_PATH` env var (dev affordance; native file dialog lands Phase 3)
- [ ] End-to-end manual verification on a real video (pending developer run of `make run`)
- [ ] Replace env-var `Open…` affordance with `rfd::FileDialog::new().pick_file()` once upstream compat is confirmed on macOS 14

## Phase 3 — FaceFusion bridge (not started)

Not in scope for this iteration. Hooks already present:

- `reel_core::logging::spawn_logged_child`
- `reel_core::media::decoder::{DecodeCmd, DecodedFrame}` types
- `sidecar/facefusion_bridge.py` stub + stdio contract

## Phase 4 — Documentation & polish (not started)

- `docs/architecture.md` ✅ (initial pass)
- `docs/USER_GUIDE.md` (pending)
- Slint UI density pass, icons, app bundle targets (pending)
