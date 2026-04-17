//! Reel desktop app entry point.

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

slint::include_modules!();

fn sync_menu(window: &AppWindow, session: &EditSession) {
    window.set_close_enabled(session.close_enabled());
    window.set_revert_enabled(session.revert_enabled());
    window.set_save_enabled(session.save_enabled());
    window.set_undo_enabled(session.undo_enabled());
    window.set_redo_enabled(session.redo_enabled());
}

fn main() -> Result<()> {
    let _log_guard = reel_core::logging::init()?;
    tracing::info!("reel starting");

    let window = AppWindow::new()?;
    window.set_media_ready(false);
    window.set_video_fit_mode(0);
    window.set_stay_on_top(false);

    let session = Rc::new(RefCell::new(EditSession::default()));

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
        window.on_file_open(move || match prompt_open_dialog() {
            Some(path) => {
                tracing::info!(?path, "file open");
                session.borrow_mut().set_media(path.clone());
                if let Some(w) = weak.upgrade() {
                    w.set_is_playing(false);
                    w.set_media_ready(false);
                    sync_menu(&w, &session.borrow());
                }
                p.send(player::Cmd::Open(path));
            }
            None => tracing::debug!("open cancelled"),
        });
    }

    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_file_close(move || {
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
            let path = session.borrow().current_media.clone();
            if let Some(path) = path {
                session.borrow_mut().revert_to_saved();
                p.send(player::Cmd::Pause);
                p.send(player::Cmd::Open(path));
                if let Some(w) = weak.upgrade() {
                    sync_menu(&w, &session.borrow());
                    w.set_status_text("Reverted — reloading…".into());
                }
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
        window.on_file_save(move || {
            let media = session.borrow().current_media.clone();
            if let Some(media) = media {
                if let Some(dest) = rfd::FileDialog::new()
                    .set_title("Save project…")
                    .add_filter("Reel project", &["reel", "json"])
                    .save_file()
                {
                    match project_io::save_project_reel(&dest, &media) {
                        Ok(()) => {
                            session.borrow_mut().mark_saved();
                            if let Some(w) = weak.upgrade() {
                                sync_menu(&w, &session.borrow());
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
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_file_insert_video(move || match prompt_insert_dialog() {
            Some(path) => {
                session.borrow_mut().push_insert(path.clone());
                if let Some(w) = weak.upgrade() {
                    sync_menu(&w, &session.borrow());
                    w.set_status_text(format!("Queued insert: {}", path.display()).into());
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
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_edit_undo(move || {
            if let Some(op) = session.borrow_mut().undo() {
                if let Some(w) = weak.upgrade() {
                    sync_menu(&w, &session.borrow());
                    w.set_status_text(format!("Undo: {op}").into());
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let session = Rc::clone(&session);
        window.on_edit_redo(move || {
            if let Some(op) = session.borrow_mut().redo() {
                if let Some(w) = weak.upgrade() {
                    sync_menu(&w, &session.borrow());
                    w.set_status_text(format!("Redo: {op}").into());
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
        window.on_help_show(move || match HelpWindow::new() {
            Ok(h) => {
                h.set_body_text(shell::bundled_help_markdown().into());
                if let Err(e) = h.show() {
                    tracing::warn!(error = %e, "help window show failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "help window create failed"),
        });
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
        session.borrow_mut().set_media(path.clone());
        sync_menu(&window, &session.borrow());
        player.cmd_sender().send(player::Cmd::Open(path));
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
