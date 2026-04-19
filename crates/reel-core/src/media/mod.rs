//! Media engine: probe, metadata types, and (later) frame decode loops.

pub mod decoder;
pub mod export;
pub mod frame;
pub mod metadata;
pub mod probe;
pub mod srt;
pub mod ttml;

pub use export::{
    build_mute_substitution_lane, export_concat_timeline, export_concat_timeline_oriented,
    export_concat_with_audio, export_concat_with_audio_lanes_oriented,
    export_concat_with_audio_lanes_oriented_with_gains, export_concat_with_audio_oriented,
    export_with_ffmpeg, ffmpeg_args_for_format,
    generate_silence_wav, ExportProgressFn, GifPreset, WebExportFormat,
};
pub use frame::grab_frame;
pub use metadata::{AudioStreamInfo, MediaMetadata, VideoStreamInfo};
pub use probe::FfmpegProbe;
pub use srt::{
    find_cue_at_seconds as find_srt_cue_at_seconds, parse_file as parse_srt_file,
    parse_str as parse_srt_str, probe_file as probe_srt_file, SrtCue, SrtProbe,
};
pub use ttml::{
    parse_file as parse_ttml_file, parse_str as parse_ttml_str, probe_file as probe_ttml_file,
};

use std::path::Path;

use crate::error::ProbeError;

/// Extension-dispatched subtitle parser — routes `.ttml` / `.xml` / `.dfxp`
/// through [`ttml::parse_file`] and everything else through
/// [`srt::parse_file`] (which handles both SubRip and WebVTT). Centralises
/// the "which parser do I use?" decision so callers (insert-subtitle,
/// overlay cache) stay format-agnostic.
///
/// Unknown extensions fall through to the SRT/WebVTT parser on the theory
/// that hand-edited caption files often lack extensions or use generic
/// ones; the parser is tolerant of malformed bodies and returns an empty
/// cue list for anything it can't decode.
pub fn parse_subtitle_file(path: &Path) -> std::io::Result<Vec<SrtCue>> {
    if is_ttml_extension(path) {
        parse_ttml_file(path)
    } else {
        parse_srt_file(path)
    }
}

/// Extension-dispatched subtitle probe — same routing rule as
/// [`parse_subtitle_file`]. Used by **File → Insert Subtitle…** to get a
/// cue count + duration without caring which format the file is in.
pub fn probe_subtitle_file(path: &Path) -> std::io::Result<SrtProbe> {
    if is_ttml_extension(path) {
        probe_ttml_file(path)
    } else {
        probe_srt_file(path)
    }
}

fn is_ttml_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("ttml") | Some("dfxp") | Some("xml")
    )
}

/// Abstract media probing.
///
/// The trait exists so consumer code (import flows, CLI, timeline) can be
/// unit-tested with `mockall`-generated mocks, while `FfmpegProbe` provides
/// the real backing implementation exercised by a small fixture suite.
#[cfg_attr(test, mockall::automock)]
pub trait MediaProbe: Send + Sync {
    fn probe(&self, path: &Path) -> Result<MediaMetadata, ProbeError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fake_metadata(path: &Path) -> MediaMetadata {
        MediaMetadata {
            path: path.to_path_buf(),
            duration_seconds: 5.0,
            container: "fake".into(),
            video: Some(VideoStreamInfo {
                codec: "h264".into(),
                width: 10,
                height: 10,
                frame_rate: 24.0,
                pixel_format: "YUV420P".into(),
                rotation: 0,
            }),
            audio: None,
            audio_disabled: false,
            video_stream_count: 1,
            audio_stream_count: 0,
            subtitle_stream_count: 0,
            audio_streams: Vec::new(),
        }
    }

    #[test]
    fn parse_subtitle_file_routes_by_extension() {
        // `.ttml` / `.dfxp` / `.xml` take the TTML path; `.srt` / `.vtt`
        // (and anything else) take the SRT+WebVTT parser. We verify routing
        // by feeding each parser an input that's **only** valid in its own
        // grammar — so routing errors show up as zero-cue parses.
        let dir = tempfile::tempdir().unwrap();

        let ttml_body = r#"<tt><body><div><p begin="1s" end="2s">ttml</p></div></body></tt>"#;
        let srt_body = "1\n00:00:01,000 --> 00:00:02,000\nsrt\n";

        let ttml_path = dir.path().join("a.ttml");
        let dfxp_path = dir.path().join("a.dfxp");
        let xml_path = dir.path().join("a.xml");
        let srt_path = dir.path().join("a.srt");
        let vtt_path = dir.path().join("a.vtt");

        std::fs::write(&ttml_path, ttml_body).unwrap();
        std::fs::write(&dfxp_path, ttml_body).unwrap();
        std::fs::write(&xml_path, ttml_body).unwrap();
        std::fs::write(&srt_path, srt_body).unwrap();
        std::fs::write(&vtt_path, format!("WEBVTT\n\n{srt_body}")).unwrap();

        for p in &[&ttml_path, &dfxp_path, &xml_path] {
            let cues = parse_subtitle_file(p).unwrap();
            assert_eq!(cues.len(), 1, "ttml routing for {}", p.display());
            assert_eq!(cues[0].text, "ttml");
        }
        for p in &[&srt_path, &vtt_path] {
            let cues = parse_subtitle_file(p).unwrap();
            assert_eq!(cues.len(), 1, "srt/vtt routing for {}", p.display());
            assert_eq!(cues[0].text, "srt");
        }

        // probe_subtitle_file must route the same way — a caller that only
        // needs duration shouldn't need to pick the right parser either.
        assert_eq!(probe_subtitle_file(&ttml_path).unwrap().cue_count, 1);
        assert_eq!(probe_subtitle_file(&srt_path).unwrap().cue_count, 1);
    }

    #[test]
    fn mock_probe_returns_expected_metadata() {
        let mut mock = MockMediaProbe::new();
        let expected_path = PathBuf::from("/tmp/fake.mp4");
        let returned = fake_metadata(&expected_path);
        mock.expect_probe()
            .withf(|p| p == Path::new("/tmp/fake.mp4"))
            .times(1)
            .return_once(move |_| Ok(returned));

        let out = (&mock as &dyn MediaProbe).probe(&expected_path).unwrap();
        assert_eq!(out.duration_seconds, 5.0);
        assert_eq!(out.video.unwrap().width, 10);
    }
}
