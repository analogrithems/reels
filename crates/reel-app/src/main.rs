//! Reel desktop app entry point.

mod autosave;
mod effects;
mod player;
mod project_io;
mod session;
mod shell;
mod timecode;
mod timeline;
mod ui_bridge;

use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use reel_core::export_concat_timeline;
use session::{
    export_format_for_path, split_enabled_for_playhead, video_lane_indices, EditSession,
};
use slint::ComponentHandle;

use crate::project_io::save_project;
use crate::ui_bridge::on_ui;

slint::include_modules!();

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

pub(crate) fn sync_menu(window: &AppWindow, session: &EditSession) {
    window.set_close_enabled(session.close_enabled());
    window.set_revert_enabled(session.revert_enabled());
    window.set_save_enabled(session.save_enabled());
    window.set_undo_enabled(session.undo_enabled());
    window.set_redo_enabled(session.redo_enabled());
    window.set_timeline_info(session.timeline_summary_line().into());
    window.set_video_track_lanes(session.video_track_row_labels().join("\n").into());
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
}

fn reload_player_timeline(sender: &player::PlayerCmdSender, session: &EditSession) {
    let Some(p) = session.project() else {
        sender.send(player::Cmd::Close);
        return;
    };
    if let Some(sync) = timeline::timeline_sync_from_project(p) {
        sender.send(player::Cmd::LoadTimeline { sync });
    } else {
        sender.send(player::Cmd::Close);
    }
}

fn sync_menu_and_autosave(
    window: &AppWindow,
    session_rc: &Rc<RefCell<EditSession>>,
    debouncer: &autosave::AutosaveDebouncer,
) {
    sync_menu(window, &session_rc.borrow());
    debouncer.nudge(Rc::clone(session_rc));
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
    let _log_guard = reel_core::logging::init()?;
    tracing::info!("reel starting");

    let window = AppWindow::new()?;
    window.set_media_ready(false);
    window.set_timecode("0:00.000 / 0:00.000".into());
    window.set_timeline_info("".into());
    window.set_video_track_lanes("".into());
    window.set_move_clip_down_enabled(false);
    window.set_move_clip_up_enabled(false);
    window.set_split_at_playhead_enabled(false);
    window.set_video_fit_mode(0);
    window.set_stay_on_top(false);

    let session = Rc::new(RefCell::new(EditSession::default()));
    let debouncer = Rc::new(autosave::AutosaveDebouncer::new(window.as_weak()));
    let export_cancel = Arc::new(Mutex::new(None::<Arc<AtomicBool>>));

    let player = match player::spawn_player(&window) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to start player threads");
            window.set_status_text(format!("Player init failed: {e}").into());
            window.run()?;
            return Err(e);
        }
    };

    let weak = window.as_weak();
    {
        let p = player_handle_ref(&player);
        let session = Rc::clone(&session);
        let debouncer = Rc::clone(&debouncer);
        window.on_file_open(move || match prompt_open_dialog() {
            Some(path) => {
                tracing::info!(?path, "file open");
                if let Some(w) = weak.upgrade() {
                    w.set_is_playing(false);
                    w.set_media_ready(false);
                }
                let open_result = session.borrow_mut().open_media(path.clone());
                match open_result {
                    Ok(()) => {
                        if let Some(w) = weak.upgrade() {
                            sync_menu_and_autosave(&w, &session, &debouncer);
                        }
                        reload_player_timeline(&p, &session.borrow());
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "open project");
                        if let Some(w) = weak.upgrade() {
                            w.set_status_text(format!("Open failed: {e}").into());
                            sync_menu_and_autosave(&w, &session, &debouncer);
                        }
                    }
                }
            }
            None => tracing::debug!("open cancelled"),
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
                            if let Some(w) = weak.upgrade() {
                                sync_menu_and_autosave(&w, &session, &debouncer);
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
        window.on_file_insert_video(move || match prompt_insert_dialog() {
            Some(insert_path) => {
                let playhead_ms = weak
                    .upgrade()
                    .map(|w| w.get_playhead_ms() as f64)
                    .unwrap_or(0.0);
                let insert_result = session
                    .borrow_mut()
                    .insert_clip_at_playhead(insert_path, playhead_ms);
                match insert_result {
                    Ok(()) => {
                        if let Some(w) = weak.upgrade() {
                            let n = session
                                .borrow()
                                .project()
                                .map(|pr| pr.clips.len())
                                .unwrap_or(0);
                            sync_menu_and_autosave(&w, &session, &debouncer);
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
        window.on_file_new_video_track(move || {
            let r = session.borrow_mut().add_video_track();
            match r {
                Ok(()) => {
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer);
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
        let export_cancel_slot = Arc::clone(&export_cancel);
        window.on_file_export(move || {
            let segs = session
                .borrow()
                .project()
                .and_then(timeline::clips_from_project)
                .map(|clips| {
                    clips
                        .into_iter()
                        .map(|c| (c.path, c.media_in_s, c.media_out_s))
                        .collect::<Vec<_>>()
                });
            let Some(segs) = segs.filter(|s| !s.is_empty()) else {
                return;
            };
            if let Some(dest) = rfd::FileDialog::new()
                .set_title("Export media…")
                .add_filter("MP4", &["mp4"])
                .add_filter("WebM", &["webm"])
                .add_filter("Matroska", &["mkv"])
                .save_file()
            {
                let fmt = match export_format_for_path(&dest) {
                    Some(f) => f,
                    None => {
                        if let Some(w) = weak.upgrade() {
                            w.set_status_text("Choose .mp4, .webm, or .mkv".into());
                        }
                        return;
                    }
                };
                let cancel = Arc::new(AtomicBool::new(false));
                *export_cancel_slot.lock().expect("export cancel mutex") = Some(cancel.clone());
                if let Some(w) = weak.upgrade() {
                    w.set_status_text("Exporting…".into());
                    w.set_export_in_progress(true);
                }
                let weak_done = weak.clone();
                let dest_disp = dest.display().to_string();
                let slot_clear = Arc::clone(&export_cancel_slot);
                let res = std::thread::Builder::new()
                    .name("reel-export".into())
                    .spawn(move || {
                        let r = export_concat_timeline(&segs, &dest, fmt, Some(cancel.as_ref()));
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
                        w.set_status_text("Could not start export".into());
                    });
                }
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
        window.on_edit_undo(move || {
            if !session.borrow_mut().undo() {
                return;
            }
            reload_player_timeline(&sender, &session.borrow());
            if let Some(w) = weak.upgrade() {
                sync_menu_and_autosave(&w, &session, &debouncer);
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
        window.on_edit_redo(move || {
            if !session.borrow_mut().redo() {
                return;
            }
            reload_player_timeline(&sender, &session.borrow());
            if let Some(w) = weak.upgrade() {
                sync_menu_and_autosave(&w, &session, &debouncer);
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
                        sync_menu_and_autosave(&w, &session, &debouncer);
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
                        sync_menu_and_autosave(&w, &session, &debouncer);
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
        window.on_edit_move_clip_up(move || {
            let r = session
                .borrow_mut()
                .move_first_clip_from_second_video_track_to_primary();
            match r {
                Ok(()) => {
                    reload_player_timeline(&sender, &session.borrow());
                    if let Some(w) = weak.upgrade() {
                        sync_menu_and_autosave(&w, &session, &debouncer);
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
        window.on_win_fit(move || {
            if let Some(w) = weak.upgrade() {
                w.set_video_fit_mode(0);
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_win_fill(move || {
            if let Some(w) = weak.upgrade() {
                w.set_video_fit_mode(1);
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_win_center(move || {
            if let Some(w) = weak.upgrade() {
                w.set_video_fit_mode(0);
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

    sync_menu(&window, &session.borrow());

    if let Some(path) = startup_auto_open_path() {
        tracing::info!(?path, "auto-opening from REEL_OPEN_PATH");
        let startup_open = session.borrow_mut().open_media(path.clone());
        match startup_open {
            Ok(()) => {
                sync_menu_and_autosave(&window, &session, &debouncer);
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
            .set_title("Open media…")
            .add_filter("Video", &["mov", "mp4", "mkv", "m4v", "webm", "avi"])
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
            .add_filter("Video", &["mov", "mp4", "mkv", "m4v", "webm", "avi"])
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
