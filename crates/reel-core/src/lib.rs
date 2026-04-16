//! `reel-core` — shared library for the Reel video editor.
//!
//! Hosts the media engine wrapper over `ffmpeg-next`, the serializable
//! `Project` model, and the `tracing` initialization used by every binary.

pub mod error;
pub mod logging;
pub mod media;
pub mod project;
pub mod sidecar;

pub use error::{ProbeError, ReelError};
pub use media::decoder::{DecodeCmd, DecodedFrame};
pub use media::{
    grab_frame, AudioStreamInfo, FfmpegProbe, MediaMetadata, MediaProbe, VideoStreamInfo,
};
pub use project::{Clip, Project, ProjectStore, Track, TrackKind};
pub use sidecar::{SidecarClient, SidecarError};
