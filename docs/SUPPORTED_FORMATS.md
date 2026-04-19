# Reel — supported formats (playback vs export)

**Maintenance:** When decode, probe, or export behavior changes, update this file, **`docs/MEDIA_FORMATS.md`** (track-level behavior), and **`docs/FEATURES.md`** if user-visible.

This document complements **`docs/MEDIA_FORMATS.md`**: that file explains *how* Reel picks streams and exports; this one maps **common delivery targets** (web, mobile, “golden stack”) to **what works in Reel today**.

**Stack:** Reel uses **FFmpeg** (Rust bindings **ffmpeg-next 7.1**; development targets **Homebrew `ffmpeg@7`**). Preview generally follows **what your `ffmpeg` can decode**; **export** is **preset-limited** (see below).

---

## How to read the status columns

### Playback (preview)

- **Yes** — The **first video** stream decodes to RGBA; audio plays at stereo float @ 48 kHz, defaulting to the **first decodable** stream but user-selectable per clip via **Edit → Audio Track** on any source with ≥ 2 decodable audio streams. **No (subs)** — Subtitle tracks are **not** shown (decode graph has no subtitle path yet).
- **Partial** — Often works, but **first stream only**, `audio_disabled` if audio fails, or **container/codec** depends on your FFmpeg build.

### Export (**File → Export…**)

The app first asks for a **preset** (MP4 remux, **MP4 — H.264 + AAC**, WebM VP8+Opus, or MKV remux), then a **save path** filtered to that extension. Each maps to `reel_core::WebExportFormat` (`crates/reel-core/src/media/export.rs`):

| Target | Container | ffmpeg behavior |
|--------|-----------|-----------------|
| **`.mp4` — remux** | MP4 | **Stream copy** (`-c copy`, `+faststart`) when mux accepts the streams. |
| **`.mp4` — H.264 + AAC** | MP4 | **Always re-encode**: `libx264 -preset medium -crf 20 -pix_fmt yuv420p`, AAC **160 kbps**, `+faststart`. Use when remux fails on codec mismatch or you need a guaranteed baseline. |
| **`.webm`** | WebM | **Always re-encode** to **VP8** + **Opus** (not VP9/AV1-out). |
| **`.mkv`** | Matroska | **Stream copy** (`-c copy`). |

**Remux** preserves source codecs when copy succeeds; **MP4 H.264 + AAC** and **WebM** are the fixed **transcode** paths.

When the project’s **first audio** lane has clips, export builds a **second** concat for audio and **muxes** it with the primary video concat (duration = primary **video** length). If that lane is empty, export behaves like **video-only** concat (audio may still come from **embedded** streams in the video files).

---

## 1. Web platforms (YouTube & TikTok)

These platforms favor formats that **encode and stream efficiently** (H.264 as the universal upload baseline; **AV1** increasingly for efficiency; **AAC-LC** and **Opus** for audio). Reel’s role is **edit + preview + export**; **upload specs** remain defined by each platform.

| Track type | Formats & codecs | Role on platform | Reel playback | Reel export |
| :-- | :-- | :-- | :-- | :-- |
| **Video** | **H.264 (AVC)**, **H.265 (HEVC)**, **VP9**, **AV1** | H.264 is the usual **compat** choice; HEVC/VP9/AV1 for **efficiency** / quality tiers. | **Yes** (FFmpeg decode; HEVC/AV1 depend on build). | **Remux** to MP4/MKV when **`-c copy`** works; **MP4 — H.264 + AAC** preset forces a guaranteed H.264/AAC MP4 re-encode; **WebM** → **VP8** only (not VP9/AV1-out). |
| **Audio** | **AAC (AAC-LC)**, **Opus**, **MP3** | AAC-LC is the common **social/delivery** default; Opus on YouTube at low bitrates. | **Yes** | **Remux** where mux allows; **WebM** → **Opus**. |
| **Subtitles** | **SRT**, **WebVTT** (`.vtt`), **TTML** | SRT for simple captions; **WebVTT** for web-native styling/positioning. | **No (subs)** — not decoded or shown in the player. | **Not** in export UI (no timeline-driven subtitle mux). |

---

## 2. Mobile device expectations (iOS & Android)

Hardware decoders on phones/tablets favor a **small set** of codecs; Reel on desktop still **depends on FFmpeg**, not on mobile silicon. The table below is **what users expect on device** vs **what Reel does today**.

### iOS (Apple)

| Kind | Typical formats | Reel playback | Reel export |
| :-- | :-- | :-- | :-- |
| **Video** | **HEVC (H.265)** (common on modern iPhones), **H.264**, **ProRes** (editing / pro) | **H.264 / HEVC:** yes (FFmpeg). **ProRes:** usually yes decode. | **Remux** to MP4/MKV/MOV when copy + mux allow; **MOV — ProRes 422 HQ + PCM** intermediate preset for pro handoff (`prores_ks -profile:v 3` + `pcm_s16le`). |
| **Audio** | **AAC**, **Apple Lossless (ALAC)**, **MP3** | **AAC / MP3:** yes. **ALAC:** typically yes (FFmpeg). | **Remux**; **WebM** → Opus. |
| **Subtitles** | **iTT** (iTunes Timed Text), **CEA-608 / 708** | **No (subs)** in UI. | **Not** exposed. |

### Android

| Kind | Typical formats | Reel playback | Reel export |
| :-- | :-- | :-- | :-- |
| **Video** | **VP9**, **H.264**, **AV1** (growing on newer OS levels) | **Yes** when FFmpeg build supports the codec. | Same as §1 video row (remux vs **WebM** VP8+Opus). |
| **Audio** | **AAC**, **Opus**, **FLAC**, **Vorbis** | **Yes** | **FLAC** → MKV remux ok; MP4 remux may reject lossless—use MKV or transcode path when we add presets. **WebM** → Opus. |
| **Subtitles** | **SRT**, **WebVTT** | **No (subs)** | **Not** exposed. |

---

## 3. Recommended “golden stack” for a multi-track app

A practical **delivery default** for wide compatibility:

| Layer | Recommendation | In Reel today |
| :-- | :-- | :-- |
| **Primary container** | **MP4** or **MOV** (interchangeable for many workflows) | **Open:** yes. **Save as project:** `.reel` JSON. **Export video:** **`.mp4`** (remux or **H.264 + AAC** transcode) / **`.mkv`** remux / **`.webm`** transcode — **no** dedicated **`.mov`** export menu target. |
| **Video** | **H.264** for maximum compatibility; **HEVC** for 4K / high-quality reels when devices support it | Decode yes; **export** = MP4 remux, **MP4 — H.264 + AAC** transcode (libx264 CRF 20 + AAC 160 kbps + `+faststart`), MKV remux, or VP8 **WebM**. HEVC encode not offered as a preset. |
| **Audio** | **AAC-LC** stereo (e.g. 128 kbps+) | Decode/remux yes; no bitrate **preset** UI. |
| **Subtitles** | **WebVTT** for styling/positioning (Reels-style captions); **ASS/SSA** for advanced typography/animation | **Not** in player or export pipeline yet. |

> **Pro note:** **ASS/SSA** is widely used for styled fansubs and lyric videos; supporting it later implies a **subtitle track model** in the timeline, not only file passthrough.

---

## Other containers & codecs (quick reference)

**File → Open…** offers a **Media (video & audio)** filter listing common extensions (including **MPEG** program stream **`.mpg` / `.mpeg`**, **MPEG‑TS** **`.ts` / `.mts` / `.m2ts`**, and typical audio types). The exact list is maintained in **`crates/reel-app/src/media_extensions.rs`** (`OPEN_MEDIA_EXTENSIONS`). Anything FFmpeg can demux may still be chosen via **All files**.

| Topic | Notes |
| :-- | :-- |
| **MPEG‑TS, FLV, OGG, AVI** | Often play; export only via MP4/MKV/WebM presets above. |
| **AC‑3, E‑AC‑3, DTS** | Often play; **remux** to MKV; MP4 may reject some streams. |
| **Chapters / metadata** | Partial via probe; not export presets. |

---

## Roadmap (gaps vs targets above — priority)

1. **VP9 and/or AV1** as **WebM** (or MP4) **export options** — Align **export** with **web platform** rows (today: **VP8 + Opus** only for `.webm`).
2. **HEVC + AAC MP4** preset — **Mobile-tier** delivery (iPhone-style), complementing the **MP4 — H.264 + AAC** preset that now ships (**shipped:** libx264 CRF 20 + AAC 160 kbps + `+faststart`).
3. **Clear remux errors** — When MP4 rejects HEVC/AC‑3/etc., surface a path to **transcode presets** (licensing / mux constraints). The **MP4 — H.264 + AAC** preset is the existing MP4-side fallback; update error copy to point users at it when remux stream-copy fails.
4. ~~**MOV export** and/or **ProRes / DNx** handoff — **iOS / pro** workflows.~~ **Shipped:** **MOV** remux preset, **MOV — ProRes 422 HQ + PCM** and **MKV — DNxHR HQ + PCM** intermediate presets (`prores_ks -profile:v 3` / `dnxhd -profile:v dnxhr_hq`, both with `pcm_s16le`).
5. **Subtitles — WebVTT, SRT, TTML** (preview, edit, mux or burn-in); **ASS/SSA** as a follow-on for styled captions.
6. **Multi-audio** stream selection — **Preview-side shipped.** **Edit → Audio Track** lists every decodable audio stream probed off the source and lets you pick per-clip; preview honours the selection, `Clip.audio_stream_index: Option<u32>` is serialised with `None` elided so existing projects round-trip byte-stable. **Remaining:** export-side stream selection (today the ffmpeg audio graph still picks the first decodable stream per clip), and a language/title-aware default for dubs-heavy sources.

**Product alignment:** The **export preset picker** (see **`docs/phases-ui.md` Phase U3**, **`docs/FEATURES.md`**) should map user-visible options to the tiers above (web + mobile **golden stack**, compatibility remux).

Tracking: **`docs/phase-status.md`** → **Format support roadmap** and **UI initiative checklist**.

---

## See also

- **`docs/MEDIA_FORMATS.md`** — First-stream rules, rotation, `audio_disabled`, concat export.
- **`crates/reel-core/src/media/export.rs`** — `WebExportFormat`, ffmpeg arguments.
- **`docs/FEATURES.md`** — User-facing roadmap.
- **`docs/phases-ui.md`** — U2–U4 milestones (**trim UI**, **View** chrome, **Open Recent**).
