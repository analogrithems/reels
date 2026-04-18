//! Footer strip: codecs, paths, save state (see **`docs/FEATURES.md`**).
//! Populated from the [`Project`] and playhead — **does not** depend on decode/`media-ready`.
//!
//! Layout matches the v0 mock: **`H.264 | AAC`** (left) · **project path** (center, `~`-shortened) · **saved state** (right).

use std::path::Path;

use reel_core::Project;

use crate::timeline;

/// Lines for the bottom footer; `None` when there is no project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FooterLines {
    /// Left column: short video + audio codec names, e.g. `H.264  |  AAC`.
    pub codec_line: String,
    /// Center: project file path (`~/…` when under the home directory).
    pub path_line: String,
    /// Right column text only (checkmark drawn in Slint), e.g. `All changes saved`.
    pub save_line: String,
    pub unsaved: bool,
}

fn path_display_tilde(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if path.starts_with(&home) {
            if let Ok(rest) = path.strip_prefix(&home) {
                let s = rest.to_string_lossy();
                return if s.is_empty() {
                    "~".into()
                } else {
                    format!("~/{}", s.trim_start_matches('/'))
                };
            }
        }
    }
    path.display().to_string()
}

/// Map ffmpeg-style codec names to short labels (mock-style).
fn short_audio_codec(raw: &str) -> String {
    let s = raw.to_lowercase();
    if s.contains("aac") {
        return "AAC".into();
    }
    if s.contains("opus") {
        return "Opus".into();
    }
    if s.contains("mp3") {
        return "MP3".into();
    }
    if s.contains("pcm") {
        return "PCM".into();
    }
    if s == "none" || s.is_empty() {
        return "—".into();
    }
    raw.to_uppercase()
}

fn short_video_codec(raw: &str) -> String {
    let s = raw.to_lowercase();
    if s.contains("hevc") || s.contains("h265") || s.contains("h.265") {
        return "HEVC".into();
    }
    if s.contains("h264") || s.contains("avc") || s.contains("h.264") || s == "libx264" {
        return "H.264".into();
    }
    if s.contains("vp9") {
        return "VP9".into();
    }
    if s.contains("vp8") {
        return "VP8".into();
    }
    if s.contains("av1") {
        return "AV1".into();
    }
    if s.contains("prores") {
        return "ProRes".into();
    }
    if s == "none" || s.is_empty() {
        return "—".into();
    }
    raw.to_uppercase()
}

pub(crate) fn format_codec_line(p: &Project, seq_ms: f64) -> String {
    let Some(vclip) = timeline::primary_clip_ref_at_seq_ms(p, seq_ms) else {
        return "— | —".to_string();
    };
    let vraw = vclip
        .metadata
        .video
        .as_ref()
        .map(|x| x.codec.as_str())
        .unwrap_or("—");
    let vshort = short_video_codec(vraw);

    let has_dedicated = timeline::clips_from_first_audio_track(p).is_some();
    let ashort = if has_dedicated {
        if let Some(aid) = timeline::first_audio_clip_id_at_seq_ms(p, seq_ms) {
            let aclip = p.clips.iter().find(|c| c.id == aid);
            aclip
                .map(|ac| {
                    if ac.metadata.audio_disabled {
                        "—".to_string()
                    } else {
                        ac.metadata
                            .audio
                            .as_ref()
                            .map(|a| short_audio_codec(&a.codec))
                            .unwrap_or_else(|| "—".into())
                    }
                })
                .unwrap_or_else(|| "—".into())
        } else {
            "—".to_string()
        }
    } else if vclip.metadata.audio_disabled {
        "—".to_string()
    } else {
        vclip
            .metadata
            .audio
            .as_ref()
            .map(|a| short_audio_codec(&a.codec))
            .unwrap_or_else(|| "—".into())
    };

    format!("{vshort}  |  {ashort}")
}

/// Build footer content whenever a project is loaded (independent of ffmpeg decode / `media-ready`).
pub(crate) fn compute_footer_lines(
    project: Option<&Project>,
    playhead_ms: f64,
    dirty: bool,
) -> Option<FooterLines> {
    let p = project?;
    let codec_line = format_codec_line(p, playhead_ms);
    let path_line = p
        .path
        .as_ref()
        .map(|x| path_display_tilde(x))
        .unwrap_or_else(|| "Untitled project — use File → Save…".to_string());
    let save_line = if dirty {
        "Unsaved changes".to_string()
    } else {
        "All changes saved".to_string()
    };
    Some(FooterLines {
        codec_line,
        path_line,
        save_line,
        unsaved: dirty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::project_io::project_from_media_path;

    fn tiny_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../reel-core/tests/fixtures/tiny_h264_aac.mp4")
    }

    /// Regression: footer must fill as soon as we have a probed project, without waiting for decode.
    #[test]
    fn footer_lines_populated_from_project_without_media_ready() {
        let path = tiny_fixture();
        assert!(path.is_file(), "missing fixture {}", path.display());
        let p = project_from_media_path(&path).expect("probe fixture");
        let f = compute_footer_lines(Some(&p), 0.0, false).expect("footer");
        assert!(
            f.codec_line.contains('|'),
            "codec line should be mock-style `Video | Audio`: {}",
            f.codec_line
        );
        assert!(
            f.path_line.contains("Untitled") || f.path_line.contains('~'),
            "path line should be project-centric: {}",
            f.path_line
        );
        assert_eq!(f.save_line, "All changes saved");
        assert!(!f.unsaved);
    }

    #[test]
    fn footer_lines_none_without_project() {
        assert!(compute_footer_lines(None, 0.0, false).is_none());
    }
}
