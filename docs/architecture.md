# Reel architecture

**See also:** `docs/DEVELOPERS.md`, `docs/MEDIA_FORMATS.md`, `docs/FEATURES.md`, **`docs/EXTERNAL_AI.md`** (why and how AI/tools run out-of-process).

## Crate graph

```
reel-core  ──┬──► reel-app   (Slint desktop binary `reel`)
             └──► reel-cli   (headless `reel-cli`)
             ▲
             └─ sidecar/ (Python, `uv run python facefusion_bridge.py`)
```

- **`reel-core`:** Media probe, decode helpers (`grab_frame`), `Project` model + schema migration, optional `ProjectStore` autosave, ffmpeg export helpers, `SidecarClient`, logging.
- **`reel-app`:** Slint UI, video/audio player threads, `EditSession`, debounced project autosave, Effects → sidecar PNG export.
- **`reel-cli`:** `probe`, `swap` (see `docs/CLI.md`).
- **`sidecar/`:** Line-delimited JSON bridge; stderr logged via `reel_core::logging`.

## Threading model in `reel-app`

```text
  UI thread (Slint event loop)
       │    Weak<AppWindow>
       ▼
  [player command channel] ───────────┐
                                       │
                                       ▼
           ┌─────────────────────┐      ┌──────────────────────────────┐
           │  video thread       │      │  audio thread                │
           │  ffmpeg → RGBA      │      │  ffmpeg → f32 stereo → cpal   │
           │  → SharedPixelBuffer│      │  → ringbuf                   │
           └──────────┬──────────┘      └──────────────┬───────────────┘
                      └────────── AudioClock ────────┘
```

Rules:

1. Upgrade `Weak<AppWindow>` only inside `slint::invoke_from_event_loop` (see `ui_bridge.rs`).
2. Ship frames as `SharedPixelBuffer` across threads; build `slint::Image` on the UI thread.
3. Audio is the **clock**; video may sleep or drop to stay near `AudioClock`.

## Autosave (library vs app)

- **`ProjectStore`** (`reel_core`): debounced worker thread, atomic `.tmp` → rename, used by tests and available for future app integration.
- **`reel-app`:** `EditSession::flush_autosave_if_needed` + Slint **single-shot** timer (~900 ms) for on-disk `.reel` after **Save** has established a path; **does not** clear undo (unlike explicit Save).

## Logging

- `reel_core::logging::init()` installs `tracing` and **always** writes a per-run session file named `reels.session.<UTC timestamp>.log` under `{data_local_dir}/reel/logs/` (override the directory with `REEL_LOG_SESSION_DIR`, or set `REEL_LOG_FILE` to a full path to pick a single file instead of the timestamped name).
- **Stdout** is optional: by default logs go only to the file (no terminal required). When **stdout is a TTY** (e.g. `cargo run`), the same records are **also** mirrored to the terminal; set `REEL_LOG_STDOUT=0` to disable, or `REEL_LOG_STDOUT=1` to force mirroring without a TTY.
- **`REEL_LOG`** (fallback `RUST_LOG`) sets the env filter; **`REEL_LOG_FORMAT`** is `pretty` (default) or `json`. Each line includes the tracing **target** (Rust module path) plus **file** and **line** for locating the call site.
- Sidecar stderr is tagged and forwarded to `tracing` (see `spawn_child_with_logged_stderr`).

## Sidecar protocol (summary)

Line-delimited JSON on stdin/stdout; RGBA payloads in tempfiles. See `crates/reel-core/src/sidecar.rs` for request/response shapes, timeouts, and crash behavior.

Transforms are registered in Python (`sidecar/facefusion_bridge.py`); `reel-cli swap` and app **Effects** use the same path.

For the **product rationale** (no fixed vendor API, `params` as an extension point, shelling out from the bridge), read **`docs/EXTERNAL_AI.md`**.

## Documentation bundling

The desktop **Help** menu loads markdown from `docs/*.md` via `include_str!` in `crates/reel-app/src/shell.rs` (`HelpDoc` enum). Updating help text means editing the markdown **and** rebuilding `reel-app`.
