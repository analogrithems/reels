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
3. **GitHub Actions** — workflow **`.github/workflows/release.yml`** runs on `push` of `v*`. It:
   1. Builds the FFmpeg stack pinned in **`build/deps.toml`** via **`scripts/macos/build_deps.sh`** (x264, x265, libvpx, libopus, libmp3lame, then FFmpeg itself with `--enable-gpl`). Cached between runs, keyed on the hash of `build/deps.toml`.
   2. Builds **`target/Reel.app`** with `PKG_CONFIG_PATH` pointing at that stack (**`scripts/macos/build_app_bundle.sh release`**).
   3. Injects the built dylibs, the `ffmpeg` CLI, and all collected license files into the bundle (**`scripts/macos/bundle_deps_into_app.sh`**).
   4. Zips the bundle + assembles a **corresponding-source tarball** (every upstream source tree the bundled binary was built from, plus `build/deps.toml` + `build_deps.sh`) for GPL compliance.
   5. Creates a GitHub Release with both assets attached.
4. On GitHub: **Releases** → open the new release → edit notes if needed (the workflow prepends text and enables **auto-generated release notes**).

## Artifacts

| Output | Description |
|--------|-------------|
| `Reel-vX.Y.Z-macos-arm64.zip` | **Reel.app** (self-contained: bundled FFmpeg + dylibs + licenses), built on `macos-14` (Apple Silicon). |
| `reel-corresponding-source-vX.Y.Z.tar.gz` | GPL corresponding source — upstream FFmpeg / x264 / x265 / libvpx / libopus / libmp3lame trees at the pinned refs, plus `build/deps.toml` and `build_deps.sh`. |

There is **no** Linux or Windows binary in CI yet. **`reel-cli`** is not published as a separate artifact in this workflow (install from source with `cargo install --path crates/reel-cli` if needed).

## Dependency pinning

Both the **bundled** FFmpeg stack and the **plugin** versions (e.g. FaceFusion for the plugin system) are pinned in a single file: **`build/deps.toml`**. Bump a version or ref there, leave `sha256 = "PENDING"` for the fetch to fail and print the actual SHA, update with the printed value, and commit. The release workflow will rebuild on the next tag push.

The **bundled** FFmpeg is configured `--enable-gpl` and links libx264 + libx265; the resulting binary is therefore distributed under **GPL-2.0-or-later** even though the Reel source is MIT OR Apache-2.0. License texts for every bundled dependency ship at `Reel.app/Contents/Resources/licenses/`; corresponding source goes in the companion tarball (above).

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
