---
title: "UI test harness phases"
status: living
phases: [UT1, UT2, UT3, UT4]
last_reviewed: 2026-04-24
owners: [core, ui]
---

# UI test phases (`docs/phases-ui-test.md`)

### Status (last updated 2026-04-17)

| Step | Status |
|------|--------|
| 1 — Strict versioning (`i-slint-backend-testing` pinned to exact Slint version) | **Done** — `=1.16.0` in workspace `[workspace.dependencies]`; `reel-app` lists it under `[dev-dependencies]` |
| 1 — Headless harness (`init_integration_test_with_system_time`) | **Done** — `ui_test_support::init()` in `crates/reel-app/src/lib.rs` calls `i_slint_backend_testing::init_integration_test_with_system_time()` behind `std::sync::Once` |
| 1b — Mockable media seam | **Done** — `&dyn MediaProbe` on `project_from_media_path_with_probe`, `EditSession::open_media_with_probe`, `insert_clip_at_playhead_with_probe`, `insert_audio_clip_at_playhead_with_probe`; zero-arg APIs delegate to `FfmpegProbe`. `FakeProbe` lives in `session::tests_fake_probe` and is used from `session` tests and from `lib.rs` unit test modules |
| 1b — Mockable save-dialog seam | **Done** — `SaveDialogProvider` + `RfdSaveDialog` + `StubSaveDialog` (in `export_payload_tests`). `prepare_export_job(&EditSession, preset_index, &dyn SaveDialogProvider) -> ExportPreflight` holds all branching; `install_export_preset_confirm_callback` only dispatches on `ExportPreflight` using `RfdSaveDialog` |
| 1b — Mockable playback engine (`player.rs`) | **Deferred** — no UI test yet needs decoded frames or audio timing |
| 1c — Design assets (v0 / Lucide) | **Done** — transport + menubar icons live under `crates/reel-app/ui/icons/lucide/*.svg`; `build.rs` emits `cargo:rerun-if-changed` for each SVG so icon edits rebuild Slint. Theme tokens in `ui/theme.slint` |
| 2 — Integration tests (Open → Edit → Export) | **Done** — see [Committed tests](#committed-tests-inventory) below |
| 3 — Visual regression (software renderer + golden PNGs) | **Done** — integration test `tests/ui_visual_golden.rs` installs Slint’s headless [`MinimalSoftwareWindow`](https://docs.rs/slint/latest/slint/platform/software_renderer/struct.MinimalSoftwareWindow.html) (`renderer-software`), sizes `AppWindow` to 400×320, `take_snapshot()`, compares RGBA to `tests/golden/app_window_default.png`. Regenerate after **any** intentional chrome change (menubar, Theme, empty-window layout): `UPDATE_UI_GOLDENS=1 cargo test -p reel-app --test ui_visual_golden`. Runs in its **own process** (not the `i-slint-backend-testing` unit-test stack). |
| 4 — Output validation (post-export file on disk) | **Done** — `export_payload_tests::roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux` runs real `export_concat_timeline` on `reel-core/tests/fixtures/tiny_h264_aac.mp4` when fixture + `ffmpeg` exist; skips otherwise. Complements `crates/reel-core/tests/export_web_formats.rs` |

**Threading:** Slint’s platform installs once per process on the thread that first constructs `AppWindow`. Parallel **library** `#[test]` functions can hit *"Slint platform was initialized in another thread"*. Unit-test Slint coverage stays in a single `#[test]` (`window_boots_and_round_trips_basic_properties`) until `serial_test` serializes more. The **visual golden** test is a separate **integration-test** binary (`tests/ui_visual_golden.rs`) — its own process, so it can install the headless software platform without conflicting with `i-slint-backend-testing`.

**Verification:** Run `cargo test -p reel-app` (library + integration golden). Expect **98** library tests + **1** golden (as of the last doc revision).

---

### Committed tests (inventory)

Library unit tests live in `crates/reel-app/src/lib.rs` unless noted. Integration tests live under `crates/reel-app/tests/`.

**`ui_smoke_tests` (Slint + `i-slint-backend-testing`)**

| Test | What it covers |
|------|----------------|
| `window_boots_and_round_trips_basic_properties` | Boots `AppWindow`, round-trips defaults (`media_ready`, export preset index 0..=3, footer visibility, etc.). Asserts **View → Video / Audio / Subtitle Tracks** default to **visible** (prefs-backed, v0-aligned). Opens media via `EditSession::open_media_with_probe` + `FakeProbe`, calls `sync_menu`, asserts `duration_ms`, `video_track_lanes`, `rotate_enabled`, `trim_enabled`, `close_enabled`. Installs `install_export_preflight_callback` and drives `invoke_file_export()` three ways: valid timeline → preset sheet opens; In/Out past clips → status *"No clips in the In/Out range"*; cleared project → silent. |

**`timeline_chips` (`crates/reel-app/src/timeline_chips.rs`)**

| Test | What it covers |
|------|----------------|
| `single_clip_project_has_one_video_chip` | Probed fixture → one video chip row |
| `single_media_mode_shows_embedded_audio_lane_from_probe` | Single-file mode: at least one audio lane from container metadata |
| `project_document_mode_lists_subtitle_tracks` | **`opened_from_project_document == true`**: appending `TrackKind::Subtitle` sets `subtitle_project_n` / `subtitle_display_n` to **1** |
| `single_media_merges_subtitle_project_count_with_probe_streams` | **`opened_from_project_document == false`**: empty subtitle track + fixture (no sub streams) still yields display count **1** (merge rule) |
| `single_media_one_video_stream_collapses_timeline_clips_to_one_chip` | Single-file mode with **one** container video stream: filmstrip row **0** is a **single** full-width chip even when the edit timeline has multiple clips on that track |

**`session` tests (`crates/reel-app/src/session.rs`)**

| Test | What it covers |
|------|----------------|
| `add_subtitle_track_appends_empty_lane` | After `open_media` on `tiny_h264_aac` fixture, `add_subtitle_track()` adds one `TrackKind::Subtitle` row; **Undo** removes it. |

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

**`ui_visual_golden` (integration test — headless `MinimalSoftwareWindow` + `renderer-software`)**

| Test | What it covers |
|------|----------------|
| `app_window_matches_golden_png` | Full `AppWindow` at 400×320 (empty project, **no media**); `Window::take_snapshot()`; byte-for-byte RGBA compare to `tests/golden/app_window_default.png`. Catches unintended shifts to **menubar**, **Theme**, and default window chrome. |

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

3. **Design parity (v0):** Icon SVGs and `ui/theme.slint` are part of the test contract indirectly: **golden** snapshots and **timeline_chips** / **session** tests guard data paths; **Lucide** assets must stay build-linked via `build.rs`.

**Deferred:** abstracting `player.rs` (decode threads, `cpal`, etc.) until a test needs frames or timing.

---

### Phase 2: Integration testing (user flow)

**Goal:** Automate Open → Edit → Export where it matters for regressions.

- **Pattern:** `ui_test_support::init()` then `AppWindow::new()` under `i-slint-backend-testing`.
- **Note:** Prefer `prepare_export_job` tests for exhaustive branch coverage; keep Slint tests small to avoid thread contention.
- **Subtitle / multi-lane model:** Covered without Slint in **`timeline_chips`** (merge + project-document mode) and **`session::tests::add_subtitle_track_*`**.

---

### Phase 3: Visual regression (effects / pixels)

**Goal:** Catch unintended layout/theme changes by comparing a rendered framebuffer to a committed PNG.

**Shipped:** `crates/reel-app/tests/ui_visual_golden.rs` installs [`slint::platform::software_renderer::MinimalSoftwareWindow`](https://docs.rs/slint/latest/slint/platform/software_renderer/struct.MinimalSoftwareWindow.html) (no GPU, no `DISPLAY`), builds `AppWindow`, sets a fixed physical size, calls `take_snapshot()`, compares RGBA pixels to `tests/golden/app_window_default.png`.

**Why not `i-slint-backend-testing` here?** That backend does not render pixels; pixel tests use a **separate** integration-test process with the minimal software window adapter (see Slint’s own `partial_renderer` tests).

**Refresh goldens** after intentional UI changes (menubar icons, Theme, transport overlay layout, default window size):

```sh
UPDATE_UI_GOLDENS=1 cargo test -p reel-app --test ui_visual_golden
```

Then commit the updated `app_window_default.png`. Font rasterization can differ slightly across OS versions; if CI diverges from your machine, regenerate on the CI image or add a small tolerance later.

---

### Phase 4: Output validation (export file)

**Goal:** Exported files exist and are valid media.

- **Shipped:** `roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux` (see inventory).
- **Pattern:** `std::fs::metadata`, then `FfmpegProbe` / core probes on the output file.

---

## How to use this with Claude / Cursor

1. **Context:** Workspace `Cargo.toml`, `crates/reel-app/build.rs`, `crates/reel-app/src/lib.rs` (test modules + `prepare_export_job`), `crates/reel-app/src/timeline_chips.rs`, `crates/reel-app/tests/ui_visual_golden.rs`, `crates/reel-app/ui/app.slint`, `crates/reel-app/ui/theme.slint`, `crates/reel-app/ui/icons/lucide/`.
2. **Prompt example:** *"Following `docs/phases-ui-test.md`, add a `prepare_export_job` branch test for … using `StubSaveDialog`."*
3. **Slint tests:** Do not split into multiple `#[test]`s that each construct `AppWindow` without `serial_test` or a single-threaded test target — see threading note above.
