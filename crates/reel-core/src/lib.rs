//! `reel-core` — shared library for the Reel video editor.
//!
//! Hosts the media engine wrapper over `ffmpeg-next`, the serializable
//! `Project` model, and the `tracing` initialization used by every binary (session log file under the
//! OS data directory; session log lines are **NDJSON** with module path, file, line, and structured
//! fields — see `logging` module).

pub mod error;
pub mod logging;
pub mod media;
pub mod project;
pub mod sidecar;

pub use error::{ProbeError, ReelError};
pub use media::decoder::{DecodeCmd, DecodedFrame};
pub use media::{
    build_mute_substitution_lane, export_concat_timeline, export_concat_timeline_oriented,
    export_concat_with_audio, export_concat_with_audio_lanes_oriented,
    export_concat_with_audio_lanes_oriented_with_gains, export_concat_with_audio_oriented,
    export_with_ffmpeg, ffmpeg_args_for_format,
    find_srt_cue_at_seconds, generate_silence_wav, grab_frame, parse_srt_file, parse_srt_str,
    probe_srt_file,
    AudioStreamInfo, ExportProgressFn, FfmpegProbe, GifPreset, MediaMetadata, MediaProbe, SrtCue,
    SrtProbe, VideoStreamInfo, WebExportFormat,
};
pub use project::{
    migrate, Clip, ClipScale, MigrationError, Project, ProjectStore, Track, TrackKind,
};
pub use sidecar::{SidecarClient, SidecarError};
