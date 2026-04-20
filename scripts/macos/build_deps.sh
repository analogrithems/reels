#!/usr/bin/env bash
# Fetches and builds FFmpeg + GPL deps pinned in build/deps.toml into
# .build/deps/out/. Produces dylibs with @rpath-relative install_names,
# ready for scripts/macos/build_app_bundle.sh to copy into
# Reel.app/Contents/Frameworks/.
#
# Idempotent: re-running skips download + rebuild for deps already present.
# `make build-deps` is the usual entry point.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEPS_TOML="$ROOT/build/deps.toml"
BUILD_ROOT="${BUILD_ROOT:-$ROOT/.build/deps}"
SRC_ROOT="$BUILD_ROOT/src"
PREFIX="$BUILD_ROOT/out"
LICENSES="$BUILD_ROOT/licenses"
JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || echo 4)}"

mkdir -p "$SRC_ROOT" "$PREFIX" "$LICENSES"

export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
export PATH="$PREFIX/bin:$PATH"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "build_deps.sh is macOS-only for now." >&2
    exit 1
fi

log()  { printf '\n\033[1;34m==> %s\033[0m\n' "$*" >&2; }
warn() { printf '\033[1;33m!!! %s\033[0m\n' "$*" >&2; }

# toml_get <section-under-bundled.> <key>  →  single string value
toml_get() {
    local section="[bundled.$1]" key="$2"
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

# toml_get_array <section> <key>  →  one value per line
toml_get_array() {
    local section="[bundled.$1]" key="$2"
    awk -v section="$section" -v key="$key" '
        $0 == section { in_s = 1; next }
        /^\[/         { in_s = 0 }
        in_s && $0 ~ "^" key "[[:space:]]*=[[:space:]]*\\[" { in_a = 1 }
        in_a {
            while (match($0, /"[^"]*"/)) {
                print substr($0, RSTART+1, RLENGTH-2)
                $0 = substr($0, RSTART + RLENGTH)
            }
            if ($0 ~ /\]/) { in_a = 0; exit }
        }
    ' "$DEPS_TOML"
}

fetch_tar() {
    local name="$1" url="$2" expected="$3" out="$4"
    if [[ -f "$out" ]]; then return; fi
    log "$name: downloading $url"
    curl -fL --retry 3 -o "$out.partial" "$url"
    mv "$out.partial" "$out"
    local actual
    actual=$(shasum -a 256 "$out" | awk '{print $1}')
    if [[ "$expected" == "PENDING" ]]; then
        warn "$name: build/deps.toml has sha256 = \"PENDING\""
        warn "$name: observed sha256 = $actual"
        warn "Update build/deps.toml with this value and re-run."
        exit 1
    fi
    if [[ "$actual" != "$expected" ]]; then
        warn "$name: sha256 mismatch"
        warn "  expected: $expected"
        warn "  actual:   $actual"
        exit 1
    fi
}

extract() {
    local name="$1" archive="$2" out="$3"
    if [[ -d "$out" ]]; then return; fi
    log "$name: extracting"
    mkdir -p "$out"
    tar -xf "$archive" -C "$out" --strip-components=1
}

copy_license() {
    local name="$1" src_dir="$2"
    mkdir -p "$LICENSES/$name"
    local any=0
    for f in LICENSE LICENSE.md LICENSE.txt COPYING COPYING.LIB COPYING.md; do
        if [[ -f "$src_dir/$f" ]]; then
            cp "$src_dir/$f" "$LICENSES/$name/"
            any=1
        fi
    done
    if [[ "$any" == "0" ]]; then
        warn "$name: no LICENSE/COPYING file found in $src_dir"
    fi
}

# After every build, rewrite install_name on every dylib in $PREFIX/lib to
# @rpath/<basename>, and rewrite inter-dylib references likewise. The dylib
# that gets linked into `reel` during `cargo build` then bakes @rpath/...
# into the binary directly — no post-link patching of the reel binary.
fixup_rpaths() {
    local dylib base ref refbase
    for dylib in "$PREFIX"/lib/*.dylib; do
        [[ -f "$dylib" && ! -L "$dylib" ]] || continue
        chmod u+w "$dylib"
        base=$(basename "$dylib")
        install_name_tool -id "@rpath/$base" "$dylib" 2>/dev/null || true
        while IFS= read -r ref; do
            case "$ref" in
                "$PREFIX"/lib/*)
                    refbase=$(basename "$ref")
                    install_name_tool -change "$ref" "@rpath/$refbase" "$dylib" 2>/dev/null || true
                    ;;
            esac
        done < <(otool -L "$dylib" | awk 'NR>1 {print $1}')
    done
}

# ---- per-dep builds -----------------------------------------------------

build_x264() {
    local ver url ref src
    ver=$(toml_get x264 version)
    url=$(toml_get x264 url)
    ref=$(toml_get x264 ref)
    src="$SRC_ROOT/x264"
    # Static build → ends up absorbed into libavcodec.dylib.
    if [[ -f "$PREFIX/lib/libx264.a" ]]; then return; fi
    if [[ ! -d "$src/.git" ]]; then
        log "x264 $ver: cloning"
        git clone "$url" "$src"
    fi
    ( cd "$src" && git fetch --all --tags -q && git checkout -q "$ref" )
    copy_license x264 "$src"
    log "x264: build"
    ( cd "$src" && ./configure \
        --prefix="$PREFIX" \
        --enable-static --enable-pic \
        --disable-cli \
      && make -j"$JOBS" && make install )
}

build_x265() {
    local ver url sha tarball src
    ver=$(toml_get x265 version)
    url=$(toml_get x265 url)
    sha=$(toml_get x265 sha256)
    tarball="$SRC_ROOT/x265-$ver.tar.gz"
    src="$SRC_ROOT/x265-$ver"
    if [[ -f "$PREFIX/lib/libx265.a" ]]; then return; fi
    fetch_tar x265 "$url" "$sha" "$tarball"
    extract x265 "$tarball" "$src"
    copy_license x265 "$src"
    log "x265: build"
    local bdir="$src/build/generic"
    mkdir -p "$bdir"
    # Static build → absorbed into libavcodec.dylib by FFmpeg.
    ( cd "$bdir" && cmake \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        -DCMAKE_INSTALL_PREFIX="$PREFIX" \
        -DENABLE_SHARED=OFF \
        -DENABLE_STATIC=ON \
        -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
        -DENABLE_CLI=OFF \
        "$src/source" \
      && make -j"$JOBS" && make install )
}

build_libvpx() {
    local ver url sha tarball src
    ver=$(toml_get libvpx version)
    url=$(toml_get libvpx url)
    sha=$(toml_get libvpx sha256)
    tarball="$SRC_ROOT/libvpx-$ver.tar.gz"
    src="$SRC_ROOT/libvpx-$ver"
    if [[ -f "$PREFIX/lib/libvpx.a" ]]; then return; fi
    fetch_tar libvpx "$url" "$sha" "$tarball"
    extract libvpx "$tarball" "$src"
    copy_license libvpx "$src"
    log "libvpx: build"
    ( cd "$src" && ./configure \
        --prefix="$PREFIX" \
        --enable-pic --disable-shared --enable-static \
        --disable-examples --disable-tools --disable-docs --disable-unit-tests \
      && make -j"$JOBS" && make install )
}

build_libopus() {
    local ver url sha tarball src
    ver=$(toml_get libopus version)
    url=$(toml_get libopus url)
    sha=$(toml_get libopus sha256)
    tarball="$SRC_ROOT/opus-$ver.tar.gz"
    src="$SRC_ROOT/opus-$ver"
    if [[ -f "$PREFIX/lib/libopus.a" ]]; then return; fi
    fetch_tar libopus "$url" "$sha" "$tarball"
    extract libopus "$tarball" "$src"
    copy_license libopus "$src"
    log "libopus: build"
    ( cd "$src" && ./configure \
        --prefix="$PREFIX" \
        --disable-shared --enable-static --with-pic \
      && make -j"$JOBS" && make install )
}

build_libmp3lame() {
    local ver url sha tarball src
    ver=$(toml_get libmp3lame version)
    url=$(toml_get libmp3lame url)
    sha=$(toml_get libmp3lame sha256)
    tarball="$SRC_ROOT/lame-$ver.tar.gz"
    src="$SRC_ROOT/lame-$ver"
    if [[ -f "$PREFIX/lib/libmp3lame.a" ]]; then return; fi
    fetch_tar libmp3lame "$url" "$sha" "$tarball"
    extract libmp3lame "$tarball" "$src"
    copy_license libmp3lame "$src"
    log "libmp3lame: build"
    ( cd "$src" && ./configure \
        --prefix="$PREFIX" \
        --disable-shared --enable-static --with-pic \
        --disable-frontend \
      && make -j"$JOBS" && make install )
}

build_ffmpeg() {
    local ver url sha tarball src
    ver=$(toml_get ffmpeg version)
    url=$(toml_get ffmpeg url)
    sha=$(toml_get ffmpeg sha256)
    tarball="$SRC_ROOT/ffmpeg-$ver.tar.xz"
    src="$SRC_ROOT/ffmpeg-$ver"
    if [[ -f "$PREFIX/bin/ffmpeg" ]]; then return; fi
    fetch_tar ffmpeg "$url" "$sha" "$tarball"
    extract ffmpeg "$tarball" "$src"
    copy_license ffmpeg "$src"

    local flags=()
    while IFS= read -r flag; do
        flags+=("$flag")
    done < <(toml_get_array ffmpeg configure_flags)

    log "ffmpeg: configure"
    ( cd "$src" && ./configure \
        --prefix="$PREFIX" \
        --install-name-dir="@rpath" \
        --extra-cflags="-I$PREFIX/include" \
        --extra-ldflags="-L$PREFIX/lib -Wl,-rpath,@loader_path/../lib" \
        "${flags[@]}" )
    log "ffmpeg: build (-j$JOBS)"
    ( cd "$src" && make -j"$JOBS" && make install )
    fixup_rpaths
}

# ---- main ---------------------------------------------------------------

log "Build prefix: $PREFIX"
build_x264
build_x265
build_libvpx
build_libopus
build_libmp3lame
build_ffmpeg
fixup_rpaths

log "Done. ffmpeg $(toml_get ffmpeg version) → $PREFIX/bin/ffmpeg"
log "Licenses collected at $LICENSES"
