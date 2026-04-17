//! In-memory edit session: [`reel_core::Project`], save baseline, undo/redo snapshots.

use std::path::{Path, PathBuf};

use anyhow::Context;
use reel_core::{Clip, FfmpegProbe, MediaProbe, Project, TrackKind};
use uuid::Uuid;

use crate::project_io::project_from_media_path;

const MAX_UNDO: usize = 48;

/// Owns the working [`Project`], last-saved snapshot, and undo/redo stacks.
#[derive(Debug, Clone, Default)]
pub struct EditSession {
    /// Primary preview path (first clip on the main video track when possible).
    pub current_media: Option<PathBuf>,
    project: Option<Project>,
    /// Snapshot from the last successful **Save**; used by Revert.
    saved_baseline: Option<Project>,
    undo: Vec<Project>,
    redo: Vec<Project>,
    pub dirty: bool,
}

impl EditSession {
    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    /// Path the player should decode (first clip on the first video track).
    pub fn playback_path(&self) -> Option<PathBuf> {
        self.project.as_ref().and_then(|p| {
            p.tracks
                .iter()
                .find(|t| t.kind == TrackKind::Video)
                .and_then(|t| t.clip_ids.first())
                .and_then(|id| p.clips.iter().find(|c| c.id == *id))
                .map(|c| c.source_path.clone())
        })
    }

    /// Load media: builds a one-clip project and clears history.
    pub fn open_media(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let p = project_from_media_path(&path)?;
        self.current_media = Some(path);
        self.project = Some(p);
        self.saved_baseline = None;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        Ok(())
    }

    pub fn clear_media(&mut self) {
        self.current_media = None;
        self.project = None;
        self.saved_baseline = None;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
    }

    pub fn has_media(&self) -> bool {
        self.project.is_some()
    }

    /// Append a new clip to the main video track (probed from disk).
    pub fn insert_clip(&mut self, path: PathBuf) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        self.push_undo_snapshot();

        let proj = self.project.as_mut().expect("project checked above");

        let probe = FfmpegProbe::new();
        let md = probe.probe(&path).context("probe insert")?;
        let id = Uuid::new_v4();
        let dur = md.duration_seconds;
        proj.clips.push(Clip {
            id,
            source_path: path,
            metadata: md,
            in_point: 0.0,
            out_point: dur,
            extensions: Default::default(),
        });

        let track = proj
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Video)
            .context("no video track")?;
        track.clip_ids.push(id);
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    fn push_undo_snapshot(&mut self) {
        if let Some(ref p) = self.project {
            if self.undo.len() >= MAX_UNDO {
                self.undo.remove(0);
            }
            self.undo.push(p.clone());
        }
    }

    fn recompute_dirty(&mut self) {
        self.dirty = match (&self.project, &self.saved_baseline) {
            (Some(cur), Some(base)) => cur != base,
            (Some(_), None) => true,
            _ => false,
        };
    }

    pub fn undo(&mut self) -> bool {
        let cur = match self.project.clone() {
            Some(p) => p,
            None => return false,
        };
        let prev = match self.undo.pop() {
            Some(p) => p,
            None => return false,
        };
        if self.redo.len() >= MAX_UNDO {
            self.redo.remove(0);
        }
        self.redo.push(cur);
        self.project = Some(prev);
        self.recompute_dirty();
        true
    }

    pub fn redo(&mut self) -> bool {
        let cur = match self.project.clone() {
            Some(p) => p,
            None => return false,
        };
        let next = match self.redo.pop() {
            Some(p) => p,
            None => return false,
        };
        if self.undo.len() >= MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push(cur);
        self.project = Some(next);
        self.recompute_dirty();
        true
    }

    /// Discard unsaved edits: restore last saved version, or a fresh single-clip project.
    pub fn revert_to_saved(&mut self) -> anyhow::Result<()> {
        if let Some(base) = self.saved_baseline.clone() {
            self.project = Some(base);
        } else if let Some(ref path) = self.current_media {
            self.project = Some(project_from_media_path(path)?);
        } else {
            anyhow::bail!("nothing to revert");
        }
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        Ok(())
    }

    pub fn mark_saved_to_path(&mut self, disk_path: PathBuf) {
        if let Some(ref mut p) = self.project {
            p.path = Some(disk_path);
        }
        self.saved_baseline = self.project.clone();
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
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

    fn tiny_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("reel-core")
            .join("tests")
            .join("fixtures")
            .join("tiny_h264_aac.mp4")
    }

    #[test]
    fn insert_undo_redo_roundtrip() {
        let f = tiny_fixture();
        if !f.is_file() {
            eprintln!("skip: fixture missing");
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f.clone()).expect("open");
        s.insert_clip(f.clone()).expect("insert");
        assert_eq!(s.project().unwrap().clips.len(), 2);
        assert!(s.dirty);
        assert!(s.undo());
        assert_eq!(s.project().unwrap().clips.len(), 1);
        assert!(s.redo());
        assert_eq!(s.project().unwrap().clips.len(), 2);
    }

    #[test]
    fn mark_saved_baseline_enables_revert_after_edit() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f.clone()).unwrap();
        s.mark_saved_to_path(PathBuf::from("/tmp/saved.reel"));
        assert!(!s.dirty);
        s.insert_clip(f.clone()).unwrap();
        assert!(s.revert_enabled());
        s.revert_to_saved().unwrap();
        assert_eq!(s.project().unwrap().clips.len(), 1);
        assert!(!s.dirty);
    }

    #[test]
    fn export_format_mapping() {
        assert_eq!(
            export_format_for_path(Path::new("x.webm")),
            Some(reel_core::WebExportFormat::WebmVp8Opus)
        );
    }
}
