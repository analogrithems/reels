---
title: "Develop like me — the Reel phases process"
status: living
last_reviewed: 2026-04-24
owners: [core]
---

# Develop like me

A guide to how we plan, track, and ship work on Reel — and why we
deliberately **did not** switch to [OpenSpec](https://openspec.dev/)
when we evaluated it. Read this if you're about to write a new phase
doc, propose a new feature, or wonder why we keep a `phases-*.md`
tree instead of something more formal.

---

## TL;DR

- We run a **living, narrative Markdown** process. Phase docs under
  `docs/phase-status.md`, `docs/phases-ui.md`, and
  `docs/phases-ui-test.md` are the source of truth for "what's
  shipped, what's in flight, what's parked."
- Each phase doc carries a small **YAML frontmatter block** exposing
  `title`, `status`, `phases`, `last_reviewed`, `owners`. That's it —
  no SHALL language, no Given/When/Then scenarios.
- A tiny lint (`scripts/lint_phases.sh`, wired into `make lint`) keeps
  the convention honest and catches dangling `docs/phases*`
  cross-references.
- We looked hard at OpenSpec. We're not adopting it now. We **may**
  layer an OpenSpec-flavoured workflow on top, for *new proposals
  only*, once the list of open prospective features grows beyond what
  narrative docs can comfortably hold. Section [Future OpenSpec
  on-ramp](#future-openspec-on-ramp) sketches what that would look
  like.

---

## How the phases process works today

### The three living docs

| File | Scope |
|---|---|
| [`docs/phase-status.md`](./phase-status.md) | Engineering & infrastructure phases (0–4), CI, logging, format roadmap. The README calls this "the engineering roadmap." |
| [`docs/phases-ui.md`](./phases-ui.md) | Product/UI phases U1–U5 — shell, deep editing, export UX, polish, AI/effects. Embedded at compile time into the in-app **Help → UI phases roadmap** window via `include_str!` in `crates/reel-app/src/shell.rs`. |
| [`docs/phases-ui-test.md`](./phases-ui-test.md) | UI test harness phases UT1–UT4 — Slint testing backend pinning, mockable seams, visual regression, post-export validation. |

Each file is a **narrative dashboard**, not a spec. You'll find
checkbox lists (`- [x] done`, `- [~] partial`, `- [ ] open`),
GitHub-flavored tables, prose rationale, and cross-links into source
and other docs. Status is carried *inline* — one bullet can describe
a shipped feature, its code references, and a follow-up gotcha in the
same breath.

### Frontmatter convention

Every phase doc begins with:

```markdown
---
title: "..."
status: living                     # living | archived | proposed | deprecated
phases: [U1, U2, U3, U4, U5]       # optional list of phase IDs this file tracks
last_reviewed: 2026-04-24          # ISO-8601 date
owners: [core, ui]                 # optional team/area tags
---

# ...
```

Rules:

- `title` / `status` / `last_reviewed` are **required**. The lint will
  fail `make lint` if any are missing.
- `status` must be one of `living`, `archived`, `proposed`, or
  `deprecated`. Anything else fails lint.
- `last_reviewed` is the date the doc was meaningfully reviewed — not
  just touched. Bump it when you revise the content; skipping the
  bump is fine for purely mechanical edits.
- `phases` / `owners` are informational. We don't enforce them today
  but tools can use them once we have more than three phase files to
  navigate.

The in-app Help window strips the frontmatter block before rendering
(`shell.rs::strip_frontmatter`), so end users never see the YAML.

### What the lint actually checks

`scripts/lint_phases.sh` runs as part of `make lint`. It:

1. Verifies every tracked phase doc has a complete, well-formed
   frontmatter block.
2. Scans README, docs, Rust/Slint source, and shell scripts for
   `docs/phase-status.md`, `docs/phases-*.md`, and
   `docs/phases/*.md` references. Any path that doesn't resolve to a
   real file is a hard error.
3. Future-proofs for a per-phase split: if `docs/phases/` exists, its
   `.md` files are automatically included in the frontmatter check.

The tracked-file list is **explicit** in the script — not a glob.
That's deliberate: renaming a phase doc without updating the list is
the kind of silent drift we want to catch.

### The authoring loop

1. **Open a phase doc.** Add or move items as you work. Use existing
   checkbox conventions (`- [x]`, `- [~]`, `- [ ]`).
2. **Cross-link generously.** If the change touches behavior users
   will see, also update [`docs/FEATURES.md`](./FEATURES.md). If it
   changes the agent-facing story, touch
   [`docs/AGENTS.md`](./AGENTS.md) too.
3. **Bump `last_reviewed`** when you review the whole doc, not when
   you type one sentence.
4. **Run `make lint`** before you push. The phase lint runs after
   clippy/ruff and catches the class of "I renamed a file and
   forgot about two README links" failure.
5. **Commit with a real message.** "chore: update phases-ui" is
   not a real message. "docs(phases-ui): mark U2-e trim handles
   shipped" is.

---

## Why we evaluated OpenSpec

[OpenSpec](https://openspec.dev/) is a lightweight product-spec
framework aimed at aligning AI coding agents around a shared spec
before code gets written. Authoring format is Markdown with required
sections — a *Purpose* statement, *Requirements* in SHALL language,
and *Scenarios* in Given/When/Then form. It ships a small CLI
(`openspec init`) and a `/opsx:*` slash-command workflow that plugs
into Claude Code and Cursor.

It's a reasonable idea for teams that (a) mostly greenfield net-new
features, (b) do that work primarily through coding agents, and (c)
want a single canonical place for "what should this thing do" that
survives chat rollovers.

## Why we're not adopting it (yet)

Five reasons, roughly in order of weight.

### 1. Bad fit for the docs it would replace

Our phase docs are an **engineering changelog + roadmap** for work
that is mostly already shipped. They're dense narrative prose with
heavy cross-file interleaving. OpenSpec is built for the *opposite*
artifact: fresh proposals written in SHALL-requirement language for
features that don't exist yet.

Converting `phases-ui.md`'s 400-word description of "U2-e: on-timeline
trim handles with ripple" into SHALL + GWT syntax would strip the
texture that reviewers actually rely on — what does the feature do,
why did we pick this approach, what's the failure mode, what's the
code surface. That texture is the whole point of the doc.

### 2. `shell.rs` embeds a phase doc at compile time

`crates/reel-app/src/shell.rs` uses `include_str!` to bake
`docs/phases-ui.md` into the binary for the **Help → UI phases
roadmap** window. Any migration that renames or splits that file is
a compile-time break. The narrow workaround (a generated shim file)
adds complexity without delivering value — we'd be maintaining two
versions of the same doc.

### 3. Framework risk

OpenSpec is a small, early-stage framework from a single vendor.
Adopting it means betting some of our process on its continued
maintenance. Plain Markdown + YAML frontmatter is a format the entire
world already supports — rendered well on GitHub, diffable in PRs,
searchable with `grep`, editable in every tool.

### 4. No standalone validator

OpenSpec's validation path is *agent-driven* — a slash command
inside Claude or Cursor runs the check. There's no equivalent of
`make lint` we can wire into CI. For a project where CI runs on
every push, that's a real gap. Our 120-line `lint_phases.sh` gives
us the pieces we actually care about (required frontmatter keys,
no dangling cross-refs) with zero external dependencies.

### 5. Cost outweighs benefit at current scale

Migrating honestly would mean:

- Rewriting ~10 specs in Purpose/SHALL/GWT form (~45 min each).
- Rewriting the `shell.rs` Help loader or shimming a generated file.
- Updating ~29 cross-references across README, docs, Rust, and
  Slint.
- Auditing `.github/workflows/wiki-sync.yml` file globs.

Realistic budget: about a week of concentrated effort. Against which
we'd get a **slightly more structured** authoring format that
non-agent contributors gain no tooling benefit from, and that we'd
still need to maintain alongside the in-app Help pipeline.

The juice isn't worth the squeeze at today's repo size.

---

## The process we chose instead

Keep the narrative, make it enforceable.

1. **Living Markdown with YAML frontmatter** (the current `docs/phases*`
   tree). Low friction to author, high readability, renders everywhere.
2. **Tiny lint in CI** (`scripts/lint_phases.sh`). Catches missing
   frontmatter, bad status values, stale dates, and dangling
   cross-references. Zero dependencies. ~100 lines of bash.
3. **Explicit tracked-file list, not a glob.** Renaming a phase doc
   is a loud failure, not a silent drop.
4. **In-app Help stays compile-time embedded.** `strip_frontmatter`
   keeps YAML out of the rendered Help window while letting lint see
   the full block.
5. **`make lint` is the one-stop check.** Phase lint runs alongside
   clippy and ruff; contributors don't need to remember to run a
   separate tool.

### When you'd write a new phase doc

- A body of related, in-flight work is big enough that tracking it in
  bullet-points inside `phase-status.md` or `phases-ui.md` is making
  those files harder to read.
- Or: the work is a self-contained *thing* with its own lifecycle
  (like the UI test harness → `phases-ui-test.md`).

New phase docs land in `docs/phases/<slug>.md` (see the future-proofing
in the lint script). If we do this, we'll update
[`crates/reel-app/src/shell.rs`](../crates/reel-app/src/shell.rs) to
either keep `phases-ui.md` as a generated aggregate or teach the Help
loader to stitch multiple files together — but that's work we'll do
when we have a real reason to split, not speculatively.

---

## Future OpenSpec on-ramp

We're not forever-closed to OpenSpec. The right framing is: **use it
prospectively, not retroactively.** Specifically:

### Phase A — Status quo (today)

- `docs/phases*` remains the home for shipped and in-flight work.
- `scripts/lint_phases.sh` enforces the convention.
- New net-new features land as bullets in the relevant phase doc.

### Phase B — Add an `openspec/` tree for net-new proposals

Once we have **three or more concurrent non-trivial proposals** that
don't fit cleanly as bullets in an existing phase doc (e.g. "AI
upscale pipeline", "timeline ripple engine", "multi-camera sync"),
we'd:

1. Run `npx -y @fission-ai/openspec@latest init` to bootstrap
   `openspec/` alongside `docs/phases*`.
2. Author each new proposal as `openspec/changes/<slug>/proposal.md`
   using OpenSpec's Purpose + SHALL + Scenario format. That's the
   place where the formal structure is genuinely useful — forcing
   us to state the *actual* requirement, not hand-wave.
3. When a proposal ships, its acceptance summary is folded back into
   the relevant `docs/phases*` doc, and the OpenSpec change is moved
   to `openspec/changes/archive/`. The phase doc remains the
   canonical "what exists today" view; OpenSpec becomes the "how we
   decided to build it" provenance.
4. Update `scripts/lint_phases.sh` to also validate `openspec/`
   proposals carry a matching frontmatter or schema check.

### Phase C — If and only if it's earning its keep, expand

- Archive shipped proposals into OpenSpec's archive layout.
- Consider teaching the in-app Help loader to surface active
  OpenSpec proposals as a "roadmap" topic separate from shipped
  phases.
- Adopt `openspec update` in CI to keep `AGENTS.md` and any related
  agent-instructions files fresh.

### Phase D — Re-evaluate, once a year

- Does OpenSpec still feel worth the round-trip to maintain both
  formats? If so, keep going. If the proposal-vs-phase split feels
  redundant, collapse back to one.
- Has OpenSpec matured (better validator, larger community, CI
  story)? That changes the calculus.

The point is to **earn the complexity**, not import it in advance.

---

## Quick reference

### Lint the phase docs locally

```sh
./scripts/lint_phases.sh         # direct
make lint                        # as part of the full lint pass
```

### Start a new phase doc

```sh
$EDITOR docs/phases/<slug>.md
# Prepend:
# ---
# title: "..."
# status: proposed
# phases: [...]
# last_reviewed: YYYY-MM-DD
# owners: [you]
# ---
```

Then add it to `phase_docs=(...)` in `scripts/lint_phases.sh` — the
explicit list is deliberate. If the file lives under `docs/phases/`,
the lint already picks it up automatically.

### Bump `last_reviewed`

Only when you actually reviewed the doc end-to-end. Mechanical edits
("fix typo in one bullet") don't count.

### Cross-linking

- In phase docs themselves, link to related docs by relative path.
- In code comments, use the full path from repo root
  (`docs/phases-ui.md#u2-e`) so the path stays stable across crate
  boundaries.
- `lint_phases.sh` will catch any broken `docs/phase*` or
  `docs/phases/*` reference across the repo.

---

## References

- [`docs/phase-status.md`](./phase-status.md) — engineering roadmap.
- [`docs/phases-ui.md`](./phases-ui.md) — UI phases, embedded in
  Help.
- [`docs/phases-ui-test.md`](./phases-ui-test.md) — UI testing
  harness phases.
- [`scripts/lint_phases.sh`](../scripts/lint_phases.sh) — the lint.
- [`crates/reel-app/src/shell.rs`](../crates/reel-app/src/shell.rs) —
  `strip_frontmatter` + `include_str!` Help wiring.
- [OpenSpec](https://openspec.dev/) — the thing we didn't adopt.
  Reassess ~yearly.
