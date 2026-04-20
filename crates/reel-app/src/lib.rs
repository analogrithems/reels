//! Reel desktop application: Slint UI, session, player, export.
//!
//! The `reel` binary (`src/main.rs`) calls [`run`]. Integration tests link against this crate
//! to exercise `AppWindow` (see `tests/ui_visual_golden.rs`).

mod asset_cache;
mod autosave;
mod effects;
mod footer;
mod help_markdown;
mod media_extensions;
mod player;
mod prefs;
mod project_io;
mod recent;
mod session;
mod shell;
mod subtitle;
mod timecode;
mod timeline;
mod timeline_chips;
mod ui_bridge;

use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use reel_core::project::{ClipOrientation, ClipScale};
use reel_core::TrackKind;
use reel_core::{
    build_mute_substitution_lane, export_concat_with_audio_lanes_oriented_with_gains,
    generate_silence_wav, ExportProgressFn,
};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use session::{
    path_matches_export_format, remux_failure_hint, split_enabled_for_playhead,
    video_lane_indices, web_export_format_from_preset_index, EditSession,
};
use slint::{ModelRc, VecModel};

/// Match v0: chevron tools popout is **closed** until the user opens it; reset whenever Rust
/// rebuilds media/timeline state so Slint does not keep it open across loads.
pub(crate) fn reset_tools_popup_ui(window: &AppWindow) {
    window.set_tools_popup_open(false);
    window.set_tools_effects_submenu_open(false);
}

fn bump_transport_forward(signed: &Arc<AtomicI32>) {
    const TIERS: [i32; 7] = [250, 500, 750, 1000, 1250, 1500, 2000];
    let cur = signed.load(Ordering::Relaxed);
    if cur <= 0 {
        signed.store(1000, Ordering::Relaxed);
        return;
    }
    let next = TIERS
        .iter()
        .find(|&&t| t > cur)
        .copied()
        .unwrap_or(TIERS[6]);
    signed.store(next, Ordering::Relaxed);
}

fn bump_transport_rewind(signed: &Arc<AtomicI32>) {
    const TIERS: [i32; 7] = [-250, -500, -750, -1000, -1250, -1500, -2000];
    let cur = signed.load(Ordering::Relaxed);
    if cur >= 0 {
        signed.store(-250, Ordering::Relaxed);
        return;
    }
    let next = TIERS
        .iter()
        .find(|&&t| t < cur)
        .copied()
        .unwrap_or(TIERS[6]);
    signed.store(next, Ordering::Relaxed);
}

fn transport_rate_label(milli: i32) -> String {
    format!("{:.2}×", milli as f64 / 1000.0)
}
use slint::ComponentHandle;
use uuid::Uuid;

use crate::media_extensions::{
    AUDIO_FILE_EXTENSIONS, OPEN_MEDIA_EXTENSIONS, SUBTITLE_FILE_EXTENSIONS,
    VIDEO_CONTAINER_EXTENSIONS,
};
use crate::prefs::AppPrefs;
use crate::project_io::{is_project_document_path, save_project};
use crate::recent::RecentStore;
use crate::ui_bridge::on_ui;

slint::include_modules!();

type ExportSpanVec = Vec<(PathBuf, f64, f64)>;

fn save_preview_zoom_prefs(app_prefs: &RefCell<AppPrefs>, w: &AppWindow) {
    let mut p = app_prefs.borrow_mut();
    p.preview_zoom_percent = w.get_preview_zoom_percent().clamp(25.0, 400.0);
    p.preview_zoom_actual = w.get_preview_zoom_actual();
    p.save();
}

/// Returns the single [`ClipOrientation`] shared by every primary-track video clip in the project,
/// or `Err` with a user-facing message when clips disagree. Identity-only returns `Ok(None)`, so
/// the export path can skip `-vf` and keep stream-copy presets when nothing was rotated.
///
/// Mixed orientations are rejected rather than silently flattened: applying a single `-vf` chain
/// to a concat of differently-oriented sources would produce a visibly broken output.
fn unified_export_video_orientation(
    session: &EditSession,
) -> Result<Option<ClipOrientation>, String> {
    let Some(p) = session.project() else {
        return Ok(None);
    };
    let Some(clips) = timeline::clips_from_project(p) else {
        return Ok(None);
    };
    // `timeline::clips_from_project` builds a reduced view; look up the real
    // `Clip.orientation` by source path so we don't have to thread `orientation`
    // through the timeline cache.
    let orientations: Vec<ClipOrientation> = clips
        .iter()
        .map(|c| c.orientation)
        .collect();
    let mut iter = orientations.into_iter();
    let Some(first) = iter.next() else {
        return Ok(None);
    };
    for o in iter {
        if o != first {
            return Err("Cannot export: primary video clips have different rotate/flip settings. \
                        Apply the same orientation to all clips (or remove the outliers) and try again."
                .into());
        }
    }
    Ok(if first.is_identity() {
        None
    } else {
        Some(first)
    })
}

/// Same policy as [`unified_export_video_orientation`] but for [`ClipScale`]. Mixed scales across
/// primary-track clips are rejected — applying one `-vf scale=…` on a concat of differently-scaled
/// sources would pick one scale and apply it uniformly, producing visible distortion.
fn unified_export_video_scale(session: &EditSession) -> Result<Option<ClipScale>, String> {
    let Some(p) = session.project() else {
        return Ok(None);
    };
    let video_track = p
        .tracks
        .iter()
        .find(|t| t.kind == reel_core::TrackKind::Video);
    let Some(track) = video_track else {
        return Ok(None);
    };
    let mut iter = track
        .clip_ids
        .iter()
        .filter_map(|id| p.clips.iter().find(|c| c.id == *id))
        .map(|c| c.scale);
    let Some(first) = iter.next() else {
        return Ok(None);
    };
    for s in iter {
        if s != first {
            return Err("Cannot export: primary video clips have different resize settings. \
                        Apply the same scale to all clips (or remove the outliers) and try again."
                .into());
        }
    }
    Ok(if first.is_identity() {
        None
    } else {
        Some(first)
    })
}

/// Primary video spans + one audio-lane span-list per dedicated audio track + per-primary-clip
/// audio-mute mask for ffmpeg export (empty timeline → `None`).
///
/// When `range_ms` is `Some`, the seek-bar In/Out markers limit the output: video and audio
/// concat inputs are sliced to the range (sequence clock) and rebased so the first span begins
/// at **0**. Returns `None` when the range produces no video coverage so the caller can tell
/// the user nothing would export.
///
/// The audio lanes are ordered by `Project.tracks` audio-track index. Empty lanes (after range
/// slicing or because the track has no clips) are dropped so the downstream ffmpeg `amix`
/// dispatcher sees only usable lanes.
///
/// The **mute mask** is parallel to `video_spans`: `true` marks a primary clip whose
/// embedded audio should not reach the output. The export thread consults it only when
/// there's no dedicated audio lane (otherwise the dual-mux / amix path already strips
/// primary audio via `-map 0:v:0`), building a silence-substituted synthetic audio lane.
fn export_timeline_payload(
    session: &EditSession,
    range_ms: Option<(f64, f64)>,
) -> Option<(ExportSpanVec, Vec<ExportSpanVec>, Vec<bool>)> {
    export_timeline_payload_with_gains(session, range_ms).map(|(v, a, m, _g)| (v, a, m))
}

/// Gain-aware variant of [`export_timeline_payload`]. Returns a per-lane
/// `gain_db` vector that is **parallel to** the returned `audio_lane_spans`
/// — i.e. both are filtered to drop empty lanes in lock-step, so
/// `gains[i]` applies to `audio_lane_spans[i]`.
///
/// The range-slicing pass can strip a lane down to nothing (e.g. an overlay
/// audio lane that sits entirely outside the In/Out window). Whenever we drop
/// that lane's spans we must also drop its gain, or the amix dispatcher
/// rejects the payload with a length mismatch. That's why gain collection
/// piggybacks on the same `filter_map` instead of reading the project
/// tracks independently.
fn export_timeline_payload_with_gains(
    session: &EditSession,
    range_ms: Option<(f64, f64)>,
) -> Option<(ExportSpanVec, Vec<ExportSpanVec>, Vec<bool>, Vec<f32>)> {
    let video_clips = session.project().and_then(timeline::clips_from_project)?;
    let video_clips = match range_ms {
        Some(r) => timeline::slice_clips_to_range_ms(&video_clips, r),
        None => video_clips,
    };
    if video_clips.is_empty() {
        return None;
    }
    let mute_mask: Vec<bool> = video_clips.iter().map(|c| c.audio_mute).collect();
    let segs: ExportSpanVec = video_clips
        .into_iter()
        .map(|c| (c.path, c.media_in_s, c.media_out_s))
        .collect();
    let lane_clip_lists = session
        .project()
        .map(timeline::clips_from_all_audio_tracks)
        .unwrap_or_default();
    // `audio_track_gains_db` is in project order over `TrackKind::Audio` and
    // matches the outer order of `lane_clip_lists` 1:1 — so zipping them is
    // safe. We then `filter_map` away empty lanes in lock-step, preserving
    // the invariant that `gains.len() == audio_lane_spans.len()`.
    let all_gains = session.audio_track_gains_db();
    let (audio_lane_spans, lane_gains_db): (Vec<ExportSpanVec>, Vec<f32>) = lane_clip_lists
        .into_iter()
        .zip(all_gains.into_iter())
        .filter_map(|(clips, gain)| {
            let sliced = match range_ms {
                Some(r) => timeline::slice_clips_to_range_ms(&clips, r),
                None => clips,
            };
            if sliced.is_empty() {
                None
            } else {
                let spans: ExportSpanVec = sliced
                    .into_iter()
                    .map(|c| (c.path, c.media_in_s, c.media_out_s))
                    .collect();
                Some((spans, gain))
            }
        })
        .unzip();
    Some((segs, audio_lane_spans, mute_mask, lane_gains_db))
}

fn export_save_dialog(fmt: reel_core::WebExportFormat) -> rfd::FileDialog {
    let d = rfd::FileDialog::new().set_title("Export media…");
    match fmt {
        reel_core::WebExportFormat::Mp4Remux => d.add_filter("MP4", &["mp4", "m4v"]),
        reel_core::WebExportFormat::Mp4H264Aac => {
            d.add_filter("MP4 (H.264 + AAC)", &["mp4", "m4v"])
        }
        reel_core::WebExportFormat::Mp4H265Aac => {
            d.add_filter("MP4 (HEVC + AAC)", &["mp4", "m4v"])
        }
        reel_core::WebExportFormat::WebmVp8Opus => d.add_filter("WebM (VP8 + Opus)", &["webm"]),
        reel_core::WebExportFormat::WebmVp9Opus => d.add_filter("WebM (VP9 + Opus)", &["webm"]),
        reel_core::WebExportFormat::WebmAv1Opus => d.add_filter("WebM (AV1 + Opus)", &["webm"]),
        reel_core::WebExportFormat::MkvRemux => d.add_filter("Matroska", &["mkv"]),
        reel_core::WebExportFormat::MovRemux => d.add_filter("QuickTime", &["mov"]),
        reel_core::WebExportFormat::MovProResHq => {
            d.add_filter("QuickTime (ProRes 422 HQ + PCM)", &["mov"])
        }
        reel_core::WebExportFormat::MkvDnxhrHq => {
            d.add_filter("Matroska (DNxHR HQ + PCM)", &["mkv"])
        }
        reel_core::WebExportFormat::GifSharp
        | reel_core::WebExportFormat::GifGood
        | reel_core::WebExportFormat::GifShare
        | reel_core::WebExportFormat::GifTiny => d.add_filter("Animated GIF", &["gif"]),
    }
}

/// Injection seam for the native "save as…" dialog that `on_export_preset_confirm`
/// opens after the user picks a preset. Production uses [`RfdSaveDialog`], which
/// delegates to `rfd::FileDialog`; tests pass a stub that returns a pre-baked
/// path (or `None` to emulate a cancelled dialog) so the callback is fully
/// exercisable under `i-slint-backend-testing` without a windowing system.
///
/// See `docs/phases-ui-test.md` Phase 1b / Phase 2.
pub(crate) trait SaveDialogProvider {
    /// Show a "save as…" dialog configured for `fmt` and return the chosen path
    /// (or `None` when the user cancels).
    fn pick(&self, fmt: reel_core::WebExportFormat) -> Option<PathBuf>;
}

/// Production [`SaveDialogProvider`] backed by [`rfd::FileDialog`].
pub(crate) struct RfdSaveDialog;

impl SaveDialogProvider for RfdSaveDialog {
    fn pick(&self, fmt: reel_core::WebExportFormat) -> Option<PathBuf> {
        export_save_dialog(fmt).save_file()
    }
}

/// Outcome of [`prepare_export_job`] — the pure decision the preset-confirm
/// callback has to make before kicking off an ffmpeg thread.
///
/// Breaking this out keeps the decision table testable in isolation: the
/// Slint callback only dispatches on the enum (status line vs. spawn a
/// background thread), so every branch the user can actually hit has a
/// corresponding `#[test]` in `export_payload_tests`.
#[derive(Debug)]
pub(crate) enum ExportPreflight {
    /// Do nothing (e.g. the user cancelled the save dialog). No status update.
    NoOp,
    /// Show a user-facing message and stop. Covers invalid preset index,
    /// empty In/Out range, mixed orientations, and wrong-extension paths.
    Status(String),
    /// Proceed to the ffmpeg export thread with these parameters.
    Spawn {
        video_spans: ExportSpanVec,
        /// One span-list per dedicated audio lane (project `TrackKind::Audio` track). Empty
        /// when the project has no audio lanes; single-entry delegates to the existing
        /// dual-mux path; 2+ entries engage the `amix` filter_complex.
        audio_lane_spans: Vec<ExportSpanVec>,
        /// Parallel to `video_spans`: per-primary-clip audio-mute flag. Only consulted
        /// when `audio_lane_spans` is empty and `mute_audio` is false — the export
        /// thread uses it to build a silence-substituted synthetic audio lane so
        /// partial-mute timelines produce the expected output instead of either
        /// `-an`'ing everything or erroring out. Ignored when a dedicated audio lane
        /// is present (primary audio is already stripped by `-map 0:v:0`).
        primary_mute_mask: Vec<bool>,
        /// Parallel to `audio_lane_spans`: per-lane **gain in dB** (0.0 = unity).
        /// Any non-zero entry routes the export through the amix path even for a
        /// single lane, applying `volume=XdB` ahead of the mix. Empty when there
        /// are no audio lanes; otherwise `len() == audio_lane_spans.len()`.
        lane_gains_db: Vec<f32>,
        orientation: Option<ClipOrientation>,
        scale: Option<ClipScale>,
        /// True when the ffmpeg call should pass `-an`, dropping audio from the
        /// output entirely. Set when **all** primary-track clips are muted and
        /// there's no dedicated audio lane to layer on top.
        mute_audio: bool,
        /// When `Some`, ffmpeg burns the `.srt` cues into the output via the
        /// `subtitles=` video filter (first clip on the first `TrackKind::Subtitle`
        /// lane, resolved by [`EditSession::primary_subtitle_path`]).
        subtitle_path: Option<PathBuf>,
        dest: PathBuf,
        fmt: reel_core::WebExportFormat,
        range_ms: Option<(f64, f64)>,
    },
}

/// Pure decision logic behind `on_export_preset_confirm`.
///
/// Walks the same gauntlet the production callback walks — preset index → payload
/// → orientation → save dialog → extension check — and returns an
/// [`ExportPreflight`]. No Slint, no threading, no ffmpeg here; the caller
/// (`install_export_preset_confirm_callback`) dispatches on the result.
///
/// `save_dialog` is injectable so tests can drive every branch
/// (`StubSaveDialog` returning `Some(path)` vs `None`).
pub(crate) fn prepare_export_job(
    session: &EditSession,
    preset_index: i32,
    save_dialog: &dyn SaveDialogProvider,
) -> ExportPreflight {
    let Some(fmt) = web_export_format_from_preset_index(preset_index) else {
        return ExportPreflight::Status("Invalid export preset.".into());
    };

    let range_ms = session.marker_range_ms();
    let Some((video_spans, audio_lane_spans, primary_mute_mask, lane_gains_db)) =
        export_timeline_payload_with_gains(session, range_ms)
    else {
        return ExportPreflight::Status(
            "No clips in the In/Out range — clear markers or adjust them to export.".into(),
        );
    };

    let orientation = match unified_export_video_orientation(session) {
        Ok(o) => o,
        Err(msg) => return ExportPreflight::Status(msg),
    };
    let scale = match unified_export_video_scale(session) {
        Ok(s) => s,
        Err(msg) => return ExportPreflight::Status(msg),
    };

    // Per-clip audio mute (U2-e "Remove audio").
    //   • All muted + no audio lane ⇒ `-an` (drop audio entirely).
    //   • Partial mute + no audio lane ⇒ silence-substitution lane (synthesized
    //     in the export thread via `generate_silence_wav` + `build_mute_substitution_lane`).
    //   • Any mute + audio lane present ⇒ ignore the mask; primary audio is
    //     already stripped by the dual-mux / amix `-map 0:v:0`.
    let all_muted = primary_mute_mask.iter().all(|m| *m) && !primary_mute_mask.is_empty();
    let mute_audio = all_muted && audio_lane_spans.is_empty();

    let Some(dest) = save_dialog.pick(fmt) else {
        return ExportPreflight::NoOp;
    };

    if !path_matches_export_format(&dest, fmt) {
        return ExportPreflight::Status(format!(
            "Use a .{} file name for this preset.",
            fmt.file_extension()
        ));
    }

    let subtitle_path = session.primary_subtitle_path();

    ExportPreflight::Spawn {
        video_spans,
        audio_lane_spans,
        primary_mute_mask,
        lane_gains_db,
        orientation,
        scale,
        mute_audio,
        subtitle_path,
        dest,
        fmt,
        range_ms,
    }
}

/// Wire the **Export preset → Confirm** callback (`on_export_preset_confirm`)
/// onto `window`.
///
/// Always closes the preset sheet, then calls [`prepare_export_job`] with
/// [`RfdSaveDialog`] and dispatches on the returned [`ExportPreflight`]:
/// `Status` routes to the status line, `Spawn` starts the ffmpeg thread with
/// progress reporting and the shared cancel flag, `NoOp` does nothing (the
/// user cancelled the save dialog).
///
/// Extracted so tests can exercise `prepare_export_job` with a `StubSaveDialog`
/// without installing Slint callbacks — see `docs/phases-ui-test.md` Phase 2.
fn install_export_preset_confirm_callback(
    window: &AppWindow,
    session: Rc<RefCell<EditSession>>,
    export_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
) {
    let weak = window.as_weak();
    window.on_export_preset_confirm(move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        let idx = w.get_export_preset_index();
        w.set_export_preset_dialog_visible(false);
        drop(w);

        let preflight = prepare_export_job(&session.borrow(), idx, &RfdSaveDialog);
        match preflight {
            ExportPreflight::NoOp => {}
            ExportPreflight::Status(msg) => {
                if let Some(w) = weak.upgrade() {
                    w.set_status_text(msg.into());
                }
            }
            ExportPreflight::Spawn {
                video_spans,
                audio_lane_spans,
                primary_mute_mask,
                lane_gains_db,
                orientation,
                scale,
                mute_audio,
                subtitle_path,
                dest,
                fmt,
                range_ms,
            } => {
                let cancel = Arc::new(AtomicBool::new(false));
                *export_cancel.lock().expect("export cancel mutex") = Some(cancel.clone());
                let start_status = match range_ms {
                    Some((i, o)) => {
                        format!("Exporting range {:.3}–{:.3} s…", i / 1000.0, o / 1000.0)
                    }
                    None => "Exporting…".to_string(),
                };
                if let Some(w) = weak.upgrade() {
                    w.set_status_text(start_status.into());
                    w.set_export_progress(0.0);
                    w.set_export_in_progress(true);
                }
                let weak_done = weak.clone();
                let weak_prog = weak.clone();
                let dest_disp = dest.display().to_string();
                let slot_clear = Arc::clone(&export_cancel);
                let on_ratio: Option<ExportProgressFn> = Some(Arc::new(move |r: f64| {
                    let pct = (r.clamp(0.0, 1.0) * 100.0).round() as i32;
                    let wk = weak_prog.clone();
                    on_ui(wk, move |win| {
                        win.set_export_progress(r.clamp(0.0, 1.0) as f32);
                        win.set_status_text(format!("Exporting… {pct}%").into());
                    });
                }));
                let res = std::thread::Builder::new()
                    .name("reel-export".into())
                    .spawn(move || {
                        // U2-e: when some primary clips are muted but there's no
                        // dedicated audio lane, synthesize a substitution lane that
                        // swaps silence in for the muted spans. `_silence_dir` stays
                        // on the stack so the tempfile outlives ffmpeg.
                        let mut audio_lane_spans = audio_lane_spans;
                        let mut lane_gains_db = lane_gains_db;
                        let _silence_dir = if !mute_audio
                            && audio_lane_spans.is_empty()
                            && primary_mute_mask.iter().any(|m| *m)
                        {
                            let max_span_s = video_spans
                                .iter()
                                .zip(primary_mute_mask.iter())
                                .filter(|(_, m)| **m)
                                .map(|((_, i, o), _)| (*o - *i).max(0.0))
                                .fold(0.0_f64, f64::max);
                            match tempfile::TempDir::new() {
                                Ok(dir) => {
                                    let silence_path = dir.path().join("reel_mute_silence.wav");
                                    if generate_silence_wav(&silence_path, max_span_s).is_ok() {
                                        let lane = build_mute_substitution_lane(
                                            &video_spans,
                                            &primary_mute_mask,
                                            &silence_path,
                                        );
                                        if !lane.is_empty() {
                                            audio_lane_spans = vec![lane];
                                            // Synthesized lane carries no user-configurable gain —
                                            // silence @ 0 dB is just silence. Keeping the gains
                                            // vector parallel to `audio_lane_spans` is what the
                                            // export dispatcher validates on entry.
                                            lane_gains_db = vec![0.0];
                                        }
                                        Some(dir)
                                    } else {
                                        None
                                    }
                                }
                                Err(_) => None,
                            }
                        } else {
                            None
                        };
                        // Gain-aware export entry point. We always pass `Some(&gains)`
                        // when there are any lanes so the dispatcher can detect the
                        // non-unity case and route through amix; `None` is reserved
                        // for the pure "no audio lane / mute_audio" path that short-
                        // circuits to `-an` before gain can even matter.
                        let gains_slice: Option<&[f32]> = if audio_lane_spans.is_empty() {
                            None
                        } else {
                            Some(&lane_gains_db)
                        };
                        let r = export_concat_with_audio_lanes_oriented_with_gains(
                            &video_spans,
                            &audio_lane_spans,
                            gains_slice,
                            orientation,
                            scale,
                            subtitle_path.as_deref(),
                            mute_audio,
                            &dest,
                            fmt,
                            Some(cancel.as_ref()),
                            on_ratio,
                        );
                        let msg = match &r {
                            Ok(()) => format!("Exported to {dest_disp}"),
                            Err(e) => {
                                if e.is_cancelled() {
                                    "Export cancelled.".into()
                                } else {
                                    let base = format!("Export failed: {e}");
                                    match remux_failure_hint(fmt) {
                                        Some(hint) => format!("{base}. {hint}"),
                                        None => base,
                                    }
                                }
                            }
                        };
                        on_ui(weak_done, move |w| {
                            *slot_clear.lock().expect("export cancel mutex") = None;
                            w.set_export_in_progress(false);
                            w.set_export_progress(0.0);
                            w.set_status_text(msg.into());
                        });
                        if let Err(e) = &r {
                            if !e.is_cancelled() {
                                tracing::error!(error = %e, "export failed");
                            }
                        }
                    });
                if let Err(e) = res {
                    tracing::error!(?e, "failed to spawn export thread");
                    *export_cancel.lock().expect("export cancel mutex") = None;
                    on_ui(weak.clone(), |w| {
                        w.set_export_in_progress(false);
                        w.set_export_progress(0.0);
                        w.set_status_text("Could not start export".into());
                    });
                }
            }
        }
    });
}

/// Wire the **Export…** pre-flight callback (`on_file_export`) onto `window`.
///
/// The pre-flight does *not* run ffmpeg and does *not* open a save dialog; it
/// only inspects the session and decides one of three outcomes:
/// 1. Empty project or no video → silent no-op (nothing the user sees).
/// 2. Both markers set but slice has no clips → status line shows the
///    "No clips in the In/Out range…" message; preset sheet stays closed.
/// 3. Otherwise → flip `export_preset_dialog_visible` on so the Slint sheet
///    appears, where the user picks a preset.
///
/// Extracted so `#[test]` modules can invoke `window.invoke_file_export()` and
/// assert against the three branches without going through a native save
/// dialog. Production uses the same helper — no behavior drift between
/// tests and `main()`. See `docs/phases-ui-test.md` Phase 2.
fn install_export_preflight_callback(window: &AppWindow, session: Rc<RefCell<EditSession>>) {
    let weak = window.as_weak();
    window.on_file_export(move || {
        let range = session.borrow().marker_range_ms();
        if export_timeline_payload(&session.borrow(), range).is_none() {
            // Distinguish "no project / no video" (silent) from "markers set but empty range".
            if range.is_some() && export_timeline_payload(&session.borrow(), None).is_some() {
                if let Some(w) = weak.upgrade() {
                    w.set_status_text(
                        "No clips in the In/Out range — clear markers or adjust them to export."
                            .into(),
                    );
                }
            }
            return;
        }
        if let Some(w) = weak.upgrade() {
            w.set_export_preset_dialog_visible(true);
        }
    });
}

/// Installs pointer-drag handlers for the scrub-bar In/Out yellow handles.
///
/// Slint clamps the dispatched `ms` so the two handles can't cross; this Rust
/// side re-clamps to `[0, duration]` defensively and writes through to
/// [`EditSession::set_in_marker_ms`] / [`EditSession::set_out_marker_ms`],
/// then refreshes the UI via [`sync_marker_ui`]. No status-text updates on
/// each drag frame — that would spam the footer. The Set-In/Set-Out *keyboard*
/// paths still announce via `status_text`; a drop-style "released" summary
/// could be added later if the user wants audible-ish feedback.
/// Install the four **Edit → Audio Lane Gain…** callbacks (open, lane-changed,
/// cancel, confirm) on `window`. Extracted so `#[cfg(test)]` smoke tests can
/// wire the same production handlers onto a headless `AppWindow` and exercise
/// them via `window.invoke_edit_audio_lane_gain()` / `invoke_gain_confirm(...)`.
/// Mirrors the shape of [`install_edit_drag_marker_callbacks`] + the
/// `install_export_preflight_callback` helper used by the export smoke test.
pub fn install_audio_lane_gain_callbacks(
    window: &AppWindow,
    session: Rc<RefCell<EditSession>>,
    debouncer: Rc<autosave::AutosaveDebouncer>,
    recent: Rc<RefCell<RecentStore>>,
) {
    {
        // Open: prefill lane 1's current gain. Re-checks the "no audio lanes"
        // case so tests that bypass the menu gate get a clean status message
        // instead of an empty sheet.
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_edit_audio_lane_gain(move || {
            let Some(w) = weak.upgrade() else { return };
            let gains = session.borrow().audio_track_gains_db();
            if gains.is_empty() {
                w.set_status_text("No audio lanes — add one first.".into());
                return;
            }
            w.set_gain_sheet_max_lane(gains.len() as i32);
            w.set_gain_sheet_lane(1);
            w.set_gain_sheet_db(gains[0]);
            w.set_gain_sheet_error("".into());
            w.set_gain_sheet_visible(true);
        });
    }

    {
        // Lane-changed: user typed a new lane number, refresh the dB field.
        // Out-of-range lanes leave the field alone — confirm will catch them.
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_gain_lane_changed(move |lane_1based| {
            let Some(w) = weak.upgrade() else { return };
            let gains = session.borrow().audio_track_gains_db();
            let idx = lane_1based as isize - 1;
            if idx >= 0 && (idx as usize) < gains.len() {
                w.set_gain_sheet_db(gains[idx as usize]);
            }
        });
    }

    {
        // Cancel: hide the sheet without touching the project.
        let weak = window.as_weak();
        window.on_gain_cancel(move || {
            if let Some(w) = weak.upgrade() {
                w.set_gain_sheet_visible(false);
                w.set_gain_sheet_error("".into());
            }
        });
    }

    {
        // Confirm: validate lane (1-based → 0-based), delegate to
        // `set_audio_track_gain_db` (clamping + NaN handling + undo), close on
        // success, surface inline errors on failure (leaving the sheet open so
        // the user can correct without reopening).
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_gain_confirm(move |lane_1based, db| {
            let Some(w) = weak.upgrade() else { return };
            if lane_1based < 1 {
                w.set_gain_sheet_error(
                    format!("Lane must be 1 or greater (got {lane_1based}).").into(),
                );
                return;
            }
            let lane = lane_1based as usize - 1;
            let lane_count = session.borrow().audio_track_gains_db().len();
            if lane >= lane_count {
                w.set_gain_sheet_error(
                    format!(
                        "Lane {lane_1based} is out of range (have {lane_count} audio lane{}).",
                        if lane_count == 1 { "" } else { "s" }
                    )
                    .into(),
                );
                return;
            }
            // Drop the mutable borrow before the match arms run — otherwise
            // the `session.borrow()` call inside `sync_menu_and_autosave` /
            // the applied-value read-back would panic with `RefCell already
            // mutably borrowed` (the scrutinee's temporary lives to end-of-match).
            let res = session.borrow_mut().set_audio_track_gain_db(lane, db);
            match res {
                Ok(()) => {
                    w.set_gain_sheet_visible(false);
                    w.set_gain_sheet_error("".into());
                    sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                    let applied = session
                        .borrow()
                        .audio_track_gain_db(lane)
                        .unwrap_or(0.0);
                    w.set_status_text(
                        format!("Lane {lane_1based} gain set to {applied:+.1} dB").into(),
                    );
                }
                Err(e) => {
                    w.set_gain_sheet_error(format!("{e:#}").into());
                }
            }
        });
    }
}

pub fn install_edit_drag_marker_callbacks(window: &AppWindow, session: Rc<RefCell<EditSession>>) {
    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_edit_drag_in_marker_ms(move |ms| {
            let Some(w) = weak.upgrade() else { return };
            let dur = w.get_duration_ms() as f64;
            let clamped = (ms as f64).clamp(0.0, dur.max(0.0));
            if session.borrow_mut().set_in_marker_ms(clamped).is_ok() {
                sync_marker_ui(&w, &session.borrow());
            }
        });
    }
    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_edit_drag_out_marker_ms(move |ms| {
            let Some(w) = weak.upgrade() else { return };
            let dur = w.get_duration_ms() as f64;
            let clamped = (ms as f64).clamp(0.0, dur.max(0.0));
            if session.borrow_mut().set_out_marker_ms(clamped).is_ok() {
                sync_marker_ui(&w, &session.borrow());
            }
        });
    }
}

/// **Edit → Scan for Errors…** wiring.
///
/// Opens the results sheet in a "running" phase, spawns a background thread
/// that calls [`reel_core::media::scan::scan_file`] against the primary
/// clip's source, streams progress back to Slint via `on_ui`, and lands on
/// either "done" (verdict + details) or "error" (couldn't open the file).
/// The `scan-dismiss` callback sets a shared `AtomicBool` cancel flag —
/// the worker notices it the next time it sees a progress tick and returns
/// early, so dismissing an in-flight scan is cheap and doesn't leak a
/// decoder thread for the rest of the file's duration.
pub fn install_scan_for_errors_callbacks(
    window: &AppWindow,
    session: Rc<RefCell<EditSession>>,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // Shared between menu-fire and dismiss so we can cancel a running scan
    // without racing against thread spawn. `true` = cancel requested.
    let cancel_flag: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let cancel_flag = Rc::clone(&cancel_flag);
        window.on_edit_scan_errors(move || {
            let Some(w) = weak.upgrade() else { return };
            // Find the source file for the clip under the playhead.
            let ph = w.get_playhead_ms() as f64;
            let sess = session.borrow();
            let Some(project) = sess.project() else {
                w.set_status_text("No project loaded — nothing to scan.".into());
                return;
            };
            let Some(clip) = timeline::primary_clip_ref_at_seq_ms(project, ph) else {
                w.set_status_text("No clip at playhead — nothing to scan.".into());
                return;
            };
            let path = clip.source_path.clone();
            drop(sess);
            if path.as_os_str().is_empty() || !path.exists() {
                w.set_status_text(
                    format!("Scan: source file missing: {}", path.display()).into(),
                );
                return;
            }

            // Prime the sheet — "running" with a 0% bar.
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            w.set_scan_file_name(file_name.into());
            w.set_scan_phase("running".into());
            w.set_scan_progress(0.0);
            w.set_scan_headline("".into());
            w.set_scan_details("".into());
            w.set_scan_verdict("".into());
            w.set_scan_repair_recommended(false);
            w.set_scan_sheet_visible(true);

            // Shared cancel flag; replace any stale one from a prior run.
            let cancel = Arc::new(AtomicBool::new(false));
            *cancel_flag.borrow_mut() = Some(Arc::clone(&cancel));

            let weak_worker = weak.clone();
            let path_for_worker = path.clone();
            let cancel_worker = Arc::clone(&cancel);
            std::thread::Builder::new()
                .name("reel-scan".into())
                .spawn(move || {
                    let weak_progress = weak_worker.clone();
                    let cancel_progress = Arc::clone(&cancel_worker);
                    let scan_result = reel_core::media::scan::scan_file(
                        &path_for_worker,
                        move |ratio| {
                            if cancel_progress.load(Ordering::SeqCst) {
                                return; // worker will check again between packets
                            }
                            let wk = weak_progress.clone();
                            on_ui(wk, move |win| {
                                if win.get_scan_sheet_visible() {
                                    win.set_scan_progress(ratio as f32);
                                }
                            });
                        },
                    );

                    // If the sheet was already dismissed, don't overwrite UI.
                    if cancel_worker.load(Ordering::SeqCst) {
                        return;
                    }

                    match scan_result {
                        Ok(report) => {
                            let verdict = report.verdict.as_str().to_string();
                            let headline = report.headline();
                            let details = if report.issues.is_empty() {
                                String::new()
                            } else {
                                let mut lines: Vec<String> = report
                                    .issues
                                    .iter()
                                    .map(|i| format!("• [{}] {}", i.kind, i.message))
                                    .collect();
                                if (report.error_count + report.warning_count) as usize
                                    > report.issues.len()
                                {
                                    lines.push(format!(
                                        "… and {} more (not shown)",
                                        (report.error_count + report.warning_count) as usize
                                            - report.issues.len()
                                    ));
                                }
                                lines.join("\n")
                            };
                            let repair = report.repair_recommended();
                            tracing::info!(
                                target: "reel_app::scan",
                                path = %path_for_worker.display(),
                                verdict = %verdict,
                                packets = report.packets_read,
                                video_frames = report.video_frames_decoded,
                                audio_frames = report.audio_frames_decoded,
                                errors = report.error_count,
                                warnings = report.warning_count,
                                "scan complete"
                            );
                            on_ui(weak_worker, move |win| {
                                if !win.get_scan_sheet_visible() {
                                    return;
                                }
                                win.set_scan_progress(1.0);
                                win.set_scan_verdict(verdict.into());
                                win.set_scan_headline(headline.into());
                                win.set_scan_details(details.into());
                                win.set_scan_repair_recommended(repair);
                                win.set_scan_phase("done".into());
                            });
                        }
                        Err(e) => {
                            let msg = format!("{e}");
                            tracing::warn!(
                                target: "reel_app::scan",
                                path = %path_for_worker.display(),
                                error = %msg,
                                "scan failed to open file"
                            );
                            on_ui(weak_worker, move |win| {
                                if !win.get_scan_sheet_visible() {
                                    return;
                                }
                                win.set_scan_headline(msg.into());
                                win.set_scan_phase("error".into());
                            });
                        }
                    }
                })
                .ok();
        });
    }

    {
        let weak = window.as_weak();
        let cancel_flag = Rc::clone(&cancel_flag);
        window.on_scan_dismiss(move || {
            // Flip the cancel flag so any in-flight worker bails on its next
            // progress tick. (A fully-finished worker's UI write is guarded
            // by `get_scan_sheet_visible()` above so dismissing after the
            // worker finishes is still safe.)
            if let Some(flag) = cancel_flag.borrow().as_ref() {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if let Some(w) = weak.upgrade() {
                w.set_scan_sheet_visible(false);
                w.set_scan_phase("idle".into());
                w.set_scan_progress(0.0);
            }
        });
    }
}

fn begin_export_effect(
    weak: slint::Weak<AppWindow>,
    session: Rc<RefCell<EditSession>>,
    effect: effects::EffectKind,
) {
    let Some(sidecar) = effects::resolve_sidecar_dir() else {
        on_ui(weak, |w| {
            w.set_status_text(
                "Sidecar not found. Set REEL_SIDECAR_DIR or run from the repo root.".into(),
            );
        });
        return;
    };
    let playhead_seq_ms = weak
        .upgrade()
        .map(|w| w.get_playhead_ms() as f64)
        .unwrap_or(0.0);
    let (media, pts_ms) = match session
        .borrow()
        .project()
        .and_then(|p| timeline::resolve_for_project(p, playhead_seq_ms))
    {
        Some(x) => x,
        None => {
            on_ui(weak, |w| w.set_status_text("No video loaded.".into()));
            return;
        }
    };

    let title = match effect {
        effects::EffectKind::FaceFusion => "Save face swap as PNG…",
        effects::EffectKind::FaceEnhance => "Save face enhance as PNG…",
        effects::EffectKind::RvmBackground => "Save background removal as PNG…",
    };
    let save = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rfd::FileDialog::new()
            .set_title(title)
            .add_filter("PNG", &["png"])
            .save_file()
    }));
    let dest = match save {
        Ok(Some(p)) => p,
        Ok(None) => return,
        Err(_) => {
            tracing::error!("rfd save dialog panicked");
            return;
        }
    };

    let res = std::thread::Builder::new()
        .name("reel-effect".into())
        .spawn(move || {
            let r = effects::apply_effect_to_png(&media, pts_ms, effect, &sidecar, &dest);
            let msg = match &r {
                Ok(()) => format!("Effect saved — {}", dest.display()),
                Err(e) => format!("Effect failed: {e:#}"),
            };
            on_ui(weak, move |w| w.set_status_text(msg.into()));
        });
    if let Err(e) = res {
        tracing::error!(?e, "failed to spawn effect thread");
    }
}

fn empty_tl_model() -> ModelRc<TlChip> {
    ModelRc::new(VecModel::from(Vec::<TlChip>::new()))
}

fn clear_timeline_models(window: &AppWindow) {
    window.set_tl_video_track_count(0);
    window.set_tl_audio_track_count(0);
    window.set_tl_subtitle_track_count(0);
    window.set_tl_video_project_track_count(0);
    window.set_tl_audio_project_track_count(0);
    window.set_tl_subtitle_project_track_count(0);
    window.set_tl_vrow0(empty_tl_model());
    window.set_tl_vrow1(empty_tl_model());
    window.set_tl_vrow2(empty_tl_model());
    window.set_tl_vrow3(empty_tl_model());
    window.set_tl_arow0(empty_tl_model());
    window.set_tl_arow1(empty_tl_model());
    window.set_tl_arow2(empty_tl_model());
    window.set_tl_arow3(empty_tl_model());
    window.set_tl_srow0(empty_tl_model());
    window.set_tl_srow1(empty_tl_model());
    window.set_tl_srow2(empty_tl_model());
    window.set_tl_srow3(empty_tl_model());
}

thread_local! {
    /// UI-thread singleton for the async asset cache (waveforms / thumbs).
    /// Populated once in [`run`] right after player spawn. Kept on a
    /// thread-local so chip enrichment doesn't require plumbing the cache
    /// handle through every `sync_timeline_chips` call site. Accessed only
    /// from the Slint event-loop thread — no cross-thread sync needed.
    static ASSET_CACHE: std::cell::RefCell<Option<asset_cache::AssetCache>> =
        const { std::cell::RefCell::new(None) };
}

/// Enrich each audio chip in `rows` with its cached waveform (if any) or
/// enqueue a generation job. Runs in-place on the TlChip slice just before
/// the model is pushed to Slint. Called from [`sync_timeline_chips`] for
/// every audio row.
fn attach_audio_waveforms(rows: &mut [TlChip], p: &reel_core::project::Project) {
    ASSET_CACHE.with(|slot| {
        let Some(cache) = slot.borrow().clone() else {
            return;
        };
        for chip in rows.iter_mut() {
            if chip.is_video || chip.is_subtitle {
                continue;
            }
            let Some((cache_key, source_path)) = resolve_chip_source(chip, p) else {
                continue;
            };
            let in_ms = chip.begin_ms.max(0) as u64;
            let out_ms = chip.end_ms.max(chip.begin_ms) as u64;
            if let Some(img) =
                cache.get_or_request_waveform(&cache_key, &source_path, in_ms, out_ms)
            {
                chip.waveform = img;
                chip.waveform_ready = true;
            }
        }
    });
}

/// Enrich each video chip in `rows` with its cached thumbnail strip (if
/// any) or enqueue a generation job. Mirrors [`attach_audio_waveforms`]
/// but for `is_video` chips and delegating to the thumbnails half of
/// the AssetCache. The placeholder stripe stays visible on
/// `thumbnails_ready == false`, so the chip shows up instantly and the
/// strip swaps in on worker completion via `refresh-timeline-chips`.
fn attach_video_thumbnails(rows: &mut [TlChip], p: &reel_core::project::Project) {
    ASSET_CACHE.with(|slot| {
        let Some(cache) = slot.borrow().clone() else {
            return;
        };
        for chip in rows.iter_mut() {
            if !chip.is_video || chip.is_subtitle {
                continue;
            }
            let Some((cache_key, source_path)) = resolve_chip_source(chip, p) else {
                continue;
            };
            let in_ms = chip.begin_ms.max(0) as u64;
            let out_ms = chip.end_ms.max(chip.begin_ms) as u64;
            if let Some(img) =
                cache.get_or_request_thumbnails(&cache_key, &source_path, in_ms, out_ms)
            {
                chip.thumbnails = img;
                chip.thumbnails_ready = true;
            }
        }
    });
}

/// Resolve a chip to `(cache_key, source_path)` for the AssetCache.
///
/// Real project chips key on their clip UUID and look the source path up
/// on the backing `Clip`. Synthetic single-media chips (empty `clip_id`)
/// key on and decode directly from `chip.source_path`, which
/// `timeline_chips::synthetic_full_width_chip` populates with the primary
/// clip's path. Returns `None` for chips that have neither — nothing to
/// decode.
fn resolve_chip_source(
    chip: &TlChip,
    p: &reel_core::project::Project,
) -> Option<(String, std::path::PathBuf)> {
    if !chip.clip_id.is_empty() {
        let uuid = Uuid::parse_str(chip.clip_id.as_str()).ok()?;
        let clip = p.clips.iter().find(|c| c.id == uuid)?;
        return Some((chip.clip_id.to_string(), clip.source_path.clone()));
    }
    if chip.source_path.is_empty() {
        return None;
    }
    let path = std::path::PathBuf::from(chip.source_path.as_str());
    Some((chip.source_path.to_string(), path))
}

fn sync_timeline_chips(window: &AppWindow, session: &EditSession) {
    let Some(p) = session.project() else {
        clear_timeline_models(window);
        return;
    };
    let mut sync = timeline_chips::timeline_chip_sync(p, session.opened_from_project_document());
    // Enrich audio chips with cached waveforms and video chips with
    // cached thumbnail strips before pushing the model. Synchronous
    // cache hit → `*_ready: true`; miss → placeholder stripe shows
    // while the worker generates it, then `refresh-timeline-chips`
    // fires and this function re-runs with the freshly-cached image.
    for row in sync.audio.iter_mut() {
        attach_audio_waveforms(row, p);
    }
    for row in sync.video.iter_mut() {
        attach_video_thumbnails(row, p);
    }
    window.set_tl_video_track_count(sync.video_display_n);
    window.set_tl_audio_track_count(sync.audio_display_n);
    window.set_tl_subtitle_track_count(sync.subtitle_display_n);
    window.set_tl_video_project_track_count(sync.video_project_n);
    window.set_tl_audio_project_track_count(sync.audio_project_n);
    window.set_tl_subtitle_project_track_count(sync.subtitle_project_n);
    window.set_tl_vrow0(ModelRc::new(VecModel::from(sync.video[0].clone())));
    window.set_tl_vrow1(ModelRc::new(VecModel::from(sync.video[1].clone())));
    window.set_tl_vrow2(ModelRc::new(VecModel::from(sync.video[2].clone())));
    window.set_tl_vrow3(ModelRc::new(VecModel::from(sync.video[3].clone())));
    window.set_tl_arow0(ModelRc::new(VecModel::from(sync.audio[0].clone())));
    window.set_tl_arow1(ModelRc::new(VecModel::from(sync.audio[1].clone())));
    window.set_tl_arow2(ModelRc::new(VecModel::from(sync.audio[2].clone())));
    window.set_tl_arow3(ModelRc::new(VecModel::from(sync.audio[3].clone())));
    window.set_tl_srow0(ModelRc::new(VecModel::from(sync.subtitle[0].clone())));
    window.set_tl_srow1(ModelRc::new(VecModel::from(sync.subtitle[1].clone())));
    window.set_tl_srow2(ModelRc::new(VecModel::from(sync.subtitle[2].clone())));
    window.set_tl_srow3(ModelRc::new(VecModel::from(sync.subtitle[3].clone())));
}

/// Push the active subtitle cue text for the current playhead into the UI
/// overlay property. Empty string when no subtitle track covers this instant
/// or we're in an inter-cue gap — Slint hides the overlay Rectangle in that
/// case so the gap blanks rather than holding the prior cue.
pub(crate) fn refresh_subtitle_overlay(
    window: &AppWindow,
    session: &EditSession,
    cache: &subtitle::SubtitleCueCache,
) {
    let text = session
        .project()
        .and_then(|p| {
            subtitle::subtitle_text_at_seq_ms(p, window.get_playhead_ms() as f64, cache)
        })
        .unwrap_or_default();
    window.set_current_subtitle_text(text.into());
}

fn sync_footer(window: &AppWindow, session: &EditSession) {
    let ph = window.get_playhead_ms() as f64;
    let has_project = session.project().is_some();
    window.set_footer_visible(has_project);
    if let Some(f) = footer::compute_footer_lines(session.project(), ph, session.dirty) {
        window.set_footer_codec_line(f.codec_line.into());
        window.set_footer_path_line(f.path_line.into());
        window.set_footer_save_line(f.save_line.into());
        window.set_footer_unsaved(f.unsaved);
    } else {
        window.set_footer_codec_line("".into());
        window.set_footer_path_line("".into());
        window.set_footer_save_line("".into());
        window.set_footer_unsaved(false);
    }
}

pub(crate) fn sync_menu(window: &AppWindow, session: &EditSession) {
    window.set_close_enabled(session.close_enabled());
    window.set_revert_enabled(session.revert_enabled());
    window.set_save_enabled(session.save_enabled());
    window.set_undo_enabled(session.undo_enabled());
    window.set_redo_enabled(session.redo_enabled());
    window.set_video_track_lanes(session.video_track_row_labels().join("\n").into());
    window.set_audio_track_lanes(session.audio_track_row_labels().join("\n").into());
    let insert_audio_ok = session
        .project()
        .map(|p| p.tracks.iter().any(|t| t.kind == TrackKind::Audio))
        .unwrap_or(false)
        && window.get_media_ready();
    window.set_insert_audio_enabled(insert_audio_ok);
    // Overlay gate is strictly `media-ready` — the helper creates the lane
    // itself, so it doesn't require one to pre-exist.
    window.set_overlay_audio_enabled(window.get_media_ready());
    // Replace shares the same gate — same reasoning (helper creates its own
    // lane and the per-primary-clip mute pass only needs a project to scan).
    window.set_replace_audio_enabled(window.get_media_ready());
    // Replace-and-clear uses the same gate as plain Replace; when there are no
    // existing audio lanes it still does the right thing (it's a no-op on the
    // clear pass and behaves identically to Replace Audio).
    window.set_replace_audio_clear_others_enabled(window.get_media_ready());
    // Audio Lane Gain needs a project AND at least one audio lane to act on;
    // the sheet would have nothing to prefill otherwise. Using `media-ready`
    // alone would let the menu open into an empty sheet, which is worse than
    // just greying it out.
    let audio_gain_ok = window.get_media_ready() && !session.audio_track_gains_db().is_empty();
    window.set_audio_gain_enabled(audio_gain_ok);
    let insert_subtitle_ok = session
        .project()
        .map(|p| p.tracks.iter().any(|t| t.kind == TrackKind::Subtitle))
        .unwrap_or(false)
        && window.get_media_ready();
    window.set_insert_subtitle_enabled(insert_subtitle_ok);
    if let Some(p) = session.project() {
        if let Some(clips) = timeline::clips_from_project(p) {
            let d = timeline::sequence_duration_ms(&clips);
            window.set_duration_ms(d.max(1.0) as f32);
        } else {
            window.set_duration_ms(0.0);
        }
        let ph = window.get_playhead_ms() as f64;
        let idxs = video_lane_indices(p);
        window.set_move_clip_down_enabled(
            idxs.len() >= 2 && timeline::primary_clip_id_at_seq_ms(p, ph).is_some(),
        );
        window.set_move_clip_up_enabled(
            idxs.len() >= 2
                && p.tracks
                    .get(idxs[1])
                    .map(|t| !t.clip_ids.is_empty())
                    .unwrap_or(false),
        );
        window.set_split_at_playhead_enabled(
            window.get_media_ready() && split_enabled_for_playhead(p, ph),
        );
        window.set_rotate_enabled(session.rotate_enabled(ph));
        window.set_trim_enabled(window.get_media_ready() && session.trim_enabled(ph));
        window.set_resize_enabled(window.get_media_ready() && session.resize_enabled(ph));
        let mute_state = session.audio_mute_state_at_seq_ms(ph);
        window.set_audio_mute_enabled(window.get_media_ready() && mute_state.is_some());
        window.set_audio_mute_active(mute_state.unwrap_or(false));
        // Edit → Scan for Errors: enabled whenever we have a primary clip with a
        // non-empty source path — scan opens the file independently of the
        // playback pipeline, so it doesn't need full `media-ready`, just a file.
        let scan_ok = timeline::primary_clip_ref_at_seq_ms(p, ph)
            .map(|c| !c.source_path.as_os_str().is_empty() && c.source_path.exists())
            .unwrap_or(false);
        window.set_scan_enabled(scan_ok);
        sync_audio_track_menu(window, session, ph);
    } else {
        window.set_duration_ms(0.0);
        window.set_move_clip_down_enabled(false);
        window.set_move_clip_up_enabled(false);
        window.set_split_at_playhead_enabled(false);
        window.set_rotate_enabled(false);
        window.set_trim_enabled(false);
        window.set_resize_enabled(false);
        window.set_audio_mute_enabled(false);
        window.set_audio_mute_active(false);
        window.set_scan_enabled(false);
        clear_audio_track_menu(window);
    }
    let ph = window.get_playhead_ms();
    let dur = window.get_duration_ms();
    timecode::refresh_time_labels(window, ph, dur);
    sync_footer(window, session);
    sync_timeline_chips(window, session);
    sync_marker_ui(window, session);
    // Ask Slint to ping `on_subtitle_refresh`, which has the cache + session
    // captured. Keeps `sync_menu`'s signature unchanged while covering every
    // edit/scrub path — playback ticks refresh via `subtitle-sync-timer` in
    // `app.slint`.
    window.invoke_subtitle_refresh();
}

/// Push the session's In/Out marker state into Slint properties. Uses `-1.0` as the
/// "unset" sentinel since Slint's `float` property can't represent `Option`.
pub(crate) fn sync_marker_ui(window: &AppWindow, session: &EditSession) {
    window.set_marker_in_ms(session.in_marker_ms().map(|v| v as f32).unwrap_or(-1.0));
    window.set_marker_out_ms(session.out_marker_ms().map(|v| v as f32).unwrap_or(-1.0));
    window.set_marker_any_set(session.has_any_marker());
}

fn sync_recent_menu(window: &AppWindow, recent: &RecentStore) {
    let labels = recent.menu_labels();
    window.set_recent_line0(labels[0].clone().into());
    window.set_recent_line1(labels[1].clone().into());
    window.set_recent_line2(labels[2].clone().into());
    window.set_recent_line3(labels[3].clone().into());
    window.set_recent_line4(labels[4].clone().into());
    window.set_recent_line5(labels[5].clone().into());
    window.set_recent_line6(labels[6].clone().into());
    window.set_recent_line7(labels[7].clone().into());
    window.set_recent_line8(labels[8].clone().into());
    window.set_recent_line9(labels[9].clone().into());
    window.set_recent_has_entries(!recent.is_empty());
}

fn reload_player_timeline(sender: &player::PlayerCmdSender, session: &EditSession) {
    let Some(p) = session.project() else {
        sender.send(player::Cmd::Close);
        return;
    };
    let Some(video) = timeline::timeline_sync_from_project(p) else {
        sender.send(player::Cmd::Close);
        return;
    };
    // Build one `AudioLaneLoad` per non-empty audio lane. `clips_from_all_audio_tracks`
    // and `audio_track_gains_db` are both in `TrackKind::Audio` project order, so
    // the zip is 1:1. Empty lanes (no clips) are filtered out in lock-step with
    // their gain — an empty resulting vec tells the audio thread to fall back to
    // the video segment's embedded audio (pre-multi-lane behavior).
    let lane_clip_lists = timeline::clips_from_all_audio_tracks(p);
    let all_gains = session.audio_track_gains_db();
    let audio_lanes: Vec<player::AudioLaneLoad> = lane_clip_lists
        .into_iter()
        .zip(all_gains.into_iter())
        .filter_map(|(clips, gain_db)| {
            timeline::TimelineSync::from_clips(&clips).map(|sync| player::AudioLaneLoad {
                timeline: sync,
                gain_db,
            })
        })
        .collect();
    sender.send(player::Cmd::LoadTimeline {
        video,
        audio_lanes,
    });
}

fn sync_menu_and_autosave(
    window: &AppWindow,
    session_rc: &Rc<RefCell<EditSession>>,
    debouncer: &autosave::AutosaveDebouncer,
    recent: &Rc<RefCell<RecentStore>>,
) {
    sync_menu(window, &session_rc.borrow());
    sync_recent_menu(window, &recent.borrow());
    debouncer.nudge(Rc::clone(session_rc));
}

/// Maximum audio-stream entries shown in **Edit → Audio Track**. The submenu
/// has this many statically-declared `MenuItem` slots in `app.slint`; extra
/// streams on an exotic file fall off the end. Kept at six to cover the
/// overwhelming majority of Blu-ray / broadcast sources (commentary +
/// language dubs + descriptive audio) without bloating the menu on files
/// that only have one or two tracks.
const AUDIO_TRACK_MENU_SLOTS: usize = 6;

/// Push the current primary-clip's audio streams into the `audio-track-*`
/// Slint properties backing **Edit → Audio Track**. `ph` is the playhead in
/// sequence ms; we consult the clip under it for both the stream list
/// (`AudioStreamInfo` vec from probe) and the current selection. Labels are
/// `"<index>: <display_label>"` so users can tell identical-codec streams
/// apart by ffmpeg index when neither `title` nor `language` is present.
///
/// `audio-track-enabled` flips true only when **≥ 2** streams exist — single-
/// stream files have nothing to switch between, so showing the submenu would
/// just frustrate. Slots beyond `streams.len()` are zeroed so a file that
/// previously showed more entries doesn't leak stale rows.
fn sync_audio_track_menu(window: &AppWindow, session: &EditSession, ph: f64) {
    let streams = session.audio_streams_at_seq_ms(ph);
    let selected = session.audio_stream_index_at_seq_ms(ph);
    let enable = window.get_media_ready() && streams.len() >= 2;
    window.set_audio_track_enabled(enable);
    window.set_audio_track_default_checked(selected.is_none());

    let setters: [(fn(&AppWindow, slint::SharedString), fn(&AppWindow, bool), fn(&AppWindow, i32)); AUDIO_TRACK_MENU_SLOTS] = [
        (AppWindow::set_audio_track_label_0, AppWindow::set_audio_track_checked_0, AppWindow::set_audio_track_index_0),
        (AppWindow::set_audio_track_label_1, AppWindow::set_audio_track_checked_1, AppWindow::set_audio_track_index_1),
        (AppWindow::set_audio_track_label_2, AppWindow::set_audio_track_checked_2, AppWindow::set_audio_track_index_2),
        (AppWindow::set_audio_track_label_3, AppWindow::set_audio_track_checked_3, AppWindow::set_audio_track_index_3),
        (AppWindow::set_audio_track_label_4, AppWindow::set_audio_track_checked_4, AppWindow::set_audio_track_index_4),
        (AppWindow::set_audio_track_label_5, AppWindow::set_audio_track_checked_5, AppWindow::set_audio_track_index_5),
    ];
    for (i, (set_label, set_checked, set_index)) in setters.iter().enumerate() {
        match streams.get(i) {
            Some(info) => {
                let label = format!("{}: {}", info.index, info.display_label());
                set_label(window, label.into());
                set_checked(window, selected == Some(info.index));
                set_index(window, info.index as i32);
            }
            None => {
                set_label(window, "".into());
                set_checked(window, false);
                set_index(window, -1);
            }
        }
    }
}

/// Blanks every **Edit → Audio Track** slot — used when no project is loaded
/// or `sync_menu` hits the "no project" branch. Without this the submenu
/// would keep showing the last-opened file's streams even after close.
fn clear_audio_track_menu(window: &AppWindow) {
    window.set_audio_track_enabled(false);
    window.set_audio_track_default_checked(true);
    let setters: [(fn(&AppWindow, slint::SharedString), fn(&AppWindow, bool), fn(&AppWindow, i32)); AUDIO_TRACK_MENU_SLOTS] = [
        (AppWindow::set_audio_track_label_0, AppWindow::set_audio_track_checked_0, AppWindow::set_audio_track_index_0),
        (AppWindow::set_audio_track_label_1, AppWindow::set_audio_track_checked_1, AppWindow::set_audio_track_index_1),
        (AppWindow::set_audio_track_label_2, AppWindow::set_audio_track_checked_2, AppWindow::set_audio_track_index_2),
        (AppWindow::set_audio_track_label_3, AppWindow::set_audio_track_checked_3, AppWindow::set_audio_track_index_3),
        (AppWindow::set_audio_track_label_4, AppWindow::set_audio_track_checked_4, AppWindow::set_audio_track_index_4),
        (AppWindow::set_audio_track_label_5, AppWindow::set_audio_track_checked_5, AppWindow::set_audio_track_index_5),
    ];
    for (set_label, set_checked, set_index) in setters.iter() {
        set_label(window, "".into());
        set_checked(window, false);
        set_index(window, -1);
    }
}

/// Clears the session and stops the player (empty **File → Close Window** state).
fn close_window_and_clear_player(
    p: &player::PlayerCmdSender,
    session: &Rc<RefCell<EditSession>>,
    weak: &slint::Weak<AppWindow>,
    subtitle_cache: &subtitle::SubtitleCueCache,
) {
    let mut s = session.borrow_mut();
    s.clear_media();
    p.send(player::Cmd::Close);
    drop(s);
    subtitle_cache.clear();
    if let Some(w) = weak.upgrade() {
        reset_tools_popup_ui(&w);
        w.set_current_subtitle_text("".into());
        sync_menu(&w, &session.borrow());
    }
}

fn open_path_from_ui(
    path: PathBuf,
    sender: &player::PlayerCmdSender,
    session: &Rc<RefCell<EditSession>>,
    weak: &slint::Weak<AppWindow>,
    debouncer: &autosave::AutosaveDebouncer,
    recent: &Rc<RefCell<RecentStore>>,
    subtitle_cache: &subtitle::SubtitleCueCache,
) {
    tracing::info!(?path, "open path");
    if let Some(w) = weak.upgrade() {
        w.set_is_playing(false);
        w.set_media_ready(false);
        w.set_current_subtitle_text("".into());
        reset_tools_popup_ui(&w);
    }
    subtitle_cache.clear();
    let open_result = session.borrow_mut().open_media(path.clone());
    match open_result {
        Ok(()) => {
            {
                let mut r = recent.borrow_mut();
                if is_project_document_path(&path) {
                    r.record_project(path.clone());
                } else {
                    r.record_media(path.clone());
                }
            }
            if let Some(w) = weak.upgrade() {
                sync_menu_and_autosave(&w, session, debouncer, recent);
            }
            reload_player_timeline(sender, &session.borrow());
        }
        Err(e) => {
            tracing::error!(error = %e, "open failed");
            if let Some(w) = weak.upgrade() {
                w.set_status_text(format!("Open failed: {e}").into());
                sync_menu_and_autosave(&w, session, debouncer, recent);
            }
        }
    }
}

fn show_help_window(doc: shell::HelpDoc) {
    let (title, body) = shell::help_bundle(doc);
    let body = help_markdown::markdown_to_styled(body);
    match HelpWindow::new() {
        Ok(h) => {
            h.set_help_title(title.into());
            h.set_body_text(body);
            if let Err(e) = h.show() {
                tracing::warn!(error = %e, "help window show failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "help window create failed"),
    }
}

pub fn run() -> Result<()> {
    // Session logs always go to a file (see `reel_core::logging`); stdout mirroring is optional.
    let _log = reel_core::logging::init()?;
    if let Some(ref p) = _log.session_log_path {
        tracing::info!(session_log = %p.display(), "reel starting");
    } else {
        tracing::info!("reel starting (tracing was already initialized)");
    }

    let cli_startup_path = match parse_cli_startup_path() {
        Ok(p) => p,
        Err(()) => {
            eprintln!(
                "Usage: reel [<path>]\n\
                 \n\
                 Opens an optional media file or .reel project. At most one path.\n\
                 You can also set REEL_OPEN_PATH when no CLI path is given."
            );
            std::process::exit(2);
        }
    };

    let window = AppWindow::new()?;
    window.set_transport_rate_label("1.00×".into());
    window.set_media_ready(false);
    reset_tools_popup_ui(&window);
    timecode::refresh_time_labels(&window, 0.0, 0.0);
    window.set_video_track_lanes("".into());
    window.set_audio_track_lanes("".into());
    clear_timeline_models(&window);
    window.set_insert_audio_enabled(false);
    window.set_overlay_audio_enabled(false);
    window.set_replace_audio_enabled(false);
    window.set_replace_audio_clear_others_enabled(false);
    window.set_audio_gain_enabled(false);
    window.set_insert_subtitle_enabled(false);
    window.set_move_clip_down_enabled(false);
    window.set_move_clip_up_enabled(false);
    window.set_split_at_playhead_enabled(false);
    window.set_rotate_enabled(false);
    window.set_trim_enabled(false);
    window.set_trim_sheet_visible(false);
    window.set_resize_enabled(false);
    window.set_resize_sheet_visible(false);
    window.set_audio_mute_enabled(false);
    window.set_audio_mute_active(false);
    clear_audio_track_menu(&window);
    window.set_marker_in_ms(-1.0);
    window.set_marker_out_ms(-1.0);
    window.set_marker_any_set(false);
    window.set_footer_visible(false);
    window.set_footer_codec_line("".into());
    window.set_footer_path_line("".into());
    window.set_footer_save_line("".into());
    window.set_footer_unsaved(false);
    window.set_video_fit_mode(0);
    window.set_stay_on_top(false);

    let app_prefs = Rc::new(RefCell::new(AppPrefs::load()));
    let master_vol = (app_prefs.borrow().master_volume.clamp(0.0, 1.0) * 1000.0).round() as u32;
    let vol_arc = Arc::new(AtomicU32::new(master_vol));
    window.set_volume_percent(app_prefs.borrow().master_volume * 100.0);
    let playback_loop = Arc::new(AtomicBool::new(app_prefs.borrow().playback_loop));
    window.set_loop_playback(app_prefs.borrow().playback_loop);
    window.set_preview_zoom_percent(app_prefs.borrow().preview_zoom_percent);
    window.set_preview_zoom_actual(app_prefs.borrow().preview_zoom_actual);
    window.set_view_show_status(app_prefs.borrow().show_footer_status);
    window.set_controls_overlay_always_visible(app_prefs.borrow().controls_overlay_always_visible);
    window.set_view_show_video_tracks(app_prefs.borrow().show_video_tracks);
    window.set_view_show_audio_tracks(app_prefs.borrow().show_audio_tracks);
    window.set_view_show_subtitle_tracks(app_prefs.borrow().show_subtitle_tracks);
    window.set_window_fullscreen(false);
    let playback_signed_milli = Arc::new(AtomicI32::new(1000));

    let session = Rc::new(RefCell::new(EditSession::default()));
    let debouncer = Rc::new(autosave::AutosaveDebouncer::new(window.as_weak()));
    let recent = Rc::new(RefCell::new(RecentStore::load()));
    // Subtitle cue cache — shared across every handler that refreshes the
    // overlay (scrub, nudge, play-tick timer) so a subtitle file is parsed
    // at most once per session.
    let subtitle_cache = Rc::new(subtitle::SubtitleCueCache::default());
    // Tracks the clip currently loaded into the Trim Clip… sheet, so `on_trim_confirm` can
    // apply the edit without racing against the playhead after the sheet opens.
    let trim_target: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));
    // Tracks the clip currently loaded into the Resize Video… sheet.
    let resize_target: Rc<RefCell<Option<Uuid>>> = Rc::new(RefCell::new(None));
    let export_cancel = Arc::new(Mutex::new(None::<Arc<AtomicBool>>));

    let player = match player::spawn_player(
        &window,
        vol_arc,
        playback_signed_milli.clone(),
        playback_loop.clone(),
    ) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to start player threads");
            window.set_status_text(format!("Player init failed: {e}").into());
            window.run()?;
            return Err(e);
        }
    };

    // Apply the persisted manual A/V offset to the audio clock before the
    // UI becomes responsive — otherwise a user with e.g. +120 ms Bluetooth
    // compensation saved would see one playback frame of misaligned
    // picture between load and the first menu interaction.
    {
        let ms = app_prefs.borrow().audio_offset_ms;
        player.audio_clock.set_user_offset_ms(ms);
        window.set_audio_offset_ms(ms);
    }

    // Background asset cache: decodes audio peaks / thumbnails off the UI
    // thread and swaps them into TlChip waveforms as they finish. The
    // placeholder diagonal-stripe tile is bound once here so every lane
    // can reference it through the `placeholder-stripe` property.
    let asset_cache = asset_cache::AssetCache::spawn(window.as_weak());
    window.set_timeline_placeholder_stripe(asset_cache.placeholder_image());
    ASSET_CACHE.with(|slot| {
        *slot.borrow_mut() = Some(asset_cache.clone());
    });
    {
        let weak = window.as_weak();
        let session_cb = Rc::clone(&session);
        window.on_refresh_timeline_chips(move || {
            if let Some(w) = weak.upgrade() {
                sync_timeline_chips(&w, &session_cb.borrow());
            }
        });
    }

    {
        let vol = player.master_volume_1000.clone();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_volume_changed(move |pct| {
            let p = (pct as f64).clamp(0.0, 100.0) / 100.0;
            vol.store((p * 1000.0).round() as u32, Ordering::Relaxed);
            app_prefs.borrow_mut().master_volume = p as f32;
            app_prefs.borrow().save();
        });
    }

    {
        let signed = playback_signed_milli.clone();
        let p = player.cmd_sender();
        let weak = window.as_weak();
        window.on_transport_forward(move || {
            bump_transport_forward(&signed);
            if let Some(w) = weak.upgrade() {
                if w.get_media_ready() {
                    w.set_transport_rate_label(
                        transport_rate_label(signed.load(Ordering::Relaxed)).into(),
                    );
                    w.set_is_playing(true);
                    p.send(player::Cmd::Play);
                }
            }
        });
    }
    {
        let signed = playback_signed_milli.clone();
        let p = player.cmd_sender();
        let weak = window.as_weak();
        window.on_transport_rewind(move || {
            bump_transport_rewind(&signed);
            if let Some(w) = weak.upgrade() {
                if w.get_media_ready() {
                    w.set_transport_rate_label(
                        transport_rate_label(signed.load(Ordering::Relaxed)).into(),
                    );
                    w.set_is_playing(true);
                    p.send(player::Cmd::Play);
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        let loop_flag = playback_loop.clone();
        window.on_view_toggle_loop(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let new = !w.get_loop_playback();
            w.set_loop_playback(new);
            loop_flag.store(new, Ordering::Relaxed);
            app_prefs.borrow_mut().playback_loop = new;
            app_prefs.borrow().save();
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_toggle_show_status(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let new = !w.get_view_show_status();
            w.set_view_show_status(new);
            app_prefs.borrow_mut().show_footer_status = new;
            app_prefs.borrow().save();
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_toggle_controls_always_visible(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let new = !w.get_controls_overlay_always_visible();
            w.set_controls_overlay_always_visible(new);
            app_prefs.borrow_mut().controls_overlay_always_visible = new;
            app_prefs.borrow().save();
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_toggle_video_tracks(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let new = !w.get_view_show_video_tracks();
            w.set_view_show_video_tracks(new);
            app_prefs.borrow_mut().show_video_tracks = new;
            app_prefs.borrow().save();
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_toggle_audio_tracks(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let new = !w.get_view_show_audio_tracks();
            w.set_view_show_audio_tracks(new);
            app_prefs.borrow_mut().show_audio_tracks = new;
            app_prefs.borrow().save();
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_toggle_subtitle_tracks(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let new = !w.get_view_show_subtitle_tracks();
            w.set_view_show_subtitle_tracks(new);
            app_prefs.borrow_mut().show_subtitle_tracks = new;
            app_prefs.borrow().save();
        });
    }

    // A/V Offset: nudge the manual output-latency offset by the caller-
    // supplied delta (±25 ms from the menu / Shift+Arrow shortcut). The
    // audio clock clamps to ±30 000 ms, then we mirror the clamped value
    // back to the window + prefs so the menu readout, the actual
    // compensation, and the on-disk pref stay in lockstep.
    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        let clock = player.audio_clock.clone();
        window.on_audio_offset_nudge(move |delta_ms| {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let next = w.get_audio_offset_ms().saturating_add(delta_ms);
            clock.set_user_offset_ms(next);
            // Read back the clamped value so the UI reflects the actual
            // state of the clock (a user spamming past the ±30 000 ms
            // cap would otherwise see a stale property).
            let applied = clock.user_offset_ms();
            w.set_audio_offset_ms(applied);
            app_prefs.borrow_mut().audio_offset_ms = applied;
            app_prefs.borrow().save();
        });
    }

    // A/V Offset reset → 0. Separate callback from nudge so the menu item
    // reads naturally and the property binding can disable "Reset" at 0.
    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        let clock = player.audio_clock.clone();
        window.on_audio_offset_reset(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            clock.set_user_offset_ms(0);
            w.set_audio_offset_ms(0);
            app_prefs.borrow_mut().audio_offset_ms = 0;
            app_prefs.borrow().save();
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_zoom_in(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            if w.get_preview_zoom_actual() {
                w.set_preview_zoom_actual(false);
                w.set_preview_zoom_percent(200.0);
            } else {
                let p = (w.get_preview_zoom_percent() + 25.0).min(400.0);
                w.set_preview_zoom_percent(p);
            }
            save_preview_zoom_prefs(&app_prefs, &w);
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_zoom_out(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            if w.get_preview_zoom_actual() {
                w.set_preview_zoom_actual(false);
                w.set_preview_zoom_percent(100.0);
            } else {
                let p = (w.get_preview_zoom_percent() - 25.0).max(25.0);
                w.set_preview_zoom_percent(p);
            }
            save_preview_zoom_prefs(&app_prefs, &w);
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_zoom_fit(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            w.set_video_fit_mode(0);
            w.set_preview_zoom_actual(false);
            w.set_preview_zoom_percent(100.0);
            // Re-center — fit-to-viewport doesn't overflow, so any carried
            // pan would just look like a stuck offset next time the user
            // zooms back in.
            w.set_preview_pan_x(0.0);
            w.set_preview_pan_y(0.0);
            save_preview_zoom_prefs(&app_prefs, &w);
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_view_zoom_actual_size(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            w.set_preview_zoom_actual(true);
            // Actual-size on a > viewport-sized frame starts centred — the
            // user can then pan from there.
            w.set_preview_pan_x(0.0);
            w.set_preview_pan_y(0.0);
            save_preview_zoom_prefs(&app_prefs, &w);
        });
    }

    {
        let weak = window.as_weak();
        window.on_view_toggle_fullscreen(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let win = w.window();
            let next = !win.is_fullscreen();
            win.set_fullscreen(next);
            w.set_window_fullscreen(next);
        });
    }

    {
        let weak = window.as_weak();
        window.on_view_exit_fullscreen(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            w.window().set_fullscreen(false);
            w.set_window_fullscreen(false);
        });
    }

    let weak = window.as_weak();
    {
        let sender = player.cmd_sender();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        let subs = Rc::clone(&subtitle_cache);
        window.on_file_open(move || match prompt_open_dialog() {
            Some(path) => {
                open_path_from_ui(path, &sender, &session, &weak, &debouncer, &recent, &subs);
            }
            None => tracing::debug!("open cancelled"),
        });
    }

    {
        let sender = player.cmd_sender();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        let subs = Rc::clone(&subtitle_cache);
        let weak = window.as_weak();
        window.on_file_open_recent(move |idx| {
            let Some(path) = recent.borrow().path_for_menu_index(idx) else {
                return;
            };
            if !path.exists() {
                recent.borrow_mut().remove_path(&path);
                if let Some(w) = weak.upgrade() {
                    sync_recent_menu(&w, &recent.borrow());
                    w.set_status_text(format!("Recent file not found: {}", path.display()).into());
                }
                return;
            }
            open_path_from_ui(path, &sender, &session, &weak, &debouncer, &recent, &subs);
        });
    }

    {
        let weak = window.as_weak();
        let recent = Rc::clone(&recent);
        window.on_file_clear_recent(move || {
            recent.borrow_mut().clear();
            if let Some(w) = weak.upgrade() {
                sync_recent_menu(&w, &recent.borrow());
            }
        });
    }

    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        let subs = Rc::clone(&subtitle_cache);
        window.on_file_close(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            if w.get_export_in_progress() {
                return;
            }

            let (has_media, dirty) = {
                let s = session.borrow();
                (s.has_media(), s.dirty)
            };
            if !has_media {
                return;
            }
            if !dirty {
                close_window_and_clear_player(&p, &session, &weak, &subs);
                return;
            }

            let res = MessageDialog::new()
                .set_level(MessageLevel::Warning)
                .set_title("Save changes?")
                .set_description(
                    "Save the project before closing? Unsaved changes will be lost if you don't save.",
                )
                .set_buttons(MessageButtons::YesNoCancelCustom(
                    "Save".into(),
                    "Don't Save".into(),
                    "Cancel".into(),
                ))
                .show();

            match res {
                MessageDialogResult::Custom(s) if s == "Save" => {
                    let had_path = session
                        .borrow()
                        .project()
                        .and_then(|p| p.path.clone());
                    if had_path.is_some() {
                        if let Err(e) = session.borrow_mut().flush_autosave_if_needed() {
                            tracing::error!(error = %e, "save before close failed");
                            if let Some(win) = weak.upgrade() {
                                win.set_status_text(format!("Could not save: {e}").into());
                            }
                            return;
                        }
                    } else {
                        let proj = session.borrow().project().cloned();
                        let Some(proj) = proj else {
                            return;
                        };
                        let Some(dest) = rfd::FileDialog::new()
                            .set_title("Save project…")
                            .add_filter("Reel project", &["reel", "json"])
                            .save_file()
                        else {
                            return;
                        };
                        match save_project(&dest, &proj) {
                            Ok(()) => {
                                session.borrow_mut().mark_saved_to_path(dest.clone());
                                recent.borrow_mut().record_project(dest.clone());
                                if let Some(win) = weak.upgrade() {
                                    sync_menu_and_autosave(&win, &session, &debouncer, &recent);
                                }
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "save before close failed");
                                if let Some(win) = weak.upgrade() {
                                    win.set_status_text(format!("Save failed: {e}").into());
                                }
                                return;
                            }
                        }
                    }
                    close_window_and_clear_player(&p, &session, &weak, &subs);
                }
                MessageDialogResult::Custom(s) if s == "Don't Save" => {
                    close_window_and_clear_player(&p, &session, &weak, &subs);
                }
                MessageDialogResult::Cancel => {}
                MessageDialogResult::Custom(s) if s == "Cancel" => {}
                _ => {}
            }
        });
    }

    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_file_revert(move || {
            let revert_result = session.borrow_mut().revert_to_saved();
            match revert_result {
                Ok(()) => {
                    p.send(player::Cmd::Pause);
                    reload_player_timeline(&p, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu(&w, &session.borrow());
                        let n = session
                            .borrow()
                            .project()
                            .map(|pr| pr.clips.len())
                            .unwrap_or(0);
                        w.set_status_text(format!("Reverted ({n} clips in timeline)").into());
                    }
                }
                Err(e) => tracing::warn!(error = %e, "revert"),
            }
        });
    }

    {
        window.on_file_new_window(move || {
            if let Err(e) = shell::spawn_new_window() {
                tracing::warn!(error = %e, "new window spawn failed");
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_file_save(move || {
            let proj = session.borrow().project().cloned();
            if let Some(proj) = proj {
                if let Some(dest) = rfd::FileDialog::new()
                    .set_title("Save project…")
                    .add_filter("Reel project", &["reel", "json"])
                    .save_file()
                {
                    match save_project(&dest, &proj) {
                        Ok(()) => {
                            session.borrow_mut().mark_saved_to_path(dest.clone());
                            recent.borrow_mut().record_project(dest.clone());
                            if let Some(w) = weak.upgrade() {
                                sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                                w.set_status_text(format!("Saved {}", dest.display()).into());
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "save failed");
                            if let Some(w) = weak.upgrade() {
                                w.set_status_text(format!("Save failed: {e}").into());
                            }
                        }
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_file_insert_audio(move || match prompt_insert_audio_dialog() {
            Some(insert_path) => {
                let mru_path = insert_path.clone();
                let playhead_ms = weak
                    .upgrade()
                    .map(|w| w.get_playhead_ms() as f64)
                    .unwrap_or(0.0);
                let insert_result = session
                    .borrow_mut()
                    .insert_audio_clip_at_playhead(insert_path, playhead_ms);
                match insert_result {
                    Ok(()) => {
                        recent.borrow_mut().record_media(mru_path);
                        if let Some(w) = weak.upgrade() {
                            let n = session
                                .borrow()
                                .project()
                                .map(|pr| pr.clips.len())
                                .unwrap_or(0);
                            sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                            w.set_status_text(
                                format!("Inserted audio @ {playhead_ms:.0} ms ({n} clips)").into(),
                            );
                        }
                        reload_player_timeline(&sender, &session.borrow());
                    }
                    Err(e) => {
                        if let Some(w) = weak.upgrade() {
                            w.set_status_text(format!("Insert audio failed: {e}").into());
                        }
                    }
                }
            }
            None => tracing::debug!("insert audio cancelled"),
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_overlay_audio(move || match prompt_insert_audio_dialog() {
            Some(overlay_path) => {
                let mru_path = overlay_path.clone();
                let playhead_ms = weak
                    .upgrade()
                    .map(|w| w.get_playhead_ms() as f64)
                    .unwrap_or(0.0);
                let result = session
                    .borrow_mut()
                    .insert_overlay_audio_clip_at_playhead(overlay_path, playhead_ms);
                match result {
                    Ok(()) => {
                        recent.borrow_mut().record_media(mru_path);
                        if let Some(w) = weak.upgrade() {
                            let lanes = session
                                .borrow()
                                .project()
                                .map(|p| {
                                    p.tracks
                                        .iter()
                                        .filter(|t| t.kind == TrackKind::Audio)
                                        .count()
                                })
                                .unwrap_or(0);
                            sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                            w.set_status_text(
                                format!(
                                    "Overlay audio added on new lane ({lanes} audio track{}). \
                                    Audible at export.",
                                    if lanes == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        }
                        reload_player_timeline(&sender, &session.borrow());
                    }
                    Err(e) => {
                        if let Some(w) = weak.upgrade() {
                            w.set_status_text(format!("Overlay audio failed: {e}").into());
                        }
                    }
                }
            }
            None => tracing::debug!("overlay audio cancelled"),
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        // Replace = mute every primary clip + overlay in one atomic step.
        // Reuses the same file picker as overlay / insert-audio so the user
        // sees a familiar dialog and the same set of audio-only filters.
        window.on_edit_replace_audio(move || match prompt_insert_audio_dialog() {
            Some(replace_path) => {
                let mru_path = replace_path.clone();
                let playhead_ms = weak
                    .upgrade()
                    .map(|w| w.get_playhead_ms() as f64)
                    .unwrap_or(0.0);
                let result = session
                    .borrow_mut()
                    .replace_audio_clip_at_playhead(replace_path, playhead_ms);
                match result {
                    Ok(()) => {
                        recent.borrow_mut().record_media(mru_path);
                        if let Some(w) = weak.upgrade() {
                            let lanes = session
                                .borrow()
                                .project()
                                .map(|p| {
                                    p.tracks
                                        .iter()
                                        .filter(|t| t.kind == TrackKind::Audio)
                                        .count()
                                })
                                .unwrap_or(0);
                            sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                            w.set_status_text(
                                format!(
                                    "Audio replaced — primary track muted, new lane added \
                                    ({lanes} audio track{}).",
                                    if lanes == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        }
                        reload_player_timeline(&sender, &session.borrow());
                    }
                    Err(e) => {
                        if let Some(w) = weak.upgrade() {
                            w.set_status_text(format!("Replace audio failed: {e}").into());
                        }
                    }
                }
            }
            None => tracing::debug!("replace audio cancelled"),
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        // Replace & Clear Other Audio = mute every primary clip, drop every
        // existing `TrackKind::Audio` lane, then append the replacement — all
        // under one undo snapshot. Same dialog as Replace so the user sees a
        // familiar picker; status text differs so "audio swapped" reads as
        // distinct from "audio added".
        window.on_edit_replace_audio_clear_others(
            move || match prompt_insert_audio_dialog() {
                Some(replace_path) => {
                    let mru_path = replace_path.clone();
                    let playhead_ms = weak
                        .upgrade()
                        .map(|w| w.get_playhead_ms() as f64)
                        .unwrap_or(0.0);
                    let result = session
                        .borrow_mut()
                        .replace_audio_clip_at_playhead_clear_others(replace_path, playhead_ms);
                    match result {
                        Ok(()) => {
                            recent.borrow_mut().record_media(mru_path);
                            if let Some(w) = weak.upgrade() {
                                sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                                w.set_status_text(
                                    "Audio replaced — existing audio lanes cleared, new lane \
                                    added (1 audio track)."
                                        .into(),
                                );
                            }
                            reload_player_timeline(&sender, &session.borrow());
                        }
                        Err(e) => {
                            if let Some(w) = weak.upgrade() {
                                w.set_status_text(format!("Replace audio failed: {e}").into());
                            }
                        }
                    }
                }
                None => tracing::debug!("replace-and-clear audio cancelled"),
            },
        );
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_file_insert_subtitle(move || match prompt_insert_subtitle_dialog() {
            Some(insert_path) => {
                let mru_path = insert_path.clone();
                let playhead_ms = weak
                    .upgrade()
                    .map(|w| w.get_playhead_ms() as f64)
                    .unwrap_or(0.0);
                let insert_result = session
                    .borrow_mut()
                    .insert_subtitle_clip_at_playhead(insert_path, playhead_ms);
                match insert_result {
                    Ok(()) => {
                        recent.borrow_mut().record_media(mru_path);
                        if let Some(w) = weak.upgrade() {
                            let n = session
                                .borrow()
                                .project()
                                .map(|pr| pr.clips.len())
                                .unwrap_or(0);
                            sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                            w.set_status_text(
                                format!(
                                    "Inserted subtitle @ {playhead_ms:.0} ms ({n} clips)"
                                )
                                .into(),
                            );
                        }
                        reload_player_timeline(&sender, &session.borrow());
                    }
                    Err(e) => {
                        if let Some(w) = weak.upgrade() {
                            w.set_status_text(format!("Insert subtitle failed: {e}").into());
                        }
                    }
                }
            }
            None => tracing::debug!("insert subtitle cancelled"),
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_file_insert_video(move || match prompt_insert_dialog() {
            Some(insert_path) => {
                let mru_path = insert_path.clone();
                let playhead_ms = weak
                    .upgrade()
                    .map(|w| w.get_playhead_ms() as f64)
                    .unwrap_or(0.0);
                let insert_result = session
                    .borrow_mut()
                    .insert_clip_at_playhead(insert_path, playhead_ms);
                match insert_result {
                    Ok(()) => {
                        recent.borrow_mut().record_media(mru_path);
                        if let Some(w) = weak.upgrade() {
                            let n = session
                                .borrow()
                                .project()
                                .map(|pr| pr.clips.len())
                                .unwrap_or(0);
                            sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                            w.set_status_text(
                                format!("Inserted @ {playhead_ms:.0} ms ({n} clips)").into(),
                            );
                        }
                        reload_player_timeline(&sender, &session.borrow());
                    }
                    Err(e) => {
                        if let Some(w) = weak.upgrade() {
                            w.set_status_text(format!("Insert failed: {e}").into());
                        }
                    }
                }
            }
            None => tracing::debug!("insert cancelled"),
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_file_new_video_track(move || {
            let r = session.borrow_mut().add_video_track();
            match r {
                Ok(()) => {
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        w.set_status_text("Added video track".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_file_new_audio_track(move || {
            let r = session.borrow_mut().add_audio_track();
            match r {
                Ok(()) => {
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        w.set_status_text("Added audio track".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_file_new_subtitle_track(move || {
            let r = session.borrow_mut().add_subtitle_track();
            match r {
                Ok(()) => {
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        w.set_status_text("Added subtitle track".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_delete_video_track(move |lane_idx| {
            let r = session
                .borrow_mut()
                .remove_video_track_lane(lane_idx as usize);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        let dur = w.get_duration_ms();
                        let ph = w.get_playhead_ms().min(dur);
                        w.set_playhead_ms(ph);
                        timecode::refresh_time_labels(&w, ph, dur);
                        sender.send(player::Cmd::SeekSequence { seq_ms: ph as u64 });
                        w.set_status_text("Removed video track".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_delete_audio_track(move |lane_idx| {
            let r = session
                .borrow_mut()
                .remove_audio_track_lane(lane_idx as usize);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        let dur = w.get_duration_ms();
                        let ph = w.get_playhead_ms().min(dur);
                        w.set_playhead_ms(ph);
                        timecode::refresh_time_labels(&w, ph, dur);
                        sender.send(player::Cmd::SeekSequence { seq_ms: ph as u64 });
                        w.set_status_text("Removed audio track".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_delete_subtitle_track(move |lane_idx| {
            let r = session
                .borrow_mut()
                .remove_subtitle_track_lane(lane_idx as usize);
            match r {
                Ok(()) => {
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        w.set_status_text("Removed subtitle track".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_clip_trim_commit(move |clip_id, edge, delta_ratio| {
            let id = match uuid::Uuid::parse_str(clip_id.as_str()) {
                Ok(u) => u,
                Err(_) => return,
            };
            let edge_u = if edge == 0 { 0u8 } else { 1u8 };
            let r = session
                .borrow_mut()
                .trim_clip_by_edge_drag(id, edge_u, delta_ratio as f64);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        let dur = w.get_duration_ms();
                        let ph = w.get_playhead_ms().min(dur);
                        w.set_playhead_ms(ph);
                        timecode::refresh_time_labels(&w, ph, dur);
                        sender.send(player::Cmd::SeekSequence { seq_ms: ph as u64 });
                        w.set_status_text("Trim applied".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("Trim failed: {e:#}").into());
                    }
                }
            }
        });
    }

    install_export_preflight_callback(&window, Rc::clone(&session));

    {
        let weak = window.as_weak();
        window.on_export_preset_cancel(move || {
            if let Some(w) = weak.upgrade() {
                w.set_export_preset_dialog_visible(false);
            }
        });
    }

    install_export_preset_confirm_callback(
        &window,
        Rc::clone(&session),
        Arc::clone(&export_cancel),
    );

    {
        let weak = window.as_weak();
        let export_cancel_flag = Arc::clone(&export_cancel);
        window.on_export_cancel(move || {
            if let Some(c) = export_cancel_flag
                .lock()
                .expect("export cancel mutex")
                .as_ref()
            {
                c.store(true, Ordering::Relaxed);
            }
            if let Some(w) = weak.upgrade() {
                w.set_status_text("Cancelling export…".into());
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_undo(move || {
            if !session.borrow_mut().undo() {
                return;
            }
            reload_player_timeline(&sender, &session.borrow());
            if let Some(w) = weak.upgrade() {
                sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                let n = session
                    .borrow()
                    .project()
                    .map(|pr| pr.clips.len())
                    .unwrap_or(0);
                w.set_status_text(format!("Undo ({n} clips)").into());
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_effect_face_swap(move || {
            begin_export_effect(
                weak.clone(),
                Rc::clone(&session),
                effects::EffectKind::FaceFusion,
            );
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_effect_face_enhance(move || {
            begin_export_effect(
                weak.clone(),
                Rc::clone(&session),
                effects::EffectKind::FaceEnhance,
            );
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_effect_remove_background(move || {
            begin_export_effect(
                weak.clone(),
                Rc::clone(&session),
                effects::EffectKind::RvmBackground,
            );
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_redo(move || {
            if !session.borrow_mut().redo() {
                return;
            }
            reload_player_timeline(&sender, &session.borrow());
            if let Some(w) = weak.upgrade() {
                sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                let n = session
                    .borrow()
                    .project()
                    .map(|pr| pr.clips.len())
                    .unwrap_or(0);
                w.set_status_text(format!("Redo ({n} clips)").into());
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_split_at_playhead(move || {
            let playhead_ms = weak
                .upgrade()
                .map(|w| w.get_playhead_ms() as f64)
                .unwrap_or(0.0);
            let r = session.borrow_mut().split_clip_at_playhead(playhead_ms);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        let dur = w.get_duration_ms();
                        let ph = w.get_playhead_ms().min(dur);
                        w.set_playhead_ms(ph);
                        timecode::refresh_time_labels(&w, ph, dur);
                        sender.send(player::Cmd::SeekSequence { seq_ms: ph as u64 });
                        w.set_status_text("Split clip at playhead".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_rotate_right(move || {
            let playhead_ms = weak
                .upgrade()
                .map(|w| w.get_playhead_ms() as f64)
                .unwrap_or(0.0);
            let r = session.borrow_mut().rotate_playhead_clip_right(playhead_ms);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        sender.send(player::Cmd::SeekSequence {
                            seq_ms: w.get_playhead_ms() as u64,
                        });
                        w.set_status_text("Rotated 90° right".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_rotate_left(move || {
            let playhead_ms = weak
                .upgrade()
                .map(|w| w.get_playhead_ms() as f64)
                .unwrap_or(0.0);
            let r = session.borrow_mut().rotate_playhead_clip_left(playhead_ms);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        sender.send(player::Cmd::SeekSequence {
                            seq_ms: w.get_playhead_ms() as u64,
                        });
                        w.set_status_text("Rotated 90° left".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_flip_horizontal(move || {
            let playhead_ms = weak
                .upgrade()
                .map(|w| w.get_playhead_ms() as f64)
                .unwrap_or(0.0);
            let r = session
                .borrow_mut()
                .flip_playhead_clip_horizontal(playhead_ms);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        sender.send(player::Cmd::SeekSequence {
                            seq_ms: w.get_playhead_ms() as u64,
                        });
                        w.set_status_text("Flipped horizontally".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_flip_vertical(move || {
            let playhead_ms = weak
                .upgrade()
                .map(|w| w.get_playhead_ms() as f64)
                .unwrap_or(0.0);
            let r = session
                .borrow_mut()
                .flip_playhead_clip_vertical(playhead_ms);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        sender.send(player::Cmd::SeekSequence {
                            seq_ms: w.get_playhead_ms() as u64,
                        });
                        w.set_status_text("Flipped vertically".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        // Edit → Trim Clip… opens the sheet prefilled from the clip under the playhead.
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let trim_target = Rc::clone(&trim_target);
        window.on_edit_trim_clip(move || {
            let Some(w) = weak.upgrade() else { return };
            let ph = w.get_playhead_ms() as f64;
            let cand = session.borrow().trim_candidate_at_seq_ms(ph);
            if let Some(c) = cand {
                *trim_target.borrow_mut() = Some(c.clip_id);
                w.set_trim_begin_s(c.current_in_s as f32);
                w.set_trim_end_s(c.current_out_s as f32);
                w.set_trim_source_duration_s(c.source_duration_s as f32);
                w.set_trim_error("".into());
                w.set_trim_sheet_visible(true);
            } else {
                w.set_status_text("No clip at playhead to trim".into());
            }
        });
    }

    {
        // Trim sheet Cancel: just hide; don't touch the project.
        let weak = window.as_weak();
        let trim_target = Rc::clone(&trim_target);
        window.on_trim_cancel(move || {
            *trim_target.borrow_mut() = None;
            if let Some(w) = weak.upgrade() {
                w.set_trim_sheet_visible(false);
                w.set_trim_error("".into());
            }
        });
    }

    {
        // Trim sheet Confirm: validate through session.trim_clip and show inline errors
        // in the sheet. On success, close sheet, reload timeline, resync playhead/menu.
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        let trim_target = Rc::clone(&trim_target);
        window.on_trim_confirm(move |begin, end| {
            let Some(w) = weak.upgrade() else { return };
            let Some(clip_id) = *trim_target.borrow() else {
                w.set_trim_sheet_visible(false);
                return;
            };
            let r = session
                .borrow_mut()
                .trim_clip(clip_id, begin as f64, end as f64);
            match r {
                Ok(()) => {
                    *trim_target.borrow_mut() = None;
                    w.set_trim_sheet_visible(false);
                    w.set_trim_error("".into());
                    reload_player_timeline(&sender, &session.borrow());
                    sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                    let dur = w.get_duration_ms();
                    let ph = w.get_playhead_ms().min(dur);
                    w.set_playhead_ms(ph);
                    timecode::refresh_time_labels(&w, ph, dur);
                    sender.send(player::Cmd::SeekSequence { seq_ms: ph as u64 });
                    w.set_status_text("Trimmed clip".into());
                }
                Err(e) => {
                    w.set_trim_error(format!("{e:#}").into());
                }
            }
        });
    }

    {
        // Edit → Resize Video… opens the sheet prefilled from the clip under the playhead.
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let resize_target = Rc::clone(&resize_target);
        window.on_edit_resize_clip(move || {
            let Some(w) = weak.upgrade() else { return };
            let ph = w.get_playhead_ms() as f64;
            let cand = session.borrow().resize_candidate_at_seq_ms(ph);
            if let Some(c) = cand {
                *resize_target.borrow_mut() = Some(c.clip_id);
                w.set_resize_percent(c.current_percent as i32);
                w.set_resize_source_width(c.source_width as i32);
                w.set_resize_source_height(c.source_height as i32);
                w.set_resize_error("".into());
                w.set_resize_sheet_visible(true);
            } else {
                w.set_status_text("No clip at playhead to resize".into());
            }
        });
    }

    {
        // Resize sheet Cancel: hide; don't touch the project.
        let weak = window.as_weak();
        let resize_target = Rc::clone(&resize_target);
        window.on_resize_cancel(move || {
            *resize_target.borrow_mut() = None;
            if let Some(w) = weak.upgrade() {
                w.set_resize_sheet_visible(false);
                w.set_resize_error("".into());
            }
        });
    }

    {
        // Resize sheet Confirm: validate through session.resize_clip, close on success.
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        let resize_target = Rc::clone(&resize_target);
        window.on_resize_confirm(move |percent| {
            let Some(w) = weak.upgrade() else { return };
            let Some(clip_id) = *resize_target.borrow() else {
                w.set_resize_sheet_visible(false);
                return;
            };
            let r = session.borrow_mut().resize_clip(clip_id, percent as u32);
            match r {
                Ok(()) => {
                    *resize_target.borrow_mut() = None;
                    w.set_resize_sheet_visible(false);
                    w.set_resize_error("".into());
                    sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                    w.set_status_text(format!("Resized clip to {percent}%").into());
                }
                Err(e) => {
                    w.set_resize_error(format!("{e:#}").into());
                }
            }
        });
    }

    install_audio_lane_gain_callbacks(
        &window,
        Rc::clone(&session),
        Rc::clone(&debouncer),
        Rc::clone(&recent),
    );

    {
        // Edit → Audio Track → <stream>: swap the current primary clip's audio
        // stream selection. `stream_index < 0` resets to `Clip.audio_stream_index = None`
        // (first-decodable fallback) so users can get back to "auto" without
        // having to remember which index that was. Menu click → session mutate →
        // sync_menu reloads the timeline so the audio thread opens the newly
        // picked stream the next time it handles a segment.
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_set_audio_track(move |stream_index| {
            let Some(w) = weak.upgrade() else { return };
            let ph = w.get_playhead_ms() as f64;
            let pick: Option<u32> = if stream_index < 0 {
                None
            } else {
                Some(stream_index as u32)
            };
            match session
                .borrow_mut()
                .set_audio_stream_index_at_seq_ms(ph, pick)
            {
                Ok(()) => {
                    sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                    match pick {
                        None => w.set_status_text("Audio track: default".into()),
                        Some(i) => w.set_status_text(format!("Audio track: stream {i}").into()),
                    }
                }
                Err(e) => w.set_status_text(format!("{e:#}").into()),
            }
        });
    }

    {
        // Edit → Mute Clip Audio: toggles audio_mute on the clip under the playhead.
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_toggle_audio_mute(move || {
            let Some(w) = weak.upgrade() else { return };
            let ph = w.get_playhead_ms() as f64;
            match session.borrow_mut().toggle_audio_mute_at_seq_ms(ph) {
                Ok(()) => {
                    sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                    let muted = session.borrow().audio_mute_state_at_seq_ms(ph).unwrap_or(false);
                    w.set_status_text(
                        if muted {
                            "Clip audio muted"
                        } else {
                            "Clip audio restored"
                        }
                        .into(),
                    );
                }
                Err(e) => w.set_status_text(format!("{e:#}").into()),
            }
        });
    }

    {
        // Set In Point: marks the current playhead as the range start. Clamped to timeline
        // duration so markers can't land past the end of a shrunk sequence.
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_edit_set_in_marker(move || {
            let Some(w) = weak.upgrade() else { return };
            let dur = w.get_duration_ms() as f64;
            let ph = (w.get_playhead_ms() as f64).clamp(0.0, dur);
            match session.borrow_mut().set_in_marker_ms(ph) {
                Ok(()) => {
                    sync_marker_ui(&w, &session.borrow());
                    w.set_status_text(format!("In point set at {:.3} s", ph / 1000.0).into());
                }
                Err(e) => w.set_status_text(format!("{e:#}").into()),
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_edit_set_out_marker(move || {
            let Some(w) = weak.upgrade() else { return };
            let dur = w.get_duration_ms() as f64;
            let ph = (w.get_playhead_ms() as f64).clamp(0.0, dur);
            match session.borrow_mut().set_out_marker_ms(ph) {
                Ok(()) => {
                    sync_marker_ui(&w, &session.borrow());
                    w.set_status_text(format!("Out point set at {:.3} s", ph / 1000.0).into());
                }
                Err(e) => w.set_status_text(format!("{e:#}").into()),
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_edit_clear_markers(move || {
            let Some(w) = weak.upgrade() else { return };
            session.borrow_mut().clear_markers();
            sync_marker_ui(&w, &session.borrow());
            w.set_status_text("Cleared range markers".into());
        });
    }

    // Edit → Scan for Errors… — decodes the primary clip's source file on a
    // worker thread and reports any demux/decode issues in a modal sheet.
    // The worker honours a shared `Arc<AtomicBool>` cancel flag so dismissing
    // the sheet mid-scan stops the decode loop on the next progress tick.
    install_scan_for_errors_callbacks(&window, Rc::clone(&session));

    // -- Yellow handle drags on the scrub bar (QuickTime-style trim handles) --
    //
    // Slint clamps the dispatched ms so the handles can't cross; we still
    // `clamp(0..=duration)` here in case a stale event slips through. We
    // intentionally *don't* set `status_text` on every drag — it would spam
    // the footer. The Set-In/Set-Out keyboard paths above still announce.
    install_edit_drag_marker_callbacks(&window, Rc::clone(&session));

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_move_clip_down(move || {
            let playhead_ms = weak
                .upgrade()
                .map(|w| w.get_playhead_ms() as f64)
                .unwrap_or(0.0);
            let r = session
                .borrow_mut()
                .move_playhead_clip_to_next_video_track(playhead_ms);
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        let dur = w.get_duration_ms();
                        let ph = w.get_playhead_ms().min(dur);
                        w.set_playhead_ms(ph);
                        timecode::refresh_time_labels(&w, ph, dur);
                        sender.send(player::Cmd::SeekSequence { seq_ms: ph as u64 });
                        w.set_status_text("Moved clip to track below".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let sender = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
        window.on_edit_move_clip_up(move || {
            let r = session
                .borrow_mut()
                .move_first_clip_from_second_video_track_to_primary();
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer, &recent);
                        let dur = w.get_duration_ms();
                        let ph = w.get_playhead_ms().min(dur);
                        w.set_playhead_ms(ph);
                        timecode::refresh_time_labels(&w, ph, dur);
                        sender.send(player::Cmd::SeekSequence { seq_ms: ph as u64 });
                        w.set_status_text("Moved clip from track below to primary".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = weak.upgrade() {
                        w.set_status_text(format!("{e:#}").into());
                    }
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_win_fit(move || {
            if let Some(w) = weak.upgrade() {
                w.set_video_fit_mode(0);
                w.set_preview_zoom_actual(false);
                w.set_preview_zoom_percent(100.0);
                w.set_preview_pan_x(0.0);
                w.set_preview_pan_y(0.0);
                save_preview_zoom_prefs(&app_prefs, &w);
            }
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_win_fill(move || {
            if let Some(w) = weak.upgrade() {
                w.set_video_fit_mode(1);
                w.set_preview_zoom_actual(false);
                w.set_preview_zoom_percent(100.0);
                w.set_preview_pan_x(0.0);
                w.set_preview_pan_y(0.0);
                save_preview_zoom_prefs(&app_prefs, &w);
            }
        });
    }

    {
        let weak = window.as_weak();
        let app_prefs = Rc::clone(&app_prefs);
        window.on_win_center(move || {
            if let Some(w) = weak.upgrade() {
                w.set_video_fit_mode(0);
                w.set_preview_zoom_actual(false);
                w.set_preview_zoom_percent(100.0);
                w.set_preview_pan_x(0.0);
                w.set_preview_pan_y(0.0);
                save_preview_zoom_prefs(&app_prefs, &w);
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_win_toggle_on_top(move || {
            if let Some(w) = weak.upgrade() {
                let on = !w.get_stay_on_top();
                w.set_stay_on_top(on);
            }
        });
    }

    {
        window.on_help_about(move || show_help_window(shell::HelpDoc::About));
    }
    {
        window.on_help_overview(move || show_help_window(shell::HelpDoc::Overview));
    }
    {
        window.on_help_features(move || show_help_window(shell::HelpDoc::Features));
    }
    {
        window.on_help_keyboard(move || show_help_window(shell::HelpDoc::Keyboard));
    }
    {
        window.on_help_media_formats(move || show_help_window(shell::HelpDoc::MediaFormats));
    }
    {
        window
            .on_help_supported_formats(move || show_help_window(shell::HelpDoc::SupportedFormats));
    }
    {
        window.on_help_cli(move || show_help_window(shell::HelpDoc::Cli));
    }
    {
        window.on_help_external_ai(move || show_help_window(shell::HelpDoc::ExternalAi));
    }
    {
        window.on_help_developers(move || show_help_window(shell::HelpDoc::Developers));
    }
    {
        window.on_help_agents(move || show_help_window(shell::HelpDoc::Agents));
    }
    {
        window.on_help_phases_ui(move || show_help_window(shell::HelpDoc::PhasesUi));
    }

    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        window.on_play_pause(move || {
            if let Some(w) = weak.upgrade() {
                if !w.get_media_ready() {
                    return;
                }
                let now_playing = !w.get_is_playing();
                w.set_is_playing(now_playing);
                p.send(if now_playing {
                    player::Cmd::Play
                } else {
                    player::Cmd::Pause
                });
            }
        });
    }

    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_seek_timeline(move |v| {
            let Some(w) = weak.upgrade() else {
                return;
            };
            if !w.get_media_ready() {
                return;
            }
            let dur = w.get_duration_ms();
            let v = timecode::clamp_playhead_ms(v, dur);
            w.set_playhead_ms(v);
            timecode::refresh_time_labels(&w, v, dur);
            sync_menu(&w, &session.borrow());
            p.send(player::Cmd::SeekSequence { seq_ms: v as u64 });
        });
    }

    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_seek_nudge_ms(move |delta| {
            let Some(w) = weak.upgrade() else {
                return;
            };
            if !w.get_media_ready() {
                return;
            }
            let cur = w.get_playhead_ms();
            let dur = w.get_duration_ms();
            let next = timecode::clamp_playhead_ms(cur + delta, dur);
            w.set_playhead_ms(next);
            timecode::refresh_time_labels(&w, next, dur);
            sync_menu(&w, &session.borrow());
            p.send(player::Cmd::SeekSequence {
                seq_ms: next as u64,
            });
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_footer_refresh(move || {
            if let Some(w) = weak.upgrade() {
                sync_footer(&w, &session.borrow());
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let cache = Rc::clone(&subtitle_cache);
        window.on_subtitle_refresh(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            refresh_subtitle_overlay(&w, &session.borrow(), &cache);
        });
    }

    sync_menu(&window, &session.borrow());
    sync_recent_menu(&window, &recent.borrow());

    if let Some(path) = resolve_startup_auto_open(cli_startup_path) {
        tracing::info!(?path, "auto-opening from CLI or REEL_OPEN_PATH");
        let startup_open = session.borrow_mut().open_media(path.clone());
        match startup_open {
            Ok(()) => {
                {
                    let mut r = recent.borrow_mut();
                    if is_project_document_path(&path) {
                        r.record_project(path);
                    } else {
                        r.record_media(path);
                    }
                }
                sync_menu_and_autosave(&window, &session, &debouncer, &recent);
                reload_player_timeline(&player.cmd_sender(), &session.borrow());
            }
            Err(e) => {
                window.set_status_text(format!("Could not open {}: {e}", path.display()).into());
            }
        }
    }

    window.run()?;
    drop(player);
    Ok(())
}

fn player_handle_ref(p: &player::PlayerHandle) -> player::PlayerCmdSender {
    p.cmd_sender()
}

fn prompt_open_dialog() -> Option<PathBuf> {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        rfd::FileDialog::new()
            .set_title("Open media or project…")
            .add_filter("Media (video & audio)", OPEN_MEDIA_EXTENSIONS)
            .add_filter("Reel project", &["reel", "json"])
            .add_filter("All files", &["*"])
            .pick_file()
    }));

    match result {
        Ok(opt) => opt,
        Err(_) => {
            tracing::error!("rfd::FileDialog panicked");
            None
        }
    }
}

fn prompt_insert_dialog() -> Option<PathBuf> {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        rfd::FileDialog::new()
            .set_title("Insert video…")
            .add_filter("Video", VIDEO_CONTAINER_EXTENSIONS)
            .pick_file()
    }));
    result.unwrap_or_default()
}

fn prompt_insert_audio_dialog() -> Option<PathBuf> {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        rfd::FileDialog::new()
            .set_title("Insert audio…")
            .add_filter("Audio", AUDIO_FILE_EXTENSIONS)
            .add_filter("Video (audio stream)", VIDEO_CONTAINER_EXTENSIONS)
            .pick_file()
    }));
    result.unwrap_or_default()
}

fn prompt_insert_subtitle_dialog() -> Option<PathBuf> {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        rfd::FileDialog::new()
            .set_title("Insert subtitle…")
            .add_filter("Subtitle", SUBTITLE_FILE_EXTENSIONS)
            .pick_file()
    }));
    result.unwrap_or_default()
}

/// Parses `argv[1..]` after the executable name. `Ok(Some)` = one path; `Ok(None)` = open empty;
/// `Err` = more than one argument (print usage, exit 2).
fn parse_cli_startup_path() -> Result<Option<PathBuf>, ()> {
    parse_single_optional_path_arg(std::env::args_os().skip(1))
}

fn parse_single_optional_path_arg(
    args: impl Iterator<Item = std::ffi::OsString>,
) -> Result<Option<PathBuf>, ()> {
    let mut it = args;
    let first = it.next();
    if it.next().is_some() {
        return Err(());
    }
    Ok(first.and_then(|s| {
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    }))
}

/// CLI path wins over [`std::env::var_os`] `REEL_OPEN_PATH`.
fn resolve_startup_auto_open(cli: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = cli {
        return Some(p);
    }
    let env = std::env::var_os("REEL_OPEN_PATH")?;
    let p = PathBuf::from(env);
    if p.as_os_str().is_empty() {
        None
    } else {
        Some(p)
    }
}

#[cfg(test)]
mod startup_path_tests {
    use super::*;

    #[test]
    fn parse_single_path() {
        let r =
            parse_single_optional_path_arg(vec![std::ffi::OsString::from("clip.mp4")].into_iter())
                .unwrap();
        assert_eq!(r, Some(PathBuf::from("clip.mp4")));
    }

    #[test]
    fn parse_none() {
        let r = parse_single_optional_path_arg(vec![].into_iter()).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn parse_rejects_two() {
        assert!(parse_single_optional_path_arg(
            vec![std::ffi::OsString::from("a"), std::ffi::OsString::from("b"),].into_iter(),
        )
        .is_err());
    }
}

#[cfg(test)]
pub(crate) mod ui_test_support {
    //! Headless UI test harness (Phase 1, `docs/phases-ui-test.md`).
    //!
    //! The Slint testing backend must be installed exactly once per process and
    //! before any `AppWindow` is constructed. This helper centralizes that so
    //! unit tests and future integration tests can share a single init point.
    use std::sync::Once;

    static INIT: Once = Once::new();

    /// Install `i-slint-backend-testing`. Safe to call from every test.
    pub fn init() {
        INIT.call_once(|| {
            i_slint_backend_testing::init_integration_test_with_system_time();
        });
    }
}

#[cfg(test)]
mod ui_smoke_tests {
    //! Slint's platform is installed once per process and pinned to the thread
    //! that first calls `AppWindow::new`. `cargo test` runs tests in parallel
    //! on independent threads, which panics with "Slint platform was
    //! initialized in another thread" for every test after the first. Until
    //! we add `serial_test` (or a dedicated test binary), each `#[test]` below
    //! serializes its assertions on the same thread; Phase 1a keeps this to
    //! one smoke case.
    use super::*;

    #[test]
    fn window_boots_and_round_trips_basic_properties() {
        ui_test_support::init();
        let window = AppWindow::new().expect("AppWindow::new in headless test");

        // Defaults we rely on at startup (main() also sets these explicitly
        // before showing the window — if the Slint-generated default drifts,
        // main + tests both need updating together, which is the value of
        // checking here).
        assert!(!window.get_media_ready());
        assert!(!window.get_trim_sheet_visible());
        assert!(!window.get_export_preset_dialog_visible());
        assert_eq!(window.get_export_preset_index(), 0);
        assert!(!window.get_footer_visible());
        assert!(!window.get_rotate_enabled());
        assert!(!window.get_split_at_playhead_enabled());
        // View → track row visibility (prefs-backed; v0 design — default show all lanes).
        assert!(window.get_view_show_video_tracks());
        assert!(window.get_view_show_audio_tracks());
        assert!(window.get_view_show_subtitle_tracks());

        // Round-trip: media-ready gate.
        window.set_media_ready(true);
        assert!(window.get_media_ready());
        window.set_media_ready(false);
        assert!(!window.get_media_ready());

        // Round-trip: export preset indices must accept the full 0..=9 range
        // so the Slint picker stays in sync with
        // `web_export_format_from_preset_index` (session.rs):
        // 0=Mp4Remux, 1=Mp4H264Aac, 2=Mp4H265Aac,
        // 3=WebmVp8Opus, 4=WebmVp9Opus, 5=WebmAv1Opus,
        // 6=MkvRemux, 7=MovRemux, 8=MovProResHq, 9=MkvDnxhrHq.
        for idx in 0..=9 {
            window.set_export_preset_index(idx);
            assert_eq!(window.get_export_preset_index(), idx);
        }

        // Phase 2 smoke: Open → sync → edit state visible in UI properties.
        //
        // Drives `EditSession::open_media_with_probe` with a headless
        // `FakeProbe`, then calls `sync_menu` (the same function `main()`
        // calls after every edit). Assertions target menu-gate properties
        // that the user sees enable/disable as clips are opened — if the
        // sync pipeline drifts away from the `EditSession` state, this test
        // fails before anyone runs the real app.
        use crate::session::tests_fake_probe::FakeProbe;
        let probe = FakeProbe::with_duration(2.0);
        let session = Rc::new(RefCell::new(EditSession::default()));
        session
            .borrow_mut()
            .open_media_with_probe(&probe, PathBuf::from("/tmp/fake-open.mp4"))
            .expect("open with fake probe");
        window.set_media_ready(true);
        sync_menu(&window, &session.borrow());

        assert_eq!(
            window.get_duration_ms(),
            2_000.0,
            "fake duration → sequence duration_ms in Slint property"
        );
        assert!(
            !window.get_video_track_lanes().is_empty(),
            "video lane labels populated after open"
        );
        assert!(
            window.get_rotate_enabled(),
            "rotate enabled when playhead is on a primary-track clip"
        );
        assert!(
            window.get_trim_enabled(),
            "trim enabled after open with media_ready=true"
        );
        assert!(
            window.get_close_enabled(),
            "close enabled whenever a project is open"
        );
        assert_eq!(probe.call_count(), 1);

        // Phase 2 — simulated click on **File → Export…** through the real
        // `on_file_export` callback. We install the production helper
        // `install_export_preflight_callback` (same code `main()` calls) and
        // trigger it via `window.invoke_file_export()`. Each branch below
        // targets one user-visible outcome:
        //
        // Branch A: valid payload → preset sheet opens.
        install_export_preflight_callback(&window, Rc::clone(&session));
        // Reset transient UI state so we can see branch effects crisply.
        window.set_export_preset_dialog_visible(false);
        window.set_status_text("".into());

        window.invoke_file_export();
        assert!(
            window.get_export_preset_dialog_visible(),
            "Branch A: export preflight should open the preset sheet when the timeline is non-empty"
        );
        assert_eq!(
            window.get_status_text().as_str(),
            "",
            "Branch A: no transient status message when the preflight succeeds"
        );

        // Branch B: markers set **past** every clip → status line warns, sheet stays closed.
        window.set_export_preset_dialog_visible(false);
        window.set_status_text("".into());
        session
            .borrow_mut()
            .set_in_marker_ms(10_000.0)
            .expect("set in marker");
        session
            .borrow_mut()
            .set_out_marker_ms(11_000.0)
            .expect("set out marker");
        window.invoke_file_export();
        assert!(
            !window.get_export_preset_dialog_visible(),
            "Branch B: empty-range must not open the preset sheet"
        );
        assert!(
            window
                .get_status_text()
                .as_str()
                .contains("No clips in the In/Out range"),
            "Branch B: empty-range shows the disambiguating status text (got {:?})",
            window.get_status_text().as_str()
        );

        // Branch C: no project at all → silent (no dialog, no status drift).
        session.borrow_mut().clear_media();
        window.set_export_preset_dialog_visible(false);
        window.set_status_text("".into());
        window.invoke_file_export();
        assert!(
            !window.get_export_preset_dialog_visible(),
            "Branch C: empty project must not open the preset sheet"
        );
        assert_eq!(
            window.get_status_text().as_str(),
            "",
            "Branch C: empty project is silent (no status bar chatter)"
        );

        // Phase U2-d — yellow trim handles on the scrub bar. We exercise the
        // `edit-drag-*-marker-ms` callbacks that fire while the user drags a
        // handle, confirming the round-trip: invoke → session marker state
        // updates → `sync_marker_ui` writes the new ms back into the Slint
        // property that drives handle position.
        let session = Rc::new(RefCell::new(EditSession::default()));
        session
            .borrow_mut()
            .open_media_with_probe(&probe, PathBuf::from("/tmp/drag-probe.mp4"))
            .expect("reopen with fake probe for drag tests");
        window.set_duration_ms(2_000.0);
        install_edit_drag_marker_callbacks(&window, Rc::clone(&session));

        // Drag In to 500ms — session + Slint property should both reflect it.
        window.invoke_edit_drag_in_marker_ms(500.0);
        assert_eq!(session.borrow().in_marker_ms(), Some(500.0));
        assert!((window.get_marker_in_ms() - 500.0).abs() < 0.001);

        // Drag Out to 1500ms — no cross, both markers now set, range valid.
        window.invoke_edit_drag_out_marker_ms(1500.0);
        assert_eq!(session.borrow().out_marker_ms(), Some(1500.0));
        assert!((window.get_marker_out_ms() - 1500.0).abs() < 0.001);
        assert_eq!(session.borrow().marker_range_ms(), Some((500.0, 1500.0)));

        // Defensive: Rust also clamps. Simulate an out-of-bounds ms (Slint's
        // UI-side clamp should prevent this, but if it ever regresses the
        // session API rejects / clamps without crashing).
        window.invoke_edit_drag_in_marker_ms(-1_000.0);
        // `set_in_marker_ms` bails on negative; the Rust wrapper clamps to 0.
        assert_eq!(session.borrow().in_marker_ms(), Some(0.0));
        assert!((window.get_marker_in_ms() - 0.0).abs() < 0.001);

        window.invoke_edit_drag_out_marker_ms(99_999.0);
        assert_eq!(session.borrow().out_marker_ms(), Some(2_000.0));
        assert!((window.get_marker_out_ms() - 2_000.0).abs() < 0.001);

        // Phase U2-e — Audio Lane Gain… sheet round-trip. Fresh session: no
        // audio lanes yet, so the menu gate must stay off even when
        // media-ready. Then append an audio lane and re-sync: the gate
        // flips on and the `on_edit_audio_lane_gain` callback opens the
        // sheet prefilled from lane 0's current gain (0 dB at start).
        let gain_session = Rc::new(RefCell::new(EditSession::default()));
        gain_session
            .borrow_mut()
            .open_media_with_probe(&probe, PathBuf::from("/tmp/gain-probe.mp4"))
            .expect("open with fake probe");
        window.set_media_ready(true);
        sync_menu(&window, &gain_session.borrow());
        assert!(
            !window.get_audio_gain_enabled(),
            "no audio lanes yet → Audio Lane Gain… must be disabled"
        );

        gain_session
            .borrow_mut()
            .add_audio_track()
            .expect("add empty audio lane");
        sync_menu(&window, &gain_session.borrow());
        assert!(
            window.get_audio_gain_enabled(),
            "one audio lane + media-ready → Audio Lane Gain… must enable"
        );

        // The install helper needs the autosave + recent Rcs production uses;
        // the smoke test doesn't actually care whether autosave fires (the
        // sheet state + session mutation are what we're asserting), but the
        // function signature is production-shaped so we construct minimal
        // stand-ins.
        let gain_debouncer = Rc::new(autosave::AutosaveDebouncer::new(window.as_weak()));
        let gain_recent = Rc::new(RefCell::new(RecentStore::default()));
        install_audio_lane_gain_callbacks(
            &window,
            Rc::clone(&gain_session),
            gain_debouncer,
            gain_recent,
        );
        window.set_gain_sheet_visible(false);
        window.invoke_edit_audio_lane_gain();
        assert!(
            window.get_gain_sheet_visible(),
            "invoking the menu callback must open the gain sheet"
        );
        assert_eq!(
            window.get_gain_sheet_lane(),
            1,
            "sheet prefills with lane 1 (1-based)"
        );
        assert!(
            (window.get_gain_sheet_db() - 0.0).abs() < 0.001,
            "sheet prefills with the lane's current gain (0 dB)"
        );
        assert_eq!(window.get_gain_sheet_max_lane(), 1);

        // Confirm with a valid value → session takes the new gain,
        // sheet closes, status line reports the applied (post-clamp) number.
        window.invoke_gain_confirm(1, 6.5);
        assert!(
            !window.get_gain_sheet_visible(),
            "successful confirm must close the sheet"
        );
        assert_eq!(gain_session.borrow().audio_track_gain_db(0), Some(6.5));
        assert!(
            window
                .get_status_text()
                .as_str()
                .contains("+6.5 dB"),
            "status line reports applied gain, got {:?}",
            window.get_status_text().as_str()
        );

        // Confirm with an out-of-range lane → sheet stays open, inline error
        // surfaces, session is untouched.
        window.invoke_edit_audio_lane_gain();
        window.invoke_gain_confirm(5, 3.0);
        assert!(
            window.get_gain_sheet_visible(),
            "out-of-range lane must keep the sheet open for correction"
        );
        assert!(
            window
                .get_gain_sheet_error()
                .as_str()
                .contains("out of range"),
            "expected inline 'out of range' error, got {:?}",
            window.get_gain_sheet_error().as_str()
        );
        assert_eq!(
            gain_session.borrow().audio_track_gain_db(0),
            Some(6.5),
            "rejected confirm must not mutate the project"
        );

        // Cancel closes the sheet without further mutation.
        window.invoke_gain_cancel();
        assert!(!window.get_gain_sheet_visible());
        assert!(window.get_gain_sheet_error().as_str().is_empty());
    }
}

#[cfg(test)]
mod export_payload_tests {
    //! Pure unit tests for `export_timeline_payload` — the function that
    //! translates an `EditSession` (plus optional In/Out range) into the
    //! `(video_spans, audio_lane_spans)` tuple the ffmpeg pipeline consumes.
    //!
    //! These don't touch Slint or ffmpeg, so they run on any test thread
    //! (unlike `ui_smoke_tests`). They're the safety net for the Phase 2
    //! Export flow: if the session → payload mapping drifts, every export
    //! preset in the product silently drifts with it.
    use super::*;
    use crate::session::tests_fake_probe::FakeProbe;

    fn opened_session(duration_s: f64, path: &str) -> (EditSession, PathBuf) {
        let probe = FakeProbe::with_duration(duration_s);
        let mut s = EditSession::default();
        let p = PathBuf::from(path);
        s.open_media_with_probe(&probe, p.clone())
            .expect("open with fake probe");
        (s, p)
    }

    #[test]
    fn payload_single_clip_full_span_when_no_range() {
        let (session, path) = opened_session(3.5, "/tmp/fake-a.mp4");
        let (video, audio_lanes, mute_mask) =
            export_timeline_payload(&session, None).expect("payload for opened project");
        assert_eq!(video.len(), 1);
        assert_eq!(video[0].0, path);
        assert!((video[0].1 - 0.0).abs() < 1e-9);
        assert!((video[0].2 - 3.5).abs() < 1e-9);
        assert!(
            audio_lanes.is_empty(),
            "single media file has no dedicated audio lane"
        );
        assert_eq!(
            mute_mask,
            vec![false],
            "default unmuted primary clip ⇒ single-false mask"
        );
    }

    #[test]
    fn payload_respects_in_out_range_markers() {
        let (mut session, path) = opened_session(4.0, "/tmp/fake-b.mp4");
        // Carve out the middle two seconds: 1.0s .. 3.0s.
        session.set_in_marker_ms(1_000.0).unwrap();
        session.set_out_marker_ms(3_000.0).unwrap();
        let range = session.marker_range_ms().expect("both markers set");

        let (video, _audio_lanes, _mute_mask) =
            export_timeline_payload(&session, Some(range)).expect("sliced payload");
        assert_eq!(video.len(), 1);
        assert_eq!(video[0].0, path, "slice keeps the original source path");
        assert!(
            (video[0].1 - 1.0).abs() < 1e-6,
            "slice media_in = 1.0s, got {}",
            video[0].1
        );
        assert!(
            (video[0].2 - 3.0).abs() < 1e-6,
            "slice media_out = 3.0s, got {}",
            video[0].2
        );
    }

    #[test]
    fn payload_returns_none_when_range_outside_all_clips() {
        // This is the signal `on_file_export` uses to show the user
        // "No clips in the In/Out range…" instead of writing an empty file.
        let (mut session, _path) = opened_session(2.0, "/tmp/fake-c.mp4");
        session.set_in_marker_ms(10_000.0).unwrap();
        session.set_out_marker_ms(12_000.0).unwrap();
        let range = session.marker_range_ms().expect("both markers set");
        assert!(
            export_timeline_payload(&session, Some(range)).is_none(),
            "range past the end of the timeline produces no payload"
        );
    }

    /// End-to-end: open a real fixture through the real `FfmpegProbe`, build
    /// the export payload, then run the **actual** ffmpeg export and re-probe
    /// the output. This is the Phase 4 ("Output Validation") check for the
    /// Session → Payload → ffmpeg path — if any link breaks (e.g. a refactor
    /// drops `in_point`/`out_point` rounding, or export args lose `+faststart`
    /// and the container is no longer muxed valid), this test fails.
    ///
    /// Skipped when the committed fixture or `ffmpeg` on PATH is missing,
    /// matching the pattern already used in `session::tests`.
    #[test]
    fn roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux() {
        use reel_core::{FfmpegProbe, MediaProbe, WebExportFormat};

        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("reel-core")
            .join("tests")
            .join("fixtures")
            .join("tiny_h264_aac.mp4");
        if !fixture.is_file() {
            eprintln!(
                "skip roundtrip_session_to_ffmpeg_to_reprobe_mp4_remux: fixture missing ({})",
                fixture.display()
            );
            return;
        }

        // Open with the real probe — this is the authenticity point: if
        // `open_media` silently started using a stub, the output would not
        // decode below.
        let mut session = EditSession::default();
        if let Err(e) = session.open_media(fixture.clone()) {
            eprintln!("skip: open_media failed (ffmpeg likely missing): {e}");
            return;
        }

        // Single-clip, full-span payload.
        let (video, audio_lanes, mute_mask) =
            export_timeline_payload(&session, None).expect("payload after open");
        assert_eq!(video.len(), 1, "one-clip fixture → one span");
        assert!(audio_lanes.is_empty(), "no dedicated audio lane");
        assert_eq!(
            mute_mask,
            vec![false],
            "single unmuted clip → single-false mask"
        );

        // Write to a tempdir so the test never races with the committed
        // `target/reel-export-verify/` artifacts.
        let scratch = tempfile::tempdir().expect("tempdir");
        let dest = scratch.path().join("session_roundtrip.mp4");
        if let Err(e) =
            reel_core::export_concat_timeline(&video, &dest, WebExportFormat::Mp4Remux, None, None)
        {
            eprintln!("skip: ffmpeg export failed (binary likely missing): {e}");
            return;
        }

        let meta = std::fs::metadata(&dest).expect("output exists");
        assert!(
            meta.len() > 64,
            "exported {} suspiciously small ({} bytes)",
            dest.display(),
            meta.len()
        );

        // Re-probe the written file: this validates that ffmpeg produced a
        // container the probe can open, and that the duration is plausible
        // (probe should report > 0s for a trivially-small but valid MP4).
        let probe = FfmpegProbe::new();
        let md = probe.probe(&dest).expect("re-probe exported mp4");
        assert!(
            md.duration_seconds > 0.0,
            "re-probed duration should be > 0, got {}",
            md.duration_seconds
        );
        assert!(md.video.is_some(), "re-probed file has a video stream");
    }

    /// Test double for [`SaveDialogProvider`]. Records every `fmt` it was
    /// asked with and returns the pre-baked `Option<PathBuf>`. The cell lets
    /// tests set `None` to emulate a cancelled dialog without rebuilding the
    /// stub.
    struct StubSaveDialog {
        reply: std::cell::RefCell<Option<PathBuf>>,
        calls: std::cell::RefCell<Vec<reel_core::WebExportFormat>>,
    }

    impl StubSaveDialog {
        fn always(path: Option<PathBuf>) -> Self {
            Self {
                reply: std::cell::RefCell::new(path),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }

        fn last_fmt(&self) -> Option<reel_core::WebExportFormat> {
            self.calls.borrow().last().copied()
        }
    }

    impl SaveDialogProvider for StubSaveDialog {
        fn pick(&self, fmt: reel_core::WebExportFormat) -> Option<PathBuf> {
            self.calls.borrow_mut().push(fmt);
            self.reply.borrow().clone()
        }
    }

    #[test]
    fn preflight_invalid_preset_index_returns_status_and_skips_dialog() {
        // Branch: bad idx → status; save dialog must NOT be shown.
        let (session, _path) = opened_session(1.0, "/tmp/fake-invalid.mp4");
        let stub = StubSaveDialog::always(Some(PathBuf::from("/tmp/unused.mp4")));
        let pf = prepare_export_job(&session, 99, &stub);
        match pf {
            ExportPreflight::Status(msg) => assert!(
                msg.contains("Invalid export preset"),
                "unexpected status: {msg}"
            ),
            other => panic!("expected Status, got {other:?}"),
        }
        assert_eq!(
            stub.call_count(),
            0,
            "save dialog must not open when preset index is invalid"
        );
    }

    #[test]
    fn preflight_empty_range_returns_status_before_dialog() {
        // Branch: markers past every clip → status; save dialog must NOT be shown.
        let (mut session, _path) = opened_session(2.0, "/tmp/fake-empty.mp4");
        session.set_in_marker_ms(10_000.0).unwrap();
        session.set_out_marker_ms(11_000.0).unwrap();
        let stub = StubSaveDialog::always(Some(PathBuf::from("/tmp/unused.mp4")));
        let pf = prepare_export_job(&session, 0, &stub);
        match pf {
            ExportPreflight::Status(msg) => assert!(
                msg.contains("No clips in the In/Out range"),
                "unexpected status: {msg}"
            ),
            other => panic!("expected Status, got {other:?}"),
        }
        assert_eq!(
            stub.call_count(),
            0,
            "save dialog must not open when the range is empty"
        );
    }

    #[test]
    fn preflight_cancelled_save_dialog_returns_noop() {
        // Branch: user cancels the save sheet → NoOp; no status message drift.
        let (session, _path) = opened_session(1.5, "/tmp/fake-cancel.mp4");
        let stub = StubSaveDialog::always(None);
        let pf = prepare_export_job(&session, 0, &stub);
        assert!(
            matches!(pf, ExportPreflight::NoOp),
            "cancelled save dialog must yield NoOp, got {pf:?}"
        );
        assert_eq!(
            stub.call_count(),
            1,
            "save dialog should have been consulted exactly once"
        );
        assert_eq!(stub.last_fmt(), Some(reel_core::WebExportFormat::Mp4Remux));
    }

    #[test]
    fn preflight_wrong_extension_returns_status() {
        // Branch: user picks a .webm path for the MP4 preset → status.
        let (session, _path) = opened_session(1.0, "/tmp/fake-wrong-ext.mp4");
        let stub = StubSaveDialog::always(Some(PathBuf::from("/tmp/out.webm")));
        // idx 0 = Mp4Remux; .webm extension should be rejected.
        let pf = prepare_export_job(&session, 0, &stub);
        match pf {
            ExportPreflight::Status(msg) => assert!(
                msg.contains("Use a .mp4 file name"),
                "unexpected status: {msg}"
            ),
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[test]
    fn preflight_happy_path_returns_spawn_with_expected_payload() {
        // Branch: valid preset, clip loaded, save dialog returns matching path
        // → Spawn carrying the payload and chosen dest.
        let (session, media) = opened_session(2.5, "/tmp/fake-ok.mp4");
        let dest = PathBuf::from("/tmp/happy.mp4");
        let stub = StubSaveDialog::always(Some(dest.clone()));
        let pf = prepare_export_job(&session, 0, &stub);
        match pf {
            ExportPreflight::Spawn {
                video_spans,
                audio_lane_spans,
                primary_mute_mask,
                orientation,
                scale,
                mute_audio,
                subtitle_path,
                dest: got_dest,
                fmt,
                range_ms,
                lane_gains_db: _,
            } => {
                assert_eq!(got_dest, dest);
                assert_eq!(fmt, reel_core::WebExportFormat::Mp4Remux);
                assert!(
                    orientation.is_none(),
                    "single unrotated clip ⇒ orientation None"
                );
                assert!(scale.is_none(), "default scale ⇒ scale None");
                assert!(!mute_audio, "default audio_mute ⇒ mute_audio false");
                assert!(
                    subtitle_path.is_none(),
                    "no subtitle track ⇒ subtitle_path None"
                );
                assert!(range_ms.is_none(), "no markers ⇒ full-span export");
                assert_eq!(video_spans.len(), 1, "one clip → one span");
                assert_eq!(video_spans[0].0, media);
                assert!(
                    audio_lane_spans.is_empty(),
                    "single media file ⇒ no dedicated audio lane"
                );
                assert_eq!(
                    primary_mute_mask,
                    vec![false],
                    "default unmuted clip ⇒ single-false mask"
                );
            }
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn preflight_carries_subtitle_path_when_subtitle_track_has_clip() {
        // A subtitle clip on the first TrackKind::Subtitle lane must surface
        // through the preflight as `subtitle_path = Some(...)` so the export
        // thread can pass it to `subtitles=` for burn-in.
        let (mut session, _media) = opened_session(3.0, "/tmp/fake-sub.mp4");
        session.add_subtitle_track().expect("add subtitle track");
        let dir = tempfile::tempdir().expect("tempdir");
        let srt = dir.path().join("cap.srt");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:02,000\nHi\n").unwrap();
        session
            .insert_subtitle_clip_at_playhead(srt.clone(), 0.0)
            .expect("insert subtitle");
        let dest = PathBuf::from("/tmp/with-subs.mp4");
        let stub = StubSaveDialog::always(Some(dest));
        match prepare_export_job(&session, 0, &stub) {
            ExportPreflight::Spawn { subtitle_path, .. } => {
                assert_eq!(subtitle_path.as_deref(), Some(srt.as_path()));
            }
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn preflight_sets_mute_audio_when_all_clips_muted() {
        // Single-clip session; mute it → preflight should request `-an`.
        let (mut session, _media) = opened_session(2.5, "/tmp/fake-mute.mp4");
        session.toggle_audio_mute_at_seq_ms(0.0).expect("toggle");
        let dest = PathBuf::from("/tmp/muted.mp4");
        let stub = StubSaveDialog::always(Some(dest.clone()));
        let pf = prepare_export_job(&session, 0, &stub);
        match pf {
            ExportPreflight::Spawn {
                mute_audio,
                primary_mute_mask,
                ..
            } => {
                assert!(mute_audio, "all clips muted ⇒ mute_audio true");
                assert_eq!(
                    primary_mute_mask,
                    vec![true],
                    "all-muted mask must reach the export thread"
                );
            }
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn preflight_partial_mute_without_audio_lane_spawns_with_mask() {
        // Two primary clips, first muted, second not ⇒ no Status refusal, no
        // full `-an`; the export thread gets the mixed mask so it can synthesize
        // a silence-substitution audio lane.
        use reel_core::project::{Clip, Project, Track};
        use reel_core::{MediaMetadata, TrackKind};
        use uuid::Uuid;

        fn clip(id: Uuid, path: &str, sec: f64, muted: bool) -> Clip {
            Clip {
                id,
                source_path: PathBuf::from(path),
                metadata: MediaMetadata {
                    path: PathBuf::from(path),
                    duration_seconds: sec,
                    container: "mp4".into(),
                    video: None,
                    audio: None,
                    audio_disabled: false,
                    video_stream_count: 0,
                    audio_stream_count: 0,
                    subtitle_stream_count: 0,
                    audio_streams: Vec::new(),
                },
                in_point: 0.0,
                out_point: sec,
                orientation: Default::default(),
                scale: Default::default(),
                audio_mute: muted,
                audio_stream_index: None,
                extensions: Default::default(),
            }
        }
        let c0 = Uuid::new_v4();
        let c1 = Uuid::new_v4();
        let mut p = Project::new("partial-mute");
        p.clips.push(clip(c0, "/tmp/part-a.mp4", 2.0, true));
        p.clips.push(clip(c1, "/tmp/part-b.mp4", 2.0, false));
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Video,
            clip_ids: vec![c0, c1],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        let session = EditSession::from_project_for_tests(p);
        let stub = StubSaveDialog::always(Some(PathBuf::from("/tmp/partial.mp4")));
        let pf = prepare_export_job(&session, 0, &stub);
        match pf {
            ExportPreflight::Spawn {
                mute_audio,
                primary_mute_mask,
                audio_lane_spans,
                ..
            } => {
                assert!(
                    !mute_audio,
                    "partial mute must NOT short-circuit to `-an` — silence substitution takes over"
                );
                assert_eq!(primary_mute_mask, vec![true, false]);
                assert!(
                    audio_lane_spans.is_empty(),
                    "no dedicated audio track ⇒ empty lane list; export thread synthesizes"
                );
            }
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn preflight_spawn_preserves_marker_range_for_status_text() {
        // The Spawn branch must echo the In/Out range so the callback can
        // produce the "Exporting range 1.000–2.000 s…" status message.
        let (mut session, _path) = opened_session(4.0, "/tmp/fake-range.mp4");
        session.set_in_marker_ms(1_000.0).unwrap();
        session.set_out_marker_ms(2_000.0).unwrap();
        let stub = StubSaveDialog::always(Some(PathBuf::from("/tmp/range.mp4")));
        let pf = prepare_export_job(&session, 0, &stub);
        match pf {
            ExportPreflight::Spawn { range_ms, .. } => {
                let (i, o) = range_ms.expect("range carried through");
                assert!((i - 1_000.0).abs() < 1e-6);
                assert!((o - 2_000.0).abs() < 1e-6);
            }
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn preflight_preset_index_maps_to_save_dialog_fmt() {
        // The stub records every `fmt` it was asked for; we use it to assert
        // preset-index → WebExportFormat wiring as seen by the save dialog
        // (covers every non-default preset — Mp4Remux is covered by the
        // happy-path test above).
        let (session, _path) = opened_session(1.0, "/tmp/fake-presets.mp4");
        for (idx, fmt, ext) in [
            (1, reel_core::WebExportFormat::Mp4H264Aac, "mp4"),
            (2, reel_core::WebExportFormat::Mp4H265Aac, "mp4"),
            (3, reel_core::WebExportFormat::WebmVp8Opus, "webm"),
            (4, reel_core::WebExportFormat::WebmVp9Opus, "webm"),
            (5, reel_core::WebExportFormat::WebmAv1Opus, "webm"),
            (6, reel_core::WebExportFormat::MkvRemux, "mkv"),
            (7, reel_core::WebExportFormat::MovRemux, "mov"),
            (8, reel_core::WebExportFormat::MovProResHq, "mov"),
            (9, reel_core::WebExportFormat::MkvDnxhrHq, "mkv"),
        ] {
            let dest = PathBuf::from(format!("/tmp/preset-{idx}.{ext}"));
            let stub = StubSaveDialog::always(Some(dest));
            let pf = prepare_export_job(&session, idx, &stub);
            assert!(
                matches!(pf, ExportPreflight::Spawn { fmt: got_fmt, .. } if got_fmt == fmt),
                "idx {idx} should map to {fmt:?}, got {pf:?}"
            );
            assert_eq!(
                stub.last_fmt(),
                Some(fmt),
                "save dialog for idx {idx} should be configured for {fmt:?}"
            );
        }
    }

    #[test]
    fn payload_exposes_first_audio_lane_when_present() {
        // Open a video, add an audio track, insert an audio clip at 0 —
        // payload's audio-lanes vec gains a single entry so ffmpeg muxing kicks in.
        let probe = FakeProbe::with_duration(4.0);
        let mut session = EditSession::default();
        let video_path = PathBuf::from("/tmp/fake-video-d.mp4");
        session
            .open_media_with_probe(&probe, video_path.clone())
            .unwrap();
        session.add_audio_track().expect("add audio track");
        let audio_path = PathBuf::from("/tmp/fake-audio-d.wav");
        session
            .insert_audio_clip_at_playhead_with_probe(&probe, audio_path.clone(), 0.0)
            .expect("insert audio");

        let (video, audio_lanes, _mute_mask) =
            export_timeline_payload(&session, None).expect("payload");
        assert_eq!(video[0].0, video_path);
        assert_eq!(audio_lanes.len(), 1, "exactly one audio lane with clips");
        assert_eq!(audio_lanes[0].len(), 1);
        assert_eq!(audio_lanes[0][0].0, audio_path);
    }

    #[test]
    fn payload_exposes_each_audio_lane_when_multiple_audio_tracks_have_clips() {
        // Two audio tracks each holding one clip ⇒ the amix dispatcher should see
        // two populated span-lists. This is the U2-b gate: if this regresses to a
        // single lane, the export drops one audio stream silently.
        //
        // `insert_audio_clip_at_playhead_with_probe` only targets the first audio
        // lane, so we synthesize the multi-lane project directly — the test
        // exercises `export_timeline_payload`, not the insert helpers.
        use reel_core::project::{Clip, Project, Track};
        use reel_core::{MediaMetadata, TrackKind};
        use uuid::Uuid;

        fn clip(id: Uuid, path: &str, sec: f64) -> Clip {
            Clip {
                id,
                source_path: PathBuf::from(path),
                metadata: MediaMetadata {
                    path: PathBuf::from(path),
                    duration_seconds: sec,
                    container: "mp4".into(),
                    video: None,
                    audio: None,
                    audio_disabled: false,
                    video_stream_count: 0,
                    audio_stream_count: 0,
                    subtitle_stream_count: 0,
                    audio_streams: Vec::new(),
                },
                in_point: 0.0,
                out_point: sec,
                orientation: Default::default(),
                scale: Default::default(),
                audio_mute: false,
                audio_stream_index: None,
                extensions: Default::default(),
            }
        }

        let v_id = Uuid::new_v4();
        let a0_id = Uuid::new_v4();
        let a1_id = Uuid::new_v4();
        let v_path = PathBuf::from("/tmp/fake-video-multi.mp4");
        let a0_path = PathBuf::from("/tmp/fake-audio-multi-0.wav");
        let a1_path = PathBuf::from("/tmp/fake-audio-multi-1.wav");

        let mut p = Project::new("multi");
        p.clips.push(clip(v_id, v_path.to_str().unwrap(), 5.0));
        p.clips.push(clip(a0_id, a0_path.to_str().unwrap(), 5.0));
        p.clips.push(clip(a1_id, a1_path.to_str().unwrap(), 5.0));
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Video,
            clip_ids: vec![v_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Audio,
            clip_ids: vec![a0_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Audio,
            clip_ids: vec![a1_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });

        let session = EditSession::from_project_for_tests(p);
        let (_video, audio_lanes, _mute_mask) =
            export_timeline_payload(&session, None).expect("payload");
        assert_eq!(
            audio_lanes.len(),
            2,
            "both audio tracks with clips should surface as lanes"
        );
        assert_eq!(audio_lanes[0][0].0, a0_path);
        assert_eq!(audio_lanes[1][0].0, a1_path);
    }
}
