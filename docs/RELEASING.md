# Releasing Reel

This document is for **maintainers** cutting a version on GitHub.

## Version source

The workspace version lives in the root **`Cargo.toml`** under **`[workspace.package]`** → **`version`**. Release **git tags** use the same number with a `v` prefix (e.g. version `0.1.0` → tag **`v0.1.0`**).

## Before tagging

1. **`CHANGELOG.md`** — add a section for the new version at the top (`## [x.y.z] - YYYY-MM-DD`) and summarize user-visible changes.
2. **`make ci`** — green locally (same as GitHub Actions CI).
3. Merge release prep to **`main`** (or the branch you release from).

## Create the release

1. **Create an annotated tag** on the commit you want to ship:
   ```bash
   git switch main
   git pull
   git tag -a v0.1.0 -m "Reel 0.1.0"
   ```
2. **Push the tag** to GitHub:
   ```bash
   git push origin v0.1.0
   ```
3. **GitHub Actions** — workflow **`.github/workflows/release.yml`** runs on `push` of `v*`. It builds **`target/Reel.app`** via **`scripts/macos/build_app_bundle.sh release`**, zips it, and creates a **Release** with the zip attached.
4. On GitHub: **Releases** → open the new release → edit notes if needed (the workflow prepends text and enables **auto-generated release notes**).

## Artifacts

| Output | Description |
|--------|-------------|
| `Reel-vX.Y.Z-macos-arm64.zip` | **Reel.app** (contents), built on `macos-14` (Apple Silicon). |

There is **no** Linux or Windows binary in CI yet. **`reel-cli`** is not published as a separate artifact in this workflow (install from source with `cargo install --path crates/reel-cli` if needed).

## Signing / notarization

Release zips are **unsigned**. First-launch users will hit a Gatekeeper refusal; the `xattr` workaround is documented in the main [README → First launch on macOS](../README.md#first-launch-on-macos-unsigned-build).

Adding Apple Developer ID signing + notarization is tracked under **Phase 4** in [docs/phase-status.md](phase-status.md).

## If the workflow fails

Fix the failure on `main`, then either:

- Move the tag: `git tag -d v0.1.0 && git push origin :refs/tags/v0.1.0`, retag, push again, or  
- Use a new patch tag after the fix.

## Related

- **CI** (lint + test on every PR/main): `.github/workflows/ci.yml`
- **Developer commands**: `docs/DEVELOPERS.md` (`make macos-app`, etc.)
