# Reel

An open-source, Rust + Slint desktop video editor with an iMovie-style workflow and a Python/FaceFusion AI sidecar. **macOS-first**, cross-platform eventually.

## Documentation

- **[docs/CONTRIBUTING.md](docs/CONTRIBUTING.md)** — how to contribute (workflow, docs to update).
- **[docs/README.md](docs/README.md)** — index of user, developer, and agent docs.
- **[docs/FEATURES.md](docs/FEATURES.md)** — what works today and what is planned.
- **[docs/DEVELOPERS.md](docs/DEVELOPERS.md)** / **[docs/AGENTS.md](docs/AGENTS.md)** — humans vs Cursor/Claude onboarding.

The desktop app also bundles these under **Help** in the menu bar.

## Status

Engineering phases (0–4) and UI roadmap (U1–U5) are tracked in **[docs/phase-status.md](docs/phase-status.md)** and **[docs/phases-ui.md](docs/phases-ui.md)**.

## Releases

Tagged versions are published on **GitHub Releases** with a **macOS `.app` zip** (see **[CHANGELOG.md](CHANGELOG.md)**). FFmpeg is not bundled; install **ffmpeg@7** separately.

### First launch on macOS (unsigned build)

Release builds are **not yet code-signed or notarized** by an Apple Developer account, so on first launch macOS will show:

> **"Reel.app" is damaged and can't be opened. You should move it to the Trash.**

The app is **not damaged** — the dialog is macOS Gatekeeper refusing to run an unsigned binary that your browser tagged with a `com.apple.quarantine` extended attribute. Strip the attribute once and the app opens normally from then on:

```sh
# wherever you unzipped it — typically ~/Downloads or /Applications:
xattr -d com.apple.quarantine /path/to/Reel.app

# if that isn't enough (rare), strip xattrs from every file in the bundle:
find /path/to/Reel.app -print0 | xargs -0 xattr -d com.apple.quarantine 2>/dev/null
```

(On older macOS the `xattr` tool lacks `-r`, so use the `find` form rather than `xattr -dr`.)

Signed + notarized releases are tracked under **Phase 4** in [docs/phase-status.md](docs/phase-status.md).

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
make run     # launch the Slint desktop app (session log: reels.session.*.log in this directory)
             # optional: make run ARGS='path/to/file.mp4' to open a file on launch
```

## Crates

| Crate | Purpose |
|-------|---------|
| `reel-core` | Media probe/decode, project model, tracing setup, shared error types |
| `reel-app`  | Slint desktop binary (`reel`) |
| `reel-cli`  | Headless binary (`reel-cli probe`, `swap`) — see [docs/CLI.md](docs/CLI.md) |

## Sidecar

`sidecar/` is a `uv`-managed Python project (`facefusion_bridge.py`). The desktop **Effects** menu and `reel-cli swap` call it; see [docs/CLI.md](docs/CLI.md) and [docs/FEATURES.md](docs/FEATURES.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
