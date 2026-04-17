//! Debounced autosave: after edits, persist to the project file on disk without clearing undo.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{Timer, TimerMode};

use crate::session::EditSession;
use crate::sync_menu;
use crate::AppWindow;

/// Single-shot timer coalescing bursts of edits (~900 ms after the last nudge).
pub struct AutosaveDebouncer {
    weak: slint::Weak<AppWindow>,
    timer: Rc<RefCell<Option<Timer>>>,
}

impl AutosaveDebouncer {
    pub fn new(weak: slint::Weak<AppWindow>) -> Self {
        Self {
            weak,
            timer: Rc::new(RefCell::new(None)),
        }
    }

    /// Call after timeline/project mutations when the session may be dirty.
    pub fn nudge(&self, session: Rc<RefCell<EditSession>>) {
        let eligible = {
            let s = session.borrow();
            s.dirty && s.project().and_then(|p| p.path.clone()).is_some()
        };
        if !eligible {
            return;
        }

        self.timer.borrow_mut().take();

        let weak = self.weak.clone();
        let timer_slot = Rc::clone(&self.timer);
        let sess = Rc::clone(&session);
        let t = Timer::default();
        t.start(
            TimerMode::SingleShot,
            Duration::from_millis(900),
            move || {
                let wrote = match sess.borrow_mut().flush_autosave_if_needed() {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!(error = %e, "autosave failed");
                        false
                    }
                };
                if wrote {
                    if let Some(w) = weak.upgrade() {
                        sync_menu(&w, &sess.borrow());
                        w.set_status_text("Autosaved project".into());
                    }
                }
                *timer_slot.borrow_mut() = None;
            },
        );
        *self.timer.borrow_mut() = Some(t);
    }
}
