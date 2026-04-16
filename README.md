# Reel

An open-source, Rust + Slint desktop video editor with an iMovie-style workflow and a Python/FaceFusion AI sidecar. **macOS-first**, cross-platform eventually.

## Status

Iteration scope: **Phases 0–2** — infrastructure, media engine under TDD, and a minimal player window with synced audio playback. See [docs/phase-status.md](docs/phase-status.md).

## Prerequisites (macOS)

```sh
brew install rustup-init ffmpeg@7 pkg-config uv
rustup-init -y
```

`ffmpeg@7` is required (not the default `ffmpeg` 8.x) because `ffmpeg-next 7.1` binds against ffmpeg 7.x headers.

## Quick start

```sh
make setup   # verify tools, fetch deps, sync sidecar venv
make test    # cargo test --workspace
make lint    # fmt + clippy + ruff
make run     # launch the Slint desktop app
```

## Crates

| Crate | Purpose |
|-------|---------|
| `reel-core` | Media probe/decode, project model, tracing setup, shared error types |
| `reel-app`  | Slint desktop binary (`reel`) |
| `reel-cli`  | Headless binary (`reel-cli probe …`) |

## Sidecar

`sidecar/` is a `uv`-managed Python 3.11 project. Phase 0–2 ships only a stub `facefusion_bridge.py`; real FaceFusion wiring arrives in Phase 3.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
