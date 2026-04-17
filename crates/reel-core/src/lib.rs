//! `reel-core` — shared library for the Reel video editor.
//!
//! Hosts the media engine wrapper over `ffmpeg-next`, the serializable
//! `Project` model, and the `tracing` initialization used by every binary (session log file under the
//! OS data directory; module path + file:line in each line — see `logging` module).

pub mod error;
pub mod logging;
pub mod media;
pub mod project;
pub mod sidecar;

pub use error::{ProbeError, ReelError};
pub use media::decoder::{DecodeCmd, DecodedFrame};
pub use media::{
    export_concat_timeline, export_concat_with_audio, export_with_ffmpeg, ffmpeg_args_for_format,
    grab_frame, AudioStreamInfo, ExportProgressFn, FfmpegProbe, MediaMetadata, MediaProbe,
    VideoStreamInfo, WebExportFormat,
};
pub use project::{migrate, Clip, MigrationError, Project, ProjectStore, Track, TrackKind};
pub use sidecar::{SidecarClient, SidecarError};
