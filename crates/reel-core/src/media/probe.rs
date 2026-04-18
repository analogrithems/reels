//! `FfmpegProbe` — real implementation of [`MediaProbe`] using `ffmpeg-next`.

use std::path::Path;
use std::sync::Once;

use ffmpeg::media::Type as MediaType;
use ffmpeg_next as ffmpeg;

use super::{AudioStreamInfo, MediaMetadata, MediaProbe, VideoStreamInfo};
use crate::error::ProbeError;

static FFMPEG_INIT: Once = Once::new();

fn ensure_ffmpeg_init() {
    FFMPEG_INIT.call_once(|| {
        // ffmpeg::init() registers codecs/formats and installs the log
        // callback. Safe to skip the Result — a failing init is a broken
        // install we can't recover from at runtime anyway.
        let _ = ffmpeg::init();
    });
}

/// Default probe implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct FfmpegProbe;

impl FfmpegProbe {
    pub fn new() -> Self {
        Self
    }
}

impl MediaProbe for FfmpegProbe {
    fn probe(&self, path: &Path) -> Result<MediaMetadata, ProbeError> {
        ensure_ffmpeg_init();

        if !path.exists() {
            return Err(ProbeError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
            });
        }

        let input = ffmpeg::format::input(&path).map_err(|e| ProbeError::FfmpegOpen {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

        let duration_seconds = {
            let d = input.duration();
            if d <= 0 {
                0.0
            } else {
                d as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
            }
        };

        let container = input.format().name().to_string();

        let mut video: Option<VideoStreamInfo> = None;
        let mut audio: Option<AudioStreamInfo> = None;
        let mut audio_disabled = false;
        let mut video_stream_count: u8 = 0;
        let mut audio_stream_count: u8 = 0;
        let mut subtitle_stream_count: u8 = 0;

        for stream in input.streams() {
            match stream.parameters().medium() {
                MediaType::Video => {
                    video_stream_count = video_stream_count.saturating_add(1);
                    if video.is_none() {
                        match ffmpeg::codec::context::Context::from_parameters(stream.parameters())
                        {
                            Ok(ctx) => match ctx.decoder().video() {
                                Ok(v) => {
                                    let avg = stream.avg_frame_rate();
                                    let fr = if avg.denominator() == 0 {
                                        0.0
                                    } else {
                                        f64::from(avg.numerator()) / f64::from(avg.denominator())
                                    };
                                    let codec_name = v
                                        .codec()
                                        .map(|c| c.name().to_string())
                                        .unwrap_or_else(|| "unknown".to_string());
                                    video = Some(VideoStreamInfo {
                                        codec: codec_name,
                                        width: v.width(),
                                        height: v.height(),
                                        frame_rate: fr,
                                        pixel_format: format!("{:?}", v.format()),
                                        rotation: read_rotation(&stream),
                                    });
                                }
                                Err(e) => {
                                    return Err(ProbeError::Unsupported {
                                        reason: format!("video decoder open: {e}"),
                                    });
                                }
                            },
                            Err(e) => {
                                return Err(ProbeError::Unsupported {
                                    reason: format!("video codec context: {e}"),
                                });
                            }
                        }
                    }
                }
                MediaType::Audio => {
                    audio_stream_count = audio_stream_count.saturating_add(1);
                    if audio.is_none() && !audio_disabled {
                        match ffmpeg::codec::context::Context::from_parameters(stream.parameters())
                            .and_then(|c| c.decoder().audio())
                        {
                            Ok(a) => {
                                audio = Some(AudioStreamInfo {
                                    codec: a
                                        .codec()
                                        .map(|c| c.name().to_string())
                                        .unwrap_or_else(|| "unknown".into()),
                                    sample_rate: a.rate(),
                                    channels: a.channels(),
                                });
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "reel_core::media",
                                    path = %path.display(),
                                    error = %e,
                                    "unrecognized audio codec; disabling audio track"
                                );
                                audio_disabled = true;
                            }
                        }
                    }
                }
                MediaType::Subtitle => {
                    subtitle_stream_count = subtitle_stream_count.saturating_add(1);
                }
                _ => {}
            }
        }

        if video.is_none() {
            return Err(ProbeError::NoVideoStream {
                path: path.to_path_buf(),
            });
        }

        Ok(MediaMetadata {
            path: path.to_path_buf(),
            duration_seconds,
            container,
            video,
            audio,
            audio_disabled,
            video_stream_count,
            audio_stream_count,
            subtitle_stream_count,
        })
    }
}

fn read_rotation(stream: &ffmpeg::format::stream::Stream) -> i32 {
    // ffmpeg exposes rotation via side data or metadata depending on version;
    // try the `rotate` metadata tag first which is the most stable.
    if let Some(v) = stream.metadata().get("rotate") {
        if let Ok(n) = v.parse::<i32>() {
            return n.rem_euclid(360);
        }
    }
    0
}
