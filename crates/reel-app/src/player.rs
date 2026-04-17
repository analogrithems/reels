//! Decoder + audio-out plumbing for the Slint window.
//!
//! # Threading model
//!
//! ```text
//!   UI thread (Slint event loop)
//!        │   Weak<AppWindow>
//!        ▼
//!   [player_cmd_tx] ──┐
//!                     │       [video thread]                      [audio thread]
//!                     ├─► DecodeCmd → video frame → invoke_from_event_loop
//!                     └─► DecodeCmd → audio decode → ringbuf ──► cpal output stream
//!                                         ▲
//!                                         │ AudioClock (AtomicU64 ms)
//!                                         │
//!                               video thread consults clock
//!                               to decide sleep / drop / present
//! ```
//!
//! Video is the *display* channel; audio is the *clock*. On Seek we flush
//! both and reset the clock.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use ffmpeg_next as ffmpeg;
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use slint::{ComponentHandle, Image, SharedPixelBuffer, Weak};

use crate::timecode::apply_playhead_transport;
use crate::timeline::TimelineSync;
use crate::ui_bridge::on_ui;
use crate::AppWindow;

#[derive(Clone)]
pub enum Cmd {
    /// Primary video track: decode across multiple clip spans in sequence time.
    LoadTimeline {
        sync: Arc<TimelineSync>,
    },
    /// Drop the current source and reset transport (no decoder teardown of threads).
    Close,
    Play,
    Pause,
    /// Seek in concatenated sequence milliseconds (primary track).
    SeekSequence {
        seq_ms: u64,
    },
    Stop,
}

/// Audio output sample rate + channel count. Fixed for simplicity; `cpal`
/// is configured to match. The `Mixer` resamples per-source audio into this.
const OUT_SAMPLE_RATE: u32 = 48_000;
const OUT_CHANNELS: u16 = 2;

/// Shared playback clock, in milliseconds since the current source start.
/// Advanced by the audio thread as samples are written to the output stream.
#[derive(Clone, Debug, Default)]
pub struct AudioClock {
    pos_ms: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
}

impl AudioClock {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, ms: u64) {
        self.pos_ms.store(ms, Ordering::Release);
    }
    pub fn get(&self) -> u64 {
        self.pos_ms.load(Ordering::Acquire)
    }
    pub fn set_playing(&self, p: bool) {
        self.playing.store(p, Ordering::Release);
    }
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }
}

/// Handle to a running player. Dropping it stops and joins both threads.
pub struct PlayerHandle {
    tx_video: Sender<Cmd>,
    tx_audio: Sender<Cmd>,
    video_thread: Option<JoinHandle<()>>,
    audio_thread: Option<JoinHandle<()>>,
}

impl PlayerHandle {
    /// Hand out a clone of the command sender so it can be captured by UI
    /// callbacks without borrowing the whole handle.
    pub fn cmd_sender(&self) -> PlayerCmdSender {
        PlayerCmdSender {
            tx_video: self.tx_video.clone(),
            tx_audio: self.tx_audio.clone(),
        }
    }
}

/// Cloneable sender that fans out each command to both the video and audio
/// threads. A single `crossbeam_channel` with two cloned receivers would
/// deliver each message to only one thread, which is not what we want.
#[derive(Clone)]
pub struct PlayerCmdSender {
    tx_video: Sender<Cmd>,
    tx_audio: Sender<Cmd>,
}

impl PlayerCmdSender {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx_video.send(cmd.clone());
        let _ = self.tx_audio.send(cmd);
    }
}

impl Drop for PlayerHandle {
    fn drop(&mut self) {
        let _ = self.tx_video.send(Cmd::Stop);
        let _ = self.tx_audio.send(Cmd::Stop);
        if let Some(t) = self.video_thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.audio_thread.take() {
            let _ = t.join();
        }
    }
}

/// Start the player; returns a handle plus the AudioClock for external wiring.
pub fn spawn_player(window: &AppWindow) -> Result<PlayerHandle> {
    // Initialize ffmpeg once.
    ffmpeg::init().context("ffmpeg::init")?;

    let (tx_video, rx_video) = crossbeam_channel::unbounded::<Cmd>();
    let (tx_audio, rx_audio) = crossbeam_channel::unbounded::<Cmd>();

    let clock = AudioClock::new();

    // Ring buffer: ~400 ms of audio at 48 kHz stereo f32.
    let rb: HeapRb<f32> = HeapRb::new((OUT_SAMPLE_RATE as usize) * OUT_CHANNELS as usize / 2);
    let (producer, consumer) = rb.split();

    let weak = window.as_weak();
    let weak_v = weak.clone();
    let clock_v = clock.clone();
    let video_thread = std::thread::Builder::new()
        .name("reel-video".into())
        .spawn(move || video_loop(rx_video, weak_v, clock_v))?;

    let weak_a = weak.clone();
    let clock_a = clock.clone();
    let audio_thread = std::thread::Builder::new()
        .name("reel-audio".into())
        .spawn(move || audio_loop(rx_audio, clock_a, producer, consumer, weak_a))?;

    Ok(PlayerHandle {
        tx_video,
        tx_audio,
        video_thread: Some(video_thread),
        audio_thread: Some(audio_thread),
    })
}

// ---------- video thread ----------

fn video_loop(rx: Receiver<Cmd>, weak: Weak<AppWindow>, clock: AudioClock) {
    let mut ctx: Option<VideoCtx> = None;
    let mut playing = false;
    let mut timeline: Option<Arc<TimelineSync>> = None;
    let mut video_seg_idx: usize = 0;

    loop {
        // Block for a command when paused / empty; otherwise poll.
        let cmd = if playing && ctx.is_some() {
            rx.try_recv().ok()
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };

        if let Some(c) = cmd {
            match c {
                Cmd::LoadTimeline { sync } => {
                    ctx = None;
                    playing = false;
                    clock.set_playing(false);
                    clock.set(0);
                    timeline = Some(sync.clone());
                    video_seg_idx = 0;
                    sync.active_index.store(0, Ordering::SeqCst);
                    let s0 = &sync.segments[0];
                    {
                        let disp = s0.path.display().to_string();
                        on_ui(weak.clone(), move |w| {
                            w.set_media_ready(false);
                            w.set_is_playing(false);
                            w.set_status_text(format!("Loading {disp}…").into());
                        });
                    }
                    match try_open_video(&s0.path) {
                        Ok(mut new_ctx) => {
                            if let Err(e) = new_ctx.seek(s0.media_in_ms) {
                                tracing::warn!(error = %e, "video initial seek failed");
                            }
                            if let Some(frame) = new_ctx.next_presentable_frame() {
                                present(&weak, frame);
                            }
                            let total = sync.total_sequence_ms();
                            ctx = Some(new_ctx);
                            on_ui(weak.clone(), move |w| {
                                w.set_status_text(format!("Ready ({total} ms sequence)").into());
                                w.set_media_ready(true);
                                apply_playhead_transport(&w, 0.0);
                            });
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "video open failed");
                            timeline = None;
                            on_ui(weak.clone(), move |w| {
                                w.set_media_ready(false);
                                w.set_status_text(format!("Open failed: {e}").into());
                            });
                        }
                    }
                }
                Cmd::Play => {
                    if ctx.is_some() {
                        playing = true;
                        clock.set_playing(true);
                        on_ui(weak.clone(), |w| w.set_is_playing(true));
                    } else {
                        tracing::debug!("Play ignored: no media loaded");
                    }
                }
                Cmd::Pause => {
                    playing = false;
                    clock.set_playing(false);
                    on_ui(weak.clone(), |w| w.set_is_playing(false));
                }
                Cmd::SeekSequence { seq_ms } => {
                    let Some(ref t) = timeline else {
                        continue;
                    };
                    if let Some((idx, local_ms)) = t.resolve_seek(seq_ms) {
                        t.active_index.store(idx, Ordering::SeqCst);
                        video_seg_idx = idx;
                        let s = &t.segments[idx];
                        match try_open_video(&s.path) {
                            Ok(mut c) => {
                                if let Err(e) = c.seek(local_ms) {
                                    tracing::warn!(error = %e, seq_ms, "video seek failed");
                                }
                                if let Some(frame) = c.next_presentable_frame() {
                                    present(&weak, frame);
                                }
                                ctx = Some(c);
                                clock.set(seq_ms);
                                let sf = seq_ms as f32;
                                on_ui(weak.clone(), move |w| apply_playhead_transport(&w, sf));
                            }
                            Err(e) => tracing::warn!(error = %e, "video reopen seek failed"),
                        }
                    }
                }
                Cmd::Close => {
                    ctx = None;
                    timeline = None;
                    video_seg_idx = 0;
                    playing = false;
                    clock.set_playing(false);
                    clock.set(0);
                    on_ui(weak.clone(), |w| {
                        w.set_media_ready(false);
                        w.set_is_playing(false);
                        w.set_status_text("No media".into());
                        w.set_timecode("0:00.000 / 0:00.000".into());
                        w.set_playhead_ms(0.0);
                    });
                }
                Cmd::Stop => return,
            }
        }

        if playing {
            if let Some(c) = ctx.as_mut() {
                match c.next_presentable_frame() {
                    Some(frame) => {
                        let now = clock.get();
                        let frame_seq = if let Some(ref t) = timeline {
                            let seg = &t.segments[video_seg_idx];
                            seg.seq_start_ms
                                .saturating_add(frame.pts_ms.saturating_sub(seg.media_in_ms))
                        } else {
                            frame.pts_ms
                        };
                        if frame_seq > now + 5 {
                            let sleep = (frame_seq - now).min(50);
                            std::thread::sleep(Duration::from_millis(sleep));
                        } else if frame_seq + 40 < now {
                            continue;
                        }
                        present(&weak, frame);
                        let weak_c = weak.clone();
                        let ck = clock.clone();
                        on_ui(weak_c, move |w| {
                            apply_playhead_transport(&w, ck.get() as f32);
                        });
                    }
                    None => {
                        if !playing {
                            continue;
                        }
                        let Some(ref t) = timeline else {
                            playing = false;
                            clock.set_playing(false);
                            on_ui(weak.clone(), |w| w.set_is_playing(false));
                            continue;
                        };
                        let n = t.segments.len();
                        if video_seg_idx >= n.saturating_sub(1) {
                            playing = false;
                            clock.set_playing(false);
                            on_ui(weak.clone(), |w| w.set_is_playing(false));
                            continue;
                        }
                        let start = Instant::now();
                        let mut opened_next = false;
                        while start.elapsed() < Duration::from_secs(3) {
                            let ai = t.active_index.load(Ordering::SeqCst);
                            if ai > video_seg_idx {
                                video_seg_idx = ai;
                                let s = &t.segments[video_seg_idx];
                                match try_open_video(&s.path) {
                                    Ok(mut nc) => {
                                        if let Err(e) = nc.seek(s.media_in_ms) {
                                            tracing::warn!(error = %e, "video segment seek");
                                        }
                                        ctx = Some(nc);
                                        opened_next = true;
                                        break;
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "video segment open");
                                        playing = false;
                                        clock.set_playing(false);
                                        on_ui(weak.clone(), |w| w.set_is_playing(false));
                                        break;
                                    }
                                }
                            }
                            if !clock.is_playing() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        if !opened_next && playing && video_seg_idx < n.saturating_sub(1) {
                            playing = false;
                            clock.set_playing(false);
                            on_ui(weak.clone(), |w| w.set_is_playing(false));
                        }
                    }
                }
            }
        }
    }
}

/// Panic-safe wrapper around [`VideoCtx::open`].
///
/// `ffmpeg_next` error paths are normally `Result`-based, but a malformed or
/// partial container has in the past been observed to trip internal asserts
/// in some codecs. `catch_unwind` turns any such panic into a surfaced error
/// instead of a UI-thread crash.
fn try_open_video(path: &Path) -> Result<VideoCtx> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| VideoCtx::open(path))) {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!("panic while opening {}", path.display())),
    }
}

/// Panic-safe wrapper around [`AudioCtx::open`]. See [`try_open_video`].
fn try_open_audio(path: &Path) -> Result<AudioCtx> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| AudioCtx::open(path))) {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!(
            "panic while opening audio {}",
            path.display()
        )),
    }
}

fn present(weak: &Weak<AppWindow>, frame: VideoFrame) {
    // Build the SharedPixelBuffer off-thread (it's Send), hand it to the UI
    // thread, and only wrap it in a (non-Send) `slint::Image` inside the
    // event-loop closure.
    let pixbuf = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        &frame.rgba,
        frame.width,
        frame.height,
    );
    on_ui(weak.clone(), move |w| {
        w.set_current_frame(Image::from_rgba8(pixbuf));
    });
}

struct VideoFrame {
    pts_ms: u64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct VideoCtx {
    input: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Video,
    scaler: ffmpeg::software::scaling::Context,
    stream_idx: usize,
    time_base: ffmpeg::Rational,
    width: u32,
    height: u32,
}

impl VideoCtx {
    fn open(path: &Path) -> Result<Self> {
        let input = ffmpeg::format::input(&path).context("ffmpeg open")?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .context("no video stream")?;
        let stream_idx = stream.index();
        let time_base = stream.time_base();
        let ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
        let decoder = ctx.decoder().video()?;
        let width = decoder.width();
        let height = decoder.height();

        let scaler = ffmpeg::software::scaling::Context::get(
            decoder.format(),
            width,
            height,
            ffmpeg::format::Pixel::RGBA,
            width,
            height,
            ffmpeg::software::scaling::Flags::FAST_BILINEAR,
        )?;

        Ok(Self {
            input,
            decoder,
            scaler,
            stream_idx,
            time_base,
            width,
            height,
        })
    }

    fn seek(&mut self, target_ms: u64) -> Result<()> {
        let ts = (target_ms as i64) * i64::from(ffmpeg::ffi::AV_TIME_BASE) / 1000;
        self.input.seek(ts, ..ts).context("ffmpeg seek")?;
        self.decoder.flush();
        Ok(())
    }

    fn next_presentable_frame(&mut self) -> Option<VideoFrame> {
        let mut decoded = ffmpeg::frame::Video::empty();
        // Pull packets until the decoder hands us a frame (or EOF).
        while let Some((stream, packet)) = self.input.packets().next() {
            if stream.index() != self.stream_idx {
                continue;
            }
            if self.decoder.send_packet(&packet).is_err() {
                continue;
            }
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                return Some(self.frame_to_rgba(&mut decoded));
            }
        }
        // Flush decoder at EOF.
        let _ = self.decoder.send_eof();
        if self.decoder.receive_frame(&mut decoded).is_ok() {
            return Some(self.frame_to_rgba(&mut decoded));
        }
        None
    }

    fn frame_to_rgba(&mut self, src: &mut ffmpeg::frame::Video) -> VideoFrame {
        let pts = src.pts().unwrap_or(0);
        let pts_ms = (pts as f64 * f64::from(self.time_base.numerator())
            / f64::from(self.time_base.denominator())
            * 1000.0) as u64;

        let empty = VideoFrame {
            pts_ms,
            width: self.width,
            height: self.height,
            rgba: vec![0u8; (self.width as usize) * (self.height as usize) * 4],
        };

        let mut dst = ffmpeg::frame::Video::empty();
        dst.set_format(ffmpeg::format::Pixel::RGBA);
        dst.set_width(self.width);
        dst.set_height(self.height);
        if let Err(e) = self.scaler.run(src, &mut dst) {
            tracing::warn!(error = %e, "scaler run failed; presenting black frame");
            return empty;
        }

        let stride = dst.stride(0);
        let width_bytes = (self.width as usize) * 4;
        if stride < width_bytes {
            tracing::warn!(
                stride,
                width_bytes,
                "scaler stride < row width; presenting black frame"
            );
            return empty;
        }

        let plane = dst.data(0);
        let required = stride * (self.height as usize).saturating_sub(1) + width_bytes;
        if plane.len() < required {
            tracing::warn!(
                plane_len = plane.len(),
                required,
                "scaler output too small; presenting black frame"
            );
            return empty;
        }

        let mut rgba = Vec::with_capacity(width_bytes * self.height as usize);
        for row in 0..self.height as usize {
            let start = row * stride;
            rgba.extend_from_slice(&plane[start..start + width_bytes]);
        }
        VideoFrame {
            pts_ms,
            width: self.width,
            height: self.height,
            rgba,
        }
    }
}

// ---------- audio thread ----------

fn audio_loop<P, C>(
    rx: Receiver<Cmd>,
    clock: AudioClock,
    mut producer: P,
    mut consumer: C,
    weak: Weak<AppWindow>,
) where
    P: Producer<Item = f32> + Send + 'static,
    C: Consumer<Item = f32> + Send + 'static,
{
    // Bring up the output stream even before any media is loaded so cpal
    // device-switch quirks are surfaced at startup, not on first Open.
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            tracing::warn!("no default audio output device; audio muted");
            drain_rx(rx);
            return;
        }
    };
    let config = cpal::StreamConfig {
        channels: OUT_CHANNELS,
        sample_rate: cpal::SampleRate(OUT_SAMPLE_RATE),
        buffer_size: cpal::BufferSize::Default,
    };

    // SAFETY: cpal callbacks capture our consumer; the callback is invoked
    // on a cpal-owned thread. `HeapConsumer` is `Send`.
    let clock_cb = clock.clone();
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
            for sample in data.iter_mut() {
                *sample = consumer.try_pop().unwrap_or(0.0);
            }
            if clock_cb.is_playing() {
                let written = data.len() as u64 / OUT_CHANNELS as u64;
                let ms = written * 1000 / OUT_SAMPLE_RATE as u64;
                let cur = clock_cb.get();
                clock_cb.set(cur + ms);
            }
        },
        |err| tracing::error!(error = %err, "audio stream error"),
        None,
    );
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "failed to build audio stream; audio muted");
            drain_rx(rx);
            return;
        }
    };
    if let Err(e) = stream.play() {
        tracing::warn!(error = %e, "audio stream play failed; audio muted");
        drain_rx(rx);
        return;
    }

    let mut actx: Option<AudioCtx> = None;
    let mut playing = false;
    let mut timeline: Option<Arc<TimelineSync>> = None;

    loop {
        let cmd = if playing && actx.is_some() {
            rx.try_recv().ok()
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };

        if let Some(c) = cmd {
            match c {
                Cmd::LoadTimeline { sync } => {
                    actx = None;
                    playing = false;
                    clock.set(0);
                    timeline = Some(sync.clone());
                    sync.active_index.store(0, Ordering::SeqCst);
                    let s0 = &sync.segments[0];
                    match try_open_audio(&s0.path) {
                        Ok(mut a) => {
                            if let Err(e) = a.seek(s0.media_in_ms) {
                                tracing::warn!(error = %e, "audio initial seek failed");
                            }
                            actx = Some(a);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "audio open failed; continuing muted");
                        }
                    }
                }
                Cmd::Play => playing = actx.is_some(),
                Cmd::Pause => playing = false,
                Cmd::SeekSequence { seq_ms } => {
                    let Some(ref t) = timeline else {
                        continue;
                    };
                    if let Some((idx, local_ms)) = t.resolve_seek(seq_ms) {
                        t.active_index.store(idx, Ordering::SeqCst);
                        let s = &t.segments[idx];
                        match try_open_audio(&s.path) {
                            Ok(mut a) => {
                                if let Err(e) = a.seek(local_ms) {
                                    tracing::warn!(error = %e, seq_ms, "audio seek failed");
                                }
                                actx = Some(a);
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "audio reopen failed");
                                actx = None;
                            }
                        }
                    }
                    clock.set(seq_ms);
                    let _ = producer.try_push(0.0);
                }
                Cmd::Close => {
                    actx = None;
                    timeline = None;
                    playing = false;
                }
                Cmd::Stop => return,
            }
        }

        if playing {
            if let Some(a) = actx.as_mut() {
                if let Some(samples) = a.next_packet_samples() {
                    for s in samples {
                        while producer.try_push(s).is_err() {
                            if !playing || !clock.is_playing() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                } else if let Some(ref t) = timeline {
                    let idx = t.active_index.load(Ordering::SeqCst);
                    if idx + 1 < t.segments.len() {
                        t.active_index.store(idx + 1, Ordering::SeqCst);
                        let s = &t.segments[idx + 1];
                        match try_open_audio(&s.path) {
                            Ok(mut na) => {
                                if let Err(e) = na.seek(s.media_in_ms) {
                                    tracing::warn!(error = %e, "audio segment seek failed");
                                    playing = false;
                                    clock.set_playing(false);
                                    on_ui(weak.clone(), |w| w.set_is_playing(false));
                                } else {
                                    actx = Some(na);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "audio segment open failed");
                                playing = false;
                                clock.set_playing(false);
                                on_ui(weak.clone(), |w| w.set_is_playing(false));
                            }
                        }
                    } else {
                        playing = false;
                        clock.set_playing(false);
                        on_ui(weak.clone(), |w| w.set_is_playing(false));
                    }
                } else {
                    playing = false;
                }
            } else {
                std::thread::sleep(Duration::from_millis(16));
            }
        }
    }
}

fn drain_rx(rx: Receiver<Cmd>) {
    while let Ok(c) = rx.recv() {
        if matches!(c, Cmd::Stop) {
            return;
        }
    }
}

struct AudioCtx {
    input: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Audio,
    resampler: ffmpeg::software::resampling::Context,
    stream_idx: usize,
}

impl AudioCtx {
    fn open(path: &Path) -> Result<Self> {
        let input = ffmpeg::format::input(&path)?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Audio)
            .context("no audio stream")?;
        let stream_idx = stream.index();
        let ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
        let decoder = ctx.decoder().audio()?;
        let resampler = ffmpeg::software::resampling::Context::get(
            decoder.format(),
            decoder.channel_layout(),
            decoder.rate(),
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
            ffmpeg::ChannelLayout::STEREO,
            OUT_SAMPLE_RATE,
        )?;
        Ok(Self {
            input,
            decoder,
            resampler,
            stream_idx,
        })
    }

    fn seek(&mut self, target_ms: u64) -> Result<()> {
        let ts = (target_ms as i64) * i64::from(ffmpeg::ffi::AV_TIME_BASE) / 1000;
        self.input.seek(ts, ..ts)?;
        self.decoder.flush();
        Ok(())
    }

    fn next_packet_samples(&mut self) -> Option<Vec<f32>> {
        let mut decoded = ffmpeg::frame::Audio::empty();
        while let Some((stream, packet)) = self.input.packets().next() {
            if stream.index() != self.stream_idx {
                continue;
            }
            if self.decoder.send_packet(&packet).is_err() {
                continue;
            }
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                let mut resampled = ffmpeg::frame::Audio::empty();
                let _ = self.resampler.run(&decoded, &mut resampled);
                let plane = resampled.data(0);
                let floats = unsafe {
                    std::slice::from_raw_parts(
                        plane.as_ptr() as *const f32,
                        plane.len() / std::mem::size_of::<f32>(),
                    )
                };
                return Some(floats.to_vec());
            }
        }
        None
    }
}
