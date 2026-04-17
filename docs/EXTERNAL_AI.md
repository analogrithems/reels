# Reel — external AI & tools integration

Reel does **not** require a fixed vendor SDK or a single blessed HTTP API for AI features. Instead, it **prepares media in a simple, stable form** and **hands work to external processes** using a **small JSON + file contract**. You can swap implementations (Python today, another runtime tomorrow) or call **other CLIs and scripts** from that layer without rewriting the Rust app for every new model.

**Maintenance:** If the handoff protocol, env vars, or entrypoints change, update this file, **`docs/architecture.md`**, and **`docs/AGENTS.md`**.

---

## Design goals

1. **Fast iteration** — New effects can ship by editing the **sidecar** (or pointing `REEL_SIDECAR_DIR` at a fork) instead of threading a new C API through `ffmpeg-next` for every experiment.
2. **No API lock-in** — The app speaks **line-delimited JSON** and **paths to raw pixels**, not “only TensorFlow” or “only FaceFusion’s REST shape.” External code decides how to load models or call other tools.
3. **Observable failures** — Sidecar **stderr** is forwarded into `tracing`; timeouts and crashes surface as structured errors in Rust (`SidecarError`). Every run also writes a **session log file** (`reels.session.*.log` under the app data directory by default) as **NDJSON** (module path, file, line, and `tracing` fields as JSON properties) so failures map back to source (see **`docs/architecture.md`**).
4. **Same path for CLI and GUI** — `reel-cli swap` and the **Effects** menu use the same `reel_core::SidecarClient` + `grab_frame` pipeline.

---

## What Reel does itself

- **Decode** video with FFmpeg (same stack as playback): one frame at a time for effects via `reel_core::grab_frame`, producing **tightly packed RGBA8** (`width × height × 4` bytes).
- **Write** that buffer to a **private tempfile** under the client’s temp directory.
- **Send** one JSON request per operation on the child’s **stdin** (see `crates/reel-core/src/sidecar.rs`), including:
  - `op` (e.g. `swap`, `ping`, `shutdown`)
  - `in_path`, `width`, `height`
  - **`params`**: arbitrary JSON object (e.g. `{"model": "rvm_chroma"}`) — **extension point** for new knobs without a Rust schema bump for every flag.
- **Read** the response JSON on **stdout**; on success, **read output bytes** from the path returned (`out_path`).
- **Effects menu** then saves the result as **PNG** (app side); the bridge only deals in raw RGBA.

The player timeline is **not** replaced automatically yet—effects are framed as **export one frame / one asset** until a richer pipeline exists.

---

## What the external tool does

The reference implementation is **`sidecar/facefusion_bridge.py`** (run via `uv run python facefusion_bridge.py`):

- Parses one JSON object per line.
- For `op: "swap"`, loads **RGBA from `in_path`**, runs a named transform keyed by **`params.model`**, writes **RGBA to `in_path + ".out"`** (or similar), returns `out_path` in JSON.
- Can **shell out** to other programs, import optional Python packages, or read **`FACE_FUSION_ROOT`** — all without changing Rust as long as the **stdin/stdout contract** is honored.

That means “AI” can be:

- Pure Python / NumPy / ONNX / PyTorch in-process, or
- A **subprocess** to `ffmpeg`, a Docker container, or a vendor CLI — the bridge script is the adapter.

---

## Adding a new capability (typical flow)

1. **Prefer reusing `op: "swap"`** — Add a new **`params.model`** value (or extra keys under `params`) in the Python `TRANSFORMS` map and implement the pixel transform.
2. **Expose it in the app** — Map a menu item to a new `EffectKind` / `model` string in `reel-app` (small Rust change), or call the same model from `reel-cli swap --model …`.
3. **If you need a new operation name** — Extend the bridge with a new `op` and add a matching method on `SidecarClient` in Rust (rarer; only when JSON shape must differ).

This keeps **most** experimentation in **Python or external CLIs**, not in the Slint/UI layer.

---

## Configuration & discovery

| Mechanism | Role |
|-----------|------|
| **`REEL_SIDECAR_DIR`** | Desktop app: override directory containing `facefusion_bridge.py` + `pyproject.toml`. |
| **`reel-cli --sidecar-dir`** | CLI: same, relative to current working directory by default (`./sidecar`). |
| **`FACE_FUSION_ROOT`** | Optional path injected into the bridge environment for FaceFusion-style checkouts. |
| **Timeouts** | `SidecarClient` per-request timeout (Effects use a longer default than tests). |

---

## Limitations & roadmap

- **Today:** Effects path is largely **single-frame** handoff; full **sequence / clip** export and re-import as timeline edits is **planned** (see **`docs/FEATURES.md`**).
- **Not required:** The sidecar does not have to stay Python forever—any executable that speaks the same **stdio JSON + tempfile** protocol can be wired via `SidecarClient::spawn_command` for experiments.

For protocol details and threading rules, see **`docs/architecture.md`** and **`crates/reel-core/src/sidecar.rs`**.
