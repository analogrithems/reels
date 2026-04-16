# Reel sidecar

Python environment for the FaceFusion bridge and future AI workers.
Managed by [`uv`](https://docs.astral.sh/uv/).

## Lifecycle

```sh
cd sidecar
uv sync                                   # create/update .venv
uv run pytest -q                          # run unit tests
uv run python facefusion_bridge.py        # smoke-test the bridge (reads JSON on stdin)
```

## Protocol

`facefusion_bridge.py` speaks a line-delimited JSON protocol over stdio with
the Rust host (`crates/reel-core/src/sidecar.rs`). Every request carries a
monotonic `id`; responses echo it so the client can multiplex concurrent
requests on a single process.

```text
→ {"id": 1, "op": "ping"}
← {"id": 1, "status": "ok"}

→ {"id": 2, "op": "swap",
   "in_path": "/tmp/in-2.rgba",
   "width": W, "height": H,
   "params": {"model": "identity" | "invert"}}
← {"id": 2, "status": "ok", "out_path": "/tmp/in-2.rgba.out"}
← {"id": 2, "status": "err", "reason": "…"}

→ {"id": 3, "op": "shutdown"}              # no response; loop exits
```

The `in_path` / `out_path` files are raw RGBA8 bytes (`width*height*4`). The
Rust client owns the tempdir and cleans up on drop.

Placeholder transforms (Phase 3; the real FaceFusion model is not installed
yet):

- `identity` — copy in → out
- `invert`   — flip R/G/B, preserve A

Test hooks on `params` (used by the Rust integration tests in
`crates/reel-core/tests/sidecar_client.rs`):

- `sleep_ms: N` — delay the response by N ms
- `crash: true` — `sys.exit(1)` before responding

stderr is piped into the Rust `tracing` stream by
`reel_core::logging::spawn_child_with_logged_stderr`.
