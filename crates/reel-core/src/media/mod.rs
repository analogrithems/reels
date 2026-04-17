//! Media engine: probe, metadata types, and (later) frame decode loops.

pub mod decoder;
pub mod export;
pub mod frame;
pub mod metadata;
pub mod probe;

pub use export::{export_with_ffmpeg, ffmpeg_args_for_format, WebExportFormat};
pub use frame::grab_frame;
pub use metadata::{AudioStreamInfo, MediaMetadata, VideoStreamInfo};
pub use probe::FfmpegProbe;

use std::path::Path;

use crate::error::ProbeError;

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
        }
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
