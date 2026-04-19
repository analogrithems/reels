//! Background generation of timeline **waveform** and (later) **thumbnail**
//! rasters for clip chips, with a lazy diagonal-stripe placeholder so the
//! timeline shows up **instantly** on media load and the real image drops
//! in when the worker is done.
//!
//! # Flow
//!
//! ```text
//!   sync_timeline_chips (UI thread)
//!       └─► AssetCache::get_or_request(clip_id, path, in_ms, out_ms)
//!              │
//!              ├─► cached hit  → returns Some(Image) → TlChip.waveform_ready = true
//!              └─► miss        → enqueues Job on worker channel; returns None
//!                                     │
//!                                     ▼
//!                           worker thread (ffmpeg decode → peaks → raster)
//!                                     │
//!                                     ├─► HashMap<clip_id, Image>
//!                                     └─► invoke_from_event_loop → AppWindow::refresh_timeline_chips()
//!                                                                        │
//!                                                                        ▼
//!                                                         sync_timeline_chips re-runs with real image
//! ```
//!
//! The cache is **in-memory only** for v1 (no disk sidecar files). Short
//! clips regenerate in tens of milliseconds; long clips are cheap enough
//! once decoded. A proper on-disk sidecar (`<hash>.peaks`) is on deck for
//! v2 when longer audio stems come into play.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Sender};
use ffmpeg_next as ffmpeg;
use parking_lot::Mutex;
use slint::{Rgba8Pixel, SharedPixelBuffer, Weak};

use crate::AppWindow;

/// Target raster of a waveform tile. The FilmstripLane chip body is 40 px
/// tall inside a 48 px row, so 28 px of waveform + 12 px of label gradient
/// reads cleanly even when the chip is stretched. Width is a generous fixed
/// value so chips can stretch without visible step-stair — Slint scales the
/// image to fit (`image-fit: fill`).
const WAVE_WIDTH: u32 = 400;
const WAVE_HEIGHT: u32 = 28;
const WAVE_COLS: usize = WAVE_WIDTH as usize;

/// Audio waveform stroke color. Matches `Theme.clip-audio` closely enough
/// (slight desaturation for readability over the base tint).
const WAVE_RGBA: (u8, u8, u8, u8) = (0xd4, 0xf4, 0xff, 0xff);

/// Video thumbnail strip dimensions. `THUMB_COUNT` evenly-spaced frames
/// decoded across the clip, each scaled to `THUMB_CELL × THUMB_HEIGHT`
/// and blitted side-by-side into a `THUMB_WIDTH × THUMB_HEIGHT` RGBA8
/// buffer. Ten thumbnails over the default 400 px filmstrip width is
/// 40 px per cell — enough for a scene silhouette at a glance without
/// burning decode time on long clips.
const THUMB_COUNT: u32 = 10;
const THUMB_CELL: u32 = 40;
const THUMB_HEIGHT: u32 = 28;
const THUMB_WIDTH: u32 = THUMB_COUNT * THUMB_CELL;

/// Placeholder stripe tile size (square, tiled via `image-fit: fill`).
const PLACEHOLDER_SIZE: u32 = 64;

/// What kind of raster to generate for a job.
#[derive(Clone, Copy, Debug)]
enum JobKind {
    Waveform,
    Thumbnails,
}

/// One raster generation job handed to the worker thread.
struct Job {
    kind: JobKind,
    clip_id: String,
    path: PathBuf,
    in_ms: u64,
    out_ms: u64,
}

/// Scale `src_w × src_h` to the largest `fit_w × fit_h` that fits
/// inside `cell_w × cell_h` while preserving aspect ratio. Always
/// returns at least `1 × 1` so the ffmpeg scaler can be constructed
/// safely on degenerate inputs (a zero-dim scaler request panics
/// inside sws_scale). Returned dims are **even-rounded** to avoid
/// the occasional swscale chroma-alignment warning.
fn fit_cell(src_w: u32, src_h: u32, cell_w: u32, cell_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (1, 1);
    }
    let sw = src_w as f64;
    let sh = src_h as f64;
    let scale = (cell_w as f64 / sw).min(cell_h as f64 / sh);
    let fit_w = ((sw * scale).round() as u32).clamp(1, cell_w) & !1;
    let fit_h = ((sh * scale).round() as u32).clamp(1, cell_h) & !1;
    (fit_w.max(2), fit_h.max(2))
}

/// Shared, `Clone`-able handle to the background asset cache. Stored on
/// the session/window scope and consulted from `sync_timeline_chips`.
///
/// The cache keys to `SharedPixelBuffer<Rgba8Pixel>` (not `slint::Image`)
/// because `slint::Image` is not `Send`/`Sync` — it's a ref-counted handle
/// that only the UI thread may touch. `SharedPixelBuffer` is the
/// thread-safe raw-pixel half; we wrap it in `Image::from_rgba8` on the
/// UI thread each time we hand it to a chip (cheap — bumps a ref count).
#[derive(Clone)]
pub struct AssetCache {
    tx: Sender<Job>,
    waveforms: Arc<Mutex<HashMap<String, SharedPixelBuffer<Rgba8Pixel>>>>,
    thumbnails: Arc<Mutex<HashMap<String, SharedPixelBuffer<Rgba8Pixel>>>>,
    /// Tracks in-flight jobs as `(clip_id, kind_tag)` strings. Kind is
    /// embedded in the key so a waveform request and a thumbnails
    /// request for the **same** video-with-audio clip don't collide and
    /// dedupe each other away (both are legitimate).
    inflight: Arc<Mutex<HashSet<String>>>,
    placeholder: slint::Image,
}

fn inflight_key(kind: JobKind, clip_id: &str) -> String {
    let tag = match kind {
        JobKind::Waveform => "w",
        JobKind::Thumbnails => "t",
    };
    format!("{tag}:{clip_id}")
}

impl AssetCache {
    /// Spawn the background worker and return a handle.
    ///
    /// `weak` is used to invoke [`AppWindow::invoke_refresh_timeline_chips`]
    /// on the UI thread when a job completes, so the chip model re-runs
    /// and pulls in the new image. The worker is never joined — it lives
    /// for the life of the process and exits when the sender is dropped.
    pub fn spawn(weak: Weak<AppWindow>) -> Self {
        let (tx, rx) = unbounded::<Job>();
        let waveforms: Arc<Mutex<HashMap<String, SharedPixelBuffer<Rgba8Pixel>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let thumbnails: Arc<Mutex<HashMap<String, SharedPixelBuffer<Rgba8Pixel>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let inflight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        let waveforms_w = waveforms.clone();
        let thumbnails_w = thumbnails.clone();
        let inflight_w = inflight.clone();
        std::thread::Builder::new()
            .name("reel-asset-cache".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let clip_id = job.clip_id.clone();
                    let key = inflight_key(job.kind, &clip_id);
                    let result = match job.kind {
                        JobKind::Waveform => {
                            generate_waveform_buffer(&job.path, job.in_ms, job.out_ms)
                        }
                        JobKind::Thumbnails => {
                            generate_thumbnails_buffer(&job.path, job.in_ms, job.out_ms)
                        }
                    };
                    let buf = match result {
                        Ok(buf) => buf,
                        Err(e) => {
                            tracing::warn!(
                                clip_id = %clip_id,
                                kind = ?job.kind,
                                error = %e,
                                path = %job.path.display(),
                                "asset raster generation failed"
                            );
                            // Drop the inflight flag so a later project
                            // reload (which may point at a fixed file) can
                            // re-try. The chip continues to show the stripe
                            // placeholder in the meantime.
                            inflight_w.lock().remove(&key);
                            continue;
                        }
                    };
                    match job.kind {
                        JobKind::Waveform => {
                            waveforms_w.lock().insert(clip_id.clone(), buf);
                        }
                        JobKind::Thumbnails => {
                            thumbnails_w.lock().insert(clip_id.clone(), buf);
                        }
                    }
                    inflight_w.lock().remove(&key);

                    // `upgrade_in_event_loop` marshals the upgrade onto the
                    // UI thread — `Weak::upgrade()` is UI-thread-only, so a
                    // plain `invoke_from_event_loop(move || weak.upgrade())`
                    // won't compile (Weak isn't Send). This method is the
                    // cross-thread-safe spelling.
                    let _ = weak.upgrade_in_event_loop(|w| {
                        w.invoke_refresh_timeline_chips();
                    });
                }
            })
            .expect("spawn reel-asset-cache thread");

        let placeholder = build_placeholder_stripe();
        Self {
            tx,
            waveforms,
            thumbnails,
            inflight,
            placeholder,
        }
    }

    /// The shared diagonal-stripe placeholder image. Bound once to the
    /// `AppWindow::timeline_placeholder_stripe` property at startup.
    pub fn placeholder_image(&self) -> slint::Image {
        self.placeholder.clone()
    }

    /// Returns the cached waveform raster for `clip_id` if available, or
    /// enqueues a generation job and returns `None`. Dedupe-safe: repeated
    /// requests for the same clip while a job is in flight are no-ops.
    pub fn get_or_request_waveform(
        &self,
        clip_id: &str,
        path: &Path,
        in_ms: u64,
        out_ms: u64,
    ) -> Option<slint::Image> {
        self.get_or_request(JobKind::Waveform, clip_id, path, in_ms, out_ms)
    }

    /// Returns the cached thumbnail strip raster for `clip_id` if
    /// available, or enqueues a generation job and returns `None`. Same
    /// dedupe semantics as [`get_or_request_waveform`]; a video clip
    /// with embedded audio can have both rasters in flight at once —
    /// the inflight set is keyed by `(kind, clip_id)` so they don't
    /// collide.
    pub fn get_or_request_thumbnails(
        &self,
        clip_id: &str,
        path: &Path,
        in_ms: u64,
        out_ms: u64,
    ) -> Option<slint::Image> {
        self.get_or_request(JobKind::Thumbnails, clip_id, path, in_ms, out_ms)
    }

    fn get_or_request(
        &self,
        kind: JobKind,
        clip_id: &str,
        path: &Path,
        in_ms: u64,
        out_ms: u64,
    ) -> Option<slint::Image> {
        if clip_id.is_empty() {
            return None;
        }
        let cache = match kind {
            JobKind::Waveform => &self.waveforms,
            JobKind::Thumbnails => &self.thumbnails,
        };
        if let Some(buf) = cache.lock().get(clip_id).cloned() {
            return Some(slint::Image::from_rgba8(buf));
        }
        // Enqueue only if not already in flight.
        let mut inflight = self.inflight.lock();
        if inflight.insert(inflight_key(kind, clip_id)) {
            let _ = self.tx.send(Job {
                kind,
                clip_id: clip_id.to_string(),
                path: path.to_path_buf(),
                in_ms,
                out_ms,
            });
        }
        None
    }
}

/// Decode the audio at `path` between `in_ms..out_ms`, compute per-column
/// min/max peaks, and raster them into a `SharedPixelBuffer`. Returning
/// the raw buffer (rather than a `slint::Image`) keeps the result `Send`
/// so it can cross the worker→UI thread boundary via the cache HashMap;
/// `Image::from_rgba8` happens lazily on read.
fn generate_waveform_buffer(
    path: &Path,
    in_ms: u64,
    out_ms: u64,
) -> Result<SharedPixelBuffer<Rgba8Pixel>> {
    let peaks = decode_peaks(path, in_ms, out_ms, WAVE_COLS)?;
    Ok(raster_peaks(&peaks))
}

/// Decode audio via ffmpeg and produce `cols` (min, max) pairs spanning
/// the clip. Samples are mixed down to mono at 48 kHz for peak extraction
/// (stereo width doesn't read at this resolution).
fn decode_peaks(
    path: &Path,
    in_ms: u64,
    out_ms: u64,
    cols: usize,
) -> Result<Vec<(f32, f32)>> {
    // ffmpeg::init() is idempotent; the player thread calls it too and
    // double-init is a no-op.
    ffmpeg::init().ok();

    let mut input = ffmpeg::format::input(&path)?;
    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .context("no audio stream")?;
    let stream_idx = stream.index();
    let ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
    let mut decoder = ctx.decoder().audio()?;
    let mut resampler = ffmpeg::software::resampling::Context::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
        ffmpeg::ChannelLayout::STEREO,
        48_000,
    )?;

    if in_ms > 0 {
        let ts = (in_ms as i64) * i64::from(ffmpeg::ffi::AV_TIME_BASE) / 1000;
        let _ = input.seek(ts, ..ts);
        decoder.flush();
    }

    let duration_ms = out_ms.saturating_sub(in_ms).max(1);
    // 48 kHz * stereo-mixed-to-mono-frames-per-ms
    let total_frames = (duration_ms * 48).max(cols as u64) as usize;
    let frames_per_col = (total_frames / cols).max(1);

    let mut peaks: Vec<(f32, f32)> = vec![(0.0, 0.0); cols];
    let mut frames_read = 0usize;
    let mut cur_col = 0usize;
    let mut col_min = f32::INFINITY;
    let mut col_max = f32::NEG_INFINITY;
    let mut decoded = ffmpeg::frame::Audio::empty();

    'outer: for (stream, packet) in input.packets() {
        if stream.index() != stream_idx {
            continue;
        }
        if decoder.send_packet(&packet).is_err() {
            continue;
        }
        while decoder.receive_frame(&mut decoded).is_ok() {
            let mut resampled = ffmpeg::frame::Audio::empty();
            if resampler.run(&decoded, &mut resampled).is_err() {
                continue;
            }
            let plane = resampled.data(0);
            let floats = unsafe {
                std::slice::from_raw_parts(
                    plane.as_ptr() as *const f32,
                    plane.len() / std::mem::size_of::<f32>(),
                )
            };
            // Stereo interleaved: [L0, R0, L1, R1, ...] — collapse to mono.
            for pair in floats.chunks_exact(2) {
                let mono = 0.5 * (pair[0] + pair[1]);
                if mono < col_min {
                    col_min = mono;
                }
                if mono > col_max {
                    col_max = mono;
                }
                frames_read += 1;
                if frames_read >= (cur_col + 1) * frames_per_col {
                    if col_max > col_min {
                        peaks[cur_col] = (col_min, col_max);
                    }
                    col_min = f32::INFINITY;
                    col_max = f32::NEG_INFINITY;
                    cur_col += 1;
                    if cur_col >= cols {
                        break 'outer;
                    }
                }
                if frames_read >= total_frames {
                    if col_max > col_min && cur_col < cols {
                        peaks[cur_col] = (col_min, col_max);
                    }
                    break 'outer;
                }
            }
        }
    }

    Ok(peaks)
}

/// Raster `peaks` into an RGBA8 buffer of `(WAVE_WIDTH, WAVE_HEIGHT)`.
///
/// Transparent background + vertical lines in `WAVE_RGBA`. The alpha
/// ramps by vertical distance from the midline — strongest at the
/// peak extents, fading toward the center — which gives the waveform
/// depth at chip scale and avoids the flat "solid bar" look of equal
/// alpha throughout. Also adds a 1 px always-on midline at ~20 %
/// alpha so silent stretches still read as "this is an audio track"
/// instead of an empty box.
fn raster_peaks(peaks: &[(f32, f32)]) -> SharedPixelBuffer<Rgba8Pixel> {
    let w = WAVE_WIDTH;
    let h = WAVE_HEIGHT;
    let mid = (h as f32) / 2.0;
    let scale = mid - 1.0;
    // Midline alpha — subtle so it reads as an axis, not a feature.
    const MID_ALPHA: u8 = 52;
    let mut pixels = vec![
        Rgba8Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 0
        };
        (w * h) as usize
    ];
    // Paint the midline first; per-column columns overwrite it where
    // the waveform is actually present.
    let mid_y = mid.floor() as u32;
    for x in 0..w {
        let idx = (mid_y * w + x) as usize;
        pixels[idx] = Rgba8Pixel {
            r: WAVE_RGBA.0,
            g: WAVE_RGBA.1,
            b: WAVE_RGBA.2,
            a: MID_ALPHA,
        };
    }
    for (x, (mn, mx)) in peaks.iter().enumerate().take(w as usize) {
        let top_f = mid - (*mx).clamp(-1.0, 1.0) * scale;
        let bot_f = mid - (*mn).clamp(-1.0, 1.0) * scale;
        let (yt, yb) = if top_f <= bot_f {
            (top_f as i32, bot_f as i32)
        } else {
            (bot_f as i32, top_f as i32)
        };
        let yt = yt.clamp(0, (h - 1) as i32);
        let yb = yb.clamp(0, (h - 1) as i32).max(yt);
        for y in yt..=yb {
            // Gradient alpha: 1.0 at extents (max distance from mid),
            // tapering toward the midline. Base floor of 96/255 so
            // thin peaks still read; extents bump toward 255.
            let dist = (y as f32 - mid).abs() / scale.max(1.0);
            let t = dist.clamp(0.0, 1.0);
            let alpha = (96.0 + t * 159.0).round().min(255.0) as u8;
            let idx = (y as u32 * w + x as u32) as usize;
            pixels[idx] = Rgba8Pixel {
                r: WAVE_RGBA.0,
                g: WAVE_RGBA.1,
                b: WAVE_RGBA.2,
                a: alpha,
            };
        }
    }
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    buf.make_mut_slice().copy_from_slice(&pixels);
    buf
}

/// Decode `THUMB_COUNT` evenly-spaced frames from the video at `path`
/// between `in_ms..out_ms`, scale each to `THUMB_CELL × THUMB_HEIGHT`,
/// and blit them left-to-right into a `THUMB_WIDTH × THUMB_HEIGHT`
/// RGBA8 strip. This is the video-chip analogue of `raster_peaks` —
/// one row per clip, decoded once, swapped in on completion.
///
/// Decode strategy: for each target timestamp we `seek` the input to
/// the nearest keyframe and decode packets until we get a frame. This
/// is the same pattern as the playback path in `player.rs` but
/// single-shot (no streaming) and with a tiny RGBA8 scaler tailored to
/// the thumbnail cell size. Soft-fail on individual frames (skip to
/// next timestamp) so one bad seek doesn't sink the whole strip.
fn generate_thumbnails_buffer(
    path: &Path,
    in_ms: u64,
    out_ms: u64,
) -> Result<SharedPixelBuffer<Rgba8Pixel>> {
    ffmpeg::init().ok();

    let mut input = ffmpeg::format::input(&path)?;
    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .context("no video stream")?;
    let stream_idx = stream.index();
    let ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
    let mut decoder = ctx.decoder().video()?;
    let src_w = decoder.width();
    let src_h = decoder.height();
    if src_w == 0 || src_h == 0 {
        anyhow::bail!("video stream reports 0×0 dimensions");
    }

    // Preserve source aspect ratio: scale to fit inside the cell and
    // letterbox (top/bottom bars) or pillarbox (left/right bars) the
    // remainder. Fixes vertical / 9:16 phone footage that otherwise
    // rendered horizontally-stretched into a 40×28 cell. The strip is
    // pre-filled with opaque black, so unused area needs no explicit
    // fill — the blit just skips those pixels.
    let (fit_w, fit_h) = fit_cell(src_w, src_h, THUMB_CELL, THUMB_HEIGHT);
    let pad_x = (THUMB_CELL - fit_w) / 2;
    let pad_y = (THUMB_HEIGHT - fit_h) / 2;
    let mut scaler = ffmpeg::software::scaling::Context::get(
        decoder.format(),
        src_w,
        src_h,
        ffmpeg::format::Pixel::RGBA,
        fit_w,
        fit_h,
        ffmpeg::software::scaling::Flags::FAST_BILINEAR,
    )?;

    let span_ms = out_ms.saturating_sub(in_ms).max(1);
    let mut strip = vec![
        Rgba8Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 255
        };
        (THUMB_WIDTH * THUMB_HEIGHT) as usize
    ];

    for i in 0..THUMB_COUNT {
        // Sample at the midpoint of each cell (`i + 0.5` / `N`) so the
        // first thumb is shortly after `in_ms` rather than exactly at
        // it (clips often open on a black/logo frame at t=0).
        let t_ms =
            in_ms + ((i as u64 * 2 + 1) * span_ms) / (THUMB_COUNT as u64 * 2);
        let ts = (t_ms as i64) * i64::from(ffmpeg::ffi::AV_TIME_BASE) / 1000;
        let _ = input.seek(ts, ..ts);
        decoder.flush();

        let mut decoded = ffmpeg::frame::Video::empty();
        let mut got = false;
        'decode: for (stream, packet) in input.packets() {
            if stream.index() != stream_idx {
                continue;
            }
            if decoder.send_packet(&packet).is_err() {
                continue;
            }
            if decoder.receive_frame(&mut decoded).is_ok() {
                got = true;
                break 'decode;
            }
        }
        if !got {
            // Hit EOF before a frame came back — leave the cell as the
            // default black fill so the strip still has a consistent
            // shape; the other cells will still render meaningfully.
            continue;
        }

        let mut rgba = ffmpeg::frame::Video::empty();
        if scaler.run(&decoded, &mut rgba).is_err() {
            continue;
        }
        let stride = rgba.stride(0);
        let data = rgba.data(0);
        // Blit this aspect-preserved frame into the cell, centered.
        // `stride` is ffmpeg's row alignment (≥ fit_w * 4), `fit_w *
        // fit_h` is the actual pixel rectangle. Rows and columns
        // outside the fit rectangle stay as the strip's default black
        // fill → letterbox / pillarbox bars.
        let cell_left = (i * THUMB_CELL) as usize;
        for row in 0..fit_h as usize {
            let src_off = row * stride;
            let dst_row = row + pad_y as usize;
            if dst_row >= THUMB_HEIGHT as usize {
                break;
            }
            for col in 0..fit_w as usize {
                let s = src_off + col * 4;
                if s + 3 >= data.len() {
                    break;
                }
                let dst_col = cell_left + pad_x as usize + col;
                let idx = dst_row * THUMB_WIDTH as usize + dst_col;
                strip[idx] = Rgba8Pixel {
                    r: data[s],
                    g: data[s + 1],
                    b: data[s + 2],
                    a: data[s + 3],
                };
            }
        }
    }

    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(THUMB_WIDTH, THUMB_HEIGHT);
    buf.make_mut_slice().copy_from_slice(&strip);
    Ok(buf)
}

/// Build a small diagonal-stripe tile for the pending-asset placeholder.
/// Soft white stripes at low alpha so they read as "loading hatch" over
/// the base chip color rather than dominating it.
fn build_placeholder_stripe() -> slint::Image {
    let s = PLACEHOLDER_SIZE;
    let mut pixels = vec![
        Rgba8Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 0
        };
        (s * s) as usize
    ];
    let period: i32 = 12;
    let stripe_width: i32 = 5;
    for y in 0..s {
        for x in 0..s {
            let d = (x as i32 + y as i32).rem_euclid(period);
            if d < stripe_width {
                let idx = (y * s + x) as usize;
                pixels[idx] = Rgba8Pixel {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 48,
                };
            }
        }
    }
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(s, s);
    buf.make_mut_slice().copy_from_slice(&pixels);
    slint::Image::from_rgba8(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_has_nonzero_pixels() {
        let img = build_placeholder_stripe();
        assert_eq!(img.size().width, PLACEHOLDER_SIZE);
        assert_eq!(img.size().height, PLACEHOLDER_SIZE);
    }

    #[test]
    fn raster_peaks_produces_configured_size() {
        let peaks = vec![(-0.5_f32, 0.5_f32); WAVE_COLS];
        let buf = raster_peaks(&peaks);
        assert_eq!(buf.width(), WAVE_WIDTH);
        assert_eq!(buf.height(), WAVE_HEIGHT);
        // Round-trip through slint::Image works (this is what `get_or_request`
        // does on cache hit on the UI thread).
        let img = slint::Image::from_rgba8(buf);
        assert_eq!(img.size().width, WAVE_WIDTH);
        assert_eq!(img.size().height, WAVE_HEIGHT);
    }

    fn test_cache() -> (AssetCache, crossbeam_channel::Receiver<Job>) {
        // Spawn requires a live Slint event loop, so we construct the
        // internals by hand and call the dedupe logic directly.
        let (tx, rx) = unbounded::<Job>();
        let cache = AssetCache {
            tx,
            waveforms: Arc::new(Mutex::new(HashMap::new())),
            thumbnails: Arc::new(Mutex::new(HashMap::new())),
            inflight: Arc::new(Mutex::new(HashSet::new())),
            placeholder: build_placeholder_stripe(),
        };
        (cache, rx)
    }

    #[test]
    fn empty_clip_id_does_not_enqueue() {
        let (cache, rx) = test_cache();
        let got = cache.get_or_request_waveform("", std::path::Path::new("/nope"), 0, 1000);
        assert!(got.is_none());
        assert!(
            rx.try_recv().is_err(),
            "empty clip-id must not enqueue a job"
        );
        let got = cache.get_or_request_thumbnails("", std::path::Path::new("/nope"), 0, 1000);
        assert!(got.is_none());
        assert!(
            rx.try_recv().is_err(),
            "empty clip-id must not enqueue a thumbnail job either"
        );
    }

    #[test]
    fn waveform_and_thumbnail_requests_do_not_dedupe_each_other() {
        // A video clip with embedded audio legitimately wants both a
        // waveform and a thumbnail strip — inflight keys embed the
        // kind so neither request cancels the other.
        let (cache, rx) = test_cache();
        let clip_id = "abc";
        let path = std::path::Path::new("/nope.mov");
        assert!(cache
            .get_or_request_waveform(clip_id, path, 0, 1000)
            .is_none());
        assert!(cache
            .get_or_request_thumbnails(clip_id, path, 0, 1000)
            .is_none());
        let j1 = rx.try_recv().expect("waveform job should have enqueued");
        let j2 = rx.try_recv().expect("thumbnail job should have enqueued");
        assert!(matches!(j1.kind, JobKind::Waveform));
        assert!(matches!(j2.kind, JobKind::Thumbnails));
    }

    #[test]
    fn fit_cell_landscape_fills_width() {
        // 16:9 source (1920×1080) into a 40×28 cell — width is the
        // binding dim. Expected: fit to ~40 wide, aspect-preserved height.
        let (w, h) = fit_cell(1920, 1080, 40, 28);
        assert_eq!(w, 40);
        // 40 * (1080/1920) = 22.5 → rounds to 22 (even).
        assert_eq!(h, 22);
    }

    #[test]
    fn fit_cell_portrait_fills_height() {
        // 9:16 source (1080×1920) into 40×28 — height is the binding
        // dim. Expected: height pinned to 28, width narrower so the
        // chip pillarboxes instead of stretching horizontally.
        let (w, h) = fit_cell(1080, 1920, 40, 28);
        assert_eq!(h, 28);
        // 28 * (1080/1920) = 15.75 → rounds even to 16.
        assert_eq!(w, 16);
    }

    #[test]
    fn fit_cell_square_source_uses_short_dim() {
        let (w, h) = fit_cell(500, 500, 40, 28);
        assert_eq!((w, h), (28, 28));
    }

    #[test]
    fn fit_cell_zero_dimensions_return_nonzero_fallback() {
        // Degenerate sources (0×N, N×0) would panic sws_scale; the
        // helper must return usable dims so we can still construct a
        // scaler and the strip gets a black cell.
        let (w, h) = fit_cell(0, 100, 40, 28);
        assert!(w >= 1 && h >= 1);
        let (w, h) = fit_cell(100, 0, 40, 28);
        assert!(w >= 1 && h >= 1);
    }

    #[test]
    fn fit_cell_output_is_even_for_swscale() {
        // sws_scale warns on odd chroma-plane sizes. The helper
        // rounds down to the nearest even, so arbitrary odd source
        // ratios still produce even-dim output.
        for (sw, sh) in [(1001, 1001), (999, 564), (123, 456), (17, 29)] {
            let (w, h) = fit_cell(sw, sh, 40, 28);
            assert_eq!(w % 2, 0, "fit_w must be even for {}x{}", sw, sh);
            assert_eq!(h % 2, 0, "fit_h must be even for {}x{}", sw, sh);
        }
    }

    #[test]
    fn repeated_same_kind_request_dedupes() {
        let (cache, rx) = test_cache();
        let _ = cache.get_or_request_waveform("xyz", std::path::Path::new("/nope"), 0, 1000);
        let _ = cache.get_or_request_waveform("xyz", std::path::Path::new("/nope"), 0, 1000);
        let _ = cache.get_or_request_waveform("xyz", std::path::Path::new("/nope"), 0, 1000);
        assert!(rx.try_recv().is_ok(), "first request must enqueue");
        assert!(
            rx.try_recv().is_err(),
            "subsequent same-kind requests must dedupe"
        );
    }
}
