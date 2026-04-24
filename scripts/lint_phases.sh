#!/usr/bin/env bash
#
# lint_phases.sh — validate the phase-doc frontmatter convention + catch
# dangling cross-references before they land on main.
#
# Why this exists: we deliberately did NOT adopt OpenSpec (see
# docs/develop_like_me.md for the argument). Our phase docs stay as
# living Markdown with a tiny YAML frontmatter block. This lint keeps the
# convention honest without a full spec framework:
#
#   1. Every tracked phase doc begins with a `---` block containing the
#      required keys: title, status, last_reviewed.
#   2. `status` is one of: living, archived, proposed, deprecated.
#   3. `last_reviewed` is an ISO date (YYYY-MM-DD).
#   4. Cross-references in README.md, docs/*.md, and Rust/Slint source
#      point at files that actually exist. Any broken `docs/phases*`
#      or `docs/phase-*` path reference is a hard error.
#
# Wired into `make lint`. Run directly:
#   ./scripts/lint_phases.sh
#
# Uses only POSIX tools + GNU-portable grep/awk so CI doesn't need yq.

set -eu

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

errors=0
err() {
  printf '\033[0;31merror:\033[0m %s\n' "$*" >&2
  errors=$((errors + 1))
}

# -----------------------------------------------------------------------------
# 1. Collect tracked phase docs.
#    Explicit list (not a glob) so renaming a file is a loud failure rather
#    than a silent drop from the lint set.
# -----------------------------------------------------------------------------
phase_docs=(
  "docs/phase-status.md"
  "docs/phases-ui.md"
  "docs/phases-ui-test.md"
)

# Future-proofing: also pick up anything under docs/phases/ once we split
# phases-ui.md per-phase (see the "Roadmap" section of
# docs/develop_like_me.md).
if [ -d docs/phases ]; then
  # shellcheck disable=SC2207  # deliberate word-split on newlines
  phase_docs+=($(find docs/phases -maxdepth 2 -name '*.md' -type f | sort))
fi

# -----------------------------------------------------------------------------
# 2. Frontmatter validation.
# -----------------------------------------------------------------------------
valid_statuses="living archived proposed deprecated"

for doc in "${phase_docs[@]}"; do
  if [ ! -f "$doc" ]; then
    err "phase doc not found: $doc (remove from the explicit list in $0 if intentional)"
    continue
  fi

  first="$(head -n 1 "$doc" || true)"
  if [ "$first" != "---" ]; then
    err "$doc: missing YAML frontmatter (first line must be '---', got '$first')"
    continue
  fi

  # Slice the frontmatter block — lines 2..N where N is the next `---`.
  fm="$(awk 'NR==1 && $0=="---" {in_fm=1; next}
             in_fm && $0=="---" {exit}
             in_fm {print}' "$doc")"
  if [ -z "$fm" ]; then
    err "$doc: empty or unclosed frontmatter block"
    continue
  fi

  title="$(printf '%s\n' "$fm" | awk -F': *' '$1=="title" {sub(/^title: */,""); print; exit}')"
  status="$(printf '%s\n' "$fm" | awk -F': *' '$1=="status" {print $2; exit}')"
  reviewed="$(printf '%s\n' "$fm" | awk -F': *' '$1=="last_reviewed" {print $2; exit}')"

  [ -n "$title" ]    || err "$doc: frontmatter missing required key 'title'"
  [ -n "$status" ]   || err "$doc: frontmatter missing required key 'status'"
  [ -n "$reviewed" ] || err "$doc: frontmatter missing required key 'last_reviewed'"

  if [ -n "$status" ]; then
    case " $valid_statuses " in
      *" $status "*) : ;;
      *) err "$doc: status '$status' not in {$valid_statuses}" ;;
    esac
  fi

  if [ -n "$reviewed" ] && ! printf '%s' "$reviewed" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'; then
    err "$doc: last_reviewed '$reviewed' is not YYYY-MM-DD"
  fi
done

# -----------------------------------------------------------------------------
# 3. Cross-reference integrity.
#    Any reference in README/docs/source to `docs/phase-status.md`,
#    `docs/phases-*.md`, or `docs/phases/*.md` must resolve to a real file.
#    Matches the path, not the anchor — missing anchors are a separate lint.
# -----------------------------------------------------------------------------
# Build the set of search targets without descending into target/, node_modules/, etc.
ref_sources=()
while IFS= read -r -d '' f; do ref_sources+=("$f"); done < <(
  find . \
    -type d \( -name target -o -name node_modules -o -name .git -o -name build -o -name dist \) -prune -o \
    \( -name '*.md' -o -name '*.rs' -o -name '*.slint' -o -name '*.toml' -o -name '*.sh' \) \
    -type f -print0
)

# Grep for phase-doc path references — anchor-stripped, quote-tolerant.
while IFS= read -r ref; do
  # The reference as seen on disk (strip leading `./`).
  ref_clean="${ref#./}"
  if [ ! -f "$ref_clean" ]; then
    # Find which files mentioned it for a useful error.
    mentioners="$(grep -lE "$(printf '%s' "$ref_clean" | sed 's|[.]|\\.|g')" "${ref_sources[@]}" 2>/dev/null | head -5 | tr '\n' ' ')"
    err "dangling reference to '$ref_clean' in: $mentioners"
  fi
done < <(
  grep -hoE 'docs/(phase-status|phases-[a-z0-9-]+|phases/[a-z0-9_-]+)\.md' "${ref_sources[@]}" 2>/dev/null \
    | sort -u
)

# -----------------------------------------------------------------------------
# Result.
# -----------------------------------------------------------------------------
if [ "$errors" -gt 0 ]; then
  printf '\n\033[0;31m%d error(s) — see docs/develop_like_me.md for the phase-doc convention.\033[0m\n' "$errors" >&2
  exit 1
fi

printf 'phase-doc lint: %d file(s) OK\n' "${#phase_docs[@]}"
