# UI test phases (`docs/phases-ui-test.md`)

### Status (last updated 2026-04-17)

| Step | Status |
|------|--------|
| 1 — Strict versioning (`i-slint-backend-testing` pinned to exact Slint version) | **Done** — `=1.16.0` in workspace `[workspace.dependencies]`; `reel-app` lists it under `[dev-dependencies]` |
| 1 — Headless harness (`init_integration_test_with_system_time`) | **Done** — `ui_test_support::init()` in `crates/reel-app/src/main.rs` calls `i_slint_backend_testing::init_integration_test_with_system_time()` behind `std::sync::Once` |
| 1b — Mockable media seam | **Done** — `&dyn MediaProbe` on `project_from_media_path_with_probe`, `EditSession::open_media_with_probe`, `insert_clip_at_playhead_with_probe`, `insert_audio_clip_at_playhead_with_probe`; zero-arg APIs delegate to `FfmpegProbe`. `FakeProbe` lives in `session::tests_fake_probe` and is used from `session` tests and from `main.rs` test modules |
| 1b — Mockable save-dialog seam | **Done** — `SaveDialogProvider` + `RfdSaveDialog` + `StubSaveDialog` (in `export_payload_tests`). `prepare_export_job(&EditSession, preset_index, &dyn SaveDialogProvider) -> ExportPreflight` holds all branching; `install_export_preset_confirm_callback` only dispatches on `ExportPreflight` using `RfdSaveDialog` |
| 1b — Mockable playback engine (`player.rs`) | **Deferred** — no UI test yet needs decoded frames or audio timing |
| 2 — Integration tests (Open → Edit → Export) | **Done** — see [Committed tests](#committed-tests-inventory) below |
| 3 — Visual regression (software renderer + golden PNGs) | **Pending** |
| 4 — Output validation (post-export file on disk) | **Done** — `export_payload_tests::roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux` runs real `export_concat_timeline` on `reel-core/tests/fixtures/tiny_h264_aac.mp4` when fixture + `ffmpeg` exist; skips otherwise. Complements `crates/reel-core/tests/export_web_formats.rs` |

**Threading:** Slint’s platform installs once per process on the thread that first constructs `AppWindow`. Parallel `#[test]` functions can hit *"Slint platform was initialized in another thread"*. All Slint-touching assertions stay in **one** `#[test]` (`window_boots_and_round_trips_basic_properties`) until `serial_test` or a dedicated `[[test]]` binary serializes UI tests.

---

### Committed tests (inventory)

All live in `crates/reel-app/src/main.rs` unless noted.

**`ui_smoke_tests` (Slint + `i-slint-backend-testing`)**

| Test | What it covers |
|------|----------------|
| `window_boots_and_round_trips_basic_properties` | Boots `AppWindow`, round-trips defaults (`media_ready`, export preset index 0..=3, footer visibility, etc.). Opens media via `EditSession::open_media_with_probe` + `FakeProbe`, calls `sync_menu`, asserts `duration_ms`, `video_track_lanes`, `rotate_enabled`, `trim_enabled`, `close_enabled`. Installs `install_export_preflight_callback` and drives `invoke_file_export()` three ways: valid timeline → preset sheet opens; In/Out past clips → status *"No clips in the In/Out range"*; cleared project → silent. |

**`export_payload_tests` (no Slint — safe on any thread)**

| Test | What it covers |
|------|----------------|
| `payload_single_clip_full_span_when_no_range` | `export_timeline_payload` single clip, no markers |
| `payload_respects_in_out_range_markers` | Sliced span when In/Out set |
| `payload_returns_none_when_range_outside_all_clips` | Empty range → `None` (same signal as UI empty-range export) |
| `payload_exposes_first_audio_lane_when_present` | Video + inserted audio lane → `Some` audio spans |
| `preflight_invalid_preset_index_returns_status_and_skips_dialog` | Bad preset index → `Status`, no save dialog |
| `preflight_empty_range_returns_status_before_dialog` | Markers past clips → `Status`, no save dialog |
| `preflight_cancelled_save_dialog_returns_noop` | Stub returns `None` → `NoOp` |
| `preflight_wrong_extension_returns_status` | Path extension mismatch vs preset |
| `preflight_happy_path_returns_spawn_with_expected_payload` | `Spawn` with `Mp4Remux`, spans, dest |
| `preflight_spawn_preserves_marker_range_for_status_text` | `Spawn.range_ms` when markers set |
| `preflight_preset_index_maps_to_save_dialog_fmt` | Indices 1–3 → `Mp4H264Aac`, `WebmVp8Opus`, `MkvRemux` + matching extension |
| `roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux` | Real probe, real export, re-probe output (Phase 4) |

**Still not covered by automated tests:** driving **`on_export_preset_confirm`** (preset sheet **Confirm**) through the real Slint callback with an injectable `SaveDialogProvider` — production uses `RfdSaveDialog` inside `install_export_preset_confirm_callback`. All **decision** branches of that path are covered via **`prepare_export_job`** + `StubSaveDialog`; wiring DI at the callback install site would be the next step for a full UI-thread end-to-end confirm.

---

### Phase 1: Foundation & dependency injection

**Goal:** Enable headless testing and separate UI side effects from the core video engine.

1. **Strict versioning:** `i-slint-backend-testing` must match the resolved `slint` crate version (Slint does not semver internal testing crates). Pin at workspace level:

    ```toml
    # Cargo.toml (workspace)
    slint = "1.8"                       # lockfile resolves to 1.16.x today
    i-slint-backend-testing = "=1.16.0" # bump in lockstep with slint
    ```

2. **Mockable engine (probe seam):** `MediaProbe` in `reel-core`; `reel-app` session/project paths take `&dyn MediaProbe` on `_with_probe` helpers; unprobed entry points use `FfmpegProbe`. `FakeProbe` returns deterministic metadata for fast tests.

**Deferred:** abstracting `player.rs` (decode threads, `cpal`, etc.) until a test needs frames or timing.

---

### Phase 2: Integration testing (user flow)

**Goal:** Automate Open → Edit → Export where it matters for regressions.

- **Pattern:** `ui_test_support::init()` then `AppWindow::new()` under `i-slint-backend-testing`.
- **Note:** Prefer `prepare_export_job` tests for exhaustive branch coverage; keep Slint tests small to avoid thread contention.

---

### Phase 3: Visual regression (effects / pixels)

**Goal:** Effects like grayscale or resize change pixels as expected.

1. Use `renderer-software` in tests to capture the UI buffer.
2. Snapshot: fixed resolution → apply effect → save frame → compare to golden PNG (or hash), with `UPDATE_EXPECTATIONS` to refresh goldens.

---

### Phase 4: Output validation (export file)

**Goal:** Exported files exist and are valid media.

- **Shipped:** `roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux` (see inventory).
- **Pattern:** `std::fs::metadata`, then `FfmpegProbe` / core probes on the output file.

---

## How to use this with Claude / Cursor

1. **Context:** Point at workspace `Cargo.toml`, `crates/reel-app/src/main.rs` (test modules + `prepare_export_job`), and `crates/reel-app/ui/app.slint`.
2. **Prompt example:** *"Following `docs/phases-ui-test.md`, add a `prepare_export_job` branch test for … using `StubSaveDialog`."*
3. **Slint tests:** Do not split into multiple `#[test]`s that each construct `AppWindow` without `serial_test` or a single-threaded test target — see threading note above.
