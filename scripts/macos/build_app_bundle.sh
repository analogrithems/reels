#!/usr/bin/env bash
# Build target/{debug|release}/reel, wrap it in target/Reel.app with
# AppIcon.icns, and bundle the FFmpeg stack built by build_deps.sh.
#
# Release builds call build_deps.sh automatically if .build/deps/out/ is
# missing, so CI and `make macos-app-release` produce identical bundles.
# Debug builds skip the bundled-deps path for speed and fall back to the
# developer's Homebrew ffmpeg@7 (old behavior).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PROFILE="${1:-release}"
ICON_SRC="${ICON_SRC:-$ROOT/crates/reel-app/ui/assets/knotreels.png}"
PLIST_SRC="$ROOT/packaging/macos/Info.plist"
OUT_APP="$ROOT/target/Reel.app"
DEPS_PREFIX="$ROOT/.build/deps/out"
DEPS_LICENSES="$ROOT/.build/deps/licenses"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "This script is for macOS only." >&2
    exit 1
fi

if [[ ! -f "$ICON_SRC" ]]; then
    echo "Missing logo: $ICON_SRC" >&2
    exit 1
fi

# Release builds use bundled FFmpeg so the resulting Reel.app has no
# Homebrew/system FFmpeg dependency. Debug builds keep the Homebrew path.
BUNDLE_DEPS=0
if [[ "$PROFILE" == "release" ]]; then
    BUNDLE_DEPS=1
    if [[ ! -x "$DEPS_PREFIX/bin/ffmpeg" ]]; then
        echo "==> Bundled FFmpeg not found — running build_deps.sh"
        "$ROOT/scripts/macos/build_deps.sh"
    fi
    export PKG_CONFIG_PATH="$DEPS_PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
fi

if [[ "$PROFILE" == "release" ]]; then
    ( cd "$ROOT" && cargo build -p reel-app --release )
    BIN="$ROOT/target/release/reel"
else
    ( cd "$ROOT" && cargo build -p reel-app )
    BIN="$ROOT/target/debug/reel"
fi

if [[ ! -x "$BIN" && ! -f "$BIN" ]]; then
    echo "Binary not found: $BIN" >&2
    exit 1
fi

VERSION="$(sed -n '/\[workspace.package\]/,/^\[/p' "$ROOT/Cargo.toml" | grep version | head -1 | sed -E 's/.*"([^"]+)".*/\1/')"
if [[ -z "$VERSION" ]]; then
    VERSION="0.1.0"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Square master for iconset (Finder / Dock expect square assets).
sips -p 1024 1024 "$ICON_SRC" --out "$TMP/master.png" >/dev/null

ICONSET="$TMP/Reel.iconset"
mkdir "$ICONSET"
M="$TMP/master.png"
sips -z 16 16     "$M" --out "$ICONSET/icon_16x16.png"     >/dev/null
sips -z 32 32     "$M" --out "$ICONSET/icon_16x16@2x.png"  >/dev/null
sips -z 32 32     "$M" --out "$ICONSET/icon_32x32.png"     >/dev/null
sips -z 64 64     "$M" --out "$ICONSET/icon_32x32@2x.png"  >/dev/null
sips -z 128 128   "$M" --out "$ICONSET/icon_128x128.png"   >/dev/null
sips -z 256 256   "$M" --out "$ICONSET/icon_128x128@2x.png">/dev/null
sips -z 256 256   "$M" --out "$ICONSET/icon_256x256.png"   >/dev/null
sips -z 512 512   "$M" --out "$ICONSET/icon_256x256@2x.png">/dev/null
sips -z 512 512   "$M" --out "$ICONSET/icon_512x512.png"   >/dev/null
sips -z 1024 1024 "$M" --out "$ICONSET/icon_512x512@2x.png">/dev/null

iconutil -c icns "$ICONSET" -o "$TMP/AppIcon.icns"

rm -rf "$OUT_APP"
mkdir -p "$OUT_APP/Contents/MacOS"
mkdir -p "$OUT_APP/Contents/Resources"

cp "$BIN" "$OUT_APP/Contents/MacOS/reel"
chmod +x "$OUT_APP/Contents/MacOS/reel"

cp "$TMP/AppIcon.icns" "$OUT_APP/Contents/Resources/AppIcon.icns"

sed "s/REEL_VERSION/$VERSION/g" "$PLIST_SRC" >"$OUT_APP/Contents/Info.plist"

if [[ "$BUNDLE_DEPS" == "1" ]]; then
    echo "==> Bundling FFmpeg stack from $DEPS_PREFIX"
    FRAMEWORKS="$OUT_APP/Contents/Frameworks"
    mkdir -p "$FRAMEWORKS"

    # Preserve symlinks so libavcodec.61.dylib → libavcodec.61.19.100.dylib etc. still resolve.
    rsync -a --include='*.dylib*' --exclude='*.a' --exclude='pkgconfig/' \
        "$DEPS_PREFIX/lib/" "$FRAMEWORKS/"

    # Ship the ffmpeg CLI too — useful for diagnostics and sidecar use.
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

    # reel: its dylib refs are already @rpath (install_names were set before
    # link time in build_deps.sh) — it just needs the rpath to find them.
    install_name_tool -add_rpath "@executable_path/../Frameworks" "$OUT_APP/Contents/MacOS/reel" 2>/dev/null || true

    # License bundle.
    LIC_OUT="$OUT_APP/Contents/Resources/licenses"
    mkdir -p "$LIC_OUT"
    if [[ -d "$DEPS_LICENSES" ]]; then
        rsync -a "$DEPS_LICENSES/" "$LIC_OUT/"
    fi
    cat > "$LIC_OUT/README.txt" <<'EOF'
This copy of Reel.app bundles FFmpeg and several of its GPL/BSD/LGPL
dependencies. Each dependency's license is included in this folder under
a subdirectory named for the project (ffmpeg/, x264/, x265/, libvpx/,
opus/, lame/).

Reel is distributed under MIT OR Apache-2.0 (see the project repository).
The bundled FFmpeg binary is configured with --enable-gpl and links
libx264 + libx265; redistribution of the combined work is therefore
covered by GPL-2.0-or-later.

Corresponding source for the GPL'd components is published alongside each
tagged release on GitHub (reel-corresponding-source-vX.Y.Z.tar.gz). If
for any reason that asset is missing from a release you received, open
an issue on the Reel GitHub repository and we will make it available.
EOF
fi

echo "Built: $OUT_APP"
echo "Launch: open \"$OUT_APP\""
