# Reel sidecar

Python environment for the FaceFusion bridge and future AI workers.
Managed by [`uv`](https://docs.astral.sh/uv/).

## Lifecycle

```sh
cd sidecar
uv sync                          # create/update .venv
uv run python facefusion_bridge.py  # smoke-test the stub
```

## Contract

`facefusion_bridge.py` will eventually speak a line-delimited JSON protocol
over stdio with the Rust host:

```text
→ {"op": "swap_face", "input": "<path>", "source": "<path>", "output": "<path>"}
← {"status": "ok",  "output": "<path>"}
← {"status": "err", "reason": "…"}
```

For Phases 0–2 the script is a stub that reads a JSON line from stdin and
echoes it back with `{"status": "stub"}`. Logs go to stderr and are piped
into the Rust `tracing` stream by
`reel_core::logging::spawn_logged_child`.
