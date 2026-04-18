//! Minimal SubRip (`.srt`) + WebVTT (`.vtt`) parser used by **File → Insert Subtitle…**.
//!
//! Scope: extract the **end time of the last cue** so a subtitle file can be
//! modelled as a [`crate::project::Clip`] with `in_point = 0.0` and
//! `out_point = duration`. We do not yet decode cue text for preview; burn-in
//! at export delegates to ffmpeg's `subtitles=` filter, which reads the file
//! directly (and auto-detects SRT vs WebVTT by extension).
//!
//! The **SRT** and **WebVTT** grammars overlap enough to share one parser:
//!
//! - SRT timestamps are `HH:MM:SS,mmm`; WebVTT uses `.` for ms and permits
//!   `MM:SS.mmm` when hours are zero.
//! - WebVTT appends optional cue settings after the end timestamp
//!   (e.g. `00:00:01.000 --> 00:00:02.000 align:center`); we strip anything
//!   past the first whitespace.
//! - WebVTT files begin with a `WEBVTT` header block and may include `NOTE`
//!   or `STYLE` blocks. They have no timing line, so [`parse_block`] rejects
//!   them and they are silently dropped — same policy as hand-edited SRT with
//!   malformed cues.
//!
//! Format (spec-ish):
//!
//! ```text
//! 1
//! 00:00:01,000 --> 00:00:03,500
//! Hello
//!
//! 2
//! 00:00:04,000 --> 00:00:07,250
//! World
//! ```
//!
//! Separators: `,` (spec) or `.` (WebVTT-style, widely tolerated).

use std::fs;
use std::path::Path;

/// A single cue `start → end` in seconds, plus the raw text lines.
#[derive(Debug, Clone, PartialEq)]
pub struct SrtCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Probe of an `.srt` file — matches the shape we need to build a `Clip`
/// (`duration_seconds` == last cue's end time).
#[derive(Debug, Clone, PartialEq)]
pub struct SrtProbe {
    pub cue_count: usize,
    pub duration_seconds: f64,
}

/// Parse an `.srt` file on disk into its cues. Returns an error for I/O
/// failures or completely unparseable files; tolerates trailing whitespace,
/// blank-line gaps, and `.`/`,` millisecond separators.
pub fn parse_file(path: &Path) -> std::io::Result<Vec<SrtCue>> {
    let body = fs::read_to_string(path)?;
    Ok(parse_str(&body))
}

/// Probe-only form: parse and return `(cue_count, duration_seconds)` where
/// `duration_seconds` is the maximum end time across all cues (0.0 for an
/// empty file).
pub fn probe_file(path: &Path) -> std::io::Result<SrtProbe> {
    let cues = parse_file(path)?;
    let duration_seconds = cues.iter().map(|c| c.end).fold(0.0_f64, f64::max);
    Ok(SrtProbe {
        cue_count: cues.len(),
        duration_seconds,
    })
}

/// Parse an in-memory SRT body. Ignores malformed blocks rather than
/// failing — matches how ffmpeg's `subtitles=` filter behaves and keeps
/// **Insert Subtitle…** tolerant of hand-edited files.
pub fn parse_str(body: &str) -> Vec<SrtCue> {
    let mut out = Vec::new();
    // SRT uses blank lines as block separators. Split on *one or more* blank lines
    // so \r\n and stray whitespace don't confuse the parser.
    for block in body.split("\n\n").flat_map(|b| b.split("\r\n\r\n")) {
        if let Some(cue) = parse_block(block.trim()) {
            out.push(cue);
        }
    }
    out
}

fn parse_block(block: &str) -> Option<SrtCue> {
    let mut lines = block.lines();
    let first = lines.next()?.trim();
    // Skip the numeric index line if present. Some writers omit it.
    let timing_line = if first.contains("-->") {
        first.to_string()
    } else {
        lines.next()?.trim().to_string()
    };
    let (start, end) = parse_timing(&timing_line)?;
    let text = lines.collect::<Vec<_>>().join("\n");
    Some(SrtCue { start, end, text })
}

fn parse_timing(line: &str) -> Option<(f64, f64)> {
    let (a, b) = line.split_once("-->")?;
    Some((parse_timestamp(a.trim())?, parse_timestamp(b.trim())?))
}

fn parse_timestamp(s: &str) -> Option<f64> {
    // Strip trailing cue settings (WebVTT), then accept both `HH:MM:SS(.|,)mmm`
    // (SRT and WebVTT long form) and `MM:SS.mmm` (WebVTT short form, hours = 0).
    let tok = s.split_whitespace().next()?.replace(',', ".");
    let parts: Vec<&str> = tok.split(':').collect();
    let (h, m, sec) = match parts.len() {
        3 => (
            parts[0].parse::<f64>().ok()?,
            parts[1].parse::<f64>().ok()?,
            parts[2].parse::<f64>().ok()?,
        ),
        2 => (
            0.0_f64,
            parts[0].parse::<f64>().ok()?,
            parts[1].parse::<f64>().ok()?,
        ),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_CUES: &str = "1\n\
00:00:01,000 --> 00:00:03,500\n\
Hello\n\
\n\
2\n\
00:00:04,000 --> 00:00:07,250\n\
World\n";

    #[test]
    fn parses_two_cues_and_reports_last_end_as_duration() {
        let cues = parse_str(TWO_CUES);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start, 1.0);
        assert_eq!(cues[0].end, 3.5);
        assert_eq!(cues[0].text, "Hello");
        assert_eq!(cues[1].end, 7.25);
    }

    #[test]
    fn tolerates_dot_millisecond_separator() {
        let body =
            "1\n00:00:00.500 --> 00:00:01.750\nHi\n";
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].end, 1.75);
    }

    #[test]
    fn skips_malformed_block_without_killing_the_rest() {
        let body = "garbage-no-timing\n\n1\n00:00:02,000 --> 00:00:03,000\nOK\n";
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "OK");
    }

    #[test]
    fn empty_body_parses_to_zero_cues() {
        assert!(parse_str("").is_empty());
        assert!(parse_str("\n\n\n").is_empty());
    }

    #[test]
    fn probe_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.srt");
        std::fs::write(&path, TWO_CUES).unwrap();
        let pr = probe_file(&path).unwrap();
        assert_eq!(pr.cue_count, 2);
        assert!((pr.duration_seconds - 7.25).abs() < 1e-9);
    }

    #[test]
    fn cue_without_index_line_still_parses() {
        let body = "00:00:00,000 --> 00:00:02,000\nno-index\n";
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "no-index");
    }

    #[test]
    fn webvtt_with_header_and_cue_settings() {
        // Realistic WebVTT: header block, cue id (non-numeric), dot-ms timestamps,
        // cue settings on the timing line, and a NOTE block — all must be tolerated.
        let body = "WEBVTT\n\n\
                    NOTE hand-authored\n\n\
                    intro\n\
                    00:00:01.000 --> 00:00:02.500 align:center line:80%\n\
                    Hello\n\n\
                    00:00:03.000 --> 00:00:04.000\n\
                    World\n";
        let cues = parse_str(body);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start, 1.0);
        assert_eq!(cues[0].end, 2.5);
        assert_eq!(cues[0].text, "Hello");
        assert_eq!(cues[1].end, 4.0);
    }

    #[test]
    fn webvtt_short_form_timestamp_without_hours() {
        // WebVTT permits MM:SS.mmm when hours are zero.
        let body = "WEBVTT\n\n01:02.500 --> 01:05.000\nShort form\n";
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert!((cues[0].start - 62.5).abs() < 1e-9);
        assert!((cues[0].end - 65.0).abs() < 1e-9);
    }

    #[test]
    fn probe_file_handles_vtt_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.vtt");
        std::fs::write(
            &path,
            "WEBVTT\n\n00:00:00.500 --> 00:00:01.750\nHi\n",
        )
        .unwrap();
        let pr = probe_file(&path).unwrap();
        assert_eq!(pr.cue_count, 1);
        assert!((pr.duration_seconds - 1.75).abs() < 1e-9);
    }
}
