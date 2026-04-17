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

/// Web-friendly outputs we guarantee in tests (see `tests/export_web_formats.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebExportFormat {
    /// MP4 container; stream copy when codecs allow (fast).
    Mp4Remux,
    /// WebM VP8 + Opus (faster to encode than VP9 for short fixtures).
    WebmVp8Opus,
    /// Matroska; stream copy for quick container swap tests.
    MkvRemux,
}

impl WebExportFormat {
    pub const ALL: [WebExportFormat; 3] = [
        WebExportFormat::Mp4Remux,
        WebExportFormat::WebmVp8Opus,
        WebExportFormat::MkvRemux,
    ];

    pub fn file_extension(self) -> &'static str {
        match self {
            WebExportFormat::Mp4Remux => "mp4",
            WebExportFormat::WebmVp8Opus => "webm",
            WebExportFormat::MkvRemux => "mkv",
        }
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
        append_format_args(&mut cmd, format);
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
    append_format_args(&mut cmd, format);
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
    let Some(audio_segments) = audio_segments.filter(|a| !a.is_empty()) else {
        return export_concat_timeline(video_segments, output, format, cancel, on_ratio);
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
    append_dual_mux_format_args(&mut cmd, format);
    cmd.arg(output);
    run_ffmpeg(cmd, output, cancel, progress_pack)
}

fn append_dual_mux_format_args(cmd: &mut Command, format: WebExportFormat) {
    match format {
        WebExportFormat::Mp4Remux => {
            cmd.args(["-c:v", "copy", "-c:a", "copy", "-movflags", "+faststart"]);
        }
        WebExportFormat::WebmVp8Opus => {
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
        WebExportFormat::MkvRemux => {
            cmd.args(["-c:v", "copy", "-c:a", "copy"]);
        }
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
    match format {
        WebExportFormat::Mp4Remux => {
            cmd.args(["-c", "copy", "-movflags", "+faststart"]);
        }
        WebExportFormat::WebmVp8Opus => {
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
        WebExportFormat::MkvRemux => {
            cmd.args(["-c", "copy"]);
        }
    }
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
        WebExportFormat::MkvRemux => vec!["-c", "copy"],
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
    fn progress_parse_takes_max_out_time_ms() {
        let s = "progress=continue\nout_time_ms=100\nout_time_ms=2500\n";
        assert_eq!(super::max_out_time_ms_from_progress(s), Some(2500));
    }
}
