# Contributing to Reel

Thanks for helping improve Reel. This page is the **entry point** for humans; automated assistants should also read **`docs/AGENTS.md`**.

## Before you code

1. **Setup** — Follow **`docs/DEVELOPERS.md`** (toolchain, `make setup`, workspace layout).
2. **Scope** — Check **`docs/FEATURES.md`** for what already ships and what is planned.
3. **Roadmap** — See **`docs/phases-ui.md`** for UI phases (U1–U5), **exit criteria**, and **Suggested next focus** (good starting points for PR-sized work).
4. **Architecture** — Skim **`docs/architecture.md`** and, for Effects/sidecar, **`docs/EXTERNAL_AI.md`**.

## Workflow

1. Create a **focused** branch (one feature or fix per PR when possible).
2. Implement with existing patterns (`reel-app` session/player, `reel-core` probe/export/sidecar).
3. Run **`make ci`** (same as CI: fmt, clippy, tests, ruff).
4. Update **docs** when user-visible behavior changes — at minimum **`docs/FEATURES.md`**; often **`docs/phases-ui.md`**, **`docs/MEDIA_FORMATS.md`**, or **`docs/CLI.md`** (see **`docs/AGENTS.md`** tables).

## AI / Cursor / Claude

If you use coding agents in this repo, share **`docs/AGENTS.md`** with the session and keep **`docs/phases-ui.md`** in sync when roadmap priority shifts.

## Questions

- **Product direction** — **`docs/phases-ui.md`**, **`docs/FEATURES.md`**.
- **Code layout** — **`docs/architecture.md`**, **`docs/DEVELOPERS.md`**.
- **Formats & codecs** — **`docs/MEDIA_FORMATS.md`**.
- **Shipping a version** — **`docs/RELEASING.md`** (tags, GitHub Releases).

## License

By contributing, you agree that your contributions are licensed under the same terms as the project (**MIT OR Apache-2.0**), unless you state otherwise.
