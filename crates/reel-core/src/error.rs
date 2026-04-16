//! Crate-wide error types.

use std::path::PathBuf;
use thiserror::Error;

/// Top-level error type for `reel-core`.
#[derive(Debug, Error)]
pub enum ReelError {
    #[error("probe failed: {0}")]
    Probe(#[from] ProbeError),

    #[error("project i/o: {0}")]
    ProjectIo(#[from] std::io::Error),

    #[error("project serde: {0}")]
    ProjectSerde(#[from] serde_json::Error),
}

/// Errors produced by [`crate::media::MediaProbe`] implementations.
///
/// Note: audio-stream decode failures are **not** errors — they log a `WARN`
/// and surface via [`crate::MediaMetadata::audio_disabled`] instead, matching
/// the graceful-degradation contract in the project spec.
#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("i/o opening {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("ffmpeg could not open {path}: {reason}")]
    FfmpegOpen { path: PathBuf, reason: String },

    #[error("no video stream in {path}")]
    NoVideoStream { path: PathBuf },

    #[error("unsupported: {reason}")]
    Unsupported { reason: String },
}
