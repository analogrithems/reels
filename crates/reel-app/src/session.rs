//! In-memory edit session: [`reel_core::Project`], save baseline, undo/redo snapshots.

use std::path::{Path, PathBuf};

use anyhow::Context;
use reel_core::{Clip, FfmpegProbe, MediaProbe, Project, TrackKind};
use uuid::Uuid;

use crate::project_io::project_from_media_path;

const MAX_UNDO: usize = 48;

/// Epsilon for sequence-ms boundaries (float noise + UI rounding).
const SEQ_MS_EPS: f64 = 1e-3;

/// How **Insert Video** should place the new clip on the main video track.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InsertPlan {
    /// Insert at `clip_ids` index `0..=len` (no split).
    AtIndex(usize),
    /// Split `clip_ids[clip_index]` at `local_ms` ms from the start of that clip’s timeline span.
    Split { clip_index: usize, local_ms: f64 },
}

/// Map playhead (concatenated sequence time, ms) to an insert or split plan.
///
/// Rules:
/// - At or before a clip’s left edge → insert before that clip.
/// - Strictly inside a clip’s span → split at playhead, insert between the two parts.
/// - Past the end → append.
pub(crate) fn insert_plan_for_playhead_ms(
    project: &Project,
    playhead_ms: f64,
) -> Option<InsertPlan> {
    let track = project.tracks.iter().find(|t| t.kind == TrackKind::Video)?;
    if track.clip_ids.is_empty() {
        return Some(InsertPlan::AtIndex(0));
    }
    let ph = playhead_ms.max(0.0);
    let mut t_ms = 0.0_f64;
    for (i, cid) in track.clip_ids.iter().enumerate() {
        let clip = project.clips.iter().find(|c| c.id == *cid)?;
        let dur_ms = (clip.out_point - clip.in_point) * 1000.0;
        if ph <= t_ms + SEQ_MS_EPS {
            return Some(InsertPlan::AtIndex(i));
        }
        if ph < t_ms + dur_ms - SEQ_MS_EPS {
            let local_ms = ph - t_ms;
            if local_ms > SEQ_MS_EPS && local_ms < dur_ms - SEQ_MS_EPS {
                return Some(InsertPlan::Split {
                    clip_index: i,
                    local_ms,
                });
            }
            return Some(InsertPlan::AtIndex(i + 1));
        }
        t_ms += dur_ms;
    }
    Some(InsertPlan::AtIndex(track.clip_ids.len()))
}

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

    /// Insert a new clip from disk at the timeline position indicated by `playhead_ms`
    /// (milliseconds on the concatenated sequence).
    pub fn insert_clip_at_playhead(
        &mut self,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        self.push_undo_snapshot();

        let proj = self.project.as_mut().expect("project checked above");

        let plan = insert_plan_for_playhead_ms(proj, playhead_ms).context("no video track")?;

        let probe = FfmpegProbe::new();
        let md = probe.probe(&path).context("probe insert")?;
        let new_id = Uuid::new_v4();
        let dur = md.duration_seconds;
        let new_clip = Clip {
            id: new_id,
            source_path: path,
            metadata: md,
            in_point: 0.0,
            out_point: dur,
            extensions: Default::default(),
        };

        let track = proj
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Video)
            .context("no video track")?;

        match plan {
            InsertPlan::AtIndex(insert_at) => {
                proj.clips.push(new_clip);
                let insert_at = insert_at.min(track.clip_ids.len());
                track.clip_ids.insert(insert_at, new_id);
            }
            InsertPlan::Split {
                clip_index,
                local_ms,
            } => {
                let old_id = *track.clip_ids.get(clip_index).context("split clip index")?;
                let old = proj
                    .clips
                    .iter()
                    .find(|c| c.id == old_id)
                    .context("split clip missing")?
                    .clone();

                let split_sec = old.in_point + (local_ms / 1000.0).max(0.0);
                debug_assert!(
                    split_sec > old.in_point && split_sec < old.out_point,
                    "InsertPlan::Split must target clip interior"
                );

                let left_id = Uuid::new_v4();
                let right_id = Uuid::new_v4();
                let left = Clip {
                    id: left_id,
                    source_path: old.source_path.clone(),
                    metadata: old.metadata.clone(),
                    in_point: old.in_point,
                    out_point: split_sec,
                    extensions: Default::default(),
                };
                let right = Clip {
                    id: right_id,
                    source_path: old.source_path.clone(),
                    metadata: old.metadata.clone(),
                    in_point: split_sec,
                    out_point: old.out_point,
                    extensions: Default::default(),
                };

                proj.clips.retain(|c| c.id != old_id);
                proj.clips.push(left);
                proj.clips.push(right);
                proj.clips.push(new_clip);

                track
                    .clip_ids
                    .splice(clip_index..=clip_index, [left_id, new_id, right_id]);
            }
        }

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
    use reel_core::{Clip, MediaMetadata, Project, Track, TrackKind};
    use uuid::Uuid;

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
        let tail_ms = {
            let p = s.project().unwrap();
            timeline_end_ms_for_tests(p).unwrap_or(0.0)
        };
        s.insert_clip_at_playhead(f.clone(), tail_ms)
            .expect("insert");
        assert_eq!(s.project().unwrap().clips.len(), 2);
        assert!(s.dirty);
        assert!(s.undo());
        assert_eq!(s.project().unwrap().clips.len(), 1);
        assert!(s.redo());
        assert_eq!(s.project().unwrap().clips.len(), 2);
    }

    #[test]
    fn insert_mid_clip_splits_and_inserts_between() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f.clone()).unwrap();
        // First clip spans full fixture duration; 500 ms is strictly inside for typical short clips.
        s.insert_clip_at_playhead(f.clone(), 500.0).unwrap();
        let p = s.project().unwrap();
        assert_eq!(p.clips.len(), 3);
        let track = p
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .unwrap();
        assert_eq!(track.clip_ids.len(), 3);
        let left = p.clips.iter().find(|c| c.id == track.clip_ids[0]).unwrap();
        let mid = p.clips.iter().find(|c| c.id == track.clip_ids[1]).unwrap();
        let right = p.clips.iter().find(|c| c.id == track.clip_ids[2]).unwrap();
        assert_eq!(left.source_path, f);
        assert_eq!(mid.source_path, f);
        assert_eq!(right.source_path, f);
        let split = left.out_point;
        assert!((left.in_point - 0.0).abs() < 1e-6);
        assert!((right.in_point - split).abs() < 1e-6);
        assert!((mid.in_point - 0.0).abs() < 1e-6);
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
        let tail_ms = timeline_end_ms_for_tests(s.project().unwrap()).unwrap_or(0.0);
        s.insert_clip_at_playhead(f.clone(), tail_ms).unwrap();
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

    fn clip_sec(id: Uuid, path: &str, sec: f64) -> Clip {
        Clip {
            id,
            source_path: PathBuf::from(path),
            metadata: MediaMetadata {
                path: PathBuf::from(path),
                duration_seconds: sec,
                container: "mp4".into(),
                video: None,
                audio: None,
                audio_disabled: false,
            },
            in_point: 0.0,
            out_point: sec,
            extensions: Default::default(),
        }
    }

    fn two_clip_project() -> Project {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let tid = Uuid::new_v4();
        let mut p = Project::new("t");
        p.clips.push(clip_sec(a, "/a.mp4", 2.0));
        p.clips.push(clip_sec(b, "/b.mp4", 3.0));
        p.tracks.push(Track {
            id: tid,
            kind: TrackKind::Video,
            clip_ids: vec![a, b],
            extensions: Default::default(),
        });
        p
    }

    #[test]
    fn insertion_at_playhead_zero_before_first() {
        let p = two_clip_project();
        assert_eq!(
            insert_plan_for_playhead_ms(&p, 0.0),
            Some(InsertPlan::AtIndex(0))
        );
    }

    #[test]
    fn insertion_inside_first_clip_splits() {
        let p = two_clip_project();
        // 500ms inside first clip (0–2000ms)
        assert_eq!(
            insert_plan_for_playhead_ms(&p, 500.0),
            Some(InsertPlan::Split {
                clip_index: 0,
                local_ms: 500.0,
            })
        );
    }

    #[test]
    fn insertion_past_end_appends() {
        let p = two_clip_project();
        assert_eq!(
            insert_plan_for_playhead_ms(&p, 999_999.0),
            Some(InsertPlan::AtIndex(2))
        );
    }

    #[test]
    fn timeline_end_sums_durations() {
        let p = two_clip_project();
        assert!((timeline_end_ms_for_tests(&p).unwrap() - 5000.0).abs() < 0.01);
    }

    fn timeline_end_ms_for_tests(project: &Project) -> Option<f64> {
        let track = project.tracks.iter().find(|t| t.kind == TrackKind::Video)?;
        let mut ms = 0.0_f64;
        for cid in &track.clip_ids {
            let clip = project.clips.iter().find(|c| c.id == *cid)?;
            ms += (clip.out_point - clip.in_point) * 1000.0;
        }
        Some(ms)
    }
}
