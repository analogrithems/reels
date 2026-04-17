//! Footer strip: codecs, paths, save state (see **`docs/FEATURES.md`**).
//! Populated from the [`Project`] and playhead — **does not** depend on decode/`media-ready`.

use reel_core::Project;

use crate::timeline;

/// Lines for the bottom footer; `None` when there is no project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FooterLines {
    pub codec_line: String,
    pub path_line: String,
    pub save_line: String,
    pub unsaved: bool,
}

pub(crate) fn format_codec_line(p: &Project, seq_ms: f64) -> String {
    let Some(vclip) = timeline::primary_clip_ref_at_seq_ms(p, seq_ms) else {
        return "No clip at playhead".to_string();
    };
    let vcodec = vclip
        .metadata
        .video
        .as_ref()
        .map(|x| x.codec.as_str())
        .unwrap_or("—");

    let has_dedicated = timeline::clips_from_first_audio_track(p).is_some();
    if has_dedicated {
        if let Some(aid) = timeline::first_audio_clip_id_at_seq_ms(p, seq_ms) {
            let aclip = p.clips.iter().find(|c| c.id == aid);
            let audio_str = aclip
                .map(|ac| {
                    if ac.metadata.audio_disabled {
                        "disabled".to_string()
                    } else {
                        ac.metadata
                            .audio
                            .as_ref()
                            .map(|a| a.codec.clone())
                            .unwrap_or_else(|| "none".into())
                    }
                })
                .unwrap_or_else(|| "—".into());
            format!("Video: {vcodec} · Audio: {audio_str} (first audio track)")
        } else {
            format!("Video: {vcodec} · Audio: silence (dedicated track — no clip at playhead)")
        }
    } else {
        let audio_str = if vclip.metadata.audio_disabled {
            "unavailable".to_string()
        } else {
            vclip
                .metadata
                .audio
                .as_ref()
                .map(|a| a.codec.clone())
                .unwrap_or_else(|| "none".into())
        };
        format!("Video: {vcodec} · Audio: {audio_str} (embedded in video file)")
    }
}

/// Build footer content whenever a project is loaded (independent of ffmpeg decode / `media-ready`).
pub(crate) fn compute_footer_lines(
    project: Option<&Project>,
    playhead_ms: f64,
    dirty: bool,
) -> Option<FooterLines> {
    let p = project?;
    let codec_line = format_codec_line(p, playhead_ms);
    let media_path = timeline::resolve_for_project(p, playhead_ms)
        .map(|(path, _)| path.display().to_string())
        .unwrap_or_else(|| "—".into());
    let proj_line = p
        .path
        .as_ref()
        .map(|x| x.display().to_string())
        .unwrap_or_else(|| "Not saved to disk".to_string());
    let path_line = format!("Current clip: {media_path} · Project file: {proj_line}");
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
            f.codec_line.contains("Video:") && f.codec_line.contains("Audio:"),
            "codec line: {}",
            f.codec_line
        );
        assert!(
            f.path_line.contains("Current clip:") && f.path_line.contains("Project file:"),
            "path line: {}",
            f.path_line
        );
        assert!(
            f.path_line.contains("tiny_h264_aac")
                || f.path_line.contains(&path.display().to_string()),
            "path line should name clip: {}",
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
