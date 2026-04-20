#!/usr/bin/env bash
# Injects the FFmpeg stack (built by build_deps.sh) into an existing
# target/Reel.app. Run AFTER scripts/macos/build_app_bundle.sh.
#
# Called from .github/workflows/release.yml. Not part of the default dev
# loop — developers building locally can keep using Homebrew ffmpeg@7 and
# skip this step; the resulting Reel.app just depends on the dev's brew
# install instead of bundled dylibs.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT_APP="$ROOT/target/Reel.app"
DEPS_PREFIX="$ROOT/.build/deps/out"
DEPS_LICENSES="$ROOT/.build/deps/licenses"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "bundle_deps_into_app.sh is macOS-only." >&2
    exit 1
fi

if [[ ! -d "$OUT_APP" ]]; then
    echo "Reel.app not found at $OUT_APP — run build_app_bundle.sh first." >&2
    exit 1
fi

if [[ ! -x "$DEPS_PREFIX/bin/ffmpeg" ]]; then
    echo "Bundled FFmpeg not found at $DEPS_PREFIX — run build_deps.sh first." >&2
    exit 1
fi

FRAMEWORKS="$OUT_APP/Contents/Frameworks"
mkdir -p "$FRAMEWORKS"

echo "==> Copying dylibs into $FRAMEWORKS"
# Preserve symlinks so libavcodec.61.dylib → libavcodec.61.19.100.dylib
# (and friends) still resolve inside the bundle.
rsync -a --include='*.dylib*' --exclude='*.a' --exclude='pkgconfig/' \
    "$DEPS_PREFIX/lib/" "$FRAMEWORKS/"

# Strip the generated pkgconfig subdir that rsync sometimes keeps from the
# source tree (paranoia — the --exclude above already covers it).
rm -rf "$FRAMEWORKS/pkgconfig"

echo "==> Copying ffmpeg CLI into Reel.app/Contents/MacOS/"
cp "$DEPS_PREFIX/bin/ffmpeg" "$OUT_APP/Contents/MacOS/ffmpeg"
chmod +x "$OUT_APP/Contents/MacOS/ffmpeg"

# Rewrite ffmpeg CLI's dylib refs to @rpath and add the app-relative rpath.
for ref in $(otool -L "$OUT_APP/Contents/MacOS/ffmpeg" | awk 'NR>1 {print $1}'); do
    case "$ref" in
        "$DEPS_PREFIX"/lib/*)
            base=$(basename "$ref")
            install_name_tool -change "$ref" "@rpath/$base" "$OUT_APP/Contents/MacOS/ffmpeg"
            ;;
    esac
done
install_name_tool -add_rpath "@executable_path/../Frameworks" "$OUT_APP/Contents/MacOS/ffmpeg" 2>/dev/null || true

# The reel binary's dylib refs are already @rpath (install_names were set
# by build_deps.sh BEFORE cargo linked against them), so it only needs the
# rpath added to find @rpath/... at runtime.
install_name_tool -add_rpath "@executable_path/../Frameworks" "$OUT_APP/Contents/MacOS/reel" 2>/dev/null || true

echo "==> Copying licenses into Reel.app/Contents/Resources/licenses/"
LIC_OUT="$OUT_APP/Contents/Resources/licenses"
mkdir -p "$LIC_OUT"
if [[ -d "$DEPS_LICENSES" ]]; then
    rsync -a "$DEPS_LICENSES/" "$LIC_OUT/"
fi

cat > "$LIC_OUT/README.txt" <<'EOF'
This copy of Reel.app bundles FFmpeg and several of its GPL/BSD/LGPL
dependencies. Each dependency's license is included in this folder under
a subdirectory named for the project (ffmpeg/, x264/, x265/, libvpx/,
libopus/, libmp3lame/).

Reel itself is distributed under MIT OR Apache-2.0 (see the project
repository). The bundled FFmpeg binary is configured with --enable-gpl
and links libx264 + libx265; redistribution of the combined work is
therefore covered by GPL-2.0-or-later.

Corresponding source for the GPL'd components is published alongside
each tagged release on GitHub as
reel-corresponding-source-vX.Y.Z.tar.gz. If that asset is missing from a
release you received, open an issue on the Reel GitHub repository and
we will make it available.
EOF

# Copy the plugin installer + its deps.toml into Reel.app so the shipped
# reel-cli can invoke it. install_plugin.sh resolves `$ROOT/build/deps.toml`
# from one level above its own directory, so the layout here is:
#   Reel.app/Contents/Resources/scripts/install_plugin.sh
#   Reel.app/Contents/Resources/build/deps.toml
# (see locate_install_plugin_script() in crates/reel-cli/src/main.rs)
mkdir -p "$OUT_APP/Contents/Resources/scripts" "$OUT_APP/Contents/Resources/build"
cp "$ROOT/scripts/install_plugin.sh" "$OUT_APP/Contents/Resources/scripts/install_plugin.sh"
cp "$ROOT/build/deps.toml"           "$OUT_APP/Contents/Resources/build/deps.toml"

echo ""
echo "Bundled FFmpeg into $OUT_APP."
echo "Verify with:"
echo "  otool -L $OUT_APP/Contents/MacOS/reel | grep -E '(libav|libsw|libpost|@rpath)'"
echo "  $OUT_APP/Contents/MacOS/ffmpeg -version"
