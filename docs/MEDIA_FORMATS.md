# Reel — media formats & track support

**Maintenance:** When probe, decode, or project semantics change for containers, codecs, or tracks, update this file and **`docs/FEATURES.md`** as needed. Agents (Cursor, Claude) should keep this accurate for collaborators adding new formats.

Reel uses **FFmpeg** (via **ffmpeg-next 7.1**; development targets **ffmpeg@7**). Anything FFmpeg can demux/decode is *likely* to work for preview, but Reel’s **logic** only understands a subset explicitly.

## Containers (files)

- **In practice:** MP4, MOV, MKV, WebM, and other formats FFmpeg can open are used in tests and daily use.
- **Probe** reports the FFmpeg **container short name** (e.g. `mov`, `mp4`, `matroska`) in `MediaMetadata.container`.
- **No explicit allowlist** in code: unsupported files fail at open/probe with an error.

## Video

- **Streams:** The **first** FFmpeg **video** stream is used for decode (preview and `grab_frame`).
- **Codec:** Whatever FFmpeg decodes (common: H.264, HEVC, VP8, VP9, AV1 where enabled in the build).
- **Pixel format:** Decoded frames are scaled to **RGBA8** for the UI and sidecar.
- **Rotation:** Probe reads **metadata** `rotate` when present and exposes it in `VideoStreamInfo.rotation` (degrees). Further rotation handling in the player may be partial—verify for your assets.
- **Multiple video streams:** Only the **first** video stream is selected; extras are ignored.

## Audio

- **Streams:** The **first** FFmpeg **audio** stream that opens in the decoder is used for playback (resampled to the app’s fixed output layout: stereo f32 @ 48 kHz via cpal).
- **Unrecognized / failing audio codec:** Probe logs a **warning**, sets **`audio_disabled: true`** in metadata, and **does not** fail the whole probe—video-only playback continues.
- **Multiple audio streams:** Only one stream is chosen (first decodable); no user-facing track picker yet.

## Subtitles, data, and other tracks

- **Subtitles (SRT, ASS, PGS, etc.):** **Not decoded or displayed** in the player. They may be present in the file; Reel does not enumerate them in `MediaMetadata` today.
- **Data / attachment / second audio:** Ignored unless future code adds support.

## Project model vs playback

- `reel_core::project::TrackKind` includes **`Video`** and **`Audio`** for serialized projects.
- The **desktop app** currently builds a **single video track** for newly opened media and uses the **first clip’s source** as the primary preview path. Multi-track editing is **not** exposed in the UI yet.

## Export

- **Export…** targets web-friendly outputs (MP4 remux, WebM VP8/Opus, MKV remux) via ffmpeg CLI helpers—codec support depends on the **ffmpeg** binary available at run time.

## Testing

- Fixtures live under `crates/reel-core/tests/fixtures/` (see `scripts/generate_fixtures.sh`).
- Adding a new **supported scenario** (e.g. a new container) should add or extend a fixture + probe/decode test when possible, then update this document.
