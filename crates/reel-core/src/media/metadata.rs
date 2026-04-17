//! Metadata structs produced by `MediaProbe`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaMetadata {
    pub path: PathBuf,
    pub duration_seconds: f64,
    pub container: String,
    pub video: Option<VideoStreamInfo>,
    pub audio: Option<AudioStreamInfo>,

    /// True iff an audio stream was present in the container but `ffmpeg`
    /// could not identify/decode it. In that case [`audio`] stays `None` and
    /// the application should run with audio muted rather than erroring out.
    pub audio_disabled: bool,

    /// Container stream counts from ffmpeg (first pass). Older JSON omits these — use
    /// [`Self::video_streams_display`] / [`Self::audio_streams_display`] for UI.
    #[serde(default)]
    pub video_stream_count: u8,
    #[serde(default)]
    pub audio_stream_count: u8,
    #[serde(default)]
    pub subtitle_stream_count: u8,
}

impl MediaMetadata {
    /// Video lanes to show in **single-media** timeline UI (capped at 4).
    pub fn video_streams_display(&self) -> u32 {
        let n = self.video_stream_count as u32;
        if n > 0 {
            n.min(4)
        } else if self.video.is_some() {
            1
        } else {
            0
        }
    }

    /// Audio lanes to show in **single-media** timeline UI (capped at 4).
    pub fn audio_streams_display(&self) -> u32 {
        let n = self.audio_stream_count as u32;
        if n > 0 {
            n.min(4)
        } else if self.audio.is_some() || self.audio_disabled {
            1
        } else {
            0
        }
    }

    /// Subtitle lanes to show in **single-media** timeline UI (capped at 4).
    pub fn subtitle_streams_display(&self) -> u32 {
        (self.subtitle_stream_count as u32).min(4)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoStreamInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub pixel_format: String,
    /// Container rotation metadata in degrees (90/180/270); 0 for none.
    pub rotation: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioStreamInfo {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
}
