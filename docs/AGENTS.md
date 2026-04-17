# Reel — guide for AI coding agents (Cursor, Claude, …)

This document is for **automated assistants** (e.g. **Cursor** Composer/Agent, **Claude** in IDE or CI) that modify the AiVideoEditor / Reel codebase. Humans start with **`docs/CONTRIBUTING.md`** and **`docs/DEVELOPERS.md`**.

## Read first

1. **`docs/CONTRIBUTING.md`** — contributor workflow (humans; points to roadmap and doc-update rules).
2. **`docs/DEVELOPERS.md`** — toolchain, `make` targets, repo layout.
3. **`docs/architecture.md`** — crates, threading, sidecar IPC.
4. **`docs/FEATURES.md`** — **source of truth** for shipped vs planned user-facing behavior.
5. **`docs/MEDIA_FORMATS.md`** — containers, streams, and limitations (update when format behavior changes).
6. **`docs/SUPPORTED_FORMATS.md`** — playback vs export matrix and format roadmap (update when presets or decode scope changes).
7. **`docs/EXTERNAL_AI.md`** — how effects hand off to external tools (update if stdin/JSON contract, tempfile rules, or env vars change).
8. **`docs/phases-ui.md`** — UI phase roadmap (U1–U5), exit criteria, sub-milestones, **logging standards (requirement)**, **suggested next focus**.

## Responsibilities when landing changes

| Change type | Update |
|-------------|--------|
| New or changed **editing / UI / export / effects** behavior | **`docs/FEATURES.md`** — move items between “Currently supported” and “Roadmap” as appropriate. |
| Probe, decode, multi-stream, subtitle, or export codec behavior | **`docs/MEDIA_FORMATS.md`**, **`docs/SUPPORTED_FORMATS.md`**, and often **`docs/FEATURES.md`**. |
| New **`reel-cli`** subcommand or important flag | **`docs/CLI.md`** + **`docs/FEATURES.md`** if user-visible. |
| **Sidecar protocol**, new `op`/`params` semantics, or handoff to non-Python tools | **`docs/EXTERNAL_AI.md`**, **`docs/architecture.md`**, and **`crates/reel-core/src/sidecar.rs`** doc comments as needed. |
| New **bundled Help** document or menu entry | **`crates/reel-app/src/shell.rs`** (`HelpDoc` + `include_str!`), **`ui/app.slint`** (Help menu), **`main.rs`** callbacks, and **`docs/README.md`** (and the new `docs/*.md` file, e.g. **`KEYBOARD.md`**). |
| Phase / milestone shift | **`docs/phases-ui.md`** (UI roadmap) and **`docs/phase-status.md`** (engineering checklist)—keep both consistent with **`docs/FEATURES.md`**. |
| Roadmap **priority** or **sub-milestone** (U2-a … U5-c) changes | **`docs/phases-ui.md`** — **Suggested next focus**, exit criteria, checkboxes. |
| New or materially changed **behavior** (flows, export, session, core paths) | Follow **`docs/phases-ui.md`** → **Logging standards** (`tracing` at appropriate levels); see **`docs/architecture.md`** for session file env vars. |

## Tooling expectations

- Run **`make ci`** (or at least `make lint` + `make test`) before considering work complete.
- Prefer **small, reviewable diffs**; do not refactor unrelated modules.
- **Sidecar:** Python changes must pass **`cd sidecar && uv run ruff check .`** and **`uv run pytest`**.

## Cursor-specific hints

- Use the repo **`Makefile`** as the canonical command interface (`make run`, `make run-cli ARGS='…'`).
- Slint UI: after editing **`.slint`**, rebuild `reel-app`; generated Rust types appear as `AppWindow`, callbacks as `on_*`.
- If the user mentions **Help** or **documentation**, check whether **`shell.rs`** and **`docs/README.md`** need the same update.

## Claude-specific hints

- When planning multi-step features, align implementation bullets with **`docs/FEATURES.md` roadmap** so the next session can mark them done.
- Long-running commands: prefer **`make test`** over ad-hoc `cargo test` unless narrowing a crate.

## Invariants (do not break casually)

- **Audio clock** drives playback; video follows (`docs/architecture.md`).
- **Sidecar protocol** is line-delimited JSON + tempfile RGBA (`crates/reel-core/src/sidecar.rs`).
- **Project** JSON schema is versioned; migrations live in `reel_core::project::schema`.

## When unsure

- Search for existing patterns (e.g. `EditSession`, `SidecarClient`, `sync_menu`).
- Add a short note to **`docs/FEATURES.md`** under Roadmap rather than inventing hidden behavior.
