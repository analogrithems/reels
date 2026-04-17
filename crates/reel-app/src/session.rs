//! Editable session state (undo/redo, dirty bit) — independent of Slint so we can unit test it.

use std::path::{Path, PathBuf};

/// Tracks unsaved work and a simple string-based undo/redo history.
#[derive(Debug, Clone, Default)]
pub struct EditSession {
    /// Media file currently loaded (or last attempted path).
    pub current_media: Option<PathBuf>,
    /// Pending insert paths (timeline integration comes later).
    pub pending_inserts: Vec<PathBuf>,
    pub dirty: bool,
    undo: Vec<String>,
    redo: Vec<String>,
}

impl EditSession {
    pub fn set_media(&mut self, path: PathBuf) {
        self.current_media = Some(path);
    }

    pub fn clear_media(&mut self) {
        self.current_media = None;
        self.pending_inserts.clear();
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
    }

    pub fn has_media(&self) -> bool {
        self.current_media.is_some()
    }

    pub fn record_edit(&mut self, description: impl Into<String>) {
        let s = description.into();
        self.undo.push(s);
        self.redo.clear();
        self.dirty = true;
    }

    pub fn undo(&mut self) -> Option<String> {
        let op = self.undo.pop()?;
        self.redo.push(op.clone());
        if self.undo.is_empty() {
            self.dirty = false;
        }
        Some(op)
    }

    pub fn redo(&mut self) -> Option<String> {
        let op = self.redo.pop()?;
        self.undo.push(op.clone());
        self.dirty = true;
        Some(op)
    }

    pub fn revert_to_saved(&mut self) {
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
    }

    pub fn push_insert(&mut self, path: PathBuf) {
        self.pending_inserts.push(path.clone());
        self.record_edit(format!("Insert {}", path.display()));
    }

    pub fn undo_enabled(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn redo_enabled(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn save_enabled(&self) -> bool {
        self.dirty && self.has_media()
    }

    pub fn revert_enabled(&self) -> bool {
        self.dirty && self.has_media()
    }

    pub fn close_enabled(&self) -> bool {
        self.has_media()
    }

    /// After a successful save to disk, clear dirty state.
    pub fn mark_saved(&mut self) {
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
    }
}

/// Map a save/export file extension to [`reel_core::WebExportFormat`].
pub fn export_format_for_path(path: &Path) -> Option<reel_core::WebExportFormat> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "mp4" | "m4v" => Some(reel_core::WebExportFormat::Mp4Remux),
        "webm" => Some(reel_core::WebExportFormat::WebmVp8Opus),
        "mkv" => Some(reel_core::WebExportFormat::MkvRemux),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_cleared_after_undo_all() {
        let mut s = EditSession::default();
        s.set_media(PathBuf::from("/a.mp4"));
        s.record_edit("cut");
        assert!(s.dirty);
        assert!(s.undo().is_some());
        assert!(!s.dirty);
    }

    #[test]
    fn redo_restores_dirty() {
        let mut s = EditSession::default();
        s.set_media(PathBuf::from("/a.mp4"));
        s.record_edit("x");
        let _ = s.undo();
        assert!(!s.dirty);
        let _ = s.redo();
        assert!(s.dirty);
    }

    #[test]
    fn revert_enabled_only_when_dirty_and_media() {
        let mut s = EditSession::default();
        assert!(!s.revert_enabled());
        s.set_media(PathBuf::from("/a.mp4"));
        s.record_edit("e");
        assert!(s.revert_enabled());
    }

    #[test]
    fn export_format_mapping() {
        assert_eq!(
            export_format_for_path(Path::new("x.webm")),
            Some(reel_core::WebExportFormat::WebmVp8Opus)
        );
    }
}
