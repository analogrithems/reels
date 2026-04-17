## 📄 `phases-ui-testing.md`

### Status (as of 2026-04-17)

| Step | Status |
|------|--------|
| 1 — Strict versioning (`i-slint-backend-testing` pinned to exact slint version) | ✅ Shipped — pinned to `=1.16.0` in the workspace `[workspace.dependencies]`; `reel-app` consumes it as a dev-dep |
| 1 — Headless harness (`init_integration_test_with_system_time`) | ✅ Shipped — `ui_test_support::init()` in `crates/reel-app/src/main.rs`; first smoke test boots `AppWindow` and round-trips properties (`ui_smoke_tests::window_boots_and_round_trips_basic_properties`) |
| 1b — Mockable media seam | ✅ Shipped — `&dyn MediaProbe` threaded through `project_from_media_path_with_probe`, `EditSession::open_media_with_probe`, `insert_clip_at_playhead_with_probe`, `insert_audio_clip_at_playhead_with_probe`. `FakeProbe` helper in `session.rs` tests (see `session::tests::open_and_insert_via_fake_probe_no_ffmpeg`) exercises Open → Insert with no ffmpeg on disk |
| 1b — Mockable save-dialog seam | ✅ Shipped — `SaveDialogProvider` trait + `RfdSaveDialog` (production) + `StubSaveDialog` (tests). `on_export_preset_confirm` body extracted into `prepare_export_job(&session, preset_index, &dyn SaveDialogProvider) -> ExportPreflight`; the callback just dispatches on the enum. Lets unit tests cover every preset-confirm branch without a windowing system |
| 1b — Mockable playback engine (`player.rs` threads) | ⏳ Deferred until a UI test actually needs it — most Phase 2 flows read UI state, not rendered frames |
| 2 — Integration tests (Open → Edit → Export) | ✅ Expanded. Three layers:<br>• **UI layer** — `ui_smoke_tests::window_boots_and_round_trips_basic_properties` opens a fake media path via `FakeProbe`, calls `sync_menu`, asserts the Slint menu-gate properties (`duration_ms`, `video_track_lanes`, `rotate_enabled`, `trim_enabled`, `close_enabled`), and drives three `invoke_file_export()` branches (valid payload opens preset sheet, empty range sets status, no project is silent) through the real `install_export_preflight_callback`.<br>• **Preset-confirm seam** — `on_export_preset_confirm` is now `prepare_export_job(&session, preset_index, &dyn SaveDialogProvider) -> ExportPreflight`. `StubSaveDialog` in `export_payload_tests` covers every branch: invalid preset index, empty In/Out range, cancelled save dialog, wrong extension for the chosen preset, happy-path `Spawn`, marker range carried through, and preset-index → `WebExportFormat` wiring for all four presets.<br>• **Export pipeline** — `export_payload_tests::payload_*` covers `export_timeline_payload` for single-clip, range-sliced, out-of-range, and first-audio-lane cases.<br>Remaining: live click through `invoke_export_preset_confirm()` exercising the Slint callback end-to-end (requires wiring `SaveDialogProvider` injection at `install_export_preset_confirm_callback` call-site, or moving it into `main()`'s state plumbing). |
| 3 — Visual regression (software renderer + golden PNGs) | ⏳ Pending |
| 4 — Output validation (post-export file check) | ✅ Shipped — `export_payload_tests::roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux` opens the real fixture through `FfmpegProbe`, builds a payload, runs **actual** `export_concat_timeline`, then re-probes the output (duration > 0, video stream present). Skips cleanly when the fixture or `ffmpeg` binary is missing. Complements the long-standing `crates/reel-core/tests/export_web_formats.rs` preset matrix |

**Threading note:** Slint's platform is installed once per process and pinned to the thread that first constructs an `AppWindow`. `cargo test` runs tests in parallel on different threads, so each additional `#[test]` panics with *"Slint platform was initialized in another thread"*. Today we keep all UI assertions in a single `#[test]` function to serialize them. When this gets unwieldy we should add `serial_test` (or move UI tests into their own `[[test]]` target forced to one thread) — don't split into multiple `#[test]`s without one of those in place.

### **Phase 1: Foundation & Dependency Injection**
**Goal:** Enable headless testing and separate UI side-effects from the core video engine.

1.  **Strict Versioning:** Ensure `i-slint-backend-testing` matches the `slint` version exactly in `Cargo.toml` (Slint does not follow semver for internal testing crates). **Done** — pin lives at workspace level so all member crates share it:
    ```toml
    # Cargo.toml (workspace)
    slint = "1.8"                       # lockfile resolves to 1.16.0 today
    i-slint-backend-testing = "=1.16.0" # bump in lockstep with slint
    ```
2.  **The "Mockable" Engine:** Abstract the video processing backend into a Trait. This allows tests to inject a fake that returns static data instead of processing 4K video, making tests instant.

    **Phase 1b (shipped — probe seam only).** Instead of abstracting the whole video engine upfront, we injected the **probe** — the one piece every edit flow calls before ffmpeg even starts. `MediaProbe` already lived in `reel-core` as a trait; `crates/reel-app/src/session.rs` + `project_io.rs` now take `&dyn MediaProbe` on `_with_probe` variants, and the zero-arg versions (`open_media`, `insert_clip_at_playhead`, `insert_audio_clip_at_playhead`, `project_from_media_path`) are thin wrappers that pass in the real `FfmpegProbe`. `FakeProbe` in `session.rs` tests returns deterministic `MediaMetadata` and counts calls so regressions ("someone hard-coded `FfmpegProbe::new()` back into the edit flow") surface fast.

    **Still deferred:** the playback engine in `crates/reel-app/src/player.rs` (video + audio decode threads, `cpal` output, `ffmpeg_next` direct use). We'll abstract that only when a UI test needs rendered frames or timing behavior; Phase 2 flows that just verify menu / property state should not need it.

### **Phase 2: Integration Testing (The "User Flow")**
**Goal:** Automate the "Open -> Edit -> Export" journey.

* **Pattern:** Use `i-slint-backend-testing::init_integration_test_with_system_time()`.
* **Instruction for AI:** > "Create a test module `tests/ui_workflow.rs`. Use `slint::spawn_local` to wrap the test logic. Identify UI elements using `ElementHandle::find_by_accessible_label`. For 'Edits', simulate a click on the effect button and verify the internal `App` state property changed before proceeding to the 'Export' button."

### **Phase 3: Visual Regression (The "Effect" Check)**
**Goal:** Ensure effects like "Grayscale" or "Resize" actually work visually.

1.  **Software Rendering:** Use the `renderer-software` feature of Slint during tests to capture the UI buffer.
2.  **Snapshot Logic:** * Initialize the UI at a specific resolution.
    * Apply an effect.
    * Save the current frame to a `temp` directory.
    * Compare the hash or pixel-diff against `tests/fixtures/golden_sepia.png`.
* **Instruction for AI:**
    > "When building effect tests, implement a helper function `capture_ui_frame(app) -> ImageBuffer`. If the environment variable `UPDATE_EXPECTATIONS` is set, save the current frame as the new truth. Otherwise, fail the test if pixels deviate by more than 1%."

### **Phase 4: Output Validation (The "Export" Check)**
**Goal:** Verify the final file is valid and formatted correctly.

* **Logic:** After the UI "Export" button click resolves, the test must check the filesystem.
* **Instruction for AI:**
    > "Add a post-test hook that uses `std::fs::metadata` to check file existence. Use a lightweight crate like `ffprobe` or `mediainfo` (if available in the environment) to verify the output WebM/MP4 matches the expected resolution and bitrate set in the UI."

---

## **How to use this with Claude/Cursor**

1.  **Context:** Upload the `Cargo.toml` and your main `.slint` file.
2.  **Prompt:** *"I am following the `phases-ui-testing.md` strategy. Please implement Phase 2 (Integration Testing) for the 'Apply Filter' workflow. Use the `i-slint-backend-testing` crate and find the 'Sepia' button by its accessible label."*
3.  **Review:** Ensure the AI uses `slint::run_event_loop_until_quit()` at the end of the test, or the async blocks will never execute.
