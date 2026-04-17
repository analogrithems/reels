//! In-memory edit session: [`reel_core::Project`], save baseline, undo/redo snapshots.

use std::path::{Path, PathBuf};

use anyhow::Context;
use reel_core::{Clip, FfmpegProbe, MediaProbe, Project, Track, TrackKind};
use uuid::Uuid;

use crate::project_io::{is_project_document_path, load_project_file, project_from_media_path};
use crate::timecode;

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

/// Map playhead (concatenated sequence time, ms) to an insert or split plan on `track`.
///
/// Rules:
/// - At or before a clip’s left edge → insert before that clip.
/// - Strictly inside a clip’s span → split at playhead, insert between the two parts.
/// - Past the end → append.
pub(crate) fn insert_plan_for_track_ms(
    project: &Project,
    track: &Track,
    playhead_ms: f64,
) -> Option<InsertPlan> {
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

/// Uses the **first** [`TrackKind::Video`] track (primary timeline). Sequence time matches the primary video concat.
pub(crate) fn insert_plan_for_playhead_ms(
    project: &Project,
    playhead_ms: f64,
) -> Option<InsertPlan> {
    let track = project.tracks.iter().find(|t| t.kind == TrackKind::Video)?;
    insert_plan_for_track_ms(project, track, playhead_ms)
}

fn primary_video_source_path(p: &Project) -> Option<PathBuf> {
    p.tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .and_then(|t| t.clip_ids.first())
        .and_then(|id| p.clips.iter().find(|c| c.id == *id))
        .map(|c| c.source_path.clone())
}

/// Insert plan on the **first** [`TrackKind::Audio`] track (same sequence clock as primary video).
pub(crate) fn insert_plan_for_first_audio_track_ms(
    project: &Project,
    playhead_ms: f64,
) -> Option<InsertPlan> {
    let track = project.tracks.iter().find(|t| t.kind == TrackKind::Audio)?;
    insert_plan_for_track_ms(project, track, playhead_ms)
}

/// True when **Split Clip at Playhead** can run (playhead strictly inside a primary-track clip).
pub(crate) fn split_enabled_for_playhead(project: &Project, playhead_ms: f64) -> bool {
    insert_plan_for_playhead_ms(project, playhead_ms)
        .map(|p| matches!(p, InsertPlan::Split { .. }))
        .unwrap_or(false)
}

fn video_track_row_lines(p: &Project) -> Vec<String> {
    let vtracks: Vec<&Track> = p
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .collect();
    vtracks
        .iter()
        .enumerate()
        .map(|(idx, t)| {
            let lane = if idx == 0 { "primary" } else { "secondary" };
            let n = t.clip_ids.len();
            let dur_ms: f64 = t
                .clip_ids
                .iter()
                .filter_map(|id| p.clips.iter().find(|c| c.id == *id))
                .map(|c| (c.out_point - c.in_point) * 1000.0)
                .sum();
            let clip_word = if n == 1 { "clip" } else { "clips" };
            format!(
                "V{} · {lane} · {n} {clip_word} · {}",
                idx + 1,
                timecode::format_ms_alone(dur_ms as f32)
            )
        })
        .collect()
}

fn audio_track_row_lines(p: &Project) -> Vec<String> {
    let atracks: Vec<&Track> = p
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .collect();
    atracks
        .iter()
        .enumerate()
        .map(|(idx, t)| {
            let n = t.clip_ids.len();
            let dur_ms: f64 = t
                .clip_ids
                .iter()
                .filter_map(|id| p.clips.iter().find(|c| c.id == *id))
                .map(|c| (c.out_point - c.in_point) * 1000.0)
                .sum();
            let clip_word = if n == 1 { "clip" } else { "clips" };
            format!(
                "A{} · audio · {n} {clip_word} · {}",
                idx + 1,
                timecode::format_ms_alone(dur_ms as f32)
            )
        })
        .collect()
}

pub(crate) fn video_lane_indices(project: &Project) -> Vec<usize> {
    project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == TrackKind::Video)
        .map(|(i, _)| i)
        .collect()
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

    /// First clip’s source path on the primary video track (same file the player opens at
    /// sequence time 0).
    #[allow(dead_code)]
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

    /// Track/clip counts and preview rule (used by crate tests; not shown in the main timeline UI).
    #[allow(dead_code)]
    pub fn timeline_summary_line(&self) -> String {
        let Some(p) = self.project.as_ref() else {
            return String::new();
        };
        let vtracks: Vec<&Track> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .collect();
        let n_a = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .count();
        let n_v = vtracks.len();
        let n_primary = vtracks.first().map(|t| t.clip_ids.len()).unwrap_or(0);
        if n_v == 0 {
            return "No video tracks".into();
        }
        let audio_from_lane = p
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Audio)
            .map(|t| !t.clip_ids.is_empty())
            .unwrap_or(false);
        let audio_note = if audio_from_lane {
            "audio from first audio track (concat)"
        } else {
            "audio embedded in primary video sources"
        };
        format!(
            "{n_v} video · {n_a} audio · {n_primary} clip(s) on primary · preview: primary video (concat); {audio_note}"
        )
    }

    /// One label per **video** track (primary first), for per-lane rows in the timeline UI.
    pub fn video_track_row_labels(&self) -> Vec<String> {
        self.project
            .as_ref()
            .map(video_track_row_lines)
            .unwrap_or_default()
    }

    /// One label per **audio** track (in project order), for per-lane rows in the timeline UI.
    pub fn audio_track_row_labels(&self) -> Vec<String> {
        self.project
            .as_ref()
            .map(audio_track_row_lines)
            .unwrap_or_default()
    }

    /// Append an empty **video** track (for multi-track projects). Undoable.
    pub fn add_video_track(&mut self) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        proj.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Video,
            clip_ids: Vec::new(),
            extensions: Default::default(),
        });
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Append an empty **audio** track. Undoable. Clips are not routed to preview yet (see timeline summary).
    pub fn add_audio_track(&mut self) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        proj.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Audio,
            clip_ids: Vec::new(),
            extensions: Default::default(),
        });
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Move the clip under `playhead_ms` on the **primary** video track to the **next** video
    /// track (second lane). Undoable.
    pub fn move_playhead_clip_to_next_video_track(
        &mut self,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        let idxs = self
            .project
            .as_ref()
            .map(video_lane_indices)
            .unwrap_or_default();
        if idxs.len() < 2 {
            anyhow::bail!("add a second video track first (File → New Video Track)");
        }
        let clip_id = {
            let p = self.project.as_ref().expect("checked");
            crate::timeline::primary_clip_id_at_seq_ms(p, playhead_ms)
                .context("playhead not on a clip — seek into the clip you want to move")?
        };
        let (primary_idx, below_idx) = (idxs[0], idxs[1]);
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        {
            let primary = proj
                .tracks
                .get_mut(primary_idx)
                .context("primary video track")?;
            let pos = primary
                .clip_ids
                .iter()
                .position(|id| *id == clip_id)
                .context("clip not on primary track")?;
            primary.clip_ids.remove(pos);
        }
        proj.tracks
            .get_mut(below_idx)
            .context("second video track")?
            .clip_ids
            .push(clip_id);
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Take the **first** clip on the **second** video track and **append** it to the end of the
    /// primary video track (inverse of moving down when the playhead targets the first clip on
    /// the lane below). Undoable.
    ///
    /// Secondary lanes are not in the preview timeline; this uses explicit lane order instead of
    /// playhead position on the lower track.
    pub fn move_first_clip_from_second_video_track_to_primary(&mut self) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        let idxs = self
            .project
            .as_ref()
            .map(video_lane_indices)
            .unwrap_or_default();
        if idxs.len() < 2 {
            anyhow::bail!("add a second video track first (File → New Video Track)");
        }
        let (primary_idx, below_idx) = (idxs[0], idxs[1]);
        let clip_id = {
            let below = self
                .project
                .as_ref()
                .expect("checked")
                .tracks
                .get(below_idx)
                .context("second video track")?;
            below
                .clip_ids
                .first()
                .copied()
                .context("no clips on the track below")?
        };
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        proj.tracks
            .get_mut(below_idx)
            .context("second video track")?
            .clip_ids
            .remove(0);
        proj.tracks
            .get_mut(primary_idx)
            .context("primary video track")?
            .clip_ids
            .push(clip_id);
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Split the primary-track clip under `playhead_ms` into two adjacent clips (same source file;
    /// in/out adjusted). Undoable. Fails if the playhead is in a gap or on a cut between clips.
    pub fn split_clip_at_playhead(&mut self, playhead_ms: f64) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        let plan = insert_plan_for_playhead_ms(self.project.as_ref().unwrap(), playhead_ms)
            .context("no video track")?;
        let InsertPlan::Split {
            clip_index,
            local_ms,
        } = plan
        else {
            anyhow::bail!("playhead must be strictly inside a clip — not in a gap or on a cut");
        };

        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");

        let track = proj
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Video)
            .context("no video track")?;

        let old_id = *track.clip_ids.get(clip_index).context("split clip index")?;
        let old = proj
            .clips
            .iter()
            .find(|c| c.id == old_id)
            .context("split clip missing")?
            .clone();

        let split_sec = old.in_point + (local_ms / 1000.0).max(0.0);
        if split_sec <= old.in_point + SEQ_MS_EPS || split_sec >= old.out_point - SEQ_MS_EPS {
            anyhow::bail!("playhead must be strictly inside a clip — not in a gap or on a cut");
        }

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

        track
            .clip_ids
            .splice(clip_index..=clip_index, [left_id, right_id]);

        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Load media or a saved **`.reel` / `.json` project**; clears undo/redo for a new media open,
    /// or establishes a save baseline when opening a project file.
    pub fn open_media(&mut self, path: PathBuf) -> anyhow::Result<()> {
        if is_project_document_path(&path) {
            let p = load_project_file(&path)?;
            self.current_media = primary_video_source_path(&p);
            self.project = Some(p);
            self.mark_saved_to_path(path);
            return Ok(());
        }
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

    /// Insert audio from disk on the **first** audio track at `playhead_ms` (primary video sequence time).
    /// Requires **File → New Audio Track** (or an existing audio lane) first.
    pub fn insert_audio_clip_at_playhead(
        &mut self,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        self.push_undo_snapshot();

        let proj = self.project.as_mut().expect("project checked above");

        let plan = insert_plan_for_first_audio_track_ms(proj, playhead_ms)
            .context("add an audio track first (File → New Audio Track)")?;

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
            .find(|t| t.kind == TrackKind::Audio)
            .context("no audio track")?;

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

    /// Write [`Project::path`] if the document is dirty and a path exists.
    ///
    /// Updates the save baseline and clears `dirty` **without** clearing undo/redo
    /// (unlike [`mark_saved_to_path`]).
    pub fn flush_autosave_if_needed(&mut self) -> anyhow::Result<bool> {
        if !self.dirty {
            return Ok(false);
        }
        let path = match self.project.as_ref().and_then(|p| p.path.as_ref()) {
            Some(p) => p.clone(),
            None => return Ok(false),
        };
        let proj = self.project.as_ref().unwrap();
        crate::project_io::save_project(&path, proj)?;
        self.saved_baseline = self.project.clone();
        self.dirty = false;
        Ok(true)
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

    #[cfg(test)]
    pub(crate) fn from_project_for_tests(p: Project) -> Self {
        Self {
            current_media: None,
            project: Some(p),
            saved_baseline: None,
            undo: vec![],
            redo: vec![],
            dirty: true,
        }
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

/// Preset row from **Export** dialog (`0` = MP4, `1` = WebM, `2` = MKV). See `docs/SUPPORTED_FORMATS.md`.
pub fn web_export_format_from_preset_index(index: i32) -> Option<reel_core::WebExportFormat> {
    match index {
        0 => Some(reel_core::WebExportFormat::Mp4Remux),
        1 => Some(reel_core::WebExportFormat::WebmVp8Opus),
        2 => Some(reel_core::WebExportFormat::MkvRemux),
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
    fn flush_autosave_writes_disk_and_keeps_undo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let reel = dir.path().join("doc.reel");
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f.clone()).unwrap();
        s.mark_saved_to_path(reel.clone());
        let tail_ms = timeline_end_ms_for_tests(s.project().unwrap()).unwrap_or(0.0);
        s.insert_clip_at_playhead(f.clone(), tail_ms).unwrap();
        assert!(s.undo_enabled());
        assert!(s.dirty);
        assert!(s.flush_autosave_if_needed().unwrap());
        assert!(!s.dirty);
        assert!(s.undo_enabled());
        assert!(reel.is_file());
    }

    #[test]
    fn export_format_mapping() {
        assert_eq!(
            export_format_for_path(Path::new("x.webm")),
            Some(reel_core::WebExportFormat::WebmVp8Opus)
        );
    }

    #[test]
    fn export_preset_index_maps_web_formats() {
        assert_eq!(
            web_export_format_from_preset_index(0),
            Some(reel_core::WebExportFormat::Mp4Remux),
        );
        assert_eq!(
            web_export_format_from_preset_index(1),
            Some(reel_core::WebExportFormat::WebmVp8Opus),
        );
        assert_eq!(
            web_export_format_from_preset_index(2),
            Some(reel_core::WebExportFormat::MkvRemux),
        );
        assert_eq!(web_export_format_from_preset_index(-1), None);
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
    fn split_enabled_matches_interior() {
        let p = two_clip_project();
        assert!(super::split_enabled_for_playhead(&p, 500.0));
        assert!(!super::split_enabled_for_playhead(&p, 0.0));
        assert!(!super::split_enabled_for_playhead(&p, 2000.0));
    }

    #[test]
    fn split_clip_at_playhead_three_clips_on_primary() {
        let mut s = EditSession::from_project_for_tests(two_clip_project());
        s.split_clip_at_playhead(500.0).unwrap();
        let p = s.project().unwrap();
        let track = p
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .unwrap();
        assert_eq!(track.clip_ids.len(), 3);
        assert_eq!(p.clips.len(), 3);
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

    #[test]
    fn move_playhead_clip_to_secondary_lane() {
        let mut p = two_clip_project();
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Video,
            clip_ids: vec![],
            extensions: Default::default(),
        });
        let mut s = EditSession::from_project_for_tests(p);
        let id_first = s.project().unwrap().clips[0].id;
        s.move_playhead_clip_to_next_video_track(500.0).unwrap();
        let p = s.project().unwrap();
        let vtracks: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .collect();
        assert_eq!(vtracks[0].clip_ids.len(), 1);
        assert_eq!(vtracks[1].clip_ids.len(), 1);
        assert_eq!(vtracks[1].clip_ids[0], id_first);
    }

    #[test]
    fn move_first_clip_from_second_track_roundtrip() {
        let mut p = two_clip_project();
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Video,
            clip_ids: vec![],
            extensions: Default::default(),
        });
        let mut s = EditSession::from_project_for_tests(p);
        s.move_playhead_clip_to_next_video_track(500.0).unwrap();
        s.move_first_clip_from_second_video_track_to_primary()
            .unwrap();
        let p = s.project().unwrap();
        let vtracks: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .collect();
        assert_eq!(vtracks[0].clip_ids.len(), 2);
        assert!(vtracks[1].clip_ids.is_empty());
    }

    #[test]
    fn video_track_row_lines_primary_two_clips() {
        let p = two_clip_project();
        let rows = video_track_row_lines(&p);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with("V1 · primary · 2 clips"));
        assert!(rows[0].contains("0:05.000"));
    }

    #[test]
    fn add_audio_track_appends_empty_lane() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f).unwrap();
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            0
        );
        s.add_audio_track().unwrap();
        let rows = s.audio_track_row_labels();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with("A1 · audio · 0 clips"));
        assert!(s.timeline_summary_line().contains("1 audio"));
        assert!(s.undo());
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            0
        );
    }

    #[test]
    fn add_video_track_appends_empty_lane() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f).unwrap();
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video)
                .count(),
            1
        );
        s.add_video_track().unwrap();
        let rows = s.video_track_row_labels();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("primary"));
        assert!(rows[1].contains("secondary") && rows[1].contains("0 clips"));
        let p = s.project().unwrap();
        let v: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .collect();
        assert_eq!(v.len(), 2);
        assert!(v[1].clip_ids.is_empty());
        assert!(s.timeline_summary_line().contains("2 video"));
        assert!(s.undo());
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video)
                .count(),
            1
        );
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
