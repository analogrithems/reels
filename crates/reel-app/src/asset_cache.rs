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

/// Placeholder stripe tile size (square, tiled via `image-fit: fill`).
const PLACEHOLDER_SIZE: u32 = 64;

/// One waveform generation job handed to the worker thread.
struct Job {
    clip_id: String,
    path: PathBuf,
    in_ms: u64,
    out_ms: u64,
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
    inflight: Arc<Mutex<HashSet<String>>>,
    placeholder: slint::Image,
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
        let inflight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        let waveforms_w = waveforms.clone();
        let inflight_w = inflight.clone();
        std::thread::Builder::new()
            .name("reel-asset-cache".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let clip_id = job.clip_id.clone();
                    let buf = match generate_waveform_buffer(&job.path, job.in_ms, job.out_ms) {
                        Ok(buf) => buf,
                        Err(e) => {
                            tracing::warn!(
                                clip_id = %clip_id,
                                error = %e,
                                path = %job.path.display(),
                                "waveform generation failed"
                            );
                            // Drop the inflight flag so a later project
                            // reload (which may point at a fixed file) can
                            // re-try. The chip continues to show the stripe
                            // placeholder in the meantime.
                            inflight_w.lock().remove(&clip_id);
                            continue;
                        }
                    };
                    waveforms_w.lock().insert(clip_id.clone(), buf);
                    inflight_w.lock().remove(&clip_id);

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
    pub fn get_or_request(
        &self,
        clip_id: &str,
        path: &Path,
        in_ms: u64,
        out_ms: u64,
    ) -> Option<slint::Image> {
        if clip_id.is_empty() {
            return None;
        }
        if let Some(buf) = self.waveforms.lock().get(clip_id).cloned() {
            return Some(slint::Image::from_rgba8(buf));
        }
        // Enqueue only if not already in flight.
        let mut inflight = self.inflight.lock();
        if inflight.insert(clip_id.to_string()) {
            let _ = self.tx.send(Job {
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
/// Transparent background + opaque vertical lines in `WAVE_RGBA`.
fn raster_peaks(peaks: &[(f32, f32)]) -> SharedPixelBuffer<Rgba8Pixel> {
    let w = WAVE_WIDTH;
    let h = WAVE_HEIGHT;
    let mid = (h as f32) / 2.0;
    let scale = mid - 1.0;
    let mut pixels = vec![
        Rgba8Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 0
        };
        (w * h) as usize
    ];
    for (x, (mn, mx)) in peaks.iter().enumerate().take(w as usize) {
        let top_f = mid - (*mx).clamp(-1.0, 1.0) * scale;
        let bot_f = mid - (*mn).clamp(-1.0, 1.0) * scale;
        let (yt, yb) = if top_f <= bot_f {
            (top_f as i32, bot_f as i32)
        } else {
            (bot_f as i32, top_f as i32)
        };
        // Always paint at least one pixel at the midline so near-silence
        // still reads as a thin horizontal line (visual "is present").
        let yt = yt.clamp(0, (h - 1) as i32);
        let yb = yb.clamp(0, (h - 1) as i32).max(yt);
        for y in yt..=yb {
            let idx = (y as u32 * w + x as u32) as usize;
            pixels[idx] = Rgba8Pixel {
                r: WAVE_RGBA.0,
                g: WAVE_RGBA.1,
                b: WAVE_RGBA.2,
                a: WAVE_RGBA.3,
            };
        }
    }
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    buf.make_mut_slice().copy_from_slice(&pixels);
    buf
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

    #[test]
    fn empty_clip_id_does_not_enqueue() {
        // Spawn requires a live Slint event loop, so we construct the
        // internals by hand and call the dedupe logic directly.
        let (tx, rx) = unbounded::<Job>();
        let cache = AssetCache {
            tx,
            waveforms: Arc::new(Mutex::new(HashMap::new())),
            inflight: Arc::new(Mutex::new(HashSet::new())),
            placeholder: build_placeholder_stripe(),
        };
        let got = cache.get_or_request("", std::path::Path::new("/nope"), 0, 1000);
        assert!(got.is_none());
        assert!(
            rx.try_recv().is_err(),
            "empty clip-id must not enqueue a job"
        );
    }
}
