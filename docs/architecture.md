# Reel architecture

## Crate graph

```
reel-core  ──┬──► reel-app  (Slint desktop binary "reel")
             └──► reel-cli  (headless binary "reel-cli")
             ▲
             └─ sidecar/ (Python, managed by uv) — called from reel-core::logging::spawn_logged_child
```

- **`reel-core`** owns every piece that is not UI: media probe, `Project` serde model, autosave store, `tracing` setup, and the child-process logging helper that the Phase-3 FaceFusion bridge will use.
- **`reel-app`** is the desktop GUI. It depends on `reel-core` for the probe + project types and carries all Slint-specific code (window, decoder, audio output, command plumbing).
- **`reel-cli`** is a thin wrapper around `reel-core` that exposes one subcommand so far (`probe`).
- **`sidecar/`** is an empty uv-managed Python project with a stub `facefusion_bridge.py`. It does not run during Phases 0–2 except through logging round-trip tests.

## Threading model in `reel-app`

```text
  UI thread (Slint event loop)
       │    Weak<AppWindow>
       ▼
  [player command channel] ───────────┐
                                       │
                                       ▼
           ┌─────────────────────┐      ┌──────────────────────────────┐
           │  reel-video thread  │      │  reel-audio thread           │
           │  ffmpeg Input       │      │  ffmpeg Input + decoder      │
           │  → scaler RGBA8     │      │  → resample to f32 stereo    │
           │  → SharedPixelBuffer│      │  → ringbuf producer          │
           │  → invoke_on_UI     │      │  → cpal output callback      │
           └─────────────────────┘      └──────────────────────────────┘
                      │                               │
                      └───────── AudioClock ◄─────────┘
```

Rules:

1. `Weak<AppWindow>` is the only Slint handle that crosses threads. It must be upgraded *inside* `slint::invoke_from_event_loop`.
2. `slint::Image` is **not** `Send` in this release. `SharedPixelBuffer` is, so video frames are shipped as `SharedPixelBuffer` and wrapped into `Image` only in the UI-thread closure.
3. Audio is the master clock. The cpal output callback advances `AudioClock` by `samples / sample_rate`; the video thread consults the clock to decide whether to sleep, drop, or present.

## Autosave

`ProjectStore` keeps the `Project` behind an `RwLock` and spawns a worker thread that debounces mutations (500 ms) before writing `project.json` atomically (`.tmp → rename`). `mutate(|p| …)` is the only entry point for state changes; reading is a zero-cost `read()` that returns an `RwLockReadGuard`.

## Logging

- Global `tracing` subscriber configured by `reel_core::logging::init()`.
- Env vars: `REEL_LOG`, `REEL_LOG_FORMAT={pretty|json}`, `REEL_LOG_FILE=<path>`.
- Child processes (the future FaceFusion sidecar) are spawned via `spawn_logged_child`, which pipes stdout→`info!` and stderr→`warn!` on dedicated reader threads. Covered by a round-trip test against `/bin/echo`.

## Phase 3 hooks already in place

- `reel_core::logging::spawn_logged_child` — the sidecar process launcher.
- `reel_core::media::decoder::{DecodeCmd, DecodedFrame}` — stable types the future AI-swap round-trip (save frame → Python → reload) will interop against.
- `sidecar/facefusion_bridge.py` — line-delimited JSON stdio contract documented in `sidecar/README.md`.

## FaceFusion bridge (Phase 3)

```text
  reel-cli / reel-app                 sidecar/ (uv-managed Python)
  ┌────────────────────┐   stdin      ┌───────────────────────────┐
  │ SidecarClient      │ ───── JSON ─►│ facefusion_bridge.py      │
  │ - swap_frame()     │              │   ops: ping / swap /      │
  │ - ping()           │◄──── JSON ── │        shutdown           │
  │ - reader thread    │   stdout     │   transforms:             │
  │ - pending by `id`  │              │     identity, invert      │
  └─────────┬──────────┘              └──────────┬────────────────┘
            │                                    │
            │  tempfile<id>.rgba  (raw RGBA8)    │
            └────────────────────────────────────┘
                           ▲       ▲
                    Rust writes   Python writes
                    in_path       out_path
```

Contract (line-delimited JSON):

```
→ {"id": N, "op": "swap",
   "in_path": "…/in-N.rgba",
   "width": W, "height": H,
   "params": {"model": "identity" | "invert", …}}
← {"id": N, "status": "ok", "out_path": "…/in-N.rgba.out"}
← {"id": N, "status": "err", "reason": "…"}
```

Pixels travel via tempfiles (owned by the client's `tempfile::TempDir`) rather
than inline base64 to keep the JSON small. The reader thread de-multiplexes
responses back to their original caller by `id`, so the client is safe to
call from multiple threads.

Test hooks on `params`:

- `sleep_ms: N` — delay the response (exercises `SidecarError::Timeout`).
- `crash: true` — `sys.exit(1)` before responding (exercises `SidecarError::Crashed`).

Real FaceFusion model integration is **not** in scope for this phase: the
bridge is proven end-to-end with identity + invert placeholders. Dropping in
the real model should require no Rust changes — only a new entry in the
Python `TRANSFORMS` table and a model download step in `uv sync`.
