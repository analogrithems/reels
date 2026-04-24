//! Media integrity scan — decodes every packet in a file and reports any
//! errors the demuxer or decoders emit along the way. Surfaces as
//! **Edit → Scan for Errors** in the desktop app.
//!
//! This is the same class of check as running
//! `ffmpeg -xerror -i <file> -f null -` from the command line: open the
//! container, iterate every packet to end-of-file, push each packet through
//! the matching decoder, and drain frames until the decoder returns `EOF`.
//! Any error returned by the demuxer, `send_packet`, or `receive_frame` is
//! captured; non-monotonic / negative PTS values are also recorded because
//! they correlate with the playback stalls we see on some mobile MOV files.
//!
//! The report is intentionally structured (counts + a bounded sample of
//! human-readable messages) so the UI can render a "No issues found" /
//! "N decode errors — repair recommended" summary without scrolling through
//! every stderr line.

use std::path::{Path, PathBuf};
use std::sync::Once;

use ffmpeg::media::Type as MediaType;
use ffmpeg_next as ffmpeg;

static FFMPEG_INIT: Once = Once::new();

fn ensure_ffmpeg_init() {
    FFMPEG_INIT.call_once(|| {
        let _ = ffmpeg::init();
    });
}

/// Overall verdict for a scanned file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanSeverity {
    /// No demux or decode errors encountered.
    Ok,
    /// Soft problems only (non-monotonic PTS, unusual stream metadata) —
    /// the file will play but may misbehave in edge cases. Repair is
    /// *optional*.
    Warn,
    /// Hard decode errors — frames will be skipped during playback / export.
    /// Repair is *recommended* (e.g. `ffmpeg -i in.mov -c copy out.mov` to
    /// rewrite the moov atom, or a full re-encode if the bitstream is
    /// damaged).
    Error,
}

impl ScanSeverity {
    /// Lower-case stable label for UI / logging (`"ok"`, `"warn"`, `"error"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// A single issue surfaced by the scan. `kind` is a short machine-readable
/// category; `message` is the human-readable detail for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanIssue {
    pub stream_index: Option<u32>,
    pub kind: String,
    pub message: String,
}

/// Structured output of [`scan_file`]. `verdict` is driven by `error_count`
/// (any → `Error`) then `warning_count` (any → `Warn`) then `Ok`.
#[derive(Debug, Clone)]
pub struct ScanReport {
    pub path: PathBuf,
    pub verdict: ScanSeverity,
    pub duration_seconds: f64,
    pub video_frames_decoded: u64,
    pub audio_frames_decoded: u64,
    pub packets_read: u64,
    pub error_count: u32,
    pub warning_count: u32,
    /// First N issues captured. Capped at [`ScanReport::MAX_ISSUES`] so a
    /// wildly broken file doesn't balloon memory or flood the UI.
    pub issues: Vec<ScanIssue>,
}

impl ScanReport {
    /// Upper bound on stored issue messages. Beyond this the scan keeps
    /// counting (`error_count` / `warning_count`) but stops appending new
    /// entries to `issues`.
    pub const MAX_ISSUES: usize = 32;

    /// `true` when the verdict justifies nudging the user toward a rewrite.
    pub fn repair_recommended(&self) -> bool {
        matches!(self.verdict, ScanSeverity::Error)
    }

    /// One-line headline for the results sheet.
    pub fn headline(&self) -> String {
        match self.verdict {
            ScanSeverity::Ok => "No issues found — file plays cleanly end-to-end.".to_string(),
            ScanSeverity::Warn => format!(
                "{} warning{} — file should play but has minor anomalies.",
                self.warning_count,
                if self.warning_count == 1 { "" } else { "s" }
            ),
            ScanSeverity::Error => {
                let errs = format!(
                    "{} decode error{}",
                    self.error_count,
                    if self.error_count == 1 { "" } else { "s" }
                );
                let warns = if self.warning_count > 0 {
                    format!(
                        " (+ {} warning{})",
                        self.warning_count,
                        if self.warning_count == 1 { "" } else { "s" }
                    )
                } else {
                    String::new()
                };
                format!("{errs}{warns} — repair recommended.")
            }
        }
    }
}

struct Stream {
    stream_index: u32,
    kind: MediaType,
    decoder: Option<ffmpeg::codec::decoder::Opened>,
    last_pts: Option<i64>,
    frames: u64,
    nonmonotonic_logged: bool,
}

fn push_issue(issues: &mut Vec<ScanIssue>, stream_index: Option<u32>, kind: &str, message: String) {
    if issues.len() < ScanReport::MAX_ISSUES {
        issues.push(ScanIssue {
            stream_index,
            kind: kind.to_string(),
            message,
        });
    }
}

fn drain(s: &mut Stream, issues: &mut Vec<ScanIssue>, error_count: &mut u32) {
    let Some(decoder) = s.decoder.as_mut() else {
        return;
    };
    loop {
        let mut frame = ffmpeg::util::frame::Video::empty();
        match decoder.receive_frame(&mut frame) {
            Ok(()) => {
                s.frames += 1;
            }
            Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::util::error::EAGAIN => {
                return;
            }
            Err(ffmpeg::Error::Eof) => return,
            Err(e) => {
                *error_count += 1;
                push_issue(
                    issues,
                    Some(s.stream_index),
                    "receive-frame",
                    format!("stream {}: {e}", s.stream_index),
                );
                return;
            }
        }
    }
}

/// Scan `path` for demux / decode errors. See module docs for semantics.
///
/// `on_progress` is called with a `0.0..=1.0` completion ratio (best effort,
/// based on demuxer PTS vs. container duration) roughly every 2% of the
/// file so a UI can show progress without flooding the channel. Pass a
/// no-op closure if progress isn't needed.
pub fn scan_file<F>(path: &Path, mut on_progress: F) -> std::io::Result<ScanReport>
where
    F: FnMut(f64),
{
    ensure_ffmpeg_init();

    if !path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("file not found: {}", path.display()),
        ));
    }

    let mut input = ffmpeg::format::input(&path).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("ffmpeg could not open {}: {e}", path.display()),
        )
    })?;

    let duration_seconds = {
        let d = input.duration();
        if d <= 0 {
            0.0
        } else {
            d as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
        }
    };

    let mut streams: Vec<Stream> = Vec::new();
    let mut issues: Vec<ScanIssue> = Vec::new();
    let mut error_count: u32 = 0;
    let mut warning_count: u32 = 0;

    for stream in input.streams() {
        let medium = stream.parameters().medium();
        if !matches!(medium, MediaType::Video | MediaType::Audio) {
            continue;
        }
        let stream_index = stream.index() as u32;
        // Open the matching decoder. For Video/Audio we have to go through
        // `.video()` / `.audio()` rather than a bare `.open()` — the latter
        // fails with "No codec provided to avcodec_open2()" because the
        // `Decoder` hasn't been bound to a concrete codec yet.
        let opened: Result<ffmpeg::codec::decoder::Opened, ffmpeg::Error> =
            match ffmpeg::codec::context::Context::from_parameters(stream.parameters()) {
                Ok(ctx) => match medium {
                    MediaType::Video => ctx.decoder().video().map(|v| v.0),
                    MediaType::Audio => ctx.decoder().audio().map(|a| a.0),
                    _ => unreachable!(),
                },
                Err(e) => Err(e),
            };
        match opened {
            Ok(dec) => {
                streams.push(Stream {
                    stream_index,
                    kind: medium,
                    decoder: Some(dec),
                    last_pts: None,
                    frames: 0,
                    nonmonotonic_logged: false,
                });
            }
            Err(e) => {
                error_count += 1;
                push_issue(
                    &mut issues,
                    Some(stream_index),
                    "decoder-open",
                    format!("failed to open decoder for stream {stream_index}: {e}"),
                );
            }
        }
    }

    // No decodable A/V streams means there's nothing meaningful we can check.
    if streams.is_empty() {
        let verdict = if error_count > 0 {
            ScanSeverity::Error
        } else {
            ScanSeverity::Warn
        };
        let warning_count = if error_count == 0 { 1 } else { warning_count };
        let final_issues = if error_count == 0 && issues.is_empty() {
            vec![ScanIssue {
                stream_index: None,
                kind: "no-decodable-streams".into(),
                message: "no decodable audio or video streams found".into(),
            }]
        } else {
            issues
        };
        return Ok(ScanReport {
            path: path.to_path_buf(),
            verdict,
            duration_seconds,
            video_frames_decoded: 0,
            audio_frames_decoded: 0,
            packets_read: 0,
            error_count,
            warning_count,
            issues: final_issues,
        });
    }

    let mut packets_read: u64 = 0;
    // Progress is the max seen PTS ratio across streams, not per-stream, so
    // interleaved audio/video packets don't make the bar jitter backwards.
    let mut last_progress_report: f64 = 0.0;
    let mut best_progress: f64 = 0.0;

    for (stream, packet) in input.packets() {
        packets_read += 1;
        let s_index = stream.index() as u32;

        // Best-effort progress: compare packet PTS to container duration.
        if duration_seconds > 0.0 {
            if let Some(pts) = packet.pts() {
                let tb = stream.time_base();
                let den = f64::from(tb.denominator());
                if den > 0.0 {
                    let t = pts as f64 * f64::from(tb.numerator()) / den;
                    let ratio = (t / duration_seconds).clamp(0.0, 1.0);
                    if ratio > best_progress {
                        best_progress = ratio;
                        if best_progress - last_progress_report > 0.02 {
                            on_progress(best_progress);
                            last_progress_report = best_progress;
                        }
                    }
                }
            }
        }

        let Some(s) = streams.iter_mut().find(|s| s.stream_index == s_index) else {
            continue; // packet belongs to a stream we aren't tracking
        };

        // Record non-monotonic PTS once per stream (log spam mitigation).
        if let Some(pts) = packet.pts() {
            if let Some(last) = s.last_pts {
                if pts < last && !s.nonmonotonic_logged {
                    warning_count += 1;
                    s.nonmonotonic_logged = true;
                    push_issue(
                        &mut issues,
                        Some(s.stream_index),
                        "nonmonotonic-pts",
                        format!(
                            "stream {}: packet PTS went backwards ({pts} after {last}); \
                             often seen on MOV files with edit lists — can correlate with playback stalls",
                            s.stream_index
                        ),
                    );
                }
            }
            s.last_pts = Some(pts);
        }

        if let Some(decoder) = s.decoder.as_mut() {
            if let Err(e) = decoder.send_packet(&packet) {
                error_count += 1;
                push_issue(
                    &mut issues,
                    Some(s.stream_index),
                    "send-packet",
                    format!("stream {}: {e}", s.stream_index),
                );
                continue;
            }
        }

        drain(s, &mut issues, &mut error_count);
    }

    // End-of-stream: flush every decoder by sending EOF.
    for s in streams.iter_mut() {
        if let Some(decoder) = s.decoder.as_mut() {
            if let Err(e) = decoder.send_eof() {
                error_count += 1;
                push_issue(
                    &mut issues,
                    Some(s.stream_index),
                    "send-eof",
                    format!("stream {}: {e}", s.stream_index),
                );
                continue;
            }
        }
        drain(s, &mut issues, &mut error_count);
    }

    on_progress(1.0);

    let mut video_frames = 0u64;
    let mut audio_frames = 0u64;
    for s in &streams {
        match s.kind {
            MediaType::Video => video_frames += s.frames,
            MediaType::Audio => audio_frames += s.frames,
            _ => {}
        }
    }

    let verdict = if error_count > 0 {
        ScanSeverity::Error
    } else if warning_count > 0 {
        ScanSeverity::Warn
    } else {
        ScanSeverity::Ok
    };

    Ok(ScanReport {
        path: path.to_path_buf(),
        verdict,
        duration_seconds,
        video_frames_decoded: video_frames,
        audio_frames_decoded: audio_frames,
        packets_read,
        error_count,
        warning_count,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_as_str_is_stable() {
        assert_eq!(ScanSeverity::Ok.as_str(), "ok");
        assert_eq!(ScanSeverity::Warn.as_str(), "warn");
        assert_eq!(ScanSeverity::Error.as_str(), "error");
    }

    #[test]
    fn headline_words_match_verdict() {
        let mut r = ScanReport {
            path: PathBuf::from("/x"),
            verdict: ScanSeverity::Ok,
            duration_seconds: 0.0,
            video_frames_decoded: 0,
            audio_frames_decoded: 0,
            packets_read: 0,
            error_count: 0,
            warning_count: 0,
            issues: vec![],
        };
        assert!(r.headline().contains("No issues"));
        r.verdict = ScanSeverity::Warn;
        r.warning_count = 1;
        assert!(r.headline().contains("1 warning"));
        r.verdict = ScanSeverity::Error;
        r.error_count = 3;
        assert!(r.headline().contains("3 decode errors"));
        assert!(r.headline().contains("repair recommended"));
    }

    #[test]
    fn repair_only_for_hard_errors() {
        let mut r = ScanReport {
            path: PathBuf::from("/x"),
            verdict: ScanSeverity::Ok,
            duration_seconds: 0.0,
            video_frames_decoded: 0,
            audio_frames_decoded: 0,
            packets_read: 0,
            error_count: 0,
            warning_count: 0,
            issues: vec![],
        };
        assert!(!r.repair_recommended());
        r.verdict = ScanSeverity::Warn;
        assert!(!r.repair_recommended());
        r.verdict = ScanSeverity::Error;
        assert!(r.repair_recommended());
    }

    #[test]
    fn scan_missing_file_errors_out() {
        let err = scan_file(Path::new("/tmp/does-not-exist.mov"), |_| {}).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
