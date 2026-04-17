#!/usr/bin/env bash
# Build target/{debug|release}/reel and wrap it in target/Reel.app with AppIcon.icns.
# Finder shows a generic “terminal” icon for bare binaries; only .app bundles get a Dock/Finder icon.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PROFILE="${1:-release}"
ICON_SRC="${ICON_SRC:-$ROOT/crates/reel-app/ui/assets/knotreels.png}"
PLIST_SRC="$ROOT/packaging/macos/Info.plist"
OUT_APP="$ROOT/target/Reel.app"

if [[ "$(uname -s)" != "Darwin" ]]; then
	echo "This script is for macOS only." >&2
	exit 1
fi

if [[ ! -f "$ICON_SRC" ]]; then
	echo "Missing logo: $ICON_SRC" >&2
	exit 1
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
sips -z 16 16 "$M" --out "$ICONSET/icon_16x16.png" >/dev/null
sips -z 32 32 "$M" --out "$ICONSET/icon_16x16@2x.png" >/dev/null
sips -z 32 32 "$M" --out "$ICONSET/icon_32x32.png" >/dev/null
sips -z 64 64 "$M" --out "$ICONSET/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$M" --out "$ICONSET/icon_128x128.png" >/dev/null
sips -z 256 256 "$M" --out "$ICONSET/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$M" --out "$ICONSET/icon_256x256.png" >/dev/null
sips -z 512 512 "$M" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$M" --out "$ICONSET/icon_512x512.png" >/dev/null
sips -z 1024 1024 "$M" --out "$ICONSET/icon_512x512@2x.png" >/dev/null

iconutil -c icns "$ICONSET" -o "$TMP/AppIcon.icns"

rm -rf "$OUT_APP"
mkdir -p "$OUT_APP/Contents/MacOS"
mkdir -p "$OUT_APP/Contents/Resources"

cp "$BIN" "$OUT_APP/Contents/MacOS/reel"
chmod +x "$OUT_APP/Contents/MacOS/reel"

cp "$TMP/AppIcon.icns" "$OUT_APP/Contents/Resources/AppIcon.icns"

sed "s/REEL_VERSION/$VERSION/g" "$PLIST_SRC" >"$OUT_APP/Contents/Info.plist"

echo "Built: $OUT_APP"
echo "Launch: open \"$OUT_APP\""
