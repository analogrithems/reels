# UI & editor phases (revised)

Reel’s shell is still early. Work is grouped so each phase ships a testable slice of the desktop experience.

## Phase U1 — Shell & menus (in progress)

- Native-style **menu bar** (File / Edit / Window / Help) via Slint `MenuBar`.
- **File:** Open, Close, Revert, New Window, Save (project file), Insert Video (at playhead; timeline model stub), Export (transcode/remux via ffmpeg).
- **Edit:** Undo / redo stacks (document-level; wired to session state).
- **Window:** Always on top, viewport **Fit** (contain) / **Fill** (cover) / **Center** (contain + centered framing).
- **Help:** In-app window showing rendered help text from `docs/HELP.md`.
- **Timeline:** Interactive scrub strip (mouse drag) driving the same seek path as transport.
- **Unit tests** (pure Rust): session state, undo/redo, export preset args, new-window command builder.
- **Integration tests** (reel-core): export fixture video to web-family outputs under `target/reel-export-verify/`.

## Phase U2 — Project & timeline (in progress)

- [x] In-memory `Project`: open builds one clip + video track; **Insert Video** probes and appends a clip.
- [x] **Save** writes the live `Project` JSON; **Revert** restores last saved snapshot or re-probes the opened file.
- [x] **Undo / redo** use project snapshots (insert/remove clip); player reloads if the first clip’s source path changes.
- [ ] Playhead-as-cursor for insert position; multi-track / audio; `ProjectStore` autosave for editor documents.

## Phase U3 — Export UX

- Presets UI (resolution, bitrate), progress, cancel.
- Batch export.

## Phase U4 — Polish

- Keyboard shortcuts parity across platforms, a11y pass, icons.
