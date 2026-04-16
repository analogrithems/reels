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
