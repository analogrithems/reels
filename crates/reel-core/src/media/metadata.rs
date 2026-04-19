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

    /// Every decodable audio stream in container order. `audio` always mirrors
    /// the first entry (or is `None` when every audio stream failed to decode).
    /// `#[serde(default)]` because older project JSON pre-dates the multi-stream
    /// field — a deserialized file with no `audio_streams` key synthesises an
    /// empty vec and UI falls back to the legacy single-stream path.
    #[serde(default)]
    pub audio_streams: Vec<AudioStreamInfo>,
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
    /// Container stream index (`ffmpeg`'s `stream.index()`). Needed for
    /// per-clip stream selection so the player / export can map `0:a:<index>`
    /// directly instead of re-walking streams. `#[serde(default)]` keeps
    /// legacy JSON loadable — missing field reads as `0`, which matches the
    /// old single-stream-only behavior.
    #[serde(default)]
    pub index: u32,
    /// `language` metadata tag (ISO 639-2/B, e.g. `"eng"`, `"jpn"`) when the
    /// container provides one. Shown alongside the codec in the Audio Track
    /// menu so users can tell tracks apart by language.
    #[serde(default)]
    pub language: Option<String>,
    /// `title` metadata tag when set (e.g. `"Director's commentary"`).
    #[serde(default)]
    pub title: Option<String>,
}

impl AudioStreamInfo {
    /// Short human label for menus / status lines — prefers the `title` tag,
    /// falls back to `"<codec> (<language>)"`, or just `"<codec>"` when no
    /// language is present. Always terse: UI space for this is tight.
    pub fn display_label(&self) -> String {
        if let Some(t) = self.title.as_deref() {
            if !t.is_empty() {
                return t.to_string();
            }
        }
        match self.language.as_deref() {
            Some(l) if !l.is_empty() => format!("{} ({})", self.codec, l),
            _ => self.codec.clone(),
        }
    }
}
