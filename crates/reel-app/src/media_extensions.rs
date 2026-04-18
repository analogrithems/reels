//! Extension lists for **native file dialogs** (Open / Insert). Playback uses FFmpeg with no
//! hard allowlist in the decoder — these lists only shape the **filter** UI; **All files** still
//! passes through. See `docs/MEDIA_FORMATS.md`.

/// Containers used for **Insert Video…** and the “video with audio stream” branch of **Insert Audio…**.
pub const VIDEO_CONTAINER_EXTENSIONS: &[&str] = &[
    "3g2", "3gp", "asf", "avi", "divx", "flv", "f4v", "m2ts", "m2v", "mkv", "mov", "mp4", "mpeg",
    "mpg", "mts", "m4v", "ogv", "ts", "webm", "wmv",
];

/// Audio-first extensions for **Insert Audio…**.
pub const AUDIO_FILE_EXTENSIONS: &[&str] = &[
    "aac", "ac3", "aif", "aiff", "caf", "dts", "eac3", "flac", "m4a", "mp3", "ogg", "opus", "wav",
    "wma",
];

/// Subtitle sidecar extensions for **Insert Subtitle…**. SubRip (`.srt`) and
/// WebVTT (`.vtt`) share a parser (see `reel_core::media::srt`); both are also
/// accepted by ffmpeg's `subtitles=` filter at export time.
pub const SUBTITLE_FILE_EXTENSIONS: &[&str] = &["srt", "vtt"];

/// **File → Open…**: combined video/mux + audio extensions (includes MPEG program stream and MPEG-TS).
pub const OPEN_MEDIA_EXTENSIONS: &[&str] = &[
    "3g2", "3gp", "aac", "ac3", "aif", "aiff", "asf", "avi", "caf", "divx", "dts", "eac3", "f4v",
    "flac", "flv", "m2ts", "m2v", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "mpeg", "mpg", "mts",
    "ogg", "ogv", "opus", "ts", "wav", "webm", "wma", "wmv",
];
