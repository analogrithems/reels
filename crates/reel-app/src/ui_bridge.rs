//! Tiny helpers around the Slint event loop. Kept apart from `player.rs` so
//! the threading rules (only upgrade `Weak<AppWindow>` inside
//! `invoke_from_event_loop`) are visible in one place.

use slint::Weak;

use crate::AppWindow;

/// Run `f` on the UI thread with a live `AppWindow` handle. No-op if the
/// window has been dropped.
pub fn on_ui<F>(weak: Weak<AppWindow>, f: F)
where
    F: FnOnce(AppWindow) + Send + 'static,
{
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            f(w);
        }
    });
}
