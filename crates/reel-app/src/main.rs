//! Reel desktop app entry point.

mod player;
mod ui_bridge;

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

use anyhow::Result;
use slint::ComponentHandle;

slint::include_modules!();

fn main() -> Result<()> {
    let _log_guard = reel_core::logging::init()?;
    tracing::info!("reel starting");

    let window = AppWindow::new()?;
    window.set_media_ready(false);

    let player = match player::spawn_player(&window) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to start player threads");
            window.set_status_text(format!("Player init failed: {e}").into());
            // Still run the window so the user sees the error rather than
            // getting a silent crash on startup.
            window.run()?;
            return Err(e);
        }
    };

    // Callbacks: open, play/pause, seek.
    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        window.on_open_file(move || {
            match prompt_open_dialog() {
                Some(path) => {
                    tracing::info!(?path, "opening file");
                    if let Some(w) = weak.upgrade() {
                        w.set_is_playing(false);
                        w.set_media_ready(false);
                    }
                    p.send(player::Cmd::Open(path));
                    // Note: we do NOT auto-Play. Playback only begins once
                    // the user clicks Play, which is gated on media-ready.
                }
                None => {
                    tracing::debug!("open dialog cancelled");
                }
            }
        });
    }
    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        window.on_play_pause(move || {
            if let Some(w) = weak.upgrade() {
                // Defensive: even though the UI disables the Play button
                // when media-ready is false, never issue Play against an
                // un-loaded source.
                if !w.get_media_ready() {
                    tracing::debug!("play-pause ignored: media not ready");
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
        window.on_seek(move |v| {
            if let Some(w) = weak.upgrade() {
                if !w.get_media_ready() {
                    return;
                }
            }
            p.send(player::Cmd::Seek { pts_ms: v as u64 });
        });
    }

    // If REEL_OPEN_PATH was set, send a one-shot Open so the app boots with
    // a file already loaded. We do this *after* all callbacks are wired so
    // the resulting media-ready flip reaches the UI.
    if let Some(path) = startup_auto_open_path() {
        tracing::info!(?path, "auto-opening from REEL_OPEN_PATH");
        player.cmd_sender().send(player::Cmd::Open(path));
    }

    window.run()?;
    drop(player);
    Ok(())
}

fn player_handle_ref(p: &player::PlayerHandle) -> player::PlayerCmdSender {
    p.cmd_sender()
}

/// Block on a native file-picker dialog.
///
/// Always opens the `rfd` native panel. `REEL_OPEN_PATH` is no longer
/// consulted here — see [`startup_auto_open_path`] — so that clicking Open
/// in the UI always gives the user a chance to pick a different file even
/// when a dev env var is set.
///
/// Wrapped in `catch_unwind` because a misbehaving platform dialog should
/// never be able to take down the Slint event loop.
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

/// Consumed once at startup: if `REEL_OPEN_PATH` is set, pre-queue an Open
/// command so smoke scripts can launch the app with a file already loaded
/// (the user must still click Play — that stays gated on media-ready).
fn startup_auto_open_path() -> Option<PathBuf> {
    let env = std::env::var_os("REEL_OPEN_PATH")?;
    let p = PathBuf::from(env);
    if p.as_os_str().is_empty() {
        None
    } else {
        Some(p)
    }
}
