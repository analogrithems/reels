#!/usr/bin/env bash
# Installs a Reel plugin (pinned in build/deps.toml) into the per-user
# plugin directory. Invoked by `reel-cli plugins install <name>`; also
# runnable standalone for debugging.
#
#   scripts/install_plugin.sh facefusion [--accept-license]
#
# Installs to:
#   macOS:  ~/Library/Application Support/Reel/plugins/<name>/
#   Linux:  ${XDG_DATA_HOME:-~/.local/share}/reel/plugins/<name>/
#
# The plugin's own license (e.g. OpenRAIL-AS for FaceFusion) is written
# into the plugin dir and must be accepted either interactively (y/n
# prompt) or up-front via `--accept-license`. Model weights are NOT
# downloaded here — the plugin fetches them on its own first run, under
# the user's acceptance of each model's upstream ToS.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEPS_TOML="$ROOT/build/deps.toml"

NAME="${1:-}"
ACCEPT_LICENSE=0
shift || true
while [[ $# -gt 0 ]]; do
    case "$1" in
        --accept-license) ACCEPT_LICENSE=1 ;;
        *) echo "unknown flag: $1" >&2; exit 2 ;;
    esac
    shift
done

if [[ -z "$NAME" ]]; then
    echo "usage: $0 <plugin-name> [--accept-license]" >&2
    exit 2
fi

# Minimal TOML reader (same shape as scripts/macos/build_deps.sh).
toml_get() {
    local section="[plugins.$1]" key="$2"
    awk -v section="$section" -v key="$key" '
        $0 == section { in_s = 1; next }
        /^\[/         { in_s = 0 }
        in_s && $1 == key {
            sub(/^[^=]*=[[:space:]]*"/, "")
            sub(/"[[:space:]]*$/, "")
            print; exit
        }
    ' "$DEPS_TOML"
}

VERSION=$(toml_get "$NAME" version)
URL=$(toml_get "$NAME" url)
REF=$(toml_get "$NAME" ref)
LICENSE=$(toml_get "$NAME" license)

if [[ -z "$VERSION" || -z "$URL" || -z "$REF" ]]; then
    echo "plugin '$NAME' not found in $DEPS_TOML" >&2
    exit 1
fi

case "$(uname -s)" in
    Darwin) PLUGIN_ROOT="$HOME/Library/Application Support/Reel/plugins" ;;
    Linux)  PLUGIN_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/reel/plugins" ;;
    *)      echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

PLUGIN_DIR="$PLUGIN_ROOT/$NAME"
SRC_DIR="$PLUGIN_DIR/src"
VENV_DIR="$PLUGIN_DIR/venv"
MARKER="$PLUGIN_DIR/.installed"

mkdir -p "$PLUGIN_DIR"

echo "==> Installing $NAME $VERSION into $PLUGIN_DIR"

# 1. Clone or update the source.
if [[ ! -d "$SRC_DIR/.git" ]]; then
    git clone "$URL" "$SRC_DIR"
fi
( cd "$SRC_DIR" && git fetch --all --tags -q && git checkout -q "$REF" )

# 2. Write the license and require acceptance.
LICENSE_FILE=""
for f in LICENSE.md LICENSE LICENSE.txt COPYING; do
    if [[ -f "$SRC_DIR/$f" ]]; then
        LICENSE_FILE="$SRC_DIR/$f"
        break
    fi
done
if [[ -n "$LICENSE_FILE" ]]; then
    cp "$LICENSE_FILE" "$PLUGIN_DIR/LICENSE"
else
    echo "warning: no LICENSE file found in $NAME source; relying on pinned tag ($LICENSE)" >&2
fi

cat > "$PLUGIN_DIR/LICENSE-NOTICE.txt" <<EOF
$NAME is distributed under $LICENSE. By installing it into Reel you agree
to abide by the full license terms in LICENSE (this directory) and by any
additional behavioral restrictions listed there.

For OpenRAIL-AS licensed plugins specifically: downstream users of Reel
inherit these use restrictions. You may not use $NAME for undisclosed
synthetic media of real people, harassment, non-consensual intimate
imagery, or the other uses prohibited by the license.
EOF

if [[ "$ACCEPT_LICENSE" -ne 1 ]]; then
    echo ""
    echo "--- $NAME license ($LICENSE) ---"
    echo "Full text: $PLUGIN_DIR/LICENSE"
    echo "Summary:   $PLUGIN_DIR/LICENSE-NOTICE.txt"
    echo ""
    read -r -p "Do you accept the license terms? [y/N] " reply
    case "$reply" in
        y|Y|yes|YES) ;;
        *) echo "Aborted — license not accepted. Removing $PLUGIN_DIR."; rm -rf "$PLUGIN_DIR"; exit 1 ;;
    esac
fi

# 3. Create a venv and install Python deps.
if [[ ! -d "$VENV_DIR" ]]; then
    python3 -m venv "$VENV_DIR"
fi
"$VENV_DIR/bin/pip" install --upgrade pip >/dev/null

case "$NAME" in
    facefusion)
        # FaceFusion's installer handles Torch / ONNX Runtime selection.
        # Default to CPU ONNX Runtime; GPU variants are a plugin-config
        # concern we will surface later.
        ( cd "$SRC_DIR" && "$VENV_DIR/bin/python" install.py --onnxruntime default --skip-conda )
        ;;
    *)
        if [[ -f "$SRC_DIR/requirements.txt" ]]; then
            "$VENV_DIR/bin/pip" install -r "$SRC_DIR/requirements.txt"
        elif [[ -f "$SRC_DIR/pyproject.toml" ]]; then
            "$VENV_DIR/bin/pip" install "$SRC_DIR"
        fi
        ;;
esac

date -u +"%Y-%m-%dT%H:%M:%SZ" > "$MARKER"
printf 'version=%s\nref=%s\nlicense=%s\n' "$VERSION" "$REF" "$LICENSE" >> "$MARKER"

echo ""
echo "Installed $NAME $VERSION."
echo "Source:   $SRC_DIR"
echo "Venv:     $VENV_DIR"
echo "License:  $PLUGIN_DIR/LICENSE"
echo ""
echo "To use from Reel, point the sidecar at this checkout:"
echo "  export FACE_FUSION_ROOT=\"$SRC_DIR\""
