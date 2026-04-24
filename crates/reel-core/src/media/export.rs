//! Export / transcode using the `ffmpeg` **CLI** (same tool as `brew install ffmpeg@7`).
//!
//! Used by the desktop app and by integration tests that write verification
//! assets under `target/`.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::project::{ClipOrientation, ClipScale};

/// Build a combined ffmpeg `-vf` filter chain from optional per-project-clip
/// transforms. Flips and rotation come first (matching [`ClipOrientation`]'s
/// contract), scale next, then subtitle burn-in **last** — so captions are
/// rendered on the final, already-oriented-and-scaled frame at delivery
/// resolution (otherwise a 50% scale would shrink the text to illegibility).
fn combined_vf_chain(
    orientation: Option<ClipOrientation>,
    scale: Option<ClipScale>,
    subtitles: Option<&Path>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(chain) = orientation.and_then(|o| o.ffmpeg_filter_chain()) {
        parts.push(chain);
    }
    if let Some(chain) = scale.and_then(|s| s.ffmpeg_filter_chain()) {
        parts.push(chain);
    }
    if let Some(sub) = subtitles {
        parts.push(format!("subtitles='{}'", escape_subtitles_path(sub)));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

/// Escape a filesystem path for ffmpeg's `subtitles='...'` filter argument.
///
/// Inside single-quoted filter-graph arguments, `\` and `'` must be escaped;
/// `:` needs an extra backslash because the outer filter parser treats `:` as
/// an option-separator. Backslashes (Windows paths) are normalised to `/`
/// first so the resulting chain stays portable.
fn escape_subtitles_path(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            ':' => out.push_str("\\:"),
            _ => out.push(c),
        }
    }
    out
}

/// Web-friendly outputs we guarantee in tests (see `tests/export_web_formats.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebExportFormat {
    /// MP4 container; stream copy when codecs allow (fast).
    Mp4Remux,
    /// MP4 container; explicit H.264 + AAC transcode — use when remux fails due to
    /// incompatible source codecs, or when a fixed delivery target (H.264 / AAC-LC)
    /// is required regardless of the input.
    Mp4H264Aac,
    /// MP4 container; HEVC (H.265) + AAC transcode — mobile-tier preset.
    /// Useful for iOS-native delivery and smaller files at equivalent quality
    /// vs H.264. Decode support in older browsers is limited.
    Mp4H265Aac,
    /// WebM VP8 + Opus (faster to encode than VP9 for short fixtures).
    WebmVp8Opus,
    /// WebM VP9 + Opus — better compression than VP8 at the cost of encode time.
    WebmVp9Opus,
    /// WebM AV1 + Opus (libaom-av1) — best compression, slowest encode. Good for
    /// archival / streaming delivery where encode time is amortized.
    WebmAv1Opus,
    /// Matroska; stream copy for quick container swap tests.
    MkvRemux,
    /// QuickTime `.mov`; stream copy — pro-handoff container that keeps
    /// H.264 / HEVC + AAC / PCM without re-encoding. Uses `+faststart` so
    /// delivery uploads can start without a post-mux index rewrite, same as
    /// the MP4 remux path.
    MovRemux,
    /// QuickTime `.mov`; **Apple ProRes 422 HQ + PCM** intermediate — pro
    /// handoff to Final Cut / DaVinci / Premiere. Always transcodes (there's
    /// no ProRes stream-copy path) via `prores_ks -profile:v 3` at
    /// `yuv422p10le`, paired with `pcm_s16le` audio. Output is visually
    /// lossless at far smaller sizes than uncompressed, but considerably
    /// larger than any web preset — this is a mastering / finishing format,
    /// not a delivery target.
    MovProResHq,
    /// Matroska `.mkv`; **DNxHR HQ + PCM** intermediate — Avid-style pro
    /// handoff. Always transcodes via `dnxhd -profile:v dnxhr_hq` at
    /// `yuv422p`, paired with `pcm_s16le` audio. DNxHR HQ targets 4:2:2
    /// 8-bit mastering; pick ProRes 422 HQ if the downstream tool prefers
    /// Apple codecs.
    MkvDnxhrHq,
    /// Animated GIF — Sharp preset (~720 px wide, 24 fps, 256 colors).
    /// Single-pass `palettegen`+`paletteuse` with Bayer dithering. No audio
    /// — GIF is a silent image format; audio tracks are dropped via `-an`.
    /// Largest of the GIF tiers; use when you need crisp motion on a
    /// full-resolution screenshot-style capture.
    GifSharp,
    /// Animated GIF — Good preset (~540 px wide, 20 fps, 128 colors).
    /// Balanced default: visibly smoother than "Share" without a dramatic
    /// size penalty. No audio.
    GifGood,
    /// Animated GIF — Share preset (~480 px wide, 15 fps, 96 colors).
    /// Sized for chat / Slack / Discord attachments; 15 fps is the
    /// perceptual floor for motion that still reads as "video". No audio.
    GifShare,
    /// Animated GIF — Tiny preset (~320 px wide, 12 fps, 64 colors).
    /// Aggressive quantization for email embedding or low-bandwidth
    /// uploads. Dithering is rougher to keep the palette budget honest.
    /// No audio.
    GifTiny,
}

/// Parameters used to build the `palettegen`+`paletteuse` filter graph for
/// a GIF export preset. Exposed so the UI can display the numeric settings
/// ("480 px · 15 fps · 96 colors") and test code can assert on the values
/// that actually get fed to ffmpeg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GifPreset {
    /// Output caps at `min(max_width, iw)` px wide, auto-computing even
    /// height with `-2`. Caps only — source smaller than `max_width`
    /// passes through untouched (no upscaling).
    pub max_width: u32,
    /// Output frame rate. 15 is the visual floor for motion; 24 is close
    /// to film. Below ~10 looks like a slideshow.
    pub fps: u32,
    /// `palettegen=max_colors=` — GIF supports up to 256. Dropping below
    /// 64 quickly produces visible banding on gradients.
    pub colors: u32,
    /// `paletteuse=dither=` argument. `"bayer:bayer_scale=N"` for ordered
    /// dithering (lower N = more dither, bigger files); `"sierra2_4a"` for
    /// error-diffusion (sharper, slightly bigger).
    pub dither: &'static str,
}

impl GifPreset {
    pub const SHARP: Self = Self {
        max_width: 720,
        fps: 24,
        colors: 256,
        dither: "bayer:bayer_scale=5",
    };
    pub const GOOD: Self = Self {
        max_width: 540,
        fps: 20,
        colors: 128,
        dither: "bayer:bayer_scale=5",
    };
    pub const SHARE: Self = Self {
        max_width: 480,
        fps: 15,
        colors: 96,
        dither: "bayer:bayer_scale=4",
    };
    pub const TINY: Self = Self {
        max_width: 320,
        fps: 12,
        colors: 64,
        dither: "bayer:bayer_scale=3",
    };
}

impl WebExportFormat {
    pub const ALL: [WebExportFormat; 14] = [
        WebExportFormat::Mp4Remux,
        WebExportFormat::Mp4H264Aac,
        WebExportFormat::Mp4H265Aac,
        WebExportFormat::WebmVp8Opus,
        WebExportFormat::WebmVp9Opus,
        WebExportFormat::WebmAv1Opus,
        WebExportFormat::MkvRemux,
        WebExportFormat::MovRemux,
        WebExportFormat::MovProResHq,
        WebExportFormat::MkvDnxhrHq,
        WebExportFormat::GifSharp,
        WebExportFormat::GifGood,
        WebExportFormat::GifShare,
        WebExportFormat::GifTiny,
    ];

    pub fn file_extension(self) -> &'static str {
        match self {
            WebExportFormat::Mp4Remux
            | WebExportFormat::Mp4H264Aac
            | WebExportFormat::Mp4H265Aac => "mp4",
            WebExportFormat::WebmVp8Opus
            | WebExportFormat::WebmVp9Opus
            | WebExportFormat::WebmAv1Opus => "webm",
            WebExportFormat::MkvRemux | WebExportFormat::MkvDnxhrHq => "mkv",
            WebExportFormat::MovRemux | WebExportFormat::MovProResHq => "mov",
            WebExportFormat::GifSharp
            | WebExportFormat::GifGood
            | WebExportFormat::GifShare
            | WebExportFormat::GifTiny => "gif",
        }
    }

    /// Returns the quantization parameters for animated GIF presets, or
    /// `None` for every non-GIF format. Used by
    /// `append_format_args_with_vf` to build the palette filter graph
    /// and by the UI to show the "WxH · FPSfps · Ncolors" line without
    /// duplicating the constants.
    pub fn gif_preset(self) -> Option<GifPreset> {
        match self {
            WebExportFormat::GifSharp => Some(GifPreset::SHARP),
            WebExportFormat::GifGood => Some(GifPreset::GOOD),
            WebExportFormat::GifShare => Some(GifPreset::SHARE),
            WebExportFormat::GifTiny => Some(GifPreset::TINY),
            _ => None,
        }
    }

    /// True for any animated GIF preset. Callers use this to skip the
    /// audio-bearing branches of the export pipeline (GIF drops audio
    /// regardless of source), to override the file-dialog filter to
    /// `*.gif`, and to show the "no audio" UI affordance.
    pub fn is_gif(self) -> bool {
        self.gif_preset().is_some()
    }
}

#[derive(Debug)]
pub struct ExportError {
    pub message: String,
}

impl ExportError {
    /// True when the caller requested cancellation (ffmpeg was interrupted).
    pub fn is_cancelled(&self) -> bool {
        self.message == EXPORT_CANCELLED_MSG
    }
}

const EXPORT_CANCELLED_MSG: &str = "export cancelled";

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExportError {}

/// Optional **0.0..=1.0** progress callback for long exports (driven by ffmpeg `-progress` `out_time_ms`
/// vs the primary timeline duration). Ignored when `None`.
pub type ExportProgressFn = Arc<dyn Fn(f64) + Send + Sync>;

/// Run `ffmpeg` to convert `input` to `output` using `format`.
///
/// Fails with a clear [`ExportError`] if `ffmpeg` is missing or returns non-zero.
pub fn export_with_ffmpeg(
    input: &Path,
    output: &Path,
    format: WebExportFormat,
) -> Result<(), ExportError> {
    if let Some(parent) = output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .stdout(Stdio::null());

    append_format_args(&mut cmd, format);
    cmd.arg(output);
    run_ffmpeg(cmd, output, None, None)
}

/// Generate a stereo 48 kHz silent WAV of the requested duration at `output`.
///
/// Used by the U2-e **partial-clip mute silence substitution** path: when some
/// primary clips are muted and there's no dedicated audio lane, the export
/// thread writes one silence file sized at the longest unmuted run and reuses
/// it across the synthetic audio concat. WAV/PCM is chosen deliberately — it
/// remuxes cleanly into any container ffmpeg wraps (MP4/MOV/MKV/WebM audio
/// transcodes always re-encode), and there's no chance of codec-mismatch
/// surprises vs the primary source.
///
/// Fails with [`ExportError`] if ffmpeg is missing, the duration is non-finite /
/// non-positive, or the write fails.
pub fn generate_silence_wav(output: &Path, duration_s: f64) -> Result<(), ExportError> {
    if !duration_s.is_finite() || duration_s <= 0.0 {
        return Err(ExportError {
            message: format!("silence generation: non-positive duration ({duration_s})"),
        });
    }
    if let Some(parent) = output.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("anullsrc=channel_layout=stereo:sample_rate=48000")
        .arg("-t")
        .arg(format!("{duration_s:.6}"))
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(output)
        .stdout(Stdio::null());
    run_ffmpeg(cmd, output, None, None)
}

/// Build a synthetic audio lane that substitutes silence (from `silence_path`)
/// for the spans whose `mute_mask` entry is `true`, and keeps the primary video
/// file's embedded audio for the rest.
///
/// Parallel indices in `video_spans` and `mute_mask` are paired. Mask length mismatches
/// or empty inputs return an empty lane — the caller is expected to route
/// through the video-only path in that case.
///
/// Silence spans are emitted as `(silence_path, 0.0, span_duration)` so the
/// ffconcat demuxer reads a fresh `span_duration` chunk of silence for each
/// muted primary clip. The silence file must be at least as long as the
/// longest muted span (callers compute this upfront).
pub fn build_mute_substitution_lane(
    video_spans: &[(PathBuf, f64, f64)],
    mute_mask: &[bool],
    silence_path: &Path,
) -> Vec<(PathBuf, f64, f64)> {
    if video_spans.is_empty() || video_spans.len() != mute_mask.len() {
        return Vec::new();
    }
    video_spans
        .iter()
        .zip(mute_mask.iter())
        .map(|((path, in_s, out_s), muted)| {
            if *muted {
                let dur = (*out_s - *in_s).max(0.0);
                (silence_path.to_path_buf(), 0.0, dur)
            } else {
                (path.clone(), *in_s, *out_s)
            }
        })
        .collect()
}

/// Export the **primary video track** as one output: one or more `(path, in_point_sec, out_point_sec)` spans.
///
/// - **One segment:** uses `-ss` / `-t` (fast seek before decode).
/// - **Multiple segments:** writes a temporary `ffconcat` script (`inpoint` / `outpoint` per file) and runs
///   `ffmpeg -f concat -safe 0 -i …`.
///
/// Paths must exist on disk. Stream copy (`-c copy`) may fail if segments use incompatible codecs; use a
/// transcode preset (e.g. [`WebExportFormat::WebmVp8Opus`]) when sources differ.
///
/// When `cancel` is set and becomes true, ffmpeg is killed and [`ExportError::is_cancelled`] is true on the result.
///
/// `on_ratio` is invoked on the **export thread** with approximate completion **0.0..=1.0** (throttled).
pub fn export_concat_timeline(
    segments: &[(PathBuf, f64, f64)],
    output: &Path,
    format: WebExportFormat,
    cancel: Option<&AtomicBool>,
    on_ratio: Option<ExportProgressFn>,
) -> Result<(), ExportError> {
    export_concat_timeline_oriented(
        segments, None, None, None, false, output, format, cancel, on_ratio,
    )
}

/// Like [`export_concat_timeline`] but applies `orientation` (rotate/flip) and/or
/// `scale` to the whole output via `-vf`. When either is non-identity, the preset
/// is forced into a transcode path regardless of whether it would normally stream-copy.
///
/// `mute_audio` emits `-an` so the output has no audio track. Paired with the
/// **Edit → Mute Clip Audio** edit when every primary-track clip is muted.
pub fn export_concat_timeline_oriented(
    segments: &[(PathBuf, f64, f64)],
    orientation: Option<ClipOrientation>,
    scale: Option<ClipScale>,
    subtitles: Option<&Path>,
    mute_audio: bool,
    output: &Path,
    format: WebExportFormat,
    cancel: Option<&AtomicBool>,
    on_ratio: Option<ExportProgressFn>,
) -> Result<(), ExportError> {
    if segments.is_empty() {
        return Err(ExportError {
            message: "timeline export: no segments".into(),
        });
    }
    for (p, in_s, out_s) in segments {
        if *out_s <= *in_s {
            return Err(ExportError {
                message: format!(
                    "timeline export: invalid span for {} (in {in_s} >= out {out_s})",
                    p.display()
                ),
            });
        }
        if !p.is_file() {
            return Err(ExportError {
                message: format!("timeline export: not a file: {}", p.display()),
            });
        }
    }

    if let Some(parent) = output.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let total_ms = timeline_total_ms(segments);
    let progress = map_progress_fn(on_ratio, total_ms);
    let progress_pack = pack_progress_tempfile(progress)?;
    let vf_chain = combined_vf_chain(orientation, scale, subtitles);

    if segments.len() == 1 {
        let (p, in_s, out_s) = &segments[0];
        let dur = out_s - in_s;
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y");
        if let Some((_, _, ref pf)) = progress_pack {
            cmd.arg("-progress").arg(pf.path());
        }
        cmd.arg("-ss")
            .arg(format!("{in_s:.6}"))
            .arg("-i")
            .arg(p)
            .arg("-t")
            .arg(format!("{dur:.6}"))
            .stdout(Stdio::null());
        append_format_args_with_vf(&mut cmd, format, vf_chain.as_deref());
        if mute_audio {
            cmd.arg("-an");
        }
        cmd.arg(output);
        return run_ffmpeg(cmd, output, cancel, progress_pack);
    }

    let dir = tempfile::tempdir().map_err(|e| ExportError {
        message: format!("timeline export: temp dir: {e}"),
    })?;
    let list_path = dir.path().join("reel_timeline.ffconcat");
    write_ffconcat_list(segments, &list_path)?;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y");
    if let Some((_, _, ref pf)) = progress_pack {
        cmd.arg("-progress").arg(pf.path());
    }
    cmd.arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&list_path)
        .stdout(Stdio::null());
    append_format_args_with_vf(&mut cmd, format, vf_chain.as_deref());
    if mute_audio {
        cmd.arg("-an");
    }
    cmd.arg(output);
    run_ffmpeg(cmd, output, cancel, progress_pack)
}

fn segments_duration_s(segments: &[(PathBuf, f64, f64)]) -> f64 {
    segments.iter().map(|(_, a, b)| b - a).sum()
}

fn timeline_total_ms(segments: &[(PathBuf, f64, f64)]) -> u64 {
    let s = segments_duration_s(segments);
    (s * 1000.0).max(1.0).round() as u64
}

fn map_progress_fn(
    on_ratio: Option<ExportProgressFn>,
    total_ms: u64,
) -> Option<(u64, ExportProgressFn)> {
    if total_ms == 0 {
        return None;
    }
    on_ratio.map(|f| (total_ms, f))
}

fn pack_progress_tempfile(
    progress: Option<(u64, ExportProgressFn)>,
) -> Result<Option<(u64, ExportProgressFn, tempfile::NamedTempFile)>, ExportError> {
    match progress {
        None => Ok(None),
        Some((ms, cb)) => {
            let f = tempfile::NamedTempFile::new().map_err(|e| ExportError {
                message: format!("export progress tempfile: {e}"),
            })?;
            Ok(Some((ms, cb, f)))
        }
    }
}

/// Export primary **video** concat and optionally mux with a **dedicated audio** concat (first audio lane).
///
/// When `audio_segments` is `None` or empty, behaves like [`export_concat_timeline`] (video-only; embedded
/// audio from each video file may still be copied if present).
///
/// When `audio_segments` is non-empty, runs ffmpeg with two concat inputs, `-map 0:v:0 -map 1:a:0`, and caps
/// output duration to the **video** timeline length (`-t`), matching in-app preview (master = primary video).
pub fn export_concat_with_audio(
    video_segments: &[(PathBuf, f64, f64)],
    audio_segments: Option<&[(PathBuf, f64, f64)]>,
    output: &Path,
    format: WebExportFormat,
    cancel: Option<&AtomicBool>,
    on_ratio: Option<ExportProgressFn>,
) -> Result<(), ExportError> {
    export_concat_with_audio_oriented(
        video_segments,
        audio_segments,
        None,
        None,
        None,
        false,
        output,
        format,
        cancel,
        on_ratio,
    )
}

/// U2-b multi-audio-lane export entry point. Accepts **0, 1, or N** audio lanes:
///
/// - **0 lanes** (`audio_lanes.is_empty()` or `mute_audio`): delegates to the
///   video-only path (`export_concat_timeline_oriented`).
/// - **1 lane**: delegates to the existing single-audio path
///   (`export_concat_with_audio_oriented`) — unchanged fast path, `-c:a copy`
///   still possible on remux presets.
/// - **2+ lanes**: mixes via ffmpeg `amix` (see
///   `export_concat_with_audio_mix_oriented`). Forces audio transcode because
///   amix produces a new stream.
///
/// `audio_lanes` must be in project order (first = preview-driving lane).
/// Empty lane vectors inside the outer vec are not expected — `timeline::
/// clips_from_all_audio_tracks` strips lanes with no clips.
pub fn export_concat_with_audio_lanes_oriented(
    video_segments: &[(PathBuf, f64, f64)],
    audio_lanes: &[Vec<(PathBuf, f64, f64)>],
    orientation: Option<ClipOrientation>,
    scale: Option<ClipScale>,
    subtitles: Option<&Path>,
    mute_audio: bool,
    output: &Path,
    format: WebExportFormat,
    cancel: Option<&AtomicBool>,
    on_ratio: Option<ExportProgressFn>,
) -> Result<(), ExportError> {
    // Gain-less convenience: delegate to the gain-aware entry point with
    // `None`. Keeps every existing call site byte-stable.
    export_concat_with_audio_lanes_oriented_with_gains(
        video_segments,
        audio_lanes,
        None,
        orientation,
        scale,
        subtitles,
        mute_audio,
        output,
        format,
        cancel,
        on_ratio,
    )
}

/// Gain-aware entry point. `lane_gains_db` — when `Some` — must have the same
/// length as `audio_lanes`; each element is the per-lane volume adjustment in
/// decibels (0.0 = unity, unaltered).
///
/// ### Routing rules
///
/// * **0 lanes / mute**: video-only path (gain irrelevant).
/// * **1 lane, unity gain**: existing single-audio dual-mux path — keeps
///   `-c:a copy` stream-copy eligibility on remux presets. This is the fast
///   case we must preserve for the "no per-lane gain configured" majority.
/// * **1 lane, non-zero gain**: routes through the amix filter with
///   `inputs=1` so `volume=XdB` can slot in front — amix-with-1-input is a
///   passthrough, which means the only thing the filter graph actually does
///   is apply the user's gain. Audio transcodes as a side effect (filter-
///   graph output can never stream-copy).
/// * **2+ lanes** (any gain configuration): amix path, per-lane `volume=XdB`
///   prepended on each non-unity input.
///
/// ### Why route N=1 through amix for gain
///
/// The single-audio dual-mux path is a direct `-map 1:a:0` that accepts only
/// a `-c:a` codec switch. Injecting `volume=XdB` there would require a parallel
/// branch with `-filter:a` and container-specific codec selection that the
/// amix path already solves. Routing N=1 through amix pays the cost of one
/// transcode for a ~10 loc function instead of duplicating the codec dispatch.
pub fn export_concat_with_audio_lanes_oriented_with_gains(
    video_segments: &[(PathBuf, f64, f64)],
    audio_lanes: &[Vec<(PathBuf, f64, f64)>],
    lane_gains_db: Option<&[f32]>,
    orientation: Option<ClipOrientation>,
    scale: Option<ClipScale>,
    subtitles: Option<&Path>,
    mute_audio: bool,
    output: &Path,
    format: WebExportFormat,
    cancel: Option<&AtomicBool>,
    on_ratio: Option<ExportProgressFn>,
) -> Result<(), ExportError> {
    if let Some(g) = lane_gains_db {
        if g.len() != audio_lanes.len() {
            return Err(ExportError {
                message: format!(
                    "timeline export: lane_gains_db length {} != audio_lanes length {}",
                    g.len(),
                    audio_lanes.len()
                ),
            });
        }
    }
    if mute_audio || audio_lanes.is_empty() || format.is_gif() {
        // GIF cannot carry audio, so any audio-lane setup is dropped on
        // the floor and we route through the single-input path. We pass
        // `mute_audio` through rather than forcing it true so preview
        // behavior for audio-bearing formats is unaffected — the GIF
        // branch just adds another "no audio in output" reason.
        return export_concat_timeline_oriented(
            video_segments,
            orientation,
            scale,
            subtitles,
            mute_audio,
            output,
            format,
            cancel,
            on_ratio,
        );
    }
    // Compute the "any non-unity gain?" flag once — this is what moves a
    // single-lane export off the fast dual-mux path onto the amix path.
    let any_gain = lane_gains_db
        .map(|g| g.iter().any(|db| *db != 0.0))
        .unwrap_or(false);

    if audio_lanes.len() == 1 && !any_gain {
        return export_concat_with_audio_oriented(
            video_segments,
            Some(audio_lanes[0].as_slice()),
            orientation,
            scale,
            subtitles,
            false,
            output,
            format,
            cancel,
            on_ratio,
        );
    }
    export_concat_with_audio_mix_oriented(
        video_segments,
        audio_lanes,
        lane_gains_db,
        orientation,
        scale,
        subtitles,
        output,
        format,
        cancel,
        on_ratio,
    )
}

/// Like [`export_concat_with_audio`] but applies `orientation` (rotate/flip) and/or
/// `scale` to the mapped video stream via `-vf`. Non-identity filters force the video
/// codec into a transcode; audio stays stream-copied for remux presets.
///
/// `mute_audio` drops the audio track from the output (`-an`); when true, this
/// function ignores `audio_segments` and delegates to the video-only pipeline.
pub fn export_concat_with_audio_oriented(
    video_segments: &[(PathBuf, f64, f64)],
    audio_segments: Option<&[(PathBuf, f64, f64)]>,
    orientation: Option<ClipOrientation>,
    scale: Option<ClipScale>,
    subtitles: Option<&Path>,
    mute_audio: bool,
    output: &Path,
    format: WebExportFormat,
    cancel: Option<&AtomicBool>,
    on_ratio: Option<ExportProgressFn>,
) -> Result<(), ExportError> {
    if mute_audio || format.is_gif() {
        return export_concat_timeline_oriented(
            video_segments,
            orientation,
            scale,
            subtitles,
            true,
            output,
            format,
            cancel,
            on_ratio,
        );
    }
    let Some(audio_segments) = audio_segments.filter(|a| !a.is_empty()) else {
        return export_concat_timeline_oriented(
            video_segments,
            orientation,
            scale,
            subtitles,
            false,
            output,
            format,
            cancel,
            on_ratio,
        );
    };

    if video_segments.is_empty() {
        return Err(ExportError {
            message: "timeline export: no video segments".into(),
        });
    }
    for (p, in_s, out_s) in video_segments {
        if *out_s <= *in_s {
            return Err(ExportError {
                message: format!(
                    "timeline export: invalid video span for {} (in {in_s} >= out {out_s})",
                    p.display()
                ),
            });
        }
        if !p.is_file() {
            return Err(ExportError {
                message: format!("timeline export: not a file: {}", p.display()),
            });
        }
    }
    for (p, in_s, out_s) in audio_segments {
        if *out_s <= *in_s {
            return Err(ExportError {
                message: format!(
                    "timeline export: invalid audio span for {} (in {in_s} >= out {out_s})",
                    p.display()
                ),
            });
        }
        if !p.is_file() {
            return Err(ExportError {
                message: format!("timeline export: not a file: {}", p.display()),
            });
        }
    }

    if let Some(parent) = output.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let video_dur = segments_duration_s(video_segments);
    if video_dur <= 0.0 {
        return Err(ExportError {
            message: "timeline export: zero video duration".into(),
        });
    }

    let total_ms = timeline_total_ms(video_segments);
    let progress = map_progress_fn(on_ratio, total_ms);
    let progress_pack = pack_progress_tempfile(progress)?;
    let vf_chain = combined_vf_chain(orientation, scale, subtitles);

    let dir = tempfile::tempdir().map_err(|e| ExportError {
        message: format!("timeline export: temp dir: {e}"),
    })?;
    let v_list = dir.path().join("reel_video.ffconcat");
    let a_list = dir.path().join("reel_audio.ffconcat");
    write_ffconcat_list(video_segments, &v_list)?;
    write_ffconcat_list(audio_segments, &a_list)?;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y");
    if let Some((_, _, ref pf)) = progress_pack {
        cmd.arg("-progress").arg(pf.path());
    }
    cmd.arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&v_list)
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&a_list)
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("1:a:0")
        .arg("-t")
        .arg(format!("{video_dur:.6}"))
        .stdout(Stdio::null());
    append_dual_mux_format_args_with_vf(&mut cmd, format, vf_chain.as_deref());
    cmd.arg(output);
    run_ffmpeg(cmd, output, cancel, progress_pack)
}

/// Mix `audio_lanes.len() >= 2` audio lanes onto a concat video timeline via
/// `-filter_complex` with an `amix` node. Each lane becomes its own `concat`
/// demuxer input; the mixed stream is emitted as `[aout]`.
///
/// `normalize=0` means lanes are summed at unit gain (no automatic half-volume
/// for N=2). Callers that need attenuation should apply it per-lane upstream or
/// request a follow-up preview-mixer change. `duration=longest` keeps the
/// output audio as long as the longest lane; the `-t video_dur` cap still
/// bounds total output length to the video timeline.
///
/// The amix output cannot be stream-copied — every preset forces a container-
/// appropriate audio codec via [`append_mixed_audio_format_args`].
fn export_concat_with_audio_mix_oriented(
    video_segments: &[(PathBuf, f64, f64)],
    audio_lanes: &[Vec<(PathBuf, f64, f64)>],
    lane_gains_db: Option<&[f32]>,
    orientation: Option<ClipOrientation>,
    scale: Option<ClipScale>,
    subtitles: Option<&Path>,
    output: &Path,
    format: WebExportFormat,
    cancel: Option<&AtomicBool>,
    on_ratio: Option<ExportProgressFn>,
) -> Result<(), ExportError> {
    // Previously this guarded `< 2` but the gain-aware dispatcher now routes
    // N=1 through here when a non-zero gain is present (amix with `inputs=1`
    // is a passthrough that lets `volume=XdB` plug in cleanly without a
    // bespoke single-audio filter path). Empty still has to fail.
    if audio_lanes.is_empty() {
        return Err(ExportError {
            message: "timeline export: amix requires at least 1 audio lane".into(),
        });
    }
    if let Some(g) = lane_gains_db {
        if g.len() != audio_lanes.len() {
            return Err(ExportError {
                message: format!(
                    "timeline export: lane_gains_db length {} != audio_lanes length {}",
                    g.len(),
                    audio_lanes.len()
                ),
            });
        }
    }
    if video_segments.is_empty() {
        return Err(ExportError {
            message: "timeline export: no video segments".into(),
        });
    }
    for (p, in_s, out_s) in video_segments {
        if *out_s <= *in_s {
            return Err(ExportError {
                message: format!(
                    "timeline export: invalid video span for {} (in {in_s} >= out {out_s})",
                    p.display()
                ),
            });
        }
        if !p.is_file() {
            return Err(ExportError {
                message: format!("timeline export: not a file: {}", p.display()),
            });
        }
    }
    for (lane_idx, lane) in audio_lanes.iter().enumerate() {
        if lane.is_empty() {
            return Err(ExportError {
                message: format!("timeline export: audio lane {lane_idx} is empty"),
            });
        }
        for (p, in_s, out_s) in lane {
            if *out_s <= *in_s {
                return Err(ExportError {
                    message: format!(
                        "timeline export: invalid audio span for {} (lane {lane_idx}, in {in_s} >= out {out_s})",
                        p.display()
                    ),
                });
            }
            if !p.is_file() {
                return Err(ExportError {
                    message: format!("timeline export: not a file: {}", p.display()),
                });
            }
        }
    }

    if let Some(parent) = output.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let video_dur = segments_duration_s(video_segments);
    if video_dur <= 0.0 {
        return Err(ExportError {
            message: "timeline export: zero video duration".into(),
        });
    }

    let total_ms = timeline_total_ms(video_segments);
    let progress = map_progress_fn(on_ratio, total_ms);
    let progress_pack = pack_progress_tempfile(progress)?;
    let vf_chain = combined_vf_chain(orientation, scale, subtitles);

    let dir = tempfile::tempdir().map_err(|e| ExportError {
        message: format!("timeline export: temp dir: {e}"),
    })?;
    let v_list = dir.path().join("reel_video.ffconcat");
    write_ffconcat_list(video_segments, &v_list)?;
    let mut a_lists = Vec::with_capacity(audio_lanes.len());
    for (i, lane) in audio_lanes.iter().enumerate() {
        let a_list = dir.path().join(format!("reel_audio_{i}.ffconcat"));
        write_ffconcat_list(lane, &a_list)?;
        a_lists.push(a_list);
    }

    let n = audio_lanes.len();
    let filter_complex =
        build_amix_filter_complex_with_gains(n, vf_chain.as_deref(), lane_gains_db);

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y");
    if let Some((_, _, ref pf)) = progress_pack {
        cmd.arg("-progress").arg(pf.path());
    }
    cmd.arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&v_list);
    for a_list in &a_lists {
        cmd.arg("-f")
            .arg("concat")
            .arg("-safe")
            .arg("0")
            .arg("-i")
            .arg(a_list);
    }
    cmd.arg("-filter_complex").arg(&filter_complex);
    if vf_chain.is_some() {
        cmd.arg("-map").arg("[vout]");
    } else {
        cmd.arg("-map").arg("0:v:0");
    }
    cmd.arg("-map")
        .arg("[aout]")
        .arg("-t")
        .arg(format!("{video_dur:.6}"))
        .stdout(Stdio::null());
    append_mixed_audio_format_args(&mut cmd, format, vf_chain.is_some());
    cmd.arg(output);
    run_ffmpeg(cmd, output, cancel, progress_pack)
}

/// Build the `-filter_complex` argument for the amix export. Audio inputs start
/// at ffmpeg input index 1 because the single video concat lives at index 0.
/// When `vf_chain` is present, the mapped video also routes through filter_complex
/// as `[0:v:0]VF[vout]` so callers can `-map [vout]`.
///
/// Thin wrapper around [`build_amix_filter_complex_with_gains`] for callers
/// that don't need per-lane volume adjustment. Preserves the historical
/// signature so older tests keep compiling unchanged.
#[cfg(test)]
fn build_amix_filter_complex(n: usize, vf_chain: Option<&str>) -> String {
    build_amix_filter_complex_with_gains(n, vf_chain, None)
}

/// Gain-aware variant of [`build_amix_filter_complex`].
///
/// When `gains` is `Some`, each non-zero entry emits a prefix
/// `[i+1:a:0]volume=XdB[aI];` clause ahead of the amix node, and the
/// corresponding amix input is relabeled from `[i+1:a:0]` to `[aI]`. Unity
/// entries (and the whole `None` case) fall through to the plain `[i+1:a:0]`
/// wire we've always emitted — so byte-for-byte parity with today's output
/// when nothing has gain set.
///
/// We stringify dB with `"{:.2}"` (six-char cap: `-12.50`, `0.50`, etc.) to
/// keep the filter graph readable in logs while preserving enough precision
/// for any slider the UI realistically ships (real audio work doesn't want
/// sub-0.01 dB anyway — tenths or hundredths of a dB are inaudible).
///
/// `inputs=1` on amix is a valid ffmpeg passthrough, so a single-lane gain
/// export routes through the identical filter-graph skeleton as N=2 — no
/// special-case code for N=1.
fn build_amix_filter_complex_with_gains(
    n: usize,
    vf_chain: Option<&str>,
    gains: Option<&[f32]>,
) -> String {
    debug_assert!(n >= 1, "amix path requires >= 1 lane");
    if let Some(g) = gains {
        debug_assert_eq!(
            g.len(),
            n,
            "gain vector length must match lane count (dispatcher already checks this)"
        );
    }
    let mut s = String::new();
    if let Some(chain) = vf_chain {
        s.push_str(&format!("[0:v:0]{chain}[vout];"));
    }

    // First pass: emit any per-lane `volume=XdB` prefilters.
    // `[i+1:a:0]volume=XdB[aI]` — we relabel non-unity lanes so the amix
    // section below can reference `[aI]` for those and `[i+1:a:0]` for the
    // pure passthroughs.
    if let Some(g) = gains {
        for (i, db) in g.iter().enumerate() {
            if *db != 0.0 {
                let input_idx = i + 1;
                s.push_str(&format!("[{input_idx}:a:0]volume={db:.2}dB[a{i}];"));
            }
        }
    }

    // Second pass: list the amix inputs, picking the relabeled tap for any
    // lane that got a volume prefilter and the raw `[i+1:a:0]` otherwise.
    for i in 0..n {
        let is_boosted = gains.map(|g| g[i] != 0.0).unwrap_or(false);
        if is_boosted {
            s.push_str(&format!("[a{i}]"));
        } else {
            let input_idx = i + 1;
            s.push_str(&format!("[{input_idx}:a:0]"));
        }
    }
    s.push_str(&format!(
        "amix=inputs={n}:duration=longest:normalize=0[aout]"
    ));
    s
}

/// Codec selection for the amix path. Unlike the single-audio dual-mux helper,
/// amix output is always filter-graph PCM and cannot be `-c:a copy` — every
/// preset forces a container-appropriate audio encoder. Video stays stream-
/// copy-eligible only when no `-vf`/`[vout]` chain is active.
fn append_mixed_audio_format_args(cmd: &mut Command, format: WebExportFormat, vf_present: bool) {
    match (format, vf_present) {
        (WebExportFormat::Mp4Remux, false) => {
            cmd.args([
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::Mp4Remux, true) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::Mp4H264Aac, _) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "20",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::Mp4H265Aac, _) => {
            cmd.args([
                "-c:v",
                "libx265",
                "-preset",
                "medium",
                "-crf",
                "24",
                "-pix_fmt",
                "yuv420p",
                "-tag:v",
                "hvc1",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::WebmVp8Opus, _) => {
            cmd.args([
                "-c:v",
                "libvpx",
                "-quality",
                "good",
                "-cpu-used",
                "4",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::WebmVp9Opus, _) => {
            cmd.args([
                "-c:v",
                "libvpx-vp9",
                "-b:v",
                "0",
                "-crf",
                "32",
                "-row-mt",
                "1",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::WebmAv1Opus, _) => {
            cmd.args([
                "-c:v",
                "libaom-av1",
                "-crf",
                "30",
                "-b:v",
                "0",
                "-cpu-used",
                "6",
                "-row-mt",
                "1",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::MkvRemux, false) => {
            cmd.args(["-c:v", "copy", "-c:a", "aac", "-b:a", "160k"]);
        }
        (WebExportFormat::MkvRemux, true) => {
            cmd.args([
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "160k",
            ]);
        }
        (WebExportFormat::MovRemux, false) => {
            cmd.args([
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::MovRemux, true) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        // ProRes and DNxHR are intermediate masters — no stream-copy path
        // (amix emits filter-graph PCM anyway) — and amix output is PCM
        // already so we just pipe it into pcm_s16le. The same arms apply
        // whether or not a video `-vf` chain is present.
        (WebExportFormat::MovProResHq, _) => {
            cmd.args([
                "-c:v",
                "prores_ks",
                "-profile:v",
                "3",
                "-pix_fmt",
                "yuv422p10le",
                "-c:a",
                "pcm_s16le",
            ]);
        }
        (WebExportFormat::MkvDnxhrHq, _) => {
            cmd.args([
                "-c:v",
                "dnxhd",
                "-profile:v",
                "dnxhr_hq",
                "-pix_fmt",
                "yuv422p",
                "-c:a",
                "pcm_s16le",
            ]);
        }
        // GIF never reaches the amix path — the public entry points
        // (`export_concat_with_audio_lanes_oriented_with_gains` et al.)
        // detect GIF and route to the single-input no-audio pipeline
        // before we get here. This arm exists only so the match stays
        // exhaustive; hitting it means the routing was bypassed.
        (
            WebExportFormat::GifSharp
            | WebExportFormat::GifGood
            | WebExportFormat::GifShare
            | WebExportFormat::GifTiny,
            _,
        ) => unreachable!("gif is routed to the audio-less single-input path"),
    }
}

/// Like [`append_format_args_with_vf`] but for two-input `-map 0:v:0 -map 1:a:0`
/// concat (dedicated audio lane). Filters apply to the mapped **video** stream only.
fn append_dual_mux_format_args_with_vf(
    cmd: &mut Command,
    format: WebExportFormat,
    vf_chain: Option<&str>,
) {
    if let Some(chain) = vf_chain {
        // Filter only the mapped video stream from input 0.
        cmd.arg("-filter:v:0").arg(chain);
    }
    match (format, vf_chain.is_some()) {
        (WebExportFormat::Mp4Remux, false) => {
            cmd.args(["-c:v", "copy", "-c:a", "copy", "-movflags", "+faststart"]);
        }
        (WebExportFormat::Mp4Remux, true) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "copy",
                "-movflags",
                "+faststart",
            ]);
        }
        // Audio stream in the dedicated-lane path already comes from input 1 as an
        // audio stream, but we can't assume it's AAC — transcode to AAC for guaranteed
        // MP4 conformance whether or not `-vf` is present.
        (WebExportFormat::Mp4H264Aac, _) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "20",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::Mp4H265Aac, _) => {
            cmd.args([
                "-c:v",
                "libx265",
                "-preset",
                "medium",
                "-crf",
                "24",
                "-pix_fmt",
                "yuv420p",
                "-tag:v",
                "hvc1",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::WebmVp8Opus, _) => {
            cmd.args([
                "-c:v",
                "libvpx",
                "-quality",
                "good",
                "-cpu-used",
                "4",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::WebmVp9Opus, _) => {
            cmd.args([
                "-c:v",
                "libvpx-vp9",
                "-b:v",
                "0",
                "-crf",
                "32",
                "-row-mt",
                "1",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::WebmAv1Opus, _) => {
            cmd.args([
                "-c:v",
                "libaom-av1",
                "-crf",
                "30",
                "-b:v",
                "0",
                "-cpu-used",
                "6",
                "-row-mt",
                "1",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::MkvRemux, false) => {
            cmd.args(["-c:v", "copy", "-c:a", "copy"]);
        }
        (WebExportFormat::MkvRemux, true) => {
            cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "copy"]);
        }
        (WebExportFormat::MovRemux, false) => {
            cmd.args(["-c:v", "copy", "-c:a", "copy", "-movflags", "+faststart"]);
        }
        (WebExportFormat::MovRemux, true) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "copy",
                "-movflags",
                "+faststart",
            ]);
        }
        // ProRes / DNxHR intermediates: always transcode video, always
        // transcode audio to PCM (intermediates don't gain anything from
        // stream-copying the dedicated audio lane, and PCM is the standard
        // pairing for Avid / Apple finishing workflows).
        (WebExportFormat::MovProResHq, _) => {
            cmd.args([
                "-c:v",
                "prores_ks",
                "-profile:v",
                "3",
                "-pix_fmt",
                "yuv422p10le",
                "-c:a",
                "pcm_s16le",
            ]);
        }
        (WebExportFormat::MkvDnxhrHq, _) => {
            cmd.args([
                "-c:v",
                "dnxhd",
                "-profile:v",
                "dnxhr_hq",
                "-pix_fmt",
                "yuv422p",
                "-c:a",
                "pcm_s16le",
            ]);
        }
        // GIF never reaches the dual-mux path — see the matching note
        // in `append_mixed_audio_format_args`.
        (
            WebExportFormat::GifSharp
            | WebExportFormat::GifGood
            | WebExportFormat::GifShare
            | WebExportFormat::GifTiny,
            _,
        ) => unreachable!("gif is routed to the audio-less single-input path"),
    }
}

fn write_ffconcat_list(segments: &[(PathBuf, f64, f64)], out: &Path) -> Result<(), ExportError> {
    let mut parts = vec!["ffconcat version 1.0".to_string()];
    for (path, in_s, out_s) in segments {
        let abs = fs::canonicalize(path).map_err(|e| ExportError {
            message: format!("timeline export: {}: {e}", path.display()),
        })?;
        let quoted = ffconcat_quote_path(&abs);
        parts.push(format!("file {quoted}"));
        parts.push(format!("inpoint {in_s:.6}"));
        parts.push(format!("outpoint {out_s:.6}"));
    }
    let body = parts.join("\n") + "\n";
    fs::write(out, body).map_err(|e| ExportError {
        message: format!("timeline export: write concat list: {e}"),
    })
}

/// Single-quote path for ffconcat `file` directive; escape `'` as `'\''`.
/// Normalizes Windows backslashes to `/` for ffmpeg portability.
fn ffconcat_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let esc = raw.replace('\'', "'\\''");
    format!("'{esc}'")
}

fn append_format_args(cmd: &mut Command, format: WebExportFormat) {
    append_format_args_with_vf(cmd, format, None);
}

/// `vf_chain` is an optional `-vf` ffmpeg filter chain (e.g. `"hflip,transpose=1"`).
/// When present, `-c copy` presets are swapped to an H.264/AAC transcode so the filter
/// can actually run (ffmpeg cannot apply a filter and stream-copy the same video stream).
fn append_format_args_with_vf(cmd: &mut Command, format: WebExportFormat, vf_chain: Option<&str>) {
    // GIF takes a completely different path: we build our own `-vf` that
    // composes the caller's orientation/scale chain with a
    // `palettegen`+`paletteuse` single-pass graph, so emit nothing here
    // and let the GIF arm below own the full `-vf`. For every other
    // format the caller's chain goes in as-is.
    if let Some(preset) = format.gif_preset() {
        let full_chain = build_gif_vf(vf_chain, preset);
        cmd.arg("-vf").arg(full_chain);
        // GIF is an image-only format; drop any input audio streams so
        // the output contains just the animated image. Also force the
        // `gif` muxer — redundant with the .gif extension in practice
        // but explicit avoids a muxer guess when exporting to a path
        // without the extension.
        cmd.args(["-an", "-c:v", "gif", "-f", "gif"]);
        return;
    }
    if let Some(chain) = vf_chain {
        cmd.arg("-vf").arg(chain);
    }
    match (format, vf_chain.is_some()) {
        (WebExportFormat::Mp4Remux, false) => {
            cmd.args(["-c", "copy", "-movflags", "+faststart"]);
        }
        (WebExportFormat::Mp4Remux, true) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::Mp4H264Aac, _) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "20",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::Mp4H265Aac, _) => {
            cmd.args([
                "-c:v",
                "libx265",
                "-preset",
                "medium",
                "-crf",
                "24",
                "-pix_fmt",
                "yuv420p",
                "-tag:v",
                "hvc1",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
            ]);
        }
        (WebExportFormat::WebmVp8Opus, _) => {
            cmd.args([
                "-c:v",
                "libvpx",
                "-quality",
                "good",
                "-cpu-used",
                "4",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::WebmVp9Opus, _) => {
            cmd.args([
                "-c:v",
                "libvpx-vp9",
                "-b:v",
                "0",
                "-crf",
                "32",
                "-row-mt",
                "1",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::WebmAv1Opus, _) => {
            cmd.args([
                "-c:v",
                "libaom-av1",
                "-crf",
                "30",
                "-b:v",
                "0",
                "-cpu-used",
                "6",
                "-row-mt",
                "1",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ]);
        }
        (WebExportFormat::MkvRemux, false) => {
            cmd.args(["-c", "copy"]);
        }
        (WebExportFormat::MkvRemux, true) => {
            cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
        }
        (WebExportFormat::MovRemux, false) => {
            cmd.args(["-c", "copy", "-movflags", "+faststart"]);
        }
        (WebExportFormat::MovRemux, true) => {
            cmd.args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-movflags",
                "+faststart",
            ]);
        }
        // ProRes / DNxHR intermediates: single-input path (embedded audio
        // or no audio). Always transcode video; audio goes to PCM.
        (WebExportFormat::MovProResHq, _) => {
            cmd.args([
                "-c:v",
                "prores_ks",
                "-profile:v",
                "3",
                "-pix_fmt",
                "yuv422p10le",
                "-c:a",
                "pcm_s16le",
            ]);
        }
        (WebExportFormat::MkvDnxhrHq, _) => {
            cmd.args([
                "-c:v",
                "dnxhd",
                "-profile:v",
                "dnxhr_hq",
                "-pix_fmt",
                "yuv422p",
                "-c:a",
                "pcm_s16le",
            ]);
        }
        // GIF is handled by the early return at the top of this
        // function. Listed here so the match stays exhaustive and any
        // future GIF variant we add doesn't silently fall through to a
        // non-GIF codec arm.
        (
            WebExportFormat::GifSharp
            | WebExportFormat::GifGood
            | WebExportFormat::GifShare
            | WebExportFormat::GifTiny,
            _,
        ) => unreachable!("gif variants handled via early return above"),
    }
}

/// Compose the caller's orientation/scale chain with the GIF palette
/// graph. Output shape:
///
/// ```text
/// [extra,]fps=F,scale='min(W,iw)':-2:flags=lanczos,split[s0][s1];
/// [s0]palettegen=max_colors=N[p];[s1][p]paletteuse=dither=D
/// ```
///
/// - `iw` is the input width; `min(max_width,iw)` caps width without
///   ever upscaling (a 180 px source stays 180 px). `-2` auto-computes
///   an even height so libavfilter won't reject odd dimensions.
/// - `flags=lanczos` gives a perceptibly sharper downscale than the
///   default `bilinear`, which matters more for GIF than for H.264 since
///   the palette step will then flatten nearby colors together.
/// - The `split`→`palettegen`/`paletteuse` branching is a single pass.
///   A two-pass approach (palette to a temp PNG, then encode) yields
///   marginally better palettes for long clips but complicates the
///   progress plumbing; Phase 1 stays single-pass.
fn build_gif_vf(extra: Option<&str>, p: GifPreset) -> String {
    let prefix = match extra {
        Some(e) if !e.is_empty() => {
            let mut s = String::with_capacity(e.len() + 1);
            s.push_str(e);
            s.push(',');
            s
        }
        _ => String::new(),
    };
    format!(
        "{prefix}fps={fps},scale='min({w}\\,iw)':-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors={c}[p];[s1][p]paletteuse=dither={d}",
        fps = p.fps,
        w = p.max_width,
        c = p.colors,
        d = p.dither,
    )
}

fn max_out_time_ms_from_progress(contents: &str) -> Option<u64> {
    contents
        .lines()
        .filter_map(|line| {
            line.strip_prefix("out_time_ms=")
                .and_then(|s| s.trim().parse::<u64>().ok())
        })
        .max()
}

fn run_ffmpeg(
    mut cmd: Command,
    output: &Path,
    cancel: Option<&AtomicBool>,
    progress: Option<(u64, ExportProgressFn, tempfile::NamedTempFile)>,
) -> Result<(), ExportError> {
    let (total_ms, on_ratio) = match &progress {
        Some((t, f, _)) => (*t, Some(f.clone())),
        None => (0, None),
    };

    let mut child = cmd.spawn().map_err(|e| ExportError {
        message: format!("failed to spawn ffmpeg: {e} (is ffmpeg on PATH? brew install ffmpeg@7)"),
    })?;

    let mut last_pct_byte: u8 = 0;
    let mut last_emit = Instant::now();

    loop {
        if let Some(c) = cancel {
            if c.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExportError {
                    message: EXPORT_CANCELLED_MSG.into(),
                });
            }
        }

        if let (Some((_, _, ref pf)), Some(cb), tm) = (&progress, on_ratio.as_ref(), total_ms) {
            if tm > 0 {
                if let Ok(s) = fs::read_to_string(pf.path()) {
                    if let Some(ot) = max_out_time_ms_from_progress(&s) {
                        let ratio = (ot as f64 / tm as f64).clamp(0.0, 1.0);
                        let pct = (ratio * 100.0) as u8;
                        if pct != last_pct_byte || last_emit.elapsed() > Duration::from_millis(400)
                        {
                            cb(ratio);
                            last_pct_byte = pct;
                            last_emit = Instant::now();
                        }
                    }
                }
            }
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(ExportError {
                        message: format!(
                            "ffmpeg failed with {:?} for export to {:?}",
                            status.code(),
                            output
                        ),
                    });
                }
                if let Some(ref cb) = on_ratio {
                    cb(1.0);
                }
                return Ok(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                return Err(ExportError {
                    message: format!("wait on ffmpeg: {e}"),
                });
            }
        }
    }
}

/// Build the ffmpeg argv for [`WebExportFormat`] (for unit tests; no I/O).
pub fn ffmpeg_args_for_format(format: WebExportFormat) -> Vec<&'static str> {
    match format {
        WebExportFormat::Mp4Remux => vec!["-c", "copy", "-movflags", "+faststart"],
        WebExportFormat::Mp4H264Aac => vec![
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-movflags",
            "+faststart",
        ],
        WebExportFormat::Mp4H265Aac => vec![
            "-c:v",
            "libx265",
            "-preset",
            "medium",
            "-crf",
            "24",
            "-pix_fmt",
            "yuv420p",
            "-tag:v",
            "hvc1",
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-movflags",
            "+faststart",
        ],
        WebExportFormat::WebmVp8Opus => vec![
            "-c:v",
            "libvpx",
            "-quality",
            "good",
            "-cpu-used",
            "4",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
        ],
        WebExportFormat::WebmVp9Opus => vec![
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "0",
            "-crf",
            "32",
            "-row-mt",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
        ],
        WebExportFormat::WebmAv1Opus => vec![
            "-c:v",
            "libaom-av1",
            "-crf",
            "30",
            "-b:v",
            "0",
            "-cpu-used",
            "6",
            "-row-mt",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
        ],
        WebExportFormat::MkvRemux => vec!["-c", "copy"],
        WebExportFormat::MovRemux => vec!["-c", "copy", "-movflags", "+faststart"],
        WebExportFormat::MovProResHq => vec![
            "-c:v",
            "prores_ks",
            "-profile:v",
            "3",
            "-pix_fmt",
            "yuv422p10le",
            "-c:a",
            "pcm_s16le",
        ],
        WebExportFormat::MkvDnxhrHq => vec![
            "-c:v",
            "dnxhd",
            "-profile:v",
            "dnxhr_hq",
            "-pix_fmt",
            "yuv422p",
            "-c:a",
            "pcm_s16le",
        ],
        // The real GIF path builds a full `-vf` chain with palettegen /
        // paletteuse per preset, so it can't be represented as a flat
        // `&'static str` argv. For tests and UI readouts we return just
        // the codec/container/no-audio tail; callers that need the
        // filter graph go through `append_format_args_with_vf`.
        WebExportFormat::GifSharp
        | WebExportFormat::GifGood
        | WebExportFormat::GifShare
        | WebExportFormat::GifTiny => {
            vec!["-an", "-c:v", "gif", "-f", "gif"]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webm_includes_vp8_and_opus() {
        let a = ffmpeg_args_for_format(WebExportFormat::WebmVp8Opus);
        assert!(a.contains(&"libvpx"));
        assert!(a.contains(&"libopus"));
    }

    #[test]
    fn mp4_remux_is_copy() {
        let a = ffmpeg_args_for_format(WebExportFormat::Mp4Remux);
        assert!(a.contains(&"-c"));
        assert!(a.contains(&"copy"));
    }

    #[test]
    fn mp4_h264_aac_transcodes_with_faststart() {
        let a = ffmpeg_args_for_format(WebExportFormat::Mp4H264Aac);
        assert!(a.contains(&"libx264"));
        assert!(a.contains(&"aac"));
        assert!(a.contains(&"yuv420p"));
        assert!(a.contains(&"+faststart"));
        assert!(!a.contains(&"copy"));
    }

    #[test]
    fn mp4_h264_aac_with_vf_still_transcodes() {
        let mut cmd = Command::new("ffmpeg");
        append_format_args_with_vf(&mut cmd, WebExportFormat::Mp4H264Aac, Some("transpose=1"));
        let args = cmd_args_as_strings(&cmd);
        let vf_idx = args.iter().position(|a| a == "-vf").expect("missing -vf");
        assert_eq!(args[vf_idx + 1], "transpose=1");
        assert!(args.iter().any(|a| a == "libx264"));
        assert!(args.iter().any(|a| a == "aac"));
        assert!(!args.iter().any(|a| a == "copy"));
    }

    #[test]
    fn mp4_h264_aac_dual_mux_transcodes_audio_too() {
        let mut cmd = Command::new("ffmpeg");
        append_dual_mux_format_args_with_vf(&mut cmd, WebExportFormat::Mp4H264Aac, None);
        let args = cmd_args_as_strings(&cmd);
        // When the dedicated-audio lane is muxed, the transcode preset must re-encode
        // audio too (input may not be AAC); stream copy is wrong for this preset.
        let c_a = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[c_a + 1], "aac");
        assert!(args.iter().any(|a| a == "libx264"));
    }

    #[test]
    fn mp4_h264_aac_shares_mp4_extension() {
        assert_eq!(WebExportFormat::Mp4H264Aac.file_extension(), "mp4");
    }

    #[test]
    fn progress_parse_takes_max_out_time_ms() {
        let s = "progress=continue\nout_time_ms=100\nout_time_ms=2500\n";
        assert_eq!(super::max_out_time_ms_from_progress(s), Some(2500));
    }

    fn cmd_args_as_strings(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn mp4_remux_without_vf_keeps_stream_copy() {
        let mut cmd = Command::new("ffmpeg");
        append_format_args_with_vf(&mut cmd, WebExportFormat::Mp4Remux, None);
        let args = cmd_args_as_strings(&cmd);
        assert!(!args.iter().any(|a| a == "-vf"));
        assert!(args.iter().any(|a| a == "copy"));
        assert!(!args.iter().any(|a| a == "libx264"));
    }

    #[test]
    fn mp4_remux_with_vf_forces_libx264_transcode() {
        let mut cmd = Command::new("ffmpeg");
        append_format_args_with_vf(&mut cmd, WebExportFormat::Mp4Remux, Some("transpose=1"));
        let args = cmd_args_as_strings(&cmd);
        let vf_idx = args.iter().position(|a| a == "-vf").expect("missing -vf");
        assert_eq!(args[vf_idx + 1], "transpose=1");
        assert!(args.iter().any(|a| a == "libx264"));
        assert!(args.iter().any(|a| a == "yuv420p"));
        assert!(!args.iter().any(|a| a == "copy"));
    }

    #[test]
    fn mkv_remux_with_vf_forces_libx264_transcode() {
        let mut cmd = Command::new("ffmpeg");
        append_format_args_with_vf(&mut cmd, WebExportFormat::MkvRemux, Some("hflip"));
        let args = cmd_args_as_strings(&cmd);
        assert!(args.iter().any(|a| a == "-vf"));
        assert!(args.iter().any(|a| a == "libx264"));
    }

    #[test]
    fn webm_with_vf_stays_libvpx() {
        let mut cmd = Command::new("ffmpeg");
        append_format_args_with_vf(&mut cmd, WebExportFormat::WebmVp8Opus, Some("transpose=2"));
        let args = cmd_args_as_strings(&cmd);
        assert!(args.iter().any(|a| a == "-vf"));
        assert!(args.iter().any(|a| a == "libvpx"));
    }

    #[test]
    fn dual_mux_with_vf_filters_only_video_stream() {
        let mut cmd = Command::new("ffmpeg");
        append_dual_mux_format_args_with_vf(&mut cmd, WebExportFormat::Mp4Remux, Some("vflip"));
        let args = cmd_args_as_strings(&cmd);
        // Must scope the filter to the mapped video stream, not the muxed audio input.
        let idx = args.iter().position(|a| a == "-filter:v:0").unwrap();
        assert_eq!(args[idx + 1], "vflip");
        // Audio still stream-copies in the remux preset.
        let c_a = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[c_a + 1], "copy");
    }

    #[test]
    fn amix_filter_complex_two_lanes_no_vf_concats_inputs_to_aout() {
        let fc = build_amix_filter_complex(2, None);
        assert_eq!(
            fc, "[1:a:0][2:a:0]amix=inputs=2:duration=longest:normalize=0[aout]",
            "amix with N=2 must tag inputs 1 & 2 (not 0, that's the video concat)"
        );
        // Explicit guards against the easy-to-regress pieces:
        assert!(!fc.contains("normalize=1"), "normalize=0 keeps unit gain");
        assert!(fc.contains("duration=longest"));
    }

    #[test]
    fn amix_filter_complex_three_lanes_prepends_all_audio_taps() {
        let fc = build_amix_filter_complex(3, None);
        assert_eq!(
            fc,
            "[1:a:0][2:a:0][3:a:0]amix=inputs=3:duration=longest:normalize=0[aout]"
        );
    }

    #[test]
    fn amix_filter_complex_prepends_video_chain_when_vf_present() {
        let fc = build_amix_filter_complex(2, Some("transpose=1,scale=-2:720"));
        // Video chain must come first, separated by `;`, ending in `[vout]`.
        let (video, audio) = fc.split_once(';').expect("video and audio segments");
        assert_eq!(video, "[0:v:0]transpose=1,scale=-2:720[vout]");
        assert_eq!(
            audio,
            "[1:a:0][2:a:0]amix=inputs=2:duration=longest:normalize=0[aout]"
        );
    }

    #[test]
    fn amix_filter_complex_with_unity_gains_matches_no_gains() {
        // Passing `Some(&[0.0, 0.0])` must emit the exact same graph as `None`
        // — that's the whole reason the "skip non-unity only" optimization
        // exists. Otherwise every vanilla multi-lane export would gratuitously
        // transcode through volume filters that do nothing.
        let without = build_amix_filter_complex_with_gains(2, None, None);
        let with_unity = build_amix_filter_complex_with_gains(2, None, Some(&[0.0, 0.0]));
        assert_eq!(without, with_unity);
    }

    #[test]
    fn amix_filter_complex_injects_volume_filter_for_non_unity_gain() {
        // Lane 0 boosted +6 dB, lane 1 left alone: only lane 0 gets a
        // volume prefilter, lane 1's amix input stays on the raw
        // `[2:a:0]` tap.
        let fc = build_amix_filter_complex_with_gains(2, None, Some(&[6.0, 0.0]));
        assert_eq!(
            fc,
            "[1:a:0]volume=6.00dB[a0];[a0][2:a:0]amix=inputs=2:duration=longest:normalize=0[aout]"
        );
    }

    #[test]
    fn amix_filter_complex_single_lane_gain_uses_amix_passthrough() {
        // N=1 with gain must route through amix (the dispatcher relies on
        // this). `amix=inputs=1` is a ffmpeg passthrough; the actual audio
        // change comes from the `volume=XdB` prefilter ahead of it.
        let fc = build_amix_filter_complex_with_gains(1, None, Some(&[-3.25]));
        assert_eq!(
            fc,
            "[1:a:0]volume=-3.25dB[a0];[a0]amix=inputs=1:duration=longest:normalize=0[aout]"
        );
    }

    #[test]
    fn amix_filter_complex_negative_gain_formats_with_sign_and_two_decimals() {
        // Attenuation (negative dB) is the common case for replace/overlay
        // mixing. Verify the string formatting — `-12.00dB`, not `-12dB` —
        // so ffmpeg's `volume` filter parses unambiguously and logs look
        // consistent across lanes.
        let fc = build_amix_filter_complex_with_gains(3, None, Some(&[0.0, -12.0, 3.5]));
        assert!(fc.contains("[2:a:0]volume=-12.00dB[a1];"), "got {fc}");
        assert!(fc.contains("[3:a:0]volume=3.50dB[a2];"), "got {fc}");
        // Lane 0 has unity gain ⇒ no volume clause for it, amix reads raw tap.
        assert!(
            !fc.contains("[1:a:0]volume"),
            "unity lane must not get a volume filter: {fc}"
        );
        assert!(
            fc.contains("[1:a:0][a1][a2]amix=inputs=3"),
            "amix input list must mix raw taps and relabeled taps per-lane: {fc}"
        );
    }

    #[test]
    fn mixed_audio_format_mp4_remux_forces_aac_even_without_vf() {
        // amix output is always filter-graph PCM; stream-copy would produce an
        // invalid MP4. `Mp4Remux` must still force aac here even though it's the
        // "no transcode" preset in the dual-mux path.
        let mut cmd = Command::new("ffmpeg");
        append_mixed_audio_format_args(&mut cmd, WebExportFormat::Mp4Remux, false);
        let args = cmd_args_as_strings(&cmd);
        let c_a = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(
            args[c_a + 1],
            "aac",
            "amix path can never stream-copy audio"
        );
        // Video can still stream-copy when no vf is active.
        let c_v = args.iter().position(|a| a == "-c:v").unwrap();
        assert_eq!(args[c_v + 1], "copy");
    }

    #[test]
    fn mixed_audio_format_webm_uses_opus() {
        let mut cmd = Command::new("ffmpeg");
        append_mixed_audio_format_args(&mut cmd, WebExportFormat::WebmVp8Opus, false);
        let args = cmd_args_as_strings(&cmd);
        let c_a = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(
            args[c_a + 1],
            "libopus",
            "webm amix must use opus (aac would fail the muxer)"
        );
    }

    #[test]
    fn mixed_audio_format_mp4_remux_vf_present_transcodes_video() {
        let mut cmd = Command::new("ffmpeg");
        append_mixed_audio_format_args(&mut cmd, WebExportFormat::Mp4Remux, true);
        let args = cmd_args_as_strings(&cmd);
        let c_v = args.iter().position(|a| a == "-c:v").unwrap();
        assert_eq!(
            args[c_v + 1],
            "libx264",
            "vf chain forces libx264 transcode"
        );
        let c_a = args.iter().position(|a| a == "-c:a").unwrap();
        assert_eq!(args[c_a + 1], "aac");
    }

    #[test]
    fn orientation_driven_vf_chain_rejects_identity() {
        let id = ClipOrientation::default();
        assert!(id.ffmpeg_filter_chain().is_none());
    }

    #[test]
    fn combined_vf_chain_orders_orientation_then_scale() {
        let mut o = ClipOrientation::default();
        o.rotate_right();
        let mut s = ClipScale::default();
        s.set_percent(50);
        let chain = combined_vf_chain(Some(o), Some(s), None).unwrap();
        let rot = chain.find("transpose=1").unwrap();
        let scl = chain.find("scale=").unwrap();
        assert!(rot < scl, "rotation must precede scale: {chain}");
    }

    #[test]
    fn combined_vf_chain_identity_is_none() {
        assert!(combined_vf_chain(None, None, None).is_none());
        assert!(combined_vf_chain(
            Some(ClipOrientation::default()),
            Some(ClipScale::default()),
            None,
        )
        .is_none());
    }

    #[test]
    fn combined_vf_chain_scale_only() {
        let mut s = ClipScale::default();
        s.set_percent(75);
        let chain = combined_vf_chain(None, Some(s), None).unwrap();
        assert!(chain.starts_with("scale="));
    }

    #[test]
    fn combined_vf_chain_appends_subtitles_last() {
        let mut o = ClipOrientation::default();
        o.rotate_right();
        let mut s = ClipScale::default();
        s.set_percent(50);
        let path = Path::new("/tmp/captions.srt");
        let chain = combined_vf_chain(Some(o), Some(s), Some(path)).unwrap();
        let rot = chain.find("transpose=1").unwrap();
        let scl = chain.find("scale=").unwrap();
        let sub = chain.find("subtitles=").unwrap();
        assert!(rot < scl && scl < sub, "subtitles must be last: {chain}");
    }

    #[test]
    fn combined_vf_chain_subtitles_only() {
        let chain = combined_vf_chain(None, None, Some(Path::new("/tmp/it.srt"))).unwrap();
        assert_eq!(chain, "subtitles='/tmp/it.srt'");
    }

    #[test]
    fn escape_subtitles_escapes_colon_and_quote() {
        // Windows-style drive letters must have `:` escaped; single quotes in
        // the filename must be backslash-escaped so the outer quotes stay paired.
        let out = escape_subtitles_path(Path::new(r"C:\Subs\it's.srt"));
        assert_eq!(out, r"C\:/Subs/it\'s.srt");
    }

    #[test]
    fn build_mute_substitution_lane_swaps_muted_spans_for_silence() {
        let v0 = PathBuf::from("/tmp/a.mp4");
        let v1 = PathBuf::from("/tmp/b.mp4");
        let v2 = PathBuf::from("/tmp/c.mp4");
        let silence = PathBuf::from("/tmp/silence.wav");
        // Clip b is muted, a and c keep their embedded audio.
        let spans = vec![
            (v0.clone(), 0.0, 2.0),
            (v1.clone(), 1.0, 3.5),
            (v2.clone(), 0.5, 1.0),
        ];
        let mask = vec![false, true, false];
        let out = build_mute_substitution_lane(&spans, &mask, &silence);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], (v0, 0.0, 2.0));
        // Muted span ⇒ silence file, 0..span_duration (2.5s).
        assert_eq!(out[1].0, silence);
        assert!((out[1].1 - 0.0).abs() < 1e-9);
        assert!(
            (out[1].2 - 2.5).abs() < 1e-9,
            "silence duration must match muted span duration"
        );
        assert_eq!(out[2], (v2, 0.5, 1.0));
    }

    #[test]
    fn build_mute_substitution_lane_mask_length_mismatch_returns_empty() {
        let spans = vec![(PathBuf::from("/tmp/a.mp4"), 0.0, 1.0)];
        let mask = vec![false, true];
        let out = build_mute_substitution_lane(&spans, &mask, Path::new("/tmp/s.wav"));
        assert!(
            out.is_empty(),
            "length mismatch must fail closed — caller falls back to video-only"
        );
    }

    #[test]
    fn build_mute_substitution_lane_all_muted_emits_all_silence() {
        let v = PathBuf::from("/tmp/a.mp4");
        let silence = PathBuf::from("/tmp/s.wav");
        let spans = vec![(v.clone(), 0.0, 1.5), (v.clone(), 2.0, 2.75)];
        let mask = vec![true, true];
        let out = build_mute_substitution_lane(&spans, &mask, &silence);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(p, _, _)| p == &silence));
        assert!((out[0].2 - 1.5).abs() < 1e-9);
        assert!((out[1].2 - 0.75).abs() < 1e-9);
    }

    #[test]
    fn generate_silence_wav_rejects_non_positive_duration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("s.wav");
        assert!(generate_silence_wav(&out, 0.0).is_err());
        assert!(generate_silence_wav(&out, -1.0).is_err());
        assert!(generate_silence_wav(&out, f64::NAN).is_err());
    }

    #[test]
    fn subtitle_burn_in_forces_transcode_on_remux() {
        let mut cmd = Command::new("ffmpeg");
        let chain = combined_vf_chain(None, None, Some(Path::new("/tmp/a.srt"))).unwrap();
        append_format_args_with_vf(&mut cmd, WebExportFormat::Mp4Remux, Some(&chain));
        let args = cmd_args_as_strings(&cmd);
        assert!(
            args.iter().any(|a| a.contains("subtitles=")),
            "missing subtitles=: {args:?}"
        );
        // A `-c copy` video preset cannot apply a filter — the remux branch with
        // a non-empty `-vf` must swap to a libx264 transcode.
        assert!(args.iter().any(|a| a == "libx264"));
        assert!(!args.iter().any(|a| a == "copy"));
    }

    // --- GIF export preset tests ---------------------------------------------------
    //
    // Phase 1 GIF export relies on three connected pieces: the `GifPreset`
    // constants, the `build_gif_vf` palettegen filter-graph builder, and the
    // "route GIF to the audio-less single-input path" shortcut baked into
    // `append_format_args_with_vf` / `is_gif()`. These tests pin each piece so
    // regressions (preset drift, missing `-an`, wrong codec, un-escaped commas in
    // the scale expression) fail loudly at `cargo test`.

    #[test]
    fn gif_formats_all_use_gif_extension() {
        for f in [
            WebExportFormat::GifSharp,
            WebExportFormat::GifGood,
            WebExportFormat::GifShare,
            WebExportFormat::GifTiny,
        ] {
            assert_eq!(f.file_extension(), "gif", "wrong ext for {f:?}");
            assert!(f.is_gif(), "is_gif should be true for {f:?}");
            assert!(f.gif_preset().is_some(), "gif_preset missing for {f:?}");
        }
    }

    #[test]
    fn non_gif_formats_report_no_gif_preset() {
        // Every non-GIF preset must return `None` — otherwise the early-return
        // in `append_format_args_with_vf` would hijack the normal audio path.
        for f in [
            WebExportFormat::Mp4Remux,
            WebExportFormat::Mp4H264Aac,
            WebExportFormat::Mp4H265Aac,
            WebExportFormat::WebmVp8Opus,
            WebExportFormat::WebmVp9Opus,
            WebExportFormat::WebmAv1Opus,
            WebExportFormat::MkvRemux,
            WebExportFormat::MovRemux,
            WebExportFormat::MovProResHq,
            WebExportFormat::MkvDnxhrHq,
        ] {
            assert!(!f.is_gif(), "{f:?} must not be GIF");
            assert!(f.gif_preset().is_none(), "{f:?} must have no preset");
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn gif_preset_constants_descend_in_size() {
        // Sharp > Good > Share > Tiny on both resolution and color budget.
        // Caught a Phase 1 drafting bug where TINY had more colors than SHARE.
        assert!(GifPreset::SHARP.max_width >= GifPreset::GOOD.max_width);
        assert!(GifPreset::GOOD.max_width >= GifPreset::SHARE.max_width);
        assert!(GifPreset::SHARE.max_width >= GifPreset::TINY.max_width);

        assert!(GifPreset::SHARP.colors >= GifPreset::GOOD.colors);
        assert!(GifPreset::GOOD.colors >= GifPreset::SHARE.colors);
        assert!(GifPreset::SHARE.colors >= GifPreset::TINY.colors);

        assert!(GifPreset::SHARP.fps >= GifPreset::TINY.fps);
        // Palette budget is honest: no preset exceeds the GIF spec maximum.
        for p in [
            GifPreset::SHARP,
            GifPreset::GOOD,
            GifPreset::SHARE,
            GifPreset::TINY,
        ] {
            assert!(p.colors <= 256, "GIF palette caps at 256 colors");
            assert!(p.fps > 0 && p.max_width > 0);
        }
    }

    #[test]
    fn gif_ffmpeg_args_strip_audio_and_select_gif_codec() {
        let args = ffmpeg_args_for_format(WebExportFormat::GifShare);
        assert!(
            args.contains(&"-an"),
            "GIF export must strip audio: {args:?}"
        );
        let c_v = args
            .iter()
            .position(|a| *a == "-c:v")
            .expect("missing -c:v");
        assert_eq!(args[c_v + 1], "gif");
        let f = args.iter().position(|a| *a == "-f").expect("missing -f");
        assert_eq!(args[f + 1], "gif");
        // No audio codec is emitted — `-an` fully drops the audio stream.
        assert!(!args.iter().any(|a| *a == "-c:a"));
    }

    #[test]
    fn build_gif_vf_emits_palette_graph_with_escaped_scale_comma() {
        // The scale expression uses `min(W,iw)`. The comma inside the filter
        // expression must be backslash-escaped, otherwise ffmpeg's top-level
        // filter parser splits the chain mid-expression and reports
        // "No such filter: iw)". Regression guard.
        let chain = build_gif_vf(None, GifPreset::SHARE);
        assert!(chain.contains("fps=15"), "{chain}");
        assert!(
            chain.contains("scale='min(480\\,iw)':-2:flags=lanczos"),
            "{chain}"
        );
        assert!(chain.contains("split[s0][s1]"), "{chain}");
        assert!(chain.contains("palettegen=max_colors=96"), "{chain}");
        assert!(
            chain.contains("[s1][p]paletteuse=dither=bayer:bayer_scale=4"),
            "{chain}"
        );
    }

    #[test]
    fn build_gif_vf_prepends_extra_chain_with_comma() {
        // When the caller has their own vf prefix (e.g. rotate for portrait
        // autorotation), it must land BEFORE the palette graph and be joined
        // with a comma — never a semicolon (that would start a new label chain
        // and orphan the downstream palettegen inputs).
        let chain = build_gif_vf(Some("transpose=1"), GifPreset::TINY);
        assert!(chain.starts_with("transpose=1,fps=12,"), "{chain}");
        assert!(chain.contains("scale='min(320\\,iw)'"), "{chain}");
    }

    #[test]
    fn build_gif_vf_empty_extra_is_noop_prefix() {
        // `Some("")` must behave like `None` — no stray leading comma that
        // would make ffmpeg interpret an empty filter name.
        let a = build_gif_vf(None, GifPreset::GOOD);
        let b = build_gif_vf(Some(""), GifPreset::GOOD);
        assert_eq!(a, b);
        assert!(!a.starts_with(','), "no leading comma: {a}");
    }

    #[test]
    fn gif_append_format_args_emits_vf_with_palette_graph() {
        // End-to-end: `append_format_args_with_vf` for any GIF preset must
        // (1) emit a single `-vf` arg containing the palette graph,
        // (2) strip audio with `-an`, (3) pick `-c:v gif`, and (4) force
        // `-f gif` regardless of the output path extension.
        let mut cmd = Command::new("ffmpeg");
        append_format_args_with_vf(&mut cmd, WebExportFormat::GifSharp, None);
        let args = cmd_args_as_strings(&cmd);
        let vf_idx = args.iter().position(|a| a == "-vf").expect("missing -vf");
        let vf = &args[vf_idx + 1];
        assert!(vf.contains("palettegen=max_colors=256"), "{vf}");
        assert!(vf.contains("fps=24"), "{vf}");
        assert!(args.iter().any(|a| a == "-an"));
        let c_v = args.iter().position(|a| a == "-c:v").unwrap();
        assert_eq!(args[c_v + 1], "gif");
        let f = args.iter().position(|a| a == "-f").unwrap();
        assert_eq!(args[f + 1], "gif");
    }

    #[test]
    fn gif_append_format_args_preserves_extra_vf_prefix() {
        let mut cmd = Command::new("ffmpeg");
        append_format_args_with_vf(&mut cmd, WebExportFormat::GifTiny, Some("transpose=2"));
        let args = cmd_args_as_strings(&cmd);
        let vf_idx = args.iter().position(|a| a == "-vf").expect("missing -vf");
        let vf = &args[vf_idx + 1];
        assert!(vf.starts_with("transpose=2,"), "rotation prefix lost: {vf}");
        assert!(vf.contains("palettegen=max_colors=64"), "{vf}");
    }
}
