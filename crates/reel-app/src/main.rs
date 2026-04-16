//! Reel desktop app entry point.

mod player;
mod ui_bridge;

use std::path::PathBuf;

use anyhow::Result;
use slint::ComponentHandle;

slint::include_modules!();

fn main() -> Result<()> {
    let _log_guard = reel_core::logging::init()?;
    tracing::info!("reel starting");

    let window = AppWindow::new()?;
    let player = player::spawn_player(&window)?;

    // Callbacks: open, play/pause, seek.
    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        window.on_open_file(move || {
            if let Some(path) = prompt_open_dialog() {
                tracing::info!(?path, "opening file");
                p.send(player::Cmd::Open(path));
                p.send(player::Cmd::Play);
                if let Some(w) = weak.upgrade() {
                    w.set_is_playing(true);
                }
            }
        });
    }
    {
        let p = player_handle_ref(&player);
        let weak = window.as_weak();
        window.on_play_pause(move || {
            if let Some(w) = weak.upgrade() {
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
        window.on_seek(move |v| {
            p.send(player::Cmd::Seek { pts_ms: v as u64 });
        });
    }

    window.run()?;
    drop(player);
    Ok(())
}

/// `player` lives past `run()` because `window.run()` blocks; each callback
/// needs a clone-able reference to the command channel. Rather than exposing
/// `Clone` on `PlayerHandle` (it owns `JoinHandle`s that aren't cloneable),
/// the handle stores a `Sender` internally; we expose a thin `Arc` around the
/// sender to share.
fn player_handle_ref(p: &player::PlayerHandle) -> player::PlayerCmdSender {
    p.cmd_sender()
}

/// Block and ask the OS for a file path. Uses `rfd` if available; falls back
/// to `None` otherwise. For Phase 2 we ship without a file dialog dependency
/// and defer a proper chooser to Phase 3 — this stub reads `REEL_OPEN_PATH`
/// from the environment as a developer affordance.
fn prompt_open_dialog() -> Option<PathBuf> {
    std::env::var_os("REEL_OPEN_PATH").map(PathBuf::from)
}
