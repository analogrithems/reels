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
- [x] **Headless harness** — `ui_test_support::init()` (`crates/reel-app/src/lib.rs`) wraps `init_integration_test_with_system_time()` behind a `Once`; first smoke test boots `AppWindow`, verifies startup defaults, and round-trips `export-preset-index` across the full 0..=9 range.
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
- [x] **Export — UX for remux failures** — when an **MP4 remux**, **MKV remux**, or **MOV remux** preset fails, the status line appends a hint pointing at the matching transcode preset (MP4 remux → **MP4 — H.264 + AAC** / **HEVC + AAC**; MKV remux → **WebM — VP9 + Opus**; MOV remux → transcode via an MP4 H.264/HEVC preset then rename). Non-remux presets already transcode, so their failures aren't preset-choice problems and the status stays terse
- [x] **Export — MOV mux + ProRes / DNx intermediates** — **MOV — remux** ships with stream-copy when source codecs are MOV-compatible (libx264 transcode fallback when `-vf` is active). **MOV — ProRes 422 HQ + PCM** (`prores_ks -profile:v 3 -pix_fmt yuv422p10le` + `pcm_s16le`) and **MKV — DNxHR HQ + PCM** (`dnxhd -profile:v dnxhr_hq -pix_fmt yuv422p` + `pcm_s16le`) add pro-handoff intermediates; both always transcode regardless of vf-chain presence since they target specific codecs
- [~] **Playback / export — Subtitles** — **SRT + WebVTT import → burn-in at export** shipped (**File → Insert Subtitle…**, `Ctrl+Alt+I`; shared parser handles `HH:MM:SS[.,]mmm` and WebVTT's `MM:SS.mmm`, cue settings, `WEBVTT` / `NOTE` / `STYLE` blocks; export appends `subtitles='<path>'` to the `-vf` chain so captions are rendered on the final oriented/scaled frame). **Remaining:** live preview of cue text, **TTML** parser, **ASS/SSA** styling, subtitle-as-stream mux (soft subtitles) — all still open (see `MEDIA_FORMATS.md`)
- [~] **Playback — Multi-audio stream selection** — **Edit → Audio Track** submenu lists every decodable audio stream probed off the source (label combines stream title, language, and codec; gated on `>= 2` streams so single-track files don't show a phantom picker). Selection is per-clip on the primary track (`Clip.audio_stream_index: Option<u32>`; `None` = legacy "first decodable"), undoable, and honored by the preview audio thread — each segment re-opens `AudioCtx` against the stored container index (falls back to `best(Audio)` when stale, e.g. after a source swap). **Remaining:** export-side stream selection (today the ffmpeg audio-graph still picks the first decodable stream per clip) and a dubs-aware default for multi-track files that mark the preferred language in metadata.

---

## UI initiative checklist (product — see `phases-ui.md`)

Implementation tracking for **menu- and timeline-visible** features described in **`docs/phases-ui.md`** (U2-d … U4, **U3** presets). Uncheck until shipped; update **`docs/FEATURES.md`** when done.

- [x] **File → Open Recent** (**U4**) — MRU **projects** and **media**; **Clear Recent**; persistence + prune on missing file (per-entry remove optional / not shipped)
- [x] **Edit** — **Rotate 90°** left/right, **flip** horizontal/vertical (**QuickTime-style**) — per-clip, **Ctrl+R** / **Ctrl+Shift+R**; preview post-scaler + ffmpeg `-vf` on export
- [x] **Trim Clip…** (**U2**) — **Edit → Trim Clip…**; per-clip begin/end in source seconds, inline validation, undoable (see **`docs/FEATURES.md`**, **`docs/KEYBOARD.md`**)
- [x] **Per-clip timeline trim handles + ripple** (**U2-c**) — 6-px TouchArea zones on the left / right edges of each filmstrip chip emit a fractional drag delta that Rust multiplies by the chip's current `(out_point - in_point)` and delegates to `session::trim_clip_by_edge_drag` → `session::trim_clip` (invariants: `begin >= 0`, `begin < end`, duration `>= 50 ms`, `end <= source_duration`; rejects without pushing undo). Gated on real project-backed clips — synthetic single-media container-stream chips have `clip_id: ""` and expose no handles. Ripple is **automatic**: the project's sequential clip model has no absolute timeline positions, so trimming any clip pulls the downstream clips forward by the same delta with no extra bookkeeping.
- [x] **Timeline** — **two markers** on the seek bar (in/out range) — **Edit → Set In/Out Point** + **Clear Range Markers**; keys **I** / **O** / **Alt+X**; cyan/magenta overlay with tinted range on the slider; ephemeral per session (clears on close / new project). **Range-scoped export** (both markers set → ffmpeg concat is sliced to the range on video and first-audio tracks, rebased to sequence 0) — **shipped**. **Follow-on:** on-timeline drag handles and any additional batch operations that should honor the range.
- [x] **Edit → Overlay Audio…** (**U2-e**) — append a fresh `TrackKind::Audio` lane and insert the picked file on it; each invocation stacks another lane so the U2-b-export `amix` dispatcher mixes all overlays into the render. Undo reverses both the lane creation and the clip insert in one step. Gated on `media-ready`; preview audio now mixes every lane (see **Preview-side multi-lane mix**) so overlays are audible during playback.
- [x] **Edit → Replace Audio…** (**U2-e**) — probe-first (bad files fail cleanly with no undo slot burned), then mark every primary-track clip `audio_mute = true` **and** append a fresh `TrackKind::Audio` lane carrying the replacement clip under **one** undo snapshot. `Edit → Undo` restores the exact pre-replace mute states (pre-existing mutes preserved) and drops the new lane atomically. Idempotent across repeat calls. Gated on `media-ready`; preview audio mixes every lane so the replacement is audible immediately.
- [x] **Edit → Replace & Clear Other Audio…** (**U2-e**) — destructive sibling of Replace Audio. Probes first, then under one undo snapshot: mutes every primary-track clip, removes every existing `TrackKind::Audio` lane (orphaned clips dropped via `remove_track_at`, indices iterated descending so later removals don't shift earlier ones), and appends a single fresh lane carrying the replacement clip. The project ends up with exactly one audio source. `Edit → Undo` restores the full pre-call project — mute states, existing lanes, and their clips — byte-exact. No-existing-lanes case is a no-op for the clear pass and behaves identically to Replace Audio. Gated on `media-ready`.
- [~] **Edit → Mute Clip Audio** (**U2-e**) — per-clip `audio_mute` toggle, undoable; export emits `-an` when **every** primary-track clip is muted and there's no dedicated audio lane. **Partial-clip mute** now ships via **silence substitution**: the export thread pre-generates an `anullsrc` WAV (sized to the longest muted span) and builds a synthetic audio concat lane (`reel_core::build_mute_substitution_lane`) that keeps each unmuted clip's embedded audio and swaps silence in for muted spans, muxed alongside the video concat. **Per-lane gain** ships on the export/data side (see below); the remaining U2-e gap is a UI affordance to drive it.
- [x] **Per-lane audio gain** (**U2-e**) — `Track::gain_db: f32` (serde-defaulted to `0.0`, unity-skip so existing projects round-trip byte-stable). `EditSession::set_audio_track_gain_db(lane, db)` clamps to `[-40, +40]` dB, folds NaN → `0.0`, dedupes redundant writes so slider jitter doesn't pollute undo, and is otherwise undoable. Export threads every lane's `gain_db` through `export_concat_with_audio_lanes_oriented_with_gains` → `build_amix_filter_complex_with_gains`, which prepends `[i+1:a:0]volume=XdB[aI]` on each non-unity lane before feeding amix. Zero-gain (unity) case is byte-stable with the pre-gain filter output. A single lane with non-unity gain is routed through `amix=inputs=1` (a passthrough) so `volume=XdB` has somewhere to slot in — pays one transcode to avoid duplicating the codec dispatch that the amix path already solves. Audio-lane row labels append a `· +X.Y dB` suffix when the lane is off-unity (sign-preserved so cuts and boosts read differently at a glance); unity lanes keep their pre-gain label text so existing snapshots don't drift. **Edit → Audio Lane Gain…** menu item opens a sheet (pattern cloned from **Trim Clip…**) with two numeric fields — 1-based lane number and dB — that delegates to `set_audio_track_gain_db`; the sheet prefills lane 1's current gain on open, refreshes the dB field when the lane number changes (`gain_lane_changed` callback), surfaces clamp/lane-OOR errors inline (leaving the sheet open for correction), and reports the applied (post-clamp) value on the status line after confirm. Menu gate requires `media-ready` AND at least one `TrackKind::Audio` lane. Escape and backdrop click cancel. Callback installation is factored into `install_audio_lane_gain_callbacks(window, session, debouncer, recent)` so the `ui_smoke_tests` headless-`AppWindow` smoke test exercises open / confirm / lane-OOR / cancel against the same production handlers. **Preview applies each lane's `gain_db`** end-to-end too: the audio thread converts `gain_db` to a linear multiplier once per `LoadTimeline` (`10^(dB/20)`, unity fast-path) and multiplies each sample on the way into the mix, so UI edits are audible in playback and match export.
- [x] **Preview-side multi-lane audio mix** (**U2-b** / **U2-e**) — the realtime audio thread was rewritten from a single `TimelineSync` to a `Vec<AudioLane>`, one per `TrackKind::Audio` lane that actually carries clips. Each lane keeps its own `AudioCtx`, pending-sample `VecDeque`, and `gain_linear` (`10^(dB/20)`). Each tick, `next_mixed_samples` refills empty lanes by decoding another packet (advancing `seg_idx` across concat boundaries, marking lanes `exhausted` when their segments run out), then `drain_and_mix(n)` pulls `min(pending.len)` samples from each active lane and sums into one buffer, applying per-lane gain. The mixed buffer goes through the existing `speed_carry` → ringbuf → cpal pipeline unchanged. When no dedicated audio lanes carry clips, the thread falls back to a single synthetic lane driven by the video timeline (embedded audio on each segment's file), preserving pre-multi-lane behavior. When all audio lanes exhaust but video is still playing, the thread silence-pads so the clock keeps advancing. Pure-math helpers (`db_to_linear`, `drain_and_mix`) are separated from decoder plumbing and covered by unit tests in `player::mix_tests` (unity fast-path, ±6 dB math, sum of two lanes with gain, exhausted-lane-is-silence, partial-drain leaves remainder pending).
- [x] **Multi-audio-lane export mix** (**U2-b-export**) — `export_concat_with_audio_lanes_oriented` dispatches on lane count: 0 → video-only (`-an` when every primary clip muted), 1 → existing dual-mux stream-copy path (kept when every lane is at unity gain), N≥2 **or** N=1 with a non-unity gain → `-filter_complex` with `[1:a:0]…[N:a:0]amix=inputs=N:duration=longest:normalize=0[aout]` plus per-lane `volume=XdB` prefilters, mapping `[aout]` and forcing aac (MP4/MOV/MKV) or libopus (WebM) since filter-graph output can't stream-copy. `normalize=0` keeps unit gain across mixed lanes. **Preview-side mixer shipped** (see **Preview-side multi-lane audio mix** above) so what you hear in playback matches what `amix` renders.
- [x] **Edit → Resize Video…** (**U2-f**) — per-clip scale percent (10–400%, 100% = identity); preset buttons (25/50/75/100/150/200%) + numeric entry; export-only (preview unchanged); composes with rotate/flip in a combined `-vf` chain; undoable
- [x] **View** — **Loop Playback** (primary-track sequence; **prefs** + **Ctrl+L** / **⌘L**)
- [x] **View** — **Show Video / Audio / Subtitle track rows** (each toggles timeline section; **prefs**; default all on)
- [x] **View** — **Zoom** (in / out / fit / actual size; **prefs** + **Ctrl+=** / **+-** / **0**); **Enter/Exit Fullscreen** (menu; **Esc**)
- [x] **Shell** — **Menubar** **Lucide** icons + shortcut annotations (**v0**-aligned; native OS bar may omit icons)
- [x] **Transport** — floating bar **Lucide** icons, **z-order** for click hit-testing, spacing vs **v0**
- [x] **File** — **New Subtitle Track** (**Ctrl+Shift+T**); **`TrackKind::Subtitle`** project lanes; timeline merge with container streams (**U2**)
- [x] **File → Insert Subtitle…** (**Ctrl+Alt+I**, **U2**) — picks a `.srt` or `.vtt`, inserts a clip at the playhead on the first `TrackKind::Subtitle` lane (splits under the playhead like video/audio insert); gated on an existing subtitle track; undoable
- [x] **Export — subtitle burn-in** (**U2**) — when the first subtitle lane has a clip, ffmpeg's `subtitles='<escaped path>'` filter is appended **last** in the `-vf` chain so captions burn onto the final oriented + scaled frame; forces the remux presets into a libx264 transcode (filters can't coexist with `-c copy`)
- [ ] **View** (optional) — **Zoom to Video**; **pan** when zoomed; **fullscreen** on playback toolbar
- [x] **Export** — **preset catalog** aligned with **`docs/SUPPORTED_FORMATS.md`** (web + mobile + pro-intermediate tiers). **Shipped today:** MP4 remux, **MP4 H.264 + AAC** (web-tier transcode), **MP4 HEVC + AAC** (mobile tier), **WebM VP8 / VP9 / AV1 + Opus**, MKV remux, **MOV remux**, **MOV ProRes 422 HQ + PCM** + **MKV DNxHR HQ + PCM** intermediates (10-row preset sheet). **Open (lower priority):** explicit per-preset resolution/bitrate fields.
- [~] **Accessibility audit** (**U4**) — in progress. Labeled so far: every transport-bar TouchArea (skip-start / rewind / play / fast-forward / skip-end / speed / loop / mute / fullscreen / tools) carries `accessible-role: button;` + a stateful `accessible-label` (play/pause, mute/unmute, loop-on/off, fullscreen enter/exit, tools open/close); volume **Slider** and main timeline scrub **Slider** carry `accessible-label`; In/Out trim drag handles (`in-handle-touch`, `out-handle-touch`) labeled "In marker — drag to set trim start" / "Out marker — drag to set trim end"; the three timeline add-track buttons (`track-add-video` / `track-add-audio` / `track-add-subtitle`) labeled "Add video track" / "Add audio track" / "Add subtitle track"; per-lane filmstrip trim handles (`left-trim` / `right-trim`) labeled "Trim clip start — drag" / "Trim clip end — drag"; lane-delete trash (`del-ta`) labeled "Delete track"; all **10 export preset rows** (MP4 remux / MP4 H.264+AAC / MP4 HEVC+AAC / WebM VP8+Opus / WebM VP9+Opus / WebM AV1+Opus / MKV remux / MOV remux / MOV ProRes 422 HQ + PCM / MKV DNxHR HQ + PCM) labeled. Still open: the Trim Clip / Audio Lane Gain / Resize Video sheet Confirm / Cancel buttons (built-in `Button` widgets expose their `text` to a11y automatically, so this is lower-priority — revisit only if a screen-reader pass surfaces a gap). Verified `cargo build -p reel-app` and `cargo test -p reel-app --lib` (192 passed) after each batch — Slint 1.16 accepts `accessible-*` on `TouchArea` and `Slider`.

---

## How to use this file

| Role | Use |
|------|-----|
| **Product / roadmap** | **`docs/phases-ui.md`** + **`docs/FEATURES.md`** |
| **Session logs & `tracing`** | **`docs/phases-ui.md`** → *Logging standards*; **`docs/architecture.md`** (paths, `REEL_LOG*`) |
| **Infra & engine completeness** | This file (Phases 0–3) |
| **Design detail** | **`docs/architecture.md`**, **`docs/EXTERNAL_AI.md`** |

When **Phase U1–U5** items in `phases-ui.md` ship, update **`FEATURES.md`**; update **this file** only if you also complete or add **engineering** deliverables (e.g. new CI job, new crate).
