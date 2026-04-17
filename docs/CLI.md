# Reel — CLI (`reel-cli`)

Headless binary built from **`crates/reel-cli`**. It uses **`reel_core`** for probe, frame grab, and the Python sidecar.

## Build & run

```bash
make run-cli ARGS='probe --help'    # or: cargo run -p reel-cli -- …
```

From the **repository root**, `sidecar/` defaults to `./sidecar` for commands that spawn the bridge (unless overridden).

## Commands

### `reel-cli probe <path>`

- Opens the file with FFmpeg and prints **`MediaMetadata` as JSON** (duration, container, video/audio stream info, `audio_disabled` if audio could not be decoded).
- Exits non-zero on missing file, no video stream, or unsupported probe errors.

### `reel-cli swap <path> --out <png>`

- Decodes **one video frame** near **`--frame-ms`** (default `0`) using `reel_core::grab_frame`.
- Sends raw **RGBA** to **`sidecar/facefusion_bridge.py`** via `SidecarClient::swap_frame`.
- Writes the returned RGBA as a **PNG**.

**Options**

| Flag | Default | Meaning |
|------|---------|---------|
| `--out` | (required) | Output PNG path. |
| `--frame-ms` | `0` | Target time in milliseconds on the timeline. |
| `--model` | `identity` | Sidecar transform name. Built-ins include `identity`, `invert`, `facefusion`, `face_enhance`, `rvm_chroma` (see `sidecar/facefusion_bridge.py`). |
| `--sidecar-dir` | `./sidecar` (from cwd) | Directory containing `facefusion_bridge.py` and `pyproject.toml` (`uv run python …`). |
| `--timeout-ms` | `10000` | Per-request sidecar timeout. |

**Environment**

- **`FACE_FUSION_ROOT`:** Optional path to a FaceFusion checkout for the `facefusion` model (import check; full inference may still be stubbed—see repo docs).
- **`REEL_SIDECAR_DIR`:** Not used by `reel-cli` today (only `reel-app`); the CLI uses `--sidecar-dir` or cwd.

**Logging:** `reel_core::logging::init()` writes a **session log file** by default and respects **`REEL_LOG`**, **`REEL_LOG_FORMAT`**, **`REEL_LOG_FILE`**, **`REEL_LOG_SESSION_DIR`**, **`REEL_LOG_STDOUT`** (see `docs/architecture.md`).

---

## CLI roadmap (future work)

- **`reel-cli export`** mirroring app export presets (codec, resolution, bitrate).
- **Batch** `swap` or **directory** input for test pipelines.
- **Probe** options: stream list, subtitle track enumeration (when the engine supports it).
- **Config file** for default sidecar dir and timeouts.

---

## Maintenance

When adding a subcommand or flag, update **`docs/CLI.md`** and **`docs/FEATURES.md`** if the feature is user-facing.
