# Reel — developer guide (humans)

This repository is a **Rust workspace** plus a **Python sidecar** managed by **uv**.

**New to contributing?** Read **`docs/CONTRIBUTING.md`** first (workflow, roadmap pointers, doc updates).

## Prerequisites

- **Rust** via `rustup` (see `rust-toolchain.toml`).
- **ffmpeg@7** and **pkg-config** (macOS: `brew install ffmpeg@7 pkg-config`; Makefile sets `PKG_CONFIG_PATH`).
- **uv** (`brew install uv`) for the sidecar virtualenv.

Run **`make check-tools`** to verify.

## First-time setup

```bash
make setup    # cargo fetch + uv sync in sidecar/
make build
make test
make run      # desktop app (reel)
```

## Workspace layout

| Path | Role |
|------|------|
| `crates/reel-core/` | Media probe, `Project` model, export, sidecar client, logging, optional `ProjectStore`. |
| `crates/reel-app/` | Slint UI, player threads, session, autosave debouncer, effects. **`src/lib.rs`** is the library (`reel_app`); **`src/main.rs`** is the thin `reel` binary calling `reel_app::run()`. Integration tests (`tests/*.rs`, e.g. visual golden PNG) link the library. |
| `crates/reel-cli/` | `probe` and `swap` commands. |
| `sidecar/` | `facefusion_bridge.py`, `uv` project, pytest. |
| `docs/` | User + developer documentation (bundled into Help). |

## Commands (Makefile)

| Target | Purpose |
|--------|---------|
| `make setup` | Fetch Rust deps; `uv sync` sidecar. |
| `make build` | `cargo build --workspace`. |
| `make test` | Rust tests + `uv run pytest` in sidecar. |
| `make lint` | `cargo fmt --check`, `clippy -D warnings`, `ruff`. |
| `make run` | `cargo run -p reel-app`; session log `reels.session.*.log` is written under the **directory you ran `make` from** (`REEL_LOG_SESSION_DIR`). |
| `make macos-app` / `make macos-app-release` | **macOS only:** builds `target/Reel.app` with `AppIcon.icns` from `crates/reel-app/ui/assets/knotreels.png`. Finder shows a generic icon for loose binaries under `target/`; use **`open target/Reel.app`** for the real Dock/Finder icon. |
| `make run-cli ARGS='probe …'` | Run `reel-cli`; same session log directory rule as `make run`. |
| `make ci` | Lint + test (CI parity). |

## Conventions

- **Formatting:** `rustfmt` (`rustfmt.toml`). Python: **ruff** in `sidecar/`.
- **Logging:** `tracing` only; no `println!` in library code except CLI output. Binaries call `reel_core::logging::init()` so each run gets a **session log file** (see `docs/architecture.md`).
- **UI thread:** Only upgrade `Weak<AppWindow>` inside `slint::invoke_from_event_loop` (see `docs/architecture.md`).
- **RefCell:** Avoid overlapping `borrow_mut` and `borrow` in one `match` scrutinee (see session handlers in `main.rs`).

## Documentation you should read

- **`docs/architecture.md`** — threading and sidecar protocol.
- **`docs/EXTERNAL_AI.md`** — how Effects / `reel-cli swap` hand pixels and JSON to external tools.
- **`docs/FEATURES.md`** — what the app does today + roadmap.
- **`docs/phases-ui.md`** / **`docs/phase-status.md`** — UI phases (U1–U5) vs engineering phases (0–4).
- **`docs/MEDIA_FORMATS.md`** — codec/track behavior.
- **`docs/AGENTS.md`** — expectations for AI assistants working in this repo.

## In-app help

Help text is **`include_str!`**’d from `docs/*.md` in `crates/reel-app/src/shell.rs`. If you add a new bundled doc, add the file, extend `HelpDoc` in `shell.rs`, and wire the **Help** menu in `ui/app.slint` + `main.rs`.

## Contributing

See **`docs/CONTRIBUTING.md`** for the full checklist. In short:

1. Branch from default branch; keep changes focused.
2. Run **`make ci`** before pushing.
3. Update **`docs/FEATURES.md`**, **`docs/phases-ui.md`** (and **`docs/phase-status.md`** when engineering milestones move), and **`docs/MEDIA_FORMATS.md`** when behavior visible to users changes.
4. For agent-driven work, ensure **`docs/AGENTS.md`** checklists stay accurate.
