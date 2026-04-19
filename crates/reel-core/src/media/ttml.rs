//! Minimal TTML (`.ttml` / `.xml` / `.dfxp`) parser used by **File → Insert Subtitle…**.
//!
//! Scope matches [`super::srt`]: extract enough of each `<p>` cue to build a
//! [`super::srt::SrtCue`] (so the preview overlay cache and the export
//! `subtitles=` filter can share one cue type across SRT / WebVTT / TTML).
//! We do **not** carry TTML styling (region, color, tts:* attrs) through to
//! preview — the overlay renders plain text on the standard caption strip.
//!
//! Supported syntax (intentionally narrow):
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <tt xmlns="http://www.w3.org/ns/ttml">
//!   <body>
//!     <div>
//!       <p begin="00:00:01.000" end="00:00:03.500">Hello</p>
//!       <p begin="4s" dur="3.25s">World</p>
//!       <p begin="00:00:08.000" end="00:00:10.000">Line one<br/>line two</p>
//!     </div>
//!   </body>
//! </tt>
//! ```
//!
//! **Timestamp forms** (per TTML 1.0 §10.3.1):
//!
//! - **Clock-time:** `HH:MM:SS[.fff]` — the common case from authoring tools.
//! - **Offset-time:** `<number><metric>` where metric ∈ `h|m|s|ms|f|t` —
//!   we accept `h`, `m`, `s`, `ms`; `f` (frames) and `t` (ticks) require a
//!   frame-rate / tick-rate attribute we don't read, so they parse as the
//!   raw number of seconds (best-effort) with a `warn!` so the session log
//!   surfaces files we can't fully honour.
//!
//! **End vs duration:** TTML permits `end` or `dur` on a `<p>`. We honour
//! whichever is present (prefer `end` when both appear — matches libass
//! behaviour and avoids double-counting).
//!
//! **Nesting and inline markup:** we flatten `<p>` inner content to text,
//! replacing `<br/>` with `\n` and stripping any other tags (e.g. `<span>`,
//! `<tt:style>`). Numeric character references (`&#10;`, `&amp;`) are decoded
//! via [`unescape_entities`]. This keeps the preview overlay legible without
//! pulling in a full XML stack.
//!
//! Tolerant of:
//!
//! - XML comments (stripped).
//! - Processing instructions (`<?xml ...?>`, stripped).
//! - Namespace prefixes on tag names (`<tt:p ...>` — we match on the local
//!   name, not the qualified name).
//! - Self-closing `<p/>` (no cue — skipped).
//! - Unknown attributes on `<p>` (ignored).
//!
//! Intolerant of:
//!
//! - Malformed XML where a `<p>` never closes — the parse just stops at the
//!   last recoverable cue and returns what it already collected. A file with
//!   zero recoverable cues returns an empty vec, matching SRT behaviour.

use std::fs;
use std::path::Path;

use super::srt::{SrtCue, SrtProbe};

/// Parse a TTML file on disk into its cues. Returns an `io::Error` only for
/// I/O failures; a malformed body parses best-effort and may yield zero cues.
pub fn parse_file(path: &Path) -> std::io::Result<Vec<SrtCue>> {
    let body = fs::read_to_string(path)?;
    Ok(parse_str(&body))
}

/// Probe-only form: parse and return `(cue_count, duration_seconds)` where
/// `duration_seconds` is the maximum end time across all cues (0.0 for an
/// empty / unparseable file). Mirrors [`super::srt::probe_file`] so
/// `insert_subtitle_clip_at_playhead` can treat SRT / WebVTT / TTML uniformly.
pub fn probe_file(path: &Path) -> std::io::Result<SrtProbe> {
    let cues = parse_file(path)?;
    let duration_seconds = cues.iter().map(|c| c.end).fold(0.0_f64, f64::max);
    Ok(SrtProbe {
        cue_count: cues.len(),
        duration_seconds,
    })
}

/// Parse an in-memory TTML body into cues. Skips malformed `<p>` blocks
/// rather than failing, so hand-edited or converter-emitted files stay
/// parseable. See module docs for supported syntax.
pub fn parse_str(body: &str) -> Vec<SrtCue> {
    let stripped = strip_comments_and_pi(body);
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = stripped.as_bytes();
    while i < bytes.len() {
        // Find the next `<p` (case-sensitive — TTML is XML, tags are lowercase
        // in every real-world file). Namespace-prefixed `<tt:p ...>` matches
        // because we only look for `<` followed by a local-name-ending `p`
        // that's either at position 1 or preceded by `:`.
        let Some(start) = find_p_open(&stripped[i..]) else {
            break;
        };
        let abs_start = i + start;
        // Walk to the end of the opening tag (the `>` that closes `<p ...>`).
        let Some(open_end_rel) = stripped[abs_start..].find('>') else {
            break;
        };
        let open_end = abs_start + open_end_rel;
        let open_tag = &stripped[abs_start..=open_end];
        // Self-closing `<p ... />` — no body, skip to after it.
        if open_tag.ends_with("/>") {
            i = open_end + 1;
            continue;
        }
        // Locate the matching `</p>` (or `</tt:p>` etc.) — we look for the
        // next closing tag whose local name is `p`.
        let after_open = open_end + 1;
        let Some(close_rel) = find_p_close(&stripped[after_open..]) else {
            break;
        };
        let inner = &stripped[after_open..after_open + close_rel];
        let close_tag_end = after_open + close_rel + stripped[after_open + close_rel..]
            .find('>')
            .map(|o| o + 1)
            .unwrap_or(0);

        if let Some(cue) = parse_p_cue(open_tag, inner) {
            out.push(cue);
        }
        i = close_tag_end;
    }
    out
}

/// Find the byte offset of the next `<p` opener (possibly namespace-prefixed).
/// Returns `None` when no more opening tags exist.
fn find_p_open(s: &str) -> Option<usize> {
    let mut pos = 0;
    while let Some(rel) = s[pos..].find('<') {
        let abs = pos + rel;
        let rest = &s[abs + 1..];
        // `<p` directly
        if rest.starts_with('p') && rest[1..].starts_with(is_tag_break_char as fn(char) -> bool) {
            return Some(abs);
        }
        // `<tt:p` / `<ns:p` — scan past the prefix
        if let Some(colon) = rest.find(':') {
            // Allow only ASCII word chars between `<` and `:` — avoids
            // matching random `<foo bar:baz="">` attribute-looking text.
            let prefix = &rest[..colon];
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                let after = &rest[colon + 1..];
                if after.starts_with('p') && after[1..].starts_with(is_tag_break_char as fn(char) -> bool) {
                    return Some(abs);
                }
            }
        }
        pos = abs + 1;
    }
    None
}

/// Find the byte offset (relative to `s`) of the next `</p>` closer
/// (possibly namespace-prefixed).
fn find_p_close(s: &str) -> Option<usize> {
    let mut pos = 0;
    while let Some(rel) = s[pos..].find("</") {
        let abs = pos + rel;
        let rest = &s[abs + 2..];
        if rest.starts_with('p') && rest[1..].starts_with(is_tag_break_char as fn(char) -> bool) {
            return Some(abs);
        }
        if let Some(colon) = rest.find(':') {
            let prefix = &rest[..colon];
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                let after = &rest[colon + 1..];
                if after.starts_with('p') && after[1..].starts_with(is_tag_break_char as fn(char) -> bool) {
                    return Some(abs);
                }
            }
        }
        pos = abs + 2;
    }
    None
}

/// Tag-name terminator predicate: whitespace, `>`, or `/` ends the name.
/// Accepts a `char` so it drops straight into [`str::starts_with`].
fn is_tag_break_char(c: char) -> bool {
    c.is_ascii_whitespace() || c == '>' || c == '/'
}

/// Build an [`SrtCue`] from a `<p>` open-tag (for attributes) and inner body
/// (for text). Returns `None` when timing attributes are missing or
/// unparseable — the caller drops the cue silently, matching SRT.
fn parse_p_cue(open_tag: &str, inner: &str) -> Option<SrtCue> {
    let begin_raw = attr_value(open_tag, "begin")?;
    let start = parse_ttml_timestamp(begin_raw)?;
    let end = if let Some(end_raw) = attr_value(open_tag, "end") {
        parse_ttml_timestamp(end_raw)?
    } else {
        let dur_raw = attr_value(open_tag, "dur")?;
        start + parse_ttml_timestamp(dur_raw)?
    };
    if end <= start {
        return None;
    }
    let text = flatten_inline(inner);
    // Drop cues whose body is entirely whitespace — authoring tools sometimes
    // emit `<p begin="..." end="..."/>` equivalents that pass the timing check
    // but have nothing to render. SRT rule: keep them only if non-empty.
    if text.trim().is_empty() {
        return None;
    }
    Some(SrtCue { start, end, text })
}

/// Extract a whitespace-tolerant attribute value from a tag's opening string.
/// Supports both single- and double-quoted values. Returns `None` when the
/// attribute isn't present. Case-insensitive on the attribute name (per XML
/// practice in subtitle files; strict XML would require case-sensitive, but
/// converters frequently emit `Begin` / `BEGIN`).
fn attr_value<'a>(open_tag: &'a str, name: &str) -> Option<&'a str> {
    // Match patterns like ` name="value"` or ` name='value'`, where the
    // attribute must be preceded by whitespace (so `begin` doesn't match
    // `xml:begin`).
    let lower = open_tag.to_ascii_lowercase();
    let needle = format!("{}=", name.to_ascii_lowercase());
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find(&needle) {
        let abs = search_from + rel;
        // Must be preceded by whitespace (or the tag's opening char — in
        // practice `<p begin=` has whitespace).
        let prev = if abs == 0 {
            ' '
        } else {
            open_tag.as_bytes()[abs - 1] as char
        };
        if !prev.is_ascii_whitespace() {
            search_from = abs + needle.len();
            continue;
        }
        let after = abs + needle.len();
        let q = open_tag.as_bytes().get(after)? ;
        if *q != b'"' && *q != b'\'' {
            search_from = after;
            continue;
        }
        let quote = *q as char;
        let val_start = after + 1;
        let close = open_tag[val_start..].find(quote)?;
        return Some(&open_tag[val_start..val_start + close]);
    }
    None
}

/// Flatten `<p>` inner markup to plain text:
///
/// - `<br/>` / `<br />` / `<tt:br/>` → `\n`
/// - every other tag (open, close, self-closing) is stripped
/// - numeric and named character references are decoded
fn flatten_inline(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Peek for `<br` (possibly prefixed, possibly with attributes).
            let rest = &inner[i + 1..];
            let is_br = rest.starts_with("br")
                && rest.as_bytes().get(2).is_some_and(|c| {
                    matches!(*c, b'/' | b'>' | b' ' | b'\t' | b'\r' | b'\n')
                });
            let is_prefixed_br = rest
                .find(':')
                .and_then(|colon| {
                    let after = &rest[colon + 1..];
                    if after.starts_with("br")
                        && after.as_bytes().get(2).is_some_and(|c| {
                            matches!(*c, b'/' | b'>' | b' ' | b'\t' | b'\r' | b'\n')
                        })
                    {
                        Some(true)
                    } else {
                        None
                    }
                })
                .unwrap_or(false);
            if is_br || is_prefixed_br {
                out.push('\n');
            }
            // Skip to the next `>`; if none, drop the rest.
            let Some(gt) = inner[i..].find('>') else {
                break;
            };
            i += gt + 1;
            continue;
        }
        let c = inner[i..].chars().next().unwrap();
        i += c.len_utf8();
        out.push(c);
    }
    let decoded = unescape_entities(&out);
    collapse_whitespace(&decoded)
}

/// Decode a small set of XML entities that appear in authoring-tool TTML
/// exports (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`) plus numeric
/// character references (`&#10;`, `&#x0a;`). Unknown entities pass through
/// verbatim so preview renders the literal source — better than silently
/// dropping a character the user might need to notice.
fn unescape_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi_rel) = s[i + 1..].find(';') {
                let entity = &s[i + 1..i + 1 + semi_rel];
                if let Some(decoded) = decode_entity(entity) {
                    out.push_str(&decoded);
                    i += semi_rel + 2;
                    continue;
                }
            }
        }
        let c = s[i..].chars().next().unwrap();
        i += c.len_utf8();
        out.push(c);
    }
    out
}

fn decode_entity(entity: &str) -> Option<String> {
    match entity {
        "amp" => Some("&".to_string()),
        "lt" => Some("<".to_string()),
        "gt" => Some(">".to_string()),
        "quot" => Some("\"".to_string()),
        "apos" => Some("'".to_string()),
        s if s.starts_with("#x") || s.starts_with("#X") => {
            let code = u32::from_str_radix(&s[2..], 16).ok()?;
            char::from_u32(code).map(|c| c.to_string())
        }
        s if s.starts_with('#') => {
            let code: u32 = s[1..].parse().ok()?;
            char::from_u32(code).map(|c| c.to_string())
        }
        _ => None,
    }
}

/// Collapse runs of horizontal whitespace inside a flattened cue body to a
/// single space, preserving `\n` (which `<br/>` emitted). TTML XML
/// pretty-printers frequently indent `<p>` bodies with tabs/newlines that are
/// **not** part of the rendered caption.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = false;
    for line in s.split('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            if last_was_space && !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            for ch in trimmed.chars() {
                if ch.is_whitespace() {
                    if !last_was_space {
                        out.push(' ');
                    }
                    last_was_space = true;
                } else {
                    out.push(ch);
                    last_was_space = false;
                }
            }
        }
        out.push('\n');
        last_was_space = false;
    }
    // Drop leading and trailing newlines we added for the pretty-printed
    // outer whitespace (tabs/newlines around `<p>content</p>`). Internal
    // `\n` from an explicit `<br/>` stays intact because we only trim at the
    // boundaries, not inside.
    let trimmed = out
        .trim_matches(|c: char| c == '\n' || c.is_ascii_whitespace())
        .to_string();
    trimmed
}

/// Parse a TTML timestamp (clock-time or offset-time) to seconds.
///
/// - **Clock-time:** `HH:MM:SS` or `HH:MM:SS.fff` or `HH:MM:SS:FF` (we
///   treat the fourth field as frames at 30 fps — real TTML would consult
///   `ttp:frameRate`, which we don't read; 30 is a reasonable fallback for
///   the common 29.97 / 30 workflow).
/// - **Offset-time:** `<number><metric>` with metric ∈ `h`, `m`, `s`, `ms`.
///   `f` / `t` metrics fall back to "seconds" with a best-effort interpretation.
fn parse_ttml_timestamp(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // Offset-time: ends with a letter metric.
    if let Some(metric_start) = s.rfind(|c: char| c.is_ascii_digit() || c == '.') {
        let suffix = &s[metric_start + 1..];
        if !suffix.is_empty() {
            let num: f64 = s[..metric_start + 1].parse().ok()?;
            return Some(match suffix {
                "ms" => num / 1000.0,
                "s" => num,
                "m" => num * 60.0,
                "h" => num * 3600.0,
                // Unknown metric (`f`, `t`, or typo) — treat as seconds so a
                // caller at least sees something, and log once at info so the
                // session log surfaces non-fully-supported timing.
                _ => {
                    tracing::debug!(
                        target: "reel_core::media::ttml",
                        raw = %s,
                        metric = %suffix,
                        "unknown TTML timestamp metric; treating value as seconds"
                    );
                    num
                }
            });
        }
    }
    // Clock-time: 2 or 3 colons.
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parts[2].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + sec)
        }
        4 => {
            // HH:MM:SS:FF — frames as fourth field (default 30 fps).
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parts[2].parse().ok()?;
            let frames: f64 = parts[3].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + sec + frames / 30.0)
        }
        _ => None,
    }
}

/// Strip XML comments (`<!-- ... -->`) and processing instructions
/// (`<?xml ...?>`) so the linear `<p>` scan doesn't have to recognise them.
/// Runs once per parse — TTML files are small (caption tracks), the
/// allocation isn't hot.
fn strip_comments_and_pi(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' && bytes.get(i + 1) == Some(&b'!') && body[i..].starts_with("<!--") {
            if let Some(end) = body[i + 4..].find("-->") {
                i += 4 + end + 3;
                continue;
            }
            break;
        }
        if bytes[i] == b'<' && bytes.get(i + 1) == Some(&b'?') {
            if let Some(end) = body[i + 2..].find("?>") {
                i += 2 + end + 2;
                continue;
            }
            break;
        }
        let c = body[i..].chars().next().unwrap();
        i += c.len_utf8();
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASIC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml">
  <body>
    <div>
      <p begin="00:00:01.000" end="00:00:03.500">Hello</p>
      <p begin="00:00:04.000" end="00:00:07.250">World</p>
    </div>
  </body>
</tt>"#;

    #[test]
    fn parses_two_basic_clock_time_cues() {
        let cues = parse_str(BASIC);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start, 1.0);
        assert_eq!(cues[0].end, 3.5);
        assert_eq!(cues[0].text, "Hello");
        assert_eq!(cues[1].start, 4.0);
        assert_eq!(cues[1].end, 7.25);
        assert_eq!(cues[1].text, "World");
    }

    #[test]
    fn accepts_dur_instead_of_end() {
        let body = r#"<tt><body><div>
            <p begin="2s" dur="3s">middle</p>
        </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start, 2.0);
        assert_eq!(cues[0].end, 5.0);
        assert_eq!(cues[0].text, "middle");
    }

    #[test]
    fn offset_time_metrics_h_m_s_ms_all_convert_to_seconds() {
        let body = r#"<tt><body><div>
            <p begin="0.5h" end="1h">hour</p>
            <p begin="10m" end="11m">minute</p>
            <p begin="500ms" end="750ms">millisecond</p>
        </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].start, 1800.0);
        assert_eq!(cues[0].end, 3600.0);
        assert_eq!(cues[1].start, 600.0);
        assert_eq!(cues[1].end, 660.0);
        assert!((cues[2].start - 0.5).abs() < 1e-9);
        assert!((cues[2].end - 0.75).abs() < 1e-9);
    }

    #[test]
    fn namespace_prefixed_tt_p_tags_parse_identically_to_unprefixed() {
        // DFXP / SMPTE-TT files frequently prefix `tt:p` rather than using
        // a default namespace — the scanner walks past the prefix.
        let body = r#"<tt:tt xmlns:tt="http://www.w3.org/ns/ttml">
            <tt:body><tt:div>
                <tt:p begin="1s" end="2s">prefixed</tt:p>
            </tt:div></tt:body></tt:tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "prefixed");
    }

    #[test]
    fn br_inside_p_becomes_newline_and_other_tags_are_stripped() {
        let body = r#"<tt><body><div>
            <p begin="0s" end="1s">line one<br/>line two</p>
            <p begin="1s" end="2s"><span tts:color="red">red</span> text</p>
        </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "line one\nline two");
        assert_eq!(cues[1].text, "red text");
    }

    #[test]
    fn xml_entities_are_decoded_in_cue_text() {
        let body = r#"<tt><body><div>
            <p begin="0s" end="1s">Me &amp; you &lt;3</p>
            <p begin="1s" end="2s">newline&#10;here</p>
        </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Me & you <3");
        // `&#10;` is a newline; collapse_whitespace preserves `\n` as a
        // line break inside the cue body.
        assert_eq!(cues[1].text, "newline\nhere");
    }

    #[test]
    fn comments_and_processing_instructions_are_ignored() {
        let body = r#"<?xml version="1.0"?>
            <!-- top comment -->
            <tt><body><div>
                <!-- inline comment before cue -->
                <p begin="0s" end="1s">after comment</p>
            </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "after comment");
    }

    #[test]
    fn self_closing_p_is_skipped_not_returned_as_empty_cue() {
        let body = r#"<tt><body><div>
            <p begin="0s" end="1s" />
            <p begin="1s" end="2s">real cue</p>
        </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "real cue");
    }

    #[test]
    fn malformed_file_returns_empty_or_partial_cues_without_panicking() {
        // Unterminated `<p>` — the scanner bails at end-of-input.
        let body = "<tt><body><div><p begin=\"0s\" end=\"1s\">never closes";
        let cues = parse_str(body);
        assert!(cues.is_empty());
        // Completely unrelated payload.
        let cues = parse_str("not xml at all; here are some bytes.");
        assert!(cues.is_empty());
    }

    #[test]
    fn blank_cue_body_is_dropped() {
        // A cue with only whitespace between open and close tags must be
        // dropped — showing a blank overlay strip is strictly worse than
        // hiding the overlay entirely.
        let body = r#"<tt><body><div>
            <p begin="0s" end="1s">   </p>
            <p begin="1s" end="2s">visible</p>
        </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "visible");
    }

    #[test]
    fn end_before_or_equal_start_is_rejected() {
        let body = r#"<tt><body><div>
            <p begin="5s" end="5s">zero-length</p>
            <p begin="10s" end="8s">reversed</p>
            <p begin="20s" end="21s">keeps-valid</p>
        </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "keeps-valid");
    }

    #[test]
    fn clock_time_with_frame_field_uses_30fps_fallback() {
        // HH:MM:SS:FF — fourth field treated as frames @ 30 fps since we
        // don't parse ttp:frameRate yet.
        let body = r#"<tt><body><div>
            <p begin="00:00:01:15" end="00:00:02:00">frames</p>
        </div></body></tt>"#;
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert!((cues[0].start - 1.5).abs() < 1e-9);
        assert!((cues[0].end - 2.0).abs() < 1e-9);
    }

    #[test]
    fn probe_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.ttml");
        std::fs::write(&path, BASIC).unwrap();
        let pr = probe_file(&path).unwrap();
        assert_eq!(pr.cue_count, 2);
        assert!((pr.duration_seconds - 7.25).abs() < 1e-9);
    }

    #[test]
    fn indented_body_collapses_to_clean_text() {
        // Pretty-printed TTML embeds tabs/newlines inside the cue body.
        // collapse_whitespace must not leak them into the rendered caption.
        let body = "<tt><body><div>\n\
                    <p begin=\"0s\" end=\"1s\">\n    spaced    out    caption\n</p>\n\
                    </div></body></tt>";
        let cues = parse_str(body);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "spaced out caption");
    }
}
