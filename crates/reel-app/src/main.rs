//! Reel desktop app entry point.

mod autosave;
mod effects;
mod footer;
mod media_extensions;
mod player;
mod prefs;
mod project_io;
mod recent;
mod session;
mod shell;
mod timecode;
mod timeline;
mod ui_bridge;

use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use reel_core::TrackKind;
use reel_core::{export_concat_with_audio, ExportProgressFn};
use session::{
    export_format_for_path, split_enabled_for_playhead, video_lane_indices,
    web_export_format_from_preset_index, EditSession,
};
use slint::ComponentHandle;

use crate::media_extensions::{
    AUDIO_FILE_EXTENSIONS, OPEN_MEDIA_EXTENSIONS, VIDEO_CONTAINER_EXTENSIONS,
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

/// Primary video spans + optional first-audio-track spans for ffmpeg export (empty timeline → `None`).
fn export_timeline_payload(
    session: &EditSession,
) -> Option<(ExportSpanVec, Option<ExportSpanVec>)> {
    let segs = session
        .project()
        .and_then(timeline::clips_from_project)
        .map(|clips| {
            clips
                .into_iter()
                .map(|c| (c.path, c.media_in_s, c.media_out_s))
                .collect::<Vec<_>>()
        })?;
    if segs.is_empty() {
        return None;
    }
    let audio_segs = session.project().and_then(|p| {
        timeline::clips_from_first_audio_track(p).map(|clips| {
            clips
                .into_iter()
                .map(|c| (c.path, c.media_in_s, c.media_out_s))
                .collect::<Vec<_>>()
        })
    });
    Some((segs, audio_segs))
}

fn export_save_dialog(fmt: reel_core::WebExportFormat) -> rfd::FileDialog {
    let d = rfd::FileDialog::new().set_title("Export media…");
    match fmt {
        reel_core::WebExportFormat::Mp4Remux => d.add_filter("MP4", &["mp4", "m4v"]),
        reel_core::WebExportFormat::WebmVp8Opus => d.add_filter("WebM", &["webm"]),
        reel_core::WebExportFormat::MkvRemux => d.add_filter("Matroska", &["mkv"]),
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

fn sync_footer(window: &AppWindow, session: &EditSession) {
    let ph = window.get_playhead_ms() as f64;
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
    } else {
        window.set_duration_ms(0.0);
        window.set_move_clip_down_enabled(false);
        window.set_move_clip_up_enabled(false);
        window.set_split_at_playhead_enabled(false);
    }
    let ph = window.get_playhead_ms();
    let dur = window.get_duration_ms();
    window.set_timecode(timecode::format_pair(ph, dur).into());
    sync_footer(window, session);
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
    if let Some(video) = timeline::timeline_sync_from_project(p) {
        let audio = timeline::dedicated_audio_timeline_sync_from_project(p);
        sender.send(player::Cmd::LoadTimeline { video, audio });
    } else {
        sender.send(player::Cmd::Close);
    }
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

fn open_path_from_ui(
    path: PathBuf,
    sender: &player::PlayerCmdSender,
    session: &Rc<RefCell<EditSession>>,
    weak: &slint::Weak<AppWindow>,
    debouncer: &autosave::AutosaveDebouncer,
    recent: &Rc<RefCell<RecentStore>>,
) {
    tracing::info!(?path, "open path");
    if let Some(w) = weak.upgrade() {
        w.set_is_playing(false);
        w.set_media_ready(false);
    }
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
    match HelpWindow::new() {
        Ok(h) => {
            h.set_help_title(title.into());
            h.set_body_text(body.into());
            if let Err(e) = h.show() {
                tracing::warn!(error = %e, "help window show failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "help window create failed"),
    }
}

fn main() -> Result<()> {
    // Session logs always go to a file (see `reel_core::logging`); stdout mirroring is optional.
    let _log = reel_core::logging::init()?;
    if let Some(ref p) = _log.session_log_path {
        tracing::info!(session_log = %p.display(), "reel starting");
    } else {
        tracing::info!("reel starting (tracing was already initialized)");
    }

    let window = AppWindow::new()?;
    window.set_media_ready(false);
    window.set_timecode("0:00.000 / 0:00.000".into());
    window.set_video_track_lanes("".into());
    window.set_audio_track_lanes("".into());
    window.set_insert_audio_enabled(false);
    window.set_move_clip_down_enabled(false);
    window.set_move_clip_up_enabled(false);
    window.set_split_at_playhead_enabled(false);
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
    window.set_window_fullscreen(false);
    let playback_speed_milli = Arc::new(AtomicU32::new(1000));

    let session = Rc::new(RefCell::new(EditSession::default()));
    let debouncer = Rc::new(autosave::AutosaveDebouncer::new(window.as_weak()));
    let recent = Rc::new(RefCell::new(RecentStore::load()));
    let export_cancel = Arc::new(Mutex::new(None::<Arc<AtomicBool>>));

    let player = match player::spawn_player(
        &window,
        vol_arc,
        playback_speed_milli.clone(),
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
        let spd = playback_speed_milli.clone();
        window.on_playback_speed_changed(move |idx| {
            let milli = match idx {
                0 => 250,
                1 => 500,
                2 => 750,
                3 => 1000,
                4 => 1250,
                5 => 1500,
                6 => 2000,
                _ => 1000,
            };
            spd.store(milli, Ordering::Relaxed);
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
        window.on_file_open(move || match prompt_open_dialog() {
            Some(path) => {
                open_path_from_ui(path, &sender, &session, &weak, &debouncer, &recent);
            }
            None => tracing::debug!("open cancelled"),
        });
    }

    {
        let sender = player.cmd_sender();
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        let recent = Rc::clone(&recent);
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
            open_path_from_ui(path, &sender, &session, &weak, &debouncer, &recent);
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
        window.on_file_close(move || {
            if let Err(e) = session.borrow_mut().flush_autosave_if_needed() {
                tracing::warn!(error = %e, "autosave before close failed");
            }
            let mut s = session.borrow_mut();
            s.clear_media();
            p.send(player::Cmd::Close);
            if let Some(w) = weak.upgrade() {
                sync_menu(&w, &s);
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
        window.on_file_export(move || {
            if export_timeline_payload(&session.borrow()).is_none() {
                return;
            }
            if let Some(w) = weak.upgrade() {
                w.set_export_preset_dialog_visible(true);
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_export_preset_cancel(move || {
            if let Some(w) = weak.upgrade() {
                w.set_export_preset_dialog_visible(false);
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        let export_cancel_slot = Arc::clone(&export_cancel);
        window.on_export_preset_confirm(move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let idx = w.get_export_preset_index();
            w.set_export_preset_dialog_visible(false);
            drop(w);

            let Some(fmt) = web_export_format_from_preset_index(idx) else {
                if let Some(w) = weak.upgrade() {
                    w.set_status_text("Invalid export preset.".into());
                }
                return;
            };

            let Some((segs, audio_segs)) = export_timeline_payload(&session.borrow()) else {
                return;
            };

            let Some(dest) = export_save_dialog(fmt).save_file() else {
                return;
            };

            if export_format_for_path(&dest) != Some(fmt) {
                if let Some(w) = weak.upgrade() {
                    w.set_status_text(
                        format!("Use a .{} file name for this preset.", fmt.file_extension())
                            .into(),
                    );
                }
                return;
            }

            let cancel = Arc::new(AtomicBool::new(false));
            *export_cancel_slot.lock().expect("export cancel mutex") = Some(cancel.clone());
            if let Some(w) = weak.upgrade() {
                w.set_status_text("Exporting…".into());
                w.set_export_progress(0.0);
                w.set_export_in_progress(true);
            }
            let weak_done = weak.clone();
            let weak_prog = weak.clone();
            let dest_disp = dest.display().to_string();
            let slot_clear = Arc::clone(&export_cancel_slot);
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
                    let r = export_concat_with_audio(
                        &segs,
                        audio_segs.as_deref(),
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
                                format!("Export failed: {e}")
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
                *export_cancel_slot.lock().expect("export cancel mutex") = None;
                on_ui(weak.clone(), |w| {
                    w.set_export_in_progress(false);
                    w.set_export_progress(0.0);
                    w.set_status_text("Could not start export".into());
                });
            }
        });
    }

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
                        w.set_timecode(timecode::format_pair(ph, dur).into());
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
                        w.set_timecode(timecode::format_pair(ph, dur).into());
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
                        w.set_timecode(timecode::format_pair(ph, dur).into());
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
            w.set_playhead_ms(v);
            let dur = w.get_duration_ms();
            w.set_timecode(timecode::format_pair(v, dur).into());
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
            let next = (cur + delta).clamp(0.0, dur);
            w.set_playhead_ms(next);
            w.set_timecode(timecode::format_pair(next, dur).into());
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

    sync_menu(&window, &session.borrow());
    sync_recent_menu(&window, &recent.borrow());

    if let Some(path) = startup_auto_open_path() {
        tracing::info!(?path, "auto-opening from REEL_OPEN_PATH");
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
                window.set_status_text(format!("REEL_OPEN_PATH failed: {e}").into());
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

fn startup_auto_open_path() -> Option<PathBuf> {
    let env = std::env::var_os("REEL_OPEN_PATH")?;
    let p = PathBuf::from(env);
    if p.as_os_str().is_empty() {
        None
    } else {
        Some(p)
    }
}
