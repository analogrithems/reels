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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};
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
use crate::{reset_tools_popup_ui, AppWindow};
use reel_core::project::ClipOrientation;

/// One audio lane handed to the player: its concat timeline plus the per-lane
/// gain in decibels (matches the `Track::gain_db` stored in the project). The
/// audio thread converts to linear scale once per `LoadTimeline` and multiplies
/// each sample on its way into the mix — parallel to the export-side
/// `[i:a:0]volume=XdB[aI]` prefilters ahead of ffmpeg `amix`.
#[derive(Clone)]
pub struct AudioLaneLoad {
    pub timeline: Arc<TimelineSync>,
    pub gain_db: f32,
}

#[derive(Clone)]
pub enum Cmd {
    /// Primary video track + per-lane dedicated audio concats.
    ///
    /// `audio_lanes` is the full project order of `TrackKind::Audio` lanes
    /// that actually carry clips. An **empty** vec means "no dedicated audio
    /// lanes" — the audio thread falls back to the embedded audio stream on
    /// each video segment's file, matching the pre-multi-lane behavior.
    LoadTimeline {
        video: Arc<TimelineSync>,
        audio_lanes: Vec<AudioLaneLoad>,
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

/// Shared playback clock, exposed in milliseconds since source start.
///
/// # Internal precision
///
/// The position is stored internally as **microseconds**, not ms. The
/// audio thread delivers callbacks with ragged frame counts (often 441
/// or 480 at 48 kHz, sometimes 512), and converting to whole ms per
/// callback via integer division would lose 0.1875 ms on a 441-frame
/// burst — ≈170 ms drift over 7 s of real playback. That's plainly
/// audible as A/V desync. Microsecond granularity rounds to within
/// ≤ 1 ms over tens of minutes even with ragged bursts.
///
/// # Output-latency compensation
///
/// `pos_us` tracks **samples handed to the OS audio buffer** — not
/// samples actually emitted by the speakers. On most systems the
/// speaker is playing audio from ~5–40 ms ago relative to the most
/// recent cpal callback (one callback buffer's worth, plus device
/// offsets). If the video thread schedules frames against `pos_us`,
/// video renders ahead of what the user hears — the classic A/V sync
/// complaint.
///
/// `output_latency_us` holds that device latency estimate. It is
/// calibrated once on the first cpal output callback (see
/// [`audio_loop`]) using `OutputCallbackInfo.timestamp()` — the delta
/// between when the callback fired and when the first sample of the
/// buffer will actually leave the speakers. That's the *true* one-way
/// latency (OS + driver + device) rather than just the callback
/// buffer size. [`AudioClock::get`] subtracts that offset so readers
/// see **audible-now**, keeping video in lockstep with sound.
/// [`AudioClock::get_raw`] returns the uncorrected value for internal
/// audio-thread use so audio bookkeeping doesn't chase its own offset.
///
/// `user_offset_us` is a **signed** per-device nudge dialed in by the
/// user (Preferences → A/V Offset). cpal's timestamp-based calibration
/// handles most devices, but Bluetooth stacks / external DACs / HDMI
/// receivers often add 40–200 ms of audio-side delay that the driver
/// doesn't surface. A positive offset says "audio is *later* than the
/// calibrated estimate" and makes the video thread hold each frame
/// proportionally longer. A negative offset (video is later than audio)
/// is accepted but clamps the effective latency at 0 — we won't run the
/// clock *ahead* of the raw write head, since there's nothing audible
/// past there yet. Persisted in `AppPrefs::audio_offset_ms`.
#[derive(Clone, Debug, Default)]
pub struct AudioClock {
    pos_us: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    output_latency_us: Arc<AtomicU64>,
    user_offset_us: Arc<AtomicI64>,
}

impl AudioClock {
    pub fn new() -> Self {
        Self::default()
    }
    /// Seek: overwrite the position to `ms` exactly.
    pub fn set(&self, ms: u64) {
        self.pos_us
            .store(ms.saturating_mul(1_000), Ordering::Release);
    }
    /// Latency-compensated read: audible-now in sequence ms. Use this from
    /// the **video** thread / UI so picture matches sound.
    ///
    /// Total compensation = `output_latency_us` (auto-calibrated) +
    /// `user_offset_us` (manual nudge). If the sum goes non-positive the
    /// compensation is pinned at 0 — the raw write head is the earliest
    /// meaningful audible-now, so reading further ahead would be lying.
    pub fn get(&self) -> u64 {
        let raw_us = self.pos_us.load(Ordering::Acquire);
        let lat_us = self.output_latency_us.load(Ordering::Acquire) as i64;
        let user_us = self.user_offset_us.load(Ordering::Acquire);
        let effective = lat_us.saturating_add(user_us);
        if effective <= 0 {
            raw_us / 1_000
        } else {
            raw_us.saturating_sub(effective as u64) / 1_000
        }
    }
    /// Raw written-to-OS position. Use this from the **audio** thread so
    /// audio decode doesn't chase its own latency offset.
    #[allow(dead_code)]
    pub fn get_raw(&self) -> u64 {
        self.pos_us.load(Ordering::Acquire) / 1_000
    }
    pub fn set_playing(&self, p: bool) {
        self.playing.store(p, Ordering::Release);
    }
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }
    /// Calibration hook: set the estimated one-way output latency (cpal
    /// callback buffer + device) in **milliseconds**. Internally stored
    /// as microseconds; the ms granularity is fine because device
    /// latency is itself only good to a few ms. Prefer
    /// [`calibrate_from_cpal`] on the audio thread — this setter is
    /// retained for tests and future direct overrides (e.g. a user-facing
    /// "manual A/V offset" slider).
    #[allow(dead_code)]
    pub fn set_output_latency_ms(&self, ms: u64) {
        self.output_latency_us
            .store(ms.saturating_mul(1_000), Ordering::Release);
    }
    #[allow(dead_code)]
    pub fn output_latency_ms(&self) -> u64 {
        self.output_latency_us.load(Ordering::Acquire) / 1_000
    }
    /// Manual user-supplied A/V offset in **signed** milliseconds. Positive
    /// values say "audio arrives later than the auto-calibrated estimate"
    /// and effectively hold the picture back; negative values pull the
    /// picture forward (clamped at 0 total compensation in [`get`]).
    /// Clamped to [`AudioClock::USER_OFFSET_RANGE_MS`] so a runaway slider
    /// can't overflow the atomic or stall the video thread indefinitely.
    pub fn set_user_offset_ms(&self, ms: i32) {
        let clamped = ms.clamp(-Self::USER_OFFSET_RANGE_MS, Self::USER_OFFSET_RANGE_MS);
        self.user_offset_us
            .store((clamped as i64) * 1_000, Ordering::Release);
    }
    /// Current user offset in signed ms. Source of truth for the UI
    /// "A/V Offset: ±NNN ms" readout — we store µs internally and only
    /// expose ms because device latency is itself only good to a few ms.
    pub fn user_offset_ms(&self) -> i32 {
        (self.user_offset_us.load(Ordering::Acquire) / 1_000) as i32
    }
    /// Absolute bound on [`set_user_offset_ms`]. ±30 000 ms (30 s) covers
    /// every real-world consumer device (Bluetooth, HDMI ARC, wireless
    /// speakers) plus long bring-your-own-audio overlay workflows where the
    /// user is dialing in a musical-intro pre-roll by ear.
    pub const USER_OFFSET_RANGE_MS: i32 = 30_000;

    /// Advance the raw position by the time represented by `frames` at
    /// `sample_rate`, scaled by preview speed (`speed_signed` is in the
    /// same ±250..=±2000 milli-units the UI uses). Negative speed is a
    /// no-op here because rewind is handled by the video thread via
    /// seeks, not by advancing the clock. Pulled out of the cpal
    /// callback so the exact advance math can be unit-tested without
    /// spinning up an audio device.
    ///
    /// Computes in **microseconds** to keep drift below 1 ms even when
    /// cpal delivers ragged frame counts (441 / 480 / 512). The old
    /// per-callback `frames * 1000 / sample_rate` division lost
    /// fractional ms and compounded into visible desync after a few
    /// seconds of playback.
    pub fn advance_by_frames(&self, frames: u64, sample_rate: u32, speed_signed: i32) {
        if !self.is_playing() || speed_signed <= 0 || frames == 0 || sample_rate == 0 {
            return;
        }
        let base_us = frames.saturating_mul(1_000_000) / sample_rate as u64;
        let sp = (speed_signed as f64).clamp(250.0, 4000.0) / 1000.0;
        let adv_us = ((base_us as f64) * sp).round() as u64;
        self.pos_us.fetch_add(adv_us, Ordering::AcqRel);
    }

    /// Calibrate output latency from cpal's `OutputCallbackInfo`
    /// timestamps when available. `playback` is when the first sample
    /// of this callback buffer will actually leave the speakers;
    /// `callback` is when the callback started. Their difference is
    /// the **true** one-way latency — buffer size + driver + device —
    /// which is what we want to subtract from `pos_us` to land on
    /// audible-now. Falls back to the buffer-size-only estimate if the
    /// OS doesn't populate the timestamps (some Linux configs).
    ///
    /// First call wins; subsequent calls are no-ops so a single glitchy
    /// callback can't thrash the offset mid-playback.
    pub fn calibrate_from_cpal(
        &self,
        timestamp_latency_nanos: Option<u128>,
        fallback_frames: u64,
        sample_rate: u32,
    ) {
        if self.output_latency_us.load(Ordering::Acquire) != 0 || sample_rate == 0 {
            return;
        }
        let us = match timestamp_latency_nanos {
            Some(nanos) if nanos > 0 => (nanos / 1_000) as u64,
            _ => fallback_frames.saturating_mul(1_000_000) / sample_rate as u64,
        };
        // Floor at 1000us (1 ms) so readers can distinguish "calibrated"
        // from "not yet calibrated" (which stores 0).
        self.output_latency_us
            .store(us.max(1_000), Ordering::Release);
    }
}

/// Handle to a running player. Dropping it stops and joins both threads.
pub struct PlayerHandle {
    tx_video: Sender<Cmd>,
    tx_audio: Sender<Cmd>,
    video_thread: Option<JoinHandle<()>>,
    audio_thread: Option<JoinHandle<()>>,
    /// Master gain **0..=1000** (linear; 1000 = unity). Applied in the cpal output callback.
    pub master_volume_1000: Arc<AtomicU32>,
    /// Preview speed **±250..=±2000** in milli-units where **±1000 = 1.0×**; **negative** = rewind (seek-based).
    #[allow(dead_code)]
    // Exposed for future UI sync / debugging; threads hold the canonical `Arc`.
    pub playback_signed_milli: Arc<AtomicI32>,
    /// Shared with the audio + video threads. The UI holds a clone to
    /// drive `set_user_offset_ms` from the Preferences → A/V Offset menu.
    pub audio_clock: AudioClock,
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

/// When loop is enabled, seek to the start and keep playing instead of stopping at EOF.
fn loop_seek_restart(loop_enabled: &Arc<AtomicBool>, restart: &PlayerCmdSender) -> bool {
    if !loop_enabled.load(Ordering::Relaxed) {
        return false;
    }
    restart.send(Cmd::SeekSequence { seq_ms: 0 });
    restart.send(Cmd::Play);
    true
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
///
/// `master_volume_1000` is shared with the UI; **0** = mute, **1000** = full level.
/// `playback_loop` is read when playback reaches the end of the sequence (both threads).
/// `playback_signed_milli` is preview rate **±250..=±2000** (**±1000** = 1.0×); negative values rewind.
pub fn spawn_player(
    window: &AppWindow,
    master_volume_1000: Arc<AtomicU32>,
    playback_signed_milli: Arc<AtomicI32>,
    playback_loop: Arc<AtomicBool>,
) -> Result<PlayerHandle> {
    // Initialize ffmpeg once.
    ffmpeg::init().context("ffmpeg::init")?;

    let (tx_video, rx_video) = crossbeam_channel::unbounded::<Cmd>();
    let (tx_audio, rx_audio) = crossbeam_channel::unbounded::<Cmd>();

    let clock = AudioClock::new();

    // Ring buffer: ~400 ms of audio at 48 kHz stereo f32.
    let rb: HeapRb<f32> = HeapRb::new((OUT_SAMPLE_RATE as usize) * OUT_CHANNELS as usize / 2);
    let (producer, consumer) = rb.split();

    let restart = PlayerCmdSender {
        tx_video: tx_video.clone(),
        tx_audio: tx_audio.clone(),
    };

    let weak = window.as_weak();
    let weak_v = weak.clone();
    let clock_v = clock.clone();
    let restart_v = restart.clone();
    let loop_v = playback_loop.clone();
    let signed_v = playback_signed_milli.clone();

    let weak_a = weak.clone();
    let clock_a = clock.clone();
    let vol_stream = master_volume_1000.clone();
    let speed_stream = playback_signed_milli.clone();
    let restart_a = restart;
    let loop_a = playback_loop;
    let audio_thread = std::thread::Builder::new()
        .name("reel-audio".into())
        .spawn(move || {
            audio_loop(
                rx_audio,
                clock_a,
                producer,
                consumer,
                weak_a,
                vol_stream,
                speed_stream,
                restart_a,
                loop_a,
            )
        })?;

    let video_thread = std::thread::Builder::new()
        .name("reel-video".into())
        .spawn(move || video_loop(rx_video, weak_v, clock_v, restart_v, loop_v, signed_v))?;

    Ok(PlayerHandle {
        tx_video,
        tx_audio,
        video_thread: Some(video_thread),
        audio_thread: Some(audio_thread),
        master_volume_1000,
        playback_signed_milli,
        audio_clock: clock,
    })
}

/// Seek primary video to `seq_ms`, open the segment decoder, and present one frame.
fn video_seek_to_sequence_ms(
    weak: &Weak<AppWindow>,
    timeline: &Arc<TimelineSync>,
    ctx: &mut Option<VideoCtx>,
    video_seg_idx: &mut usize,
    clock: &AudioClock,
    seq_ms: u64,
) -> bool {
    let cap = timeline.total_sequence_ms();
    let seq_ms = seq_ms.min(cap);
    if let Some((idx, local_ms)) = timeline.resolve_seek(seq_ms) {
        timeline.active_index.store(idx, Ordering::SeqCst);
        *video_seg_idx = idx;
        let s = &timeline.segments[idx];
        match try_open_video(&s.path) {
            Ok(mut c) => {
                c.set_orientation(s.orientation);
                if let Err(e) = c.seek(local_ms) {
                    tracing::warn!(error = %e, seq_ms, "video seek failed");
                }
                if let Some(frame) = c.next_presentable_frame() {
                    present(weak, frame);
                }
                *ctx = Some(c);
                clock.set(seq_ms);
                let sf = seq_ms as f32;
                let w = weak.clone();
                on_ui(w, move |win| apply_playhead_transport(&win, sf));
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "video reopen seek failed");
                false
            }
        }
    } else {
        false
    }
}

// ---------- video thread ----------

fn video_loop(
    rx: Receiver<Cmd>,
    weak: Weak<AppWindow>,
    clock: AudioClock,
    restart: PlayerCmdSender,
    loop_playback: Arc<AtomicBool>,
    playback_signed: Arc<AtomicI32>,
) {
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
                Cmd::LoadTimeline {
                    video: sync,
                    audio_lanes: _,
                } => {
                    ctx = None;
                    playing = false;
                    clock.set_playing(false);
                    clock.set(0);
                    timeline = Some(sync.clone());
                    video_seg_idx = 0;
                    sync.active_index.store(0, Ordering::SeqCst);
                    let s0 = &sync.segments[0];
                    {
                        on_ui(weak.clone(), move |w| {
                            w.set_media_ready(false);
                            w.set_is_playing(false);
                            w.set_status_text("".into());
                            reset_tools_popup_ui(&w);
                        });
                    }
                    match try_open_video(&s0.path) {
                        Ok(mut new_ctx) => {
                            new_ctx.set_orientation(s0.orientation);
                            if let Err(e) = new_ctx.seek(s0.media_in_ms) {
                                tracing::warn!(error = %e, "video initial seek failed");
                            }
                            if let Some(frame) = new_ctx.next_presentable_frame() {
                                present(&weak, frame);
                            }
                            ctx = Some(new_ctx);
                            on_ui(weak.clone(), move |w| {
                                w.set_status_text("".into());
                                w.set_media_ready(true);
                                apply_playhead_transport(&w, 0.0);
                                reset_tools_popup_ui(&w);
                            });
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "video open failed");
                            timeline = None;
                            on_ui(weak.clone(), move |w| {
                                w.set_media_ready(false);
                                w.set_status_text(format!("Open failed: {e}").into());
                                reset_tools_popup_ui(&w);
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
                        // `main` may have toggled `is_playing` before sending; keep UI consistent.
                        on_ui(weak.clone(), |w| w.set_is_playing(false));
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
                    let _ = video_seek_to_sequence_ms(
                        &weak,
                        t,
                        &mut ctx,
                        &mut video_seg_idx,
                        &clock,
                        seq_ms,
                    );
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
                        w.set_status_text("".into());
                        w.set_time_elapsed("0:00.0".into());
                        w.set_time_total("0:00.0".into());
                        w.set_playhead_ms(0.0);
                        reset_tools_popup_ui(&w);
                    });
                }
                Cmd::Stop => return,
            }
        }

        if playing {
            let spd = playback_signed.load(Ordering::Relaxed);
            if spd < 0 {
                if let Some(ref t) = timeline {
                    let cur = clock.get();
                    if cur <= 1 {
                        playing = false;
                        clock.set_playing(false);
                        playback_signed.store(1000, Ordering::Relaxed);
                        on_ui(weak.clone(), |w| {
                            w.set_is_playing(false);
                            w.set_transport_rate_label("1.00×".into());
                        });
                        std::thread::sleep(Duration::from_millis(16));
                        continue;
                    }
                    let abs = (-spd).clamp(250, 2000) as u64;
                    let step_ms = (abs * 80 / 1000).max(1).min(cur);
                    let new_seq = cur.saturating_sub(step_ms);
                    if !video_seek_to_sequence_ms(
                        &weak,
                        t,
                        &mut ctx,
                        &mut video_seg_idx,
                        &clock,
                        new_seq,
                    ) {
                        playing = false;
                        clock.set_playing(false);
                        playback_signed.store(1000, Ordering::Relaxed);
                        on_ui(weak.clone(), |w| {
                            w.set_is_playing(false);
                            w.set_transport_rate_label("1.00×".into());
                        });
                    }
                    let pace = (128u64).saturating_sub(abs / 40).clamp(8, 64);
                    std::thread::sleep(Duration::from_millis(pace));
                }
                continue;
            }
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
                            if loop_seek_restart(&loop_playback, &restart) {
                                continue;
                            }
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
                                        nc.set_orientation(s.orientation);
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
        w.set_preview_frame_width(frame.width as f32);
        w.set_preview_frame_height(frame.height as f32);
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
    orientation: ClipOrientation,
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
            orientation: ClipOrientation::default(),
        })
    }

    fn set_orientation(&mut self, orientation: ClipOrientation) {
        self.orientation = orientation;
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

        match self.orientation.apply_rgba(&rgba, self.width, self.height) {
            Some((out_rgba, out_w, out_h)) => VideoFrame {
                pts_ms,
                width: out_w,
                height: out_h,
                rgba: out_rgba,
            },
            None => VideoFrame {
                pts_ms,
                width: self.width,
                height: self.height,
                rgba,
            },
        }
    }
}

/// Stereo `f32` interleaved buffer + fractional read position for variable playback speed.
#[derive(Default)]
struct AudioSpeedCarry {
    carry: Vec<f32>,
    pos: f64,
}

impl AudioSpeedCarry {
    fn reset(&mut self) {
        self.carry.clear();
        self.pos = 0.0;
    }

    /// Consumes decoded stereo samples and pushes speed-adjusted pairs to `producer`.
    fn push_speed_samples<P: Producer<Item = f32>>(
        &mut self,
        chunk: Vec<f32>,
        speed: f64,
        producer: &mut P,
        clock_playing: impl Fn() -> bool,
    ) {
        self.carry.extend(chunk);
        const MAX_CARRY: usize = 48000 * 8 * 2;
        loop {
            if self.pos + 2.0 * speed > self.carry.len() as f64 {
                break;
            }
            let pair_base = (self.pos / 2.0).floor() as usize * 2;
            if pair_base + 2 > self.carry.len() {
                break;
            }
            let l = self.carry[pair_base];
            let r = self.carry[pair_base + 1];
            while producer.try_push(l).is_err() {
                if !clock_playing() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            while producer.try_push(r).is_err() {
                if !clock_playing() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            self.pos += 2.0 * speed;
        }
        let drop = (self.pos / 2.0).floor() as usize * 2;
        let drop = drop.min(self.carry.len());
        if drop > 0 {
            self.carry.drain(..drop);
            self.pos -= drop as f64;
        }
        if self.carry.len() > MAX_CARRY {
            let excess = self.carry.len() - MAX_CARRY / 2;
            if excess > 0 && excess < self.carry.len() {
                self.carry.drain(..excess);
                self.pos = (self.pos - excess as f64).max(0.0);
            }
        }
    }
}

// ---------- audio thread ----------

/// Per-lane playback state for the multi-audio mixer. Mirrors what the
/// pre-multi-lane code tracked via a single `(AudioCtx, TimelineSync,
/// active_index)` triple, but one copy per audio lane so the mixer can pull
/// from all lanes each tick.
///
/// `pending` buffers the most recently decoded interleaved stereo samples
/// from `ctx` — the mixer drains `min(pending.len)` across lanes per pass so
/// the output stays sample-aligned regardless of lanes returning
/// different-sized packets. `exhausted` flips true when a lane has no more
/// segments to open; from then on the lane contributes silence (it's
/// skipped in the mix pass, which is mathematically equivalent to adding 0).
struct AudioLane {
    timeline: Arc<TimelineSync>,
    ctx: Option<AudioCtx>,
    pending: std::collections::VecDeque<f32>,
    gain_linear: f32,
    seg_idx: usize,
    exhausted: bool,
}

impl AudioLane {
    fn new(load: AudioLaneLoad) -> Self {
        Self {
            timeline: load.timeline,
            ctx: None,
            pending: std::collections::VecDeque::new(),
            gain_linear: db_to_linear(load.gain_db),
            seg_idx: 0,
            exhausted: false,
        }
    }
}

/// Convert a gain in decibels to its linear multiplier. Matches the
/// `volume=XdB` ffmpeg filter used on the export side: `10^(dB/20)`. Unity
/// (`0 dB`) returns exactly `1.0` so unity lanes are a no-op in the mix.
///
/// The session clamps input to `[-40, +40]` dB, and `f32::NAN` becomes `0.0`
/// at the setter, so this function doesn't need to guard those ranges — but
/// it stays well-defined for any finite input a caller might hand it.
fn db_to_linear(db: f32) -> f32 {
    if db == 0.0 {
        return 1.0;
    }
    10f32.powf(db / 20.0)
}

/// Pull one mix pass of samples from `lanes`, applying per-lane gain.
///
/// Strategy: for each lane that has no pending samples and isn't exhausted,
/// refill once from its current `AudioCtx`. If refill hits EOF, advance to
/// the next segment; if there are no more segments, mark the lane
/// exhausted. Then mix `n = min(pending.len)` across lanes that still have
/// pending samples — exhausted / empty lanes contribute `0.0` implicitly.
///
/// Returns `None` only when **every** lane is exhausted *and* drained —
/// the caller is expected to treat that as the "fall back to silence-pad"
/// signal (dedicated_audio + video still playing) or to stop (no audio
/// anywhere).
fn next_mixed_samples(lanes: &mut [AudioLane]) -> Option<Vec<f32>> {
    for lane in lanes.iter_mut() {
        if lane.exhausted || !lane.pending.is_empty() {
            continue;
        }
        let Some(ctx) = lane.ctx.as_mut() else {
            lane.exhausted = true;
            continue;
        };
        if let Some(samples) = ctx.next_packet_samples() {
            lane.pending.extend(samples);
            continue;
        }
        // Current segment ran out; advance to the next one if any.
        let next_idx = lane.seg_idx + 1;
        if next_idx < lane.timeline.segments.len() {
            lane.seg_idx = next_idx;
            lane.timeline
                .active_index
                .store(next_idx, Ordering::SeqCst);
            let s = &lane.timeline.segments[next_idx];
            match try_open_audio(&s.path) {
                Ok(mut a) => {
                    if let Err(e) = a.seek(s.media_in_ms) {
                        tracing::warn!(error = %e, "audio lane segment seek failed");
                        lane.exhausted = true;
                        lane.ctx = None;
                    } else {
                        lane.ctx = Some(a);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "audio lane segment open failed");
                    lane.exhausted = true;
                    lane.ctx = None;
                }
            }
        } else {
            lane.exhausted = true;
            lane.ctx = None;
        }
    }

    let min_pending = lanes
        .iter()
        .filter(|l| !l.pending.is_empty())
        .map(|l| l.pending.len())
        .min()?;
    Some(drain_and_mix(lanes, min_pending))
}

/// Drain `n` samples from each non-empty lane, applying `gain_linear`, and
/// sum into one `Vec<f32>`. Separated from `next_mixed_samples` so the mix
/// math is unit-testable without ffmpeg / segment advancement.
fn drain_and_mix(lanes: &mut [AudioLane], n: usize) -> Vec<f32> {
    let mut mix = vec![0f32; n];
    for lane in lanes.iter_mut() {
        if lane.pending.is_empty() {
            continue;
        }
        let g = lane.gain_linear;
        for mix_slot in mix.iter_mut() {
            match lane.pending.pop_front() {
                Some(s) => *mix_slot += s * g,
                None => break,
            }
        }
    }
    mix
}


#[allow(clippy::too_many_arguments)]
fn audio_loop<P, C>(
    rx: Receiver<Cmd>,
    clock: AudioClock,
    mut producer: P,
    mut consumer: C,
    weak: Weak<AppWindow>,
    master_volume_1000: Arc<AtomicU32>,
    playback_signed_milli: Arc<AtomicI32>,
    restart: PlayerCmdSender,
    loop_playback: Arc<AtomicBool>,
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
    let vol_cb = master_volume_1000;
    let speed_cb = playback_signed_milli.clone();
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
            let g = vol_cb.load(Ordering::Relaxed) as f32 * (1.0 / 1000.0);
            for sample in data.iter_mut() {
                *sample = consumer.try_pop().unwrap_or(0.0) * g;
            }
            // Output-latency calibration (first non-empty callback wins).
            // cpal's `timestamp().playback` is when the first sample of
            // this buffer will hit the speakers; `.callback` is when
            // the callback started. Their delta is the true one-way
            // latency (buffer + driver + device), which is what
            // `AudioClock::get()` subtracts so picture tracks sound.
            let frames = (data.len() / OUT_CHANNELS as usize) as u64;
            if frames > 0 {
                let ts = info.timestamp();
                let latency_nanos = ts
                    .playback
                    .duration_since(&ts.callback)
                    .map(|d| d.as_nanos());
                clock_cb.calibrate_from_cpal(latency_nanos, frames, OUT_SAMPLE_RATE);
            }
            let sgn = speed_cb.load(Ordering::Relaxed);
            clock_cb.advance_by_frames(frames, OUT_SAMPLE_RATE, sgn);
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

    let mut speed_carry = AudioSpeedCarry::default();
    let clock_playing = clock.clone();
    let mut lanes: Vec<AudioLane> = Vec::new();
    let mut playing = false;
    let mut video_master: Option<Arc<TimelineSync>> = None;
    // At least one dedicated `TrackKind::Audio` lane carries clips. When
    // false, `lanes` contains a single synthetic lane sourced from the
    // video timeline (embedded audio in each video file).
    let mut dedicated_audio = false;
    // All lanes exhausted but video still playing — push zeros so the
    // clock keeps advancing until video ends.
    let mut silence_pad = false;

    loop {
        let any_active = lanes.iter().any(|l| !l.exhausted || !l.pending.is_empty());
        let cmd = if playing && (any_active || silence_pad) {
            rx.try_recv().ok()
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };

        if let Some(c) = cmd {
            match c {
                Cmd::LoadTimeline {
                    video,
                    audio_lanes,
                } => {
                    lanes.clear();
                    playing = false;
                    silence_pad = false;
                    speed_carry.reset();
                    clock.set(0);
                    video_master = Some(video.clone());
                    dedicated_audio = !audio_lanes.is_empty();
                    // Build per-lane state. Empty vec → fall back to embedded
                    // audio from video files (single synthetic lane at unit
                    // gain).
                    let loads: Vec<AudioLaneLoad> = if audio_lanes.is_empty() {
                        vec![AudioLaneLoad {
                            timeline: video.clone(),
                            gain_db: 0.0,
                        }]
                    } else {
                        audio_lanes
                    };
                    for load in loads {
                        if load.timeline.segments.is_empty() {
                            continue;
                        }
                        let mut lane = AudioLane::new(load);
                        lane.timeline.active_index.store(0, Ordering::SeqCst);
                        let s0 = &lane.timeline.segments[0];
                        match try_open_audio(&s0.path) {
                            Ok(mut a) => {
                                if let Err(e) = a.seek(s0.media_in_ms) {
                                    tracing::warn!(error = %e, "audio initial seek failed");
                                }
                                lane.ctx = Some(a);
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "audio lane open failed");
                                lane.exhausted = true;
                            }
                        }
                        lanes.push(lane);
                    }
                    if lanes.is_empty() {
                        video_master = None;
                    } else if lanes.iter().all(|l| l.exhausted) {
                        // Every lane failed to open but there's still video
                        // — treat as silence-pad so the clock advances.
                        silence_pad = dedicated_audio && video.total_sequence_ms() > 0;
                    }
                }
                Cmd::Play => {
                    if lanes.is_empty() {
                        continue;
                    }
                    let sgn = playback_signed_milli.load(Ordering::Relaxed);
                    if sgn < 0 {
                        playing = true;
                        clock.set_playing(true);
                        continue;
                    }
                    let any_open = lanes.iter().any(|l| l.ctx.is_some());
                    if !any_open && !silence_pad {
                        continue;
                    }
                    playing = true;
                    clock.set_playing(true);
                }
                Cmd::Pause => {
                    playing = false;
                    clock.set_playing(false);
                }
                Cmd::SeekSequence { seq_ms } => {
                    let Some(ref vm) = video_master else {
                        continue;
                    };
                    if lanes.is_empty() {
                        continue;
                    }
                    let cap = vm.total_sequence_ms();
                    let seq_ms = seq_ms.min(cap);
                    silence_pad = false;
                    let mut any_open = false;
                    for lane in lanes.iter_mut() {
                        lane.pending.clear();
                        lane.ctx = None;
                        let dt = &lane.timeline;
                        if seq_ms >= dt.total_sequence_ms() {
                            lane.exhausted = true;
                            if !dt.segments.is_empty() {
                                dt.active_index
                                    .store(dt.segments.len() - 1, Ordering::SeqCst);
                                lane.seg_idx = dt.segments.len() - 1;
                            }
                            continue;
                        }
                        let Some((idx, local_ms)) = dt.resolve_seek(seq_ms) else {
                            lane.exhausted = true;
                            continue;
                        };
                        lane.seg_idx = idx;
                        lane.exhausted = false;
                        dt.active_index.store(idx, Ordering::SeqCst);
                        let s = &dt.segments[idx];
                        match try_open_audio(&s.path) {
                            Ok(mut a) => {
                                if let Err(e) = a.seek(local_ms) {
                                    tracing::warn!(error = %e, seq_ms, "audio seek failed");
                                }
                                lane.ctx = Some(a);
                                any_open = true;
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "audio reopen failed");
                                lane.exhausted = true;
                            }
                        }
                    }
                    if !any_open && dedicated_audio && seq_ms < cap {
                        silence_pad = true;
                    }
                    clock.set(seq_ms);
                    speed_carry.reset();
                    let _ = producer.try_push(0.0);
                }
                Cmd::Close => {
                    lanes.clear();
                    video_master = None;
                    dedicated_audio = false;
                    silence_pad = false;
                    playing = false;
                    speed_carry.reset();
                }
                Cmd::Stop => return,
            }
        }

        if playing {
            let sgn = playback_signed_milli.load(Ordering::Relaxed);
            let all_done = !lanes.is_empty()
                && lanes
                    .iter()
                    .all(|l| l.exhausted && l.pending.is_empty());
            let should_output_silence = sgn < 0
                || silence_pad
                || (dedicated_audio
                    && all_done
                    && video_master
                        .as_ref()
                        .map(|v| clock.get() < v.total_sequence_ms())
                        .unwrap_or(false));

            if should_output_silence {
                let vm_end = video_master
                    .as_ref()
                    .map(|v| v.total_sequence_ms())
                    .unwrap_or(0);
                if clock.get() >= vm_end {
                    if !loop_seek_restart(&loop_playback, &restart) {
                        playing = false;
                        silence_pad = false;
                        clock.set_playing(false);
                        on_ui(weak.clone(), |w| w.set_is_playing(false));
                    }
                } else {
                    let chunk = (OUT_SAMPLE_RATE as usize / 25) * OUT_CHANNELS as usize;
                    for _ in 0..chunk {
                        while producer.try_push(0.0).is_err() {
                            if !playing || !clock.is_playing() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                }
            } else if !lanes.is_empty() {
                match next_mixed_samples(&mut lanes) {
                    Some(mixed) => {
                        let sp = (playback_signed_milli
                            .load(Ordering::Relaxed)
                            .unsigned_abs() as f64)
                            .clamp(250.0, 4000.0)
                            / 1000.0;
                        speed_carry.push_speed_samples(mixed, sp, &mut producer, || {
                            clock_playing.is_playing()
                        });
                    }
                    None => {
                        // All lanes exhausted. For dedicated-audio timelines
                        // with video still running, flip into silence-pad on
                        // the next iteration; otherwise stop (or loop).
                        if dedicated_audio {
                            if let Some(vm) = video_master.as_ref() {
                                if clock.get() < vm.total_sequence_ms() {
                                    speed_carry.reset();
                                    silence_pad = true;
                                } else if !loop_seek_restart(&loop_playback, &restart) {
                                    playing = false;
                                    clock.set_playing(false);
                                    on_ui(weak.clone(), |w| w.set_is_playing(false));
                                }
                            } else if !loop_seek_restart(&loop_playback, &restart) {
                                playing = false;
                                clock.set_playing(false);
                                on_ui(weak.clone(), |w| w.set_is_playing(false));
                            }
                        } else if !loop_seek_restart(&loop_playback, &restart) {
                            playing = false;
                            clock.set_playing(false);
                            on_ui(weak.clone(), |w| w.set_is_playing(false));
                        }
                    }
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

#[cfg(test)]
mod sync_tests {
    //! Synchronization math tests for [`AudioClock`].
    //!
    //! The cpal output callback is the one place where sync bugs breed —
    //! it runs on a driver thread, has to advance the clock, compensate
    //! for output latency, and keep video lined up with sound. These
    //! tests exercise the extracted helpers ([`AudioClock::advance_by_frames`]
    //! and [`AudioClock::calibrate_from_cpal`]) without spinning up an
    //! actual audio device, so any regression in the math surfaces here
    //! instead of as user-reported A/V drift.
    //!
    //! Coverage:
    //! - `get` returns raw minus calibrated latency (picture vs sound).
    //! - Advance accumulates at 1× with exactly `frames / sample_rate` ms.
    //! - Speed scaling works at 0.5× and 2×.
    //! - Non-playing / negative-speed / zero-frames callbacks don't advance.
    //! - Calibration prefers cpal's timestamp delta, falls back to frame
    //!   count, and is sticky after the first call.
    //! - 30-second simulated playback drifts less than 1 ms from wall-time
    //!   audio advance (i.e. the math is rounding-stable, not cumulative).
    use super::*;
    use std::time::Duration;
    #[cfg(test)]
    use std::time::Instant;
    #[allow(unused_imports)]
    use std::time::SystemTime;
    #[cfg(test)]
    use std::time::UNIX_EPOCH;
    // cpal::StreamInstant / OutputStreamTimestamp aren't constructible
    // from user code, so the calibrate tests feed `Option<u128>` nanos
    // directly — same code path the production callback takes after
    // computing `duration_since`.

    #[test]
    fn get_returns_raw_minus_latency() {
        let c = AudioClock::new();
        c.set(100);
        c.set_output_latency_ms(20);
        assert_eq!(c.get_raw(), 100);
        assert_eq!(c.get(), 80);
    }

    #[test]
    fn get_saturates_below_zero_instead_of_wrapping() {
        // If the clock is at ms=5 and the output latency is 30ms, the
        // audible-now read is "negative" — which has no meaningful
        // answer, so we pin to 0 rather than wrap AtomicU64.
        let c = AudioClock::new();
        c.set(5);
        c.set_output_latency_ms(30);
        assert_eq!(c.get(), 0);
    }

    #[test]
    fn advance_at_unit_speed_is_frames_over_rate() {
        let c = AudioClock::new();
        c.set_playing(true);
        // 48000 frames @ 48kHz = 1000 ms.
        c.advance_by_frames(48_000, 48_000, 1_000);
        assert_eq!(c.get_raw(), 1_000);
    }

    #[test]
    fn advance_at_half_speed_halves_the_ms() {
        let c = AudioClock::new();
        c.set_playing(true);
        c.advance_by_frames(48_000, 48_000, 500);
        assert_eq!(c.get_raw(), 500);
    }

    #[test]
    fn advance_at_double_speed_doubles_the_ms() {
        let c = AudioClock::new();
        c.set_playing(true);
        c.advance_by_frames(48_000, 48_000, 2_000);
        assert_eq!(c.get_raw(), 2_000);
    }

    #[test]
    fn advance_no_op_when_paused() {
        let c = AudioClock::new();
        c.set_playing(false);
        c.advance_by_frames(48_000, 48_000, 1_000);
        assert_eq!(c.get_raw(), 0);
    }

    #[test]
    fn advance_no_op_on_negative_speed() {
        // Rewind is handled by video-thread seeks, not by advancing.
        let c = AudioClock::new();
        c.set_playing(true);
        c.advance_by_frames(48_000, 48_000, -1_000);
        assert_eq!(c.get_raw(), 0);
    }

    #[test]
    fn advance_no_op_on_zero_frames() {
        let c = AudioClock::new();
        c.set_playing(true);
        c.advance_by_frames(0, 48_000, 1_000);
        assert_eq!(c.get_raw(), 0);
    }

    #[test]
    fn calibrate_prefers_cpal_timestamp_when_available() {
        let c = AudioClock::new();
        c.calibrate_from_cpal(Some(25_000_000), 480, 48_000); // 25 ms timestamp; fallback would be 10 ms
        assert_eq!(c.output_latency_ms(), 25);
    }

    #[test]
    fn calibrate_falls_back_to_frame_count_when_timestamp_missing() {
        let c = AudioClock::new();
        c.calibrate_from_cpal(None, 480, 48_000); // 480 frames @ 48k = 10 ms
        assert_eq!(c.output_latency_ms(), 10);
    }

    #[test]
    fn calibrate_falls_back_when_timestamp_is_zero_nanos() {
        // Some cpal backends report a zero-length playback/callback
        // delta on the first callback. Treat as "missing" and fall
        // back rather than trusting a bogus 0 ms latency.
        let c = AudioClock::new();
        c.calibrate_from_cpal(Some(0), 480, 48_000);
        assert_eq!(c.output_latency_ms(), 10);
    }

    #[test]
    fn calibrate_is_sticky_after_first_call() {
        // A single glitchy callback must not thrash the offset
        // mid-playback — first calibration wins for the session.
        let c = AudioClock::new();
        c.calibrate_from_cpal(Some(15_000_000), 480, 48_000);
        assert_eq!(c.output_latency_ms(), 15);
        c.calibrate_from_cpal(Some(50_000_000), 480, 48_000);
        assert_eq!(c.output_latency_ms(), 15);
    }

    #[test]
    fn calibrate_floors_at_one_ms() {
        // A 0-ms calibration would be indistinguishable from
        // "not calibrated yet" and a later callback would try to
        // re-calibrate. Floor at 1 ms so the sticky guard actually
        // guards.
        let c = AudioClock::new();
        c.calibrate_from_cpal(None, 1, 48_000); // 1 frame / 48k = ~0 ms
        assert_eq!(c.output_latency_ms(), 1);
    }

    #[test]
    fn thirty_seconds_of_callbacks_drifts_less_than_one_ms() {
        // Simulate 30 s of 10 ms callbacks (480 frames each at 48 kHz
        // stereo). Accumulated advance should round-trip to 30_000 ms
        // exactly. Catches any "lost a ms per callback" arithmetic bug
        // that would silently desync by ~3 s per audio minute.
        let c = AudioClock::new();
        c.set_playing(true);
        const CALLBACK_FRAMES: u64 = 480;
        const CALLBACKS: u64 = 30 * 100; // 30 s * (100 callbacks / s)
        for _ in 0..CALLBACKS {
            c.advance_by_frames(CALLBACK_FRAMES, 48_000, 1_000);
        }
        assert_eq!(c.get_raw(), 30_000);
    }

    #[test]
    fn ragged_frame_counts_stay_within_one_ms_of_wall_time() {
        // Real cpal callbacks don't always deliver 480 frames — they
        // vary with OS scheduling. Interleave 441, 480, 512 frame
        // bursts and verify the accumulated ms stays within ±1 of the
        // naive `frames * 1000 / rate` sum. This is the stability
        // property a renderer actually depends on.
        let c = AudioClock::new();
        c.set_playing(true);
        let bursts: &[u64] = &[441, 480, 512, 441, 480, 512, 480];
        let mut naive_frames: u64 = 0;
        for _ in 0..100 {
            for &f in bursts {
                c.advance_by_frames(f, 48_000, 1_000);
                naive_frames += f;
            }
        }
        let naive_ms = naive_frames * 1000 / 48_000;
        let measured = c.get_raw();
        let diff = (measured as i64 - naive_ms as i64).abs();
        assert!(
            diff <= 1,
            "clock drifted {} ms from naive sum (measured {}, naive {})",
            diff,
            measured,
            naive_ms
        );
    }

    #[test]
    fn video_read_equals_audio_raw_minus_latency_after_burst() {
        // End-to-end: audio thread advances the clock, video thread
        // reads via `get()`, answer is raw - latency. Models the
        // actual hot-path that produces the on-screen picture.
        let c = AudioClock::new();
        c.set_playing(true);
        c.calibrate_from_cpal(Some(20_000_000), 480, 48_000); // 20 ms
        // Audio writes 500 ms worth of samples to the ring.
        c.advance_by_frames(24_000, 48_000, 1_000);
        let audio_write_head = c.get_raw();
        let video_read_head = c.get();
        assert_eq!(audio_write_head, 500);
        assert_eq!(video_read_head, 480);
        assert_eq!(audio_write_head - video_read_head, 20);
    }

    #[test]
    fn set_overrides_previous_position_exactly() {
        // A seek writes the new sequence-ms via `set`; the clock must
        // honour it even if the previous value was far ahead. Prevents
        // "seek to 0 but clock stayed at 30 s" regressions.
        let c = AudioClock::new();
        c.set_playing(true);
        c.advance_by_frames(48_000, 48_000, 1_000);
        assert_eq!(c.get_raw(), 1_000);
        c.set(42);
        assert_eq!(c.get_raw(), 42);
    }

    #[test]
    fn playing_flag_is_observed_by_advance() {
        // Flipping `set_playing(false)` between callbacks must freeze
        // the clock mid-flight. If the flag read isn't respected, a
        // paused player would keep advancing and resume would snap
        // forward by the pause duration.
        let c = AudioClock::new();
        c.set_playing(true);
        c.advance_by_frames(4_800, 48_000, 1_000);
        assert_eq!(c.get_raw(), 100);
        c.set_playing(false);
        c.advance_by_frames(4_800, 48_000, 1_000);
        assert_eq!(c.get_raw(), 100);
        c.set_playing(true);
        c.advance_by_frames(4_800, 48_000, 1_000);
        assert_eq!(c.get_raw(), 200);
    }

    #[test]
    fn user_offset_defaults_to_zero() {
        // Brand-new clocks must start neutral — any non-zero default
        // would silently corrupt sync on first launch before the user
        // has dialed anything in.
        let c = AudioClock::new();
        assert_eq!(c.user_offset_ms(), 0);
    }

    #[test]
    fn positive_user_offset_adds_to_calibrated_latency() {
        // User reports "picture runs 30 ms ahead of sound" → +30 ms
        // offset holds the frame back that much longer on top of the
        // 20 ms cpal estimate. Picture should read 50 ms behind raw.
        let c = AudioClock::new();
        c.set(1_000);
        c.set_output_latency_ms(20);
        c.set_user_offset_ms(30);
        assert_eq!(c.get_raw(), 1_000);
        assert_eq!(c.get(), 950);
    }

    #[test]
    fn negative_user_offset_subtracts_from_calibrated_latency() {
        // Over-estimated auto-latency: user dials back -10 ms so the
        // 20 ms estimate effectively becomes 10 ms of compensation.
        let c = AudioClock::new();
        c.set(1_000);
        c.set_output_latency_ms(20);
        c.set_user_offset_ms(-10);
        assert_eq!(c.get(), 990);
    }

    #[test]
    fn effective_latency_clamps_at_zero_when_user_offset_exceeds_calibration() {
        // If the user nukes the offset below -calibrated, `get()` must
        // not run *ahead* of raw — picture can't legitimately arrive
        // before the audio write head. Pin at `raw / 1000` instead of
        // wrapping or reading future samples.
        let c = AudioClock::new();
        c.set(1_000);
        c.set_output_latency_ms(20);
        c.set_user_offset_ms(-100);
        assert_eq!(c.get(), 1_000);
    }

    #[test]
    fn user_offset_clamps_to_declared_range() {
        // Guarantees the atomic can't overflow. ±30 000 ms is the public
        // contract documented on the const; values within the range pass
        // through unchanged, values outside saturate at the boundary.
        let c = AudioClock::new();
        c.set_user_offset_ms(25_000);
        assert_eq!(c.user_offset_ms(), 25_000);
        c.set_user_offset_ms(500_000);
        assert_eq!(c.user_offset_ms(), AudioClock::USER_OFFSET_RANGE_MS);
        c.set_user_offset_ms(-500_000);
        assert_eq!(c.user_offset_ms(), -AudioClock::USER_OFFSET_RANGE_MS);
    }

    #[test]
    fn user_offset_survives_roundtrip_through_setter() {
        // Prefs → clock → readout must round-trip exactly so the UI
        // displayed value stays consistent with what the clock is
        // actually applying.
        let c = AudioClock::new();
        c.set_user_offset_ms(42);
        assert_eq!(c.user_offset_ms(), 42);
        c.set_user_offset_ms(-37);
        assert_eq!(c.user_offset_ms(), -37);
        c.set_user_offset_ms(0);
        assert_eq!(c.user_offset_ms(), 0);
    }

    #[test]
    fn user_offset_does_not_mutate_raw_position() {
        // A nudge to the compensation is read-side only — the audio
        // write head must not move, or seeking / EOF detection on the
        // audio thread would drift with every slider wiggle.
        let c = AudioClock::new();
        c.set(1_234);
        c.set_user_offset_ms(250);
        assert_eq!(c.get_raw(), 1_234);
        c.set_user_offset_ms(-250);
        assert_eq!(c.get_raw(), 1_234);
    }

    #[test]
    fn user_offset_does_not_block_calibration_stickiness() {
        // The user nudge is orthogonal to auto-calibration; the
        // sticky-after-first-call guard still applies to the auto half.
        let c = AudioClock::new();
        c.set_user_offset_ms(50);
        c.calibrate_from_cpal(Some(15_000_000), 480, 48_000);
        assert_eq!(c.output_latency_ms(), 15);
        // A later callback must not overwrite the first one even though
        // the user has dialed in an offset.
        c.calibrate_from_cpal(Some(60_000_000), 480, 48_000);
        assert_eq!(c.output_latency_ms(), 15);
        assert_eq!(c.user_offset_ms(), 50);
    }

    #[test]
    fn atomic_ordering_serializes_across_threads() {
        // Advance from one thread, read from another — verifies the
        // Acquire/Release ordering on the atomics is tight enough
        // that the reader sees the advance. (Memory-ordering bugs
        // would manifest here as the reader observing stale values
        // or panicking on atomic weirdness under loom, but without
        // loom this still exercises the real Arc<Atomic> handoff.)
        let c = AudioClock::new();
        c.set_playing(true);
        let c2 = c.clone();
        let t = std::thread::spawn(move || {
            for _ in 0..1_000 {
                c2.advance_by_frames(48, 48_000, 1_000);
            }
        });
        // Spin the reader for roughly the same duration; we just need
        // to prove the handshake works, not benchmark it.
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut last = 0;
        while Instant::now() < deadline {
            let now = c.get_raw();
            assert!(now >= last, "clock went backwards ({last} -> {now})");
            last = now;
        }
        t.join().unwrap();
        // 1000 iterations × 48 frames @ 48k = 48_000 frames = 1000 ms.
        // The property we care about is just that we got here without
        // the reader observing a decreasing value; the final position
        // is a sanity check on the handoff.
        assert_eq!(c.get_raw(), 1_000);
        // Use `UNIX_EPOCH` to silence the unused-import lint on
        // platforms where the `time` imports are otherwise untouched.
        let _ = UNIX_EPOCH;
    }
}

#[cfg(test)]
mod mix_tests {
    //! Pure-math tests for the multi-lane mix helpers. Exercises the gain
    //! conversion and the drain/sum step without touching ffmpeg, cpal, or
    //! segment advancement — so a regression in mix arithmetic surfaces
    //! here instead of as a quiet gain error in preview playback.
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;

    fn empty_timeline() -> Arc<TimelineSync> {
        Arc::new(TimelineSync {
            segments: Arc::new(Vec::new()),
            active_index: AtomicUsize::new(0),
        })
    }

    fn lane(samples: &[f32], gain_linear: f32) -> AudioLane {
        AudioLane {
            timeline: empty_timeline(),
            ctx: None,
            pending: samples.iter().copied().collect::<VecDeque<_>>(),
            gain_linear,
            seg_idx: 0,
            exhausted: false,
        }
    }

    #[test]
    fn db_to_linear_unity_is_exact_one() {
        // Unity is a fast-path: we want it *exactly* 1.0, not `10^0` which
        // rounds to ~0.99999994 on f32.
        assert_eq!(db_to_linear(0.0), 1.0);
    }

    #[test]
    fn db_to_linear_minus_six_is_about_half() {
        // -6 dB ≈ 0.5012. Matches the export-side `volume=-6dB` filter so
        // preview and render stay audibly in sync.
        let v = db_to_linear(-6.0);
        assert!((v - 0.5011872).abs() < 1e-5, "got {v}");
    }

    #[test]
    fn db_to_linear_plus_six_is_about_two() {
        let v = db_to_linear(6.0);
        assert!((v - 1.9952624).abs() < 1e-5, "got {v}");
    }

    #[test]
    fn drain_and_mix_single_lane_passes_through_at_unity() {
        let mut lanes = vec![lane(&[0.25, -0.5, 0.75], 1.0)];
        let out = drain_and_mix(&mut lanes, 3);
        assert_eq!(out, vec![0.25, -0.5, 0.75]);
        assert!(lanes[0].pending.is_empty());
    }

    #[test]
    fn drain_and_mix_applies_per_lane_gain() {
        let mut lanes = vec![lane(&[1.0, 1.0, 1.0], 0.5)];
        let out = drain_and_mix(&mut lanes, 3);
        assert_eq!(out, vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn drain_and_mix_sums_two_lanes_with_gain() {
        // Two equal-length lanes, one at unity and one halved. The output
        // should be a sample-wise sum — this is the core mix invariant.
        let mut lanes = vec![
            lane(&[1.0, 0.5, -0.25], 1.0),
            lane(&[0.1, 0.2, 0.3], 0.5),
        ];
        let out = drain_and_mix(&mut lanes, 3);
        assert_eq!(out.len(), 3);
        assert!((out[0] - 1.05).abs() < 1e-6);
        assert!((out[1] - 0.6).abs() < 1e-6);
        assert!((out[2] - (-0.1)).abs() < 1e-6);
    }

    #[test]
    fn drain_and_mix_skips_empty_lane() {
        // An exhausted / drained lane must contribute silence (i.e. it's
        // skipped, which is mathematically the same as adding 0) — any
        // other behavior would produce clicks every time a lane runs out.
        let mut lanes = vec![lane(&[], 1.0), lane(&[0.3, 0.3, 0.3], 1.0)];
        let out = drain_and_mix(&mut lanes, 3);
        assert_eq!(out, vec![0.3, 0.3, 0.3]);
    }

    #[test]
    fn drain_and_mix_leaves_extra_samples_pending() {
        // If `n` is smaller than a lane's pending queue, the remainder
        // must stay in `pending` for the next pass — otherwise we'd lose
        // audio at every mix boundary.
        let mut lanes = vec![lane(&[1.0, 2.0, 3.0, 4.0], 1.0)];
        let out = drain_and_mix(&mut lanes, 2);
        assert_eq!(out, vec![1.0, 2.0]);
        assert_eq!(lanes[0].pending.len(), 2);
        assert_eq!(lanes[0].pending.front().copied(), Some(3.0));
    }
}
