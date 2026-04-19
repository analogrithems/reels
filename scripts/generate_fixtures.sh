#!/usr/bin/env bash
# Regenerate tiny test fixtures using the system `ffmpeg` binary.
#
# The generated files are committed to the repo so CI doesn't need to
# regenerate them on every run; this script exists so they can be reproduced
# deterministically.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$here/../crates/reel-core/tests/fixtures"
mkdir -p "$out"

# 1) happy path: H.264 video + AAC audio, 1 second, 64x64
ffmpeg -y -v error \
    -f lavfi -i "testsrc=size=64x64:rate=15:duration=1" \
    -f lavfi -i "sine=frequency=440:duration=1" \
    -c:v libx264 -preset ultrafast -tune zerolatency -g 15 -pix_fmt yuv420p \
    -c:a aac -b:a 32k \
    -shortest \
    "$out/tiny_h264_aac.mp4"

# 1b) same as (1) but 256x144 — meets the DNxHR encoder's 256x120 minimum,
#     so the MKV DNxHR HQ export test has an input the `dnxhd` encoder will
#     accept. All other presets still run against the 64x64 fixture above.
ffmpeg -y -v error \
    -f lavfi -i "testsrc=size=256x144:rate=15:duration=1" \
    -f lavfi -i "sine=frequency=440:duration=1" \
    -c:v libx264 -preset ultrafast -tune zerolatency -g 15 -pix_fmt yuv420p \
    -c:a aac -b:a 32k \
    -shortest \
    "$out/tiny_h264_aac_256x144.mp4"

# 2) video-only
ffmpeg -y -v error \
    -f lavfi -i "testsrc=size=64x64:rate=15:duration=1" \
    -c:v libx264 -preset ultrafast -tune zerolatency -g 15 -pix_fmt yuv420p \
    "$out/no_audio.mp4"

# 3) "weird audio" — codec that ffmpeg-next may or may not decode reliably.
#    We mux PCM_F32LE into an MKV which the decoder opens cleanly on most
#    builds; the stream is still present for probe-test verification. If we
#    ever need a truly unrecognized codec we can repack a raw blob into a
#    container; for now this covers the "audio stream exists" case.
ffmpeg -y -v error \
    -f lavfi -i "testsrc=size=64x64:rate=15:duration=1" \
    -f lavfi -i "sine=frequency=660:duration=1" \
    -c:v libx264 -preset ultrafast -g 15 -pix_fmt yuv420p \
    -c:a pcm_f32le \
    -shortest \
    "$out/weird_audio.mkv"

echo "fixtures written to $out"
ls -lh "$out"/tiny_h264_aac.mp4 "$out"/tiny_h264_aac_256x144.mp4 "$out"/no_audio.mp4 "$out"/weird_audio.mkv
