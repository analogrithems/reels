# Reel documentation

| Document | Purpose |
|----------|---------|
| [HELP.md](HELP.md) | In-app **Help → Overview**; quick start and links (by topic name). |
| [FEATURES.md](FEATURES.md) | Editing features shipped today + **roadmap** (update when features land). |
| [MEDIA_FORMATS.md](MEDIA_FORMATS.md) | Containers, video/audio/subtitle **support level** in Reel (update when format behavior changes). |
| [CLI.md](CLI.md) | `reel-cli` usage + CLI **roadmap**. |
| [EXTERNAL_AI.md](EXTERNAL_AI.md) | How Reel hands frames to **external processes** (JSON + files, no fixed vendor API). |
| [CONTRIBUTING.md](CONTRIBUTING.md) | **Start here** for contributing (links roadmap, workflow, CI). |
| [DEVELOPERS.md](DEVELOPERS.md) | Human contributor setup (build, test, layout). |
| [AGENTS.md](AGENTS.md) | Guidance for **Cursor** and **Claude** (and similar agents) + doc-update duties. |
| [architecture.md](architecture.md) | Crate graph, threading, sidecar protocol. |
| [phases-ui.md](phases-ui.md) | **Product/UI phases** U1–U5: status, exit criteria, sub-milestones (U2-a … U5-c), dependencies, suggested next focus. |
| [phase-status.md](phase-status.md) | **Engineering phases** 0–4 + doc milestones (infra, player, sidecar); points to `phases-ui.md` for U-roadmap. |

In the desktop app, open **Help** and choose a topic; bodies are bundled from these files at compile time (`crates/reel-app/src/shell.rs`).
