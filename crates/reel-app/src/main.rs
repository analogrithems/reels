//! Reel desktop app entry point.

mod autosave;
mod effects;
mod player;
mod project_io;
mod session;
mod shell;
mod ui_bridge;

use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;
use reel_core::export_with_ffmpeg;
use session::{export_format_for_path, EditSession};
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
    let media = match session.borrow().playback_path() {
        Some(p) => p,
        None => {
            on_ui(weak, |w| w.set_status_text("No video loaded.".into()));
            return;
        }
    };
    let playhead_ms = weak
        .upgrade()
        .map(|w| w.get_playhead_ms() as u64)
        .unwrap_or(0);

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
            let r = effects::apply_effect_to_png(&media, playhead_ms, effect, &sidecar, &dest);
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
    window.set_video_fit_mode(0);
    window.set_stay_on_top(false);

    let session = Rc::new(RefCell::new(EditSession::default()));
    let debouncer = Rc::new(autosave::AutosaveDebouncer::new(window.as_weak()));

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
                        p.send(player::Cmd::Open(path));
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
                    if let Some(pb) = session.borrow().playback_path() {
                        p.send(player::Cmd::Open(pb));
                    }
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
                let before_pb = session.borrow().playback_path();
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
                        let after_pb = session.borrow().playback_path();
                        if after_pb != before_pb {
                            if let Some(pb) = after_pb {
                                sender.send(player::Cmd::Open(pb));
                            }
                        }
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
        window.on_file_export(move || {
            let src = session.borrow().current_media.clone();
            if let Some(src) = src {
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
                    match export_with_ffmpeg(&src, &dest, fmt) {
                        Ok(()) => {
                            if let Some(w) = weak.upgrade() {
                                w.set_status_text(format!("Exported to {}", dest.display()).into());
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "export failed");
                            if let Some(w) = weak.upgrade() {
                                w.set_status_text(format!("Export failed: {e}").into());
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
        window.on_edit_undo(move || {
            let before = session.borrow().playback_path();
            if !session.borrow_mut().undo() {
                return;
            }
            let after = session.borrow().playback_path();
            if after != before {
                if let Some(pb) = after {
                    sender.send(player::Cmd::Open(pb));
                }
            }
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
            let before = session.borrow().playback_path();
            if !session.borrow_mut().redo() {
                return;
            }
            let after = session.borrow().playback_path();
            if after != before {
                if let Some(pb) = after {
                    sender.send(player::Cmd::Open(pb));
                }
            }
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
        window.on_help_media_formats(move || show_help_window(shell::HelpDoc::MediaFormats));
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
        window.on_seek_timeline(move |v| {
            if let Some(w) = weak.upgrade() {
                if !w.get_media_ready() {
                    return;
                }
            }
            p.send(player::Cmd::Seek { pts_ms: v as u64 });
        });
    }

    sync_menu(&window, &session.borrow());

    if let Some(path) = startup_auto_open_path() {
        tracing::info!(?path, "auto-opening from REEL_OPEN_PATH");
        let startup_open = session.borrow_mut().open_media(path.clone());
        match startup_open {
            Ok(()) => {
                sync_menu_and_autosave(&window, &session, &debouncer);
                player.cmd_sender().send(player::Cmd::Open(path));
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
