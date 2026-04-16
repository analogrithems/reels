//! One-shot frame grabber: decode a single RGBA8 frame near a target pts.
//!
//! Used by `reel-cli swap` to feed the sidecar bridge. Intentionally minimal
//! — the player pipeline in `reel-app::player` has its own streaming decoder
//! and does not share state with this helper.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use ffmpeg_next as ffmpeg;

use super::decoder::DecodedFrame;

/// Decode a single frame at or just after `pts_ms` and return it as
/// tightly-packed RGBA8 (`width * height * 4` bytes, stride `width * 4`).
///
/// Seeks to the nearest keyframe ≤ target and decodes forward until a frame
/// with `pts_ms` ≥ target is produced; if the stream ends first, the last
/// decoded frame is returned. A missing video stream is an error.
pub fn grab_frame(path: &Path, pts_ms: u64) -> Result<DecodedFrame> {
    ffmpeg::init().context("ffmpeg init")?;

    let mut input =
        ffmpeg::format::input(&path).with_context(|| format!("ffmpeg open {}", path.display()))?;

    let (stream_idx, time_base, params) = {
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| anyhow!("no video stream in {}", path.display()))?;
        (stream.index(), stream.time_base(), stream.parameters())
    };

    let codec_ctx = ffmpeg::codec::context::Context::from_parameters(params)?;
    let mut decoder = codec_ctx.decoder().video()?;
    let width = decoder.width();
    let height = decoder.height();
    if width == 0 || height == 0 {
        return Err(anyhow!("degenerate video dimensions {width}x{height}"));
    }

    let mut scaler = ffmpeg::software::scaling::Context::get(
        decoder.format(),
        width,
        height,
        ffmpeg::format::Pixel::RGBA,
        width,
        height,
        ffmpeg::software::scaling::Flags::FAST_BILINEAR,
    )?;

    // Seek to nearest keyframe ≤ target, then decode forward.
    if pts_ms > 0 {
        let ts = (pts_ms as i64) * i64::from(ffmpeg::ffi::AV_TIME_BASE) / 1000;
        input
            .seek(ts, ..ts)
            .with_context(|| format!("ffmpeg seek to {pts_ms}ms"))?;
        decoder.flush();
    }

    let mut decoded = ffmpeg::frame::Video::empty();
    let mut last: Option<DecodedFrame> = None;

    for (stream, packet) in input.packets() {
        if stream.index() != stream_idx {
            continue;
        }
        if decoder.send_packet(&packet).is_err() {
            continue;
        }
        while decoder.receive_frame(&mut decoded).is_ok() {
            let frame = scale_to_rgba(&mut decoded, &mut scaler, width, height, time_base)?;
            if frame.pts_ms >= pts_ms {
                return Ok(frame);
            }
            last = Some(frame);
        }
    }

    // Flush decoder at EOF.
    let _ = decoder.send_eof();
    while decoder.receive_frame(&mut decoded).is_ok() {
        let frame = scale_to_rgba(&mut decoded, &mut scaler, width, height, time_base)?;
        if frame.pts_ms >= pts_ms {
            return Ok(frame);
        }
        last = Some(frame);
    }

    last.ok_or_else(|| anyhow!("no frames decoded from {}", path.display()))
}

fn scale_to_rgba(
    src: &mut ffmpeg::frame::Video,
    scaler: &mut ffmpeg::software::scaling::Context,
    width: u32,
    height: u32,
    time_base: ffmpeg::Rational,
) -> Result<DecodedFrame> {
    let mut dst = ffmpeg::frame::Video::empty();
    dst.set_format(ffmpeg::format::Pixel::RGBA);
    dst.set_width(width);
    dst.set_height(height);
    scaler.run(src, &mut dst).context("sws scaler")?;

    let stride = dst.stride(0);
    let row_bytes = (width as usize) * 4;
    if stride < row_bytes {
        return Err(anyhow!("scaler stride {stride} < row_bytes {row_bytes}"));
    }
    let plane = dst.data(0);
    let required = stride * (height as usize).saturating_sub(1) + row_bytes;
    if plane.len() < required {
        return Err(anyhow!(
            "scaler output {} < required {required}",
            plane.len()
        ));
    }

    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        rgba.extend_from_slice(&plane[start..start + row_bytes]);
    }

    let pts = src.pts().unwrap_or(0);
    let pts_ms = (pts as f64 * f64::from(time_base.numerator())
        / f64::from(time_base.denominator())
        * 1000.0) as u64;

    Ok(DecodedFrame {
        pts_ms,
        width,
        height,
        rgba: std::sync::Arc::from(rgba.into_boxed_slice()),
    })
}
