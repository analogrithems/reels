//! In-memory edit session: [`reel_core::Project`], save baseline, undo/redo snapshots.

use std::path::{Path, PathBuf};

use anyhow::Context;
use reel_core::project::ClipOrientation;
use reel_core::{Clip, FfmpegProbe, MediaMetadata, MediaProbe, Project, Track, TrackKind};
use uuid::Uuid;

use crate::project_io::{is_project_document_path, load_project_file, project_from_media_path};
use crate::timecode;

const MAX_UNDO: usize = 48;

/// Epsilon for sequence-ms boundaries (float noise + UI rounding).
const SEQ_MS_EPS: f64 = 1e-3;

/// Smallest allowed clip span after a trim. Matches the threshold split/insert math already
/// uses implicitly via `SEQ_MS_EPS` — one low-FPS frame (~16 ms) is comfortable above it.
pub const MIN_TRIM_DURATION_S: f64 = 0.05;

/// Data the trim-clip sheet needs to prefill: the target clip's id, its current in/out points,
/// and the source file duration (0.0 when the probe didn't report one — the sheet then skips
/// the upper-bound check and relies on ffmpeg to clamp during seek).
#[derive(Debug, Clone, Copy)]
pub struct TrimCandidate {
    pub clip_id: Uuid,
    pub current_in_s: f64,
    pub current_out_s: f64,
    pub source_duration_s: f64,
}

/// Data the **Resize Video…** sheet needs to prefill: the target clip's id,
/// the current scale percent (100 = identity), and the source pixel dimensions
/// (width × height). Zero dims mean the probe didn't report them, so the sheet
/// shows `—` instead of an estimated output size.
#[derive(Debug, Clone, Copy)]
pub struct ResizeCandidate {
    pub clip_id: Uuid,
    pub current_percent: u32,
    pub source_width: u32,
    pub source_height: u32,
}

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

/// Insert plan on the **first** [`TrackKind::Subtitle`] track (same sequence clock as primary video).
pub(crate) fn insert_plan_for_first_subtitle_track_ms(
    project: &Project,
    playhead_ms: f64,
) -> Option<InsertPlan> {
    let track = project
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Subtitle)?;
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
            // Unity gain is the overwhelming default — we keep the label stable
            // for that case so existing tests / UI snapshots don't drift, and only
            // append a dB suffix when the lane has actually been boosted/cut. The
            // sign is preserved explicitly so users can tell "+3 dB" from "3 dB"
            // at a glance.
            let gain_suffix = if t.gain_db != 0.0 {
                format!(" · {:+.1} dB", t.gain_db)
            } else {
                String::new()
            };
            format!(
                "A{} · audio · {n} {clip_word} · {}{gain_suffix}",
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

pub(crate) fn audio_lane_indices(project: &Project) -> Vec<usize> {
    project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == TrackKind::Audio)
        .map(|(i, _)| i)
        .collect()
}

pub(crate) fn subtitle_lane_indices(project: &Project) -> Vec<usize> {
    project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == TrackKind::Subtitle)
        .map(|(i, _)| i)
        .collect()
}

fn remove_track_at(proj: &mut Project, track_index: usize) -> anyhow::Result<()> {
    if track_index >= proj.tracks.len() {
        anyhow::bail!("invalid track index");
    }
    let removed = proj.tracks.remove(track_index);
    for clip_id in removed.clip_ids {
        let still_used = proj.tracks.iter().any(|t| t.clip_ids.contains(&clip_id));
        if !still_used {
            proj.clips.retain(|c| c.id != clip_id);
        }
    }
    proj.touch();
    Ok(())
}

/// Owns the working [`Project`], last-saved snapshot, and undo/redo stacks.
#[derive(Debug, Clone, Default)]
pub struct EditSession {
    /// Primary preview path (first clip on the main video track when possible).
    pub current_media: Option<PathBuf>,
    /// `true` when the user opened a saved **`.reel` / `.json`** project; `false` for a probed media file.
    /// Timeline UI uses this to show container **video / audio / subtitle** stream lanes for single media.
    opened_from_project_document: bool,
    project: Option<Project>,
    /// Snapshot from the last successful **Save**; used by Revert.
    saved_baseline: Option<Project>,
    undo: Vec<Project>,
    redo: Vec<Project>,
    pub dirty: bool,
    /// Seek-bar **In** marker (sequence ms). Ephemeral per session — **not** persisted in
    /// the project JSON. `None` means "no in marker set"; when both markers are set we
    /// guarantee `in_marker_ms < out_marker_ms` via [`set_in_marker_ms`] / [`set_out_marker_ms`].
    in_marker_ms: Option<f64>,
    /// Seek-bar **Out** marker (sequence ms). See [`in_marker_ms`] for semantics.
    out_marker_ms: Option<f64>,
}

impl EditSession {
    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    /// When `false`, the session was built from a **single media** open — show stream-based lanes in the timeline strip.
    pub fn opened_from_project_document(&self) -> bool {
        self.opened_from_project_document
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
            gain_db: 0.0,
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
            gain_db: 0.0,
            extensions: Default::default(),
        });
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Sensible slider range for **U2-e** per-lane gain. Anything outside this
    /// window is clamped on entry in [`EditSession::set_audio_track_gain_db`].
    /// Picked by ear / industry standard — a ±40 dB range is wider than any
    /// realistic mixing move (you'd mute before attenuating past -40 dB, and
    /// boosting past +40 dB pins any source to clipping), so clamping here
    /// turns off-by-magnitude-of-10 bugs into silent no-ops instead of ear-
    /// rupturing surprises.
    pub const AUDIO_GAIN_DB_MIN: f32 = -40.0;
    /// See [`EditSession::AUDIO_GAIN_DB_MIN`].
    pub const AUDIO_GAIN_DB_MAX: f32 = 40.0;

    /// Set the per-lane `gain_db` on the **`lane`-th** [`TrackKind::Audio`]
    /// track (0-based index among audio tracks only, matching the order used
    /// by [`EditSession::audio_track_row_labels`] and the lane-label UI).
    ///
    /// `db` is clamped into `[AUDIO_GAIN_DB_MIN, AUDIO_GAIN_DB_MAX]` before
    /// write, and NaN falls back to `0.0` (unity). Sub-`0.01` dB deltas are
    /// inaudible and still write — the caller can dedupe if they care about
    /// undo noise from slider jitter.
    ///
    /// Fails without mutation (and without pushing undo) when no project is
    /// loaded or the audio-lane index is out of range. **Undoable**: one
    /// snapshot per successful change.
    pub fn set_audio_track_gain_db(&mut self, lane: usize, db: f32) -> anyhow::Result<()> {
        let p = self
            .project
            .as_ref()
            .context("no project — open a file first")?;
        // Resolve the audio-lane index into the full track list so we can
        // borrow-check cleanly when we grab `&mut` below.
        let track_vec_idx = p
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == TrackKind::Audio)
            .nth(lane)
            .map(|(i, _)| i)
            .with_context(|| format!("no audio lane at index {lane}"))?;

        let new_gain = if db.is_finite() {
            db.clamp(Self::AUDIO_GAIN_DB_MIN, Self::AUDIO_GAIN_DB_MAX)
        } else {
            0.0
        };

        // Don't push undo when the write is a no-op — avoids polluting the
        // undo stack when a slider emits a redundant value on release.
        let current = p.tracks[track_vec_idx].gain_db;
        if current == new_gain {
            return Ok(());
        }

        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked above");
        proj.tracks[track_vec_idx].gain_db = new_gain;
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Read the current per-lane gain in dB for the `lane`-th audio track, or
    /// `None` when no project is loaded or the index is out of range. Used by
    /// the UI to seed the slider / badge when the project loads.
    pub fn audio_track_gain_db(&self, lane: usize) -> Option<f32> {
        let p = self.project.as_ref()?;
        p.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .nth(lane)
            .map(|t| t.gain_db)
    }

    /// Collect `gain_db` from every [`TrackKind::Audio`] track, in project
    /// order. Mirrors the lane order produced by
    /// [`crate::timeline::clips_from_all_audio_tracks`], so the returned vec
    /// is the correct argument for
    /// [`reel_core::media::export::export_concat_with_audio_lanes_oriented_with_gains`].
    /// Returns an empty vec when no project is loaded — the export dispatcher
    /// treats empty gains as "all unity", which is the right default.
    pub fn audio_track_gains_db(&self) -> Vec<f32> {
        let Some(p) = self.project.as_ref() else {
            return Vec::new();
        };
        p.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .map(|t| t.gain_db)
            .collect()
    }

    /// Append an empty **subtitle** track (timed-text lane). Undoable.
    pub fn add_subtitle_track(&mut self) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        proj.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Subtitle,
            clip_ids: Vec::new(),
            gain_db: 0.0,
            extensions: Default::default(),
        });
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Remove the **lane**-th video track (`0` = V1 / primary). Undoable.
    ///
    /// The last remaining video track cannot be removed.
    pub fn remove_video_track_lane(&mut self, lane: usize) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        let proj = self.project.as_ref().expect("checked");
        let vi = video_lane_indices(proj);
        if vi.len() <= 1 {
            anyhow::bail!("cannot remove the only video track");
        }
        let track_index = *vi.get(lane).context("no such video track")?;
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        remove_track_at(proj, track_index)?;
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Remove the **lane**-th audio track (`0` = A1). Undoable.
    pub fn remove_audio_track_lane(&mut self, lane: usize) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        let proj = self.project.as_ref().expect("checked");
        let ai = audio_lane_indices(proj);
        let track_index = *ai.get(lane).context("no such audio track")?;
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        remove_track_at(proj, track_index)?;
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Remove the **lane**-th subtitle track (`0` = S1). Undoable.
    pub fn remove_subtitle_track_lane(&mut self, lane: usize) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        let proj = self.project.as_ref().expect("checked");
        let si = subtitle_lane_indices(proj);
        let track_index = *si.get(lane).context("no such subtitle track")?;
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        remove_track_at(proj, track_index)?;
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
            orientation: old.orientation,
            scale: old.scale,
            audio_mute: old.audio_mute,
            audio_stream_index: old.audio_stream_index,
            extensions: Default::default(),
        };
        let right = Clip {
            id: right_id,
            source_path: old.source_path.clone(),
            metadata: old.metadata.clone(),
            in_point: split_sec,
            out_point: old.out_point,
            orientation: old.orientation,
            scale: old.scale,
            audio_mute: old.audio_mute,
            audio_stream_index: old.audio_stream_index,
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

    /// Mutate the primary-track clip under `playhead_ms` with `mutate(&mut orientation)` and push an
    /// undo snapshot. Fails (without pushing undo) when there is no project or the playhead isn't on
    /// a primary-track clip.
    fn mutate_playhead_clip_orientation(
        &mut self,
        playhead_ms: f64,
        mutate: impl FnOnce(&mut ClipOrientation),
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        let clip_id = {
            let p = self.project.as_ref().expect("checked");
            let seq = crate::timeline::sequence_ms_for_primary_clip_lookup(p, playhead_ms);
            crate::timeline::primary_clip_id_at_seq_ms(p, seq)
                .context("playhead not on a clip — seek into the clip you want to rotate / flip")?
        };
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        let clip = proj
            .clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .context("clip missing from project")?;
        mutate(&mut clip.orientation);
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Rotate the clip under `playhead_ms` by 90° clockwise. Undoable.
    pub fn rotate_playhead_clip_right(&mut self, playhead_ms: f64) -> anyhow::Result<()> {
        self.mutate_playhead_clip_orientation(playhead_ms, |o| o.rotate_right())
    }

    /// Rotate the clip under `playhead_ms` by 90° counter-clockwise. Undoable.
    pub fn rotate_playhead_clip_left(&mut self, playhead_ms: f64) -> anyhow::Result<()> {
        self.mutate_playhead_clip_orientation(playhead_ms, |o| o.rotate_left())
    }

    /// Toggle horizontal flip on the clip under `playhead_ms`. Undoable.
    pub fn flip_playhead_clip_horizontal(&mut self, playhead_ms: f64) -> anyhow::Result<()> {
        self.mutate_playhead_clip_orientation(playhead_ms, |o| o.toggle_flip_h())
    }

    /// Toggle vertical flip on the clip under `playhead_ms`. Undoable.
    pub fn flip_playhead_clip_vertical(&mut self, playhead_ms: f64) -> anyhow::Result<()> {
        self.mutate_playhead_clip_orientation(playhead_ms, |o| o.toggle_flip_v())
    }

    /// True when rotate/flip can run at `playhead_ms` (primary-track clip under the playhead).
    ///
    /// Does not require the decoder to be ready — orientation edits are project state.
    pub fn rotate_enabled(&self, playhead_ms: f64) -> bool {
        let Some(p) = self.project.as_ref() else {
            return false;
        };
        let seq = crate::timeline::sequence_ms_for_primary_clip_lookup(p, playhead_ms);
        crate::timeline::primary_clip_id_at_seq_ms(p, seq).is_some()
    }

    /// Snapshot needed to populate the trim sheet for the clip at `seq_ms`.
    /// `None` when no project is loaded or the playhead isn't on a primary-track clip.
    pub fn trim_candidate_at_seq_ms(&self, seq_ms: f64) -> Option<TrimCandidate> {
        let p = self.project.as_ref()?;
        let id = crate::timeline::primary_clip_id_at_seq_ms(p, seq_ms)?;
        let c = p.clips.iter().find(|c| c.id == id)?;
        Some(TrimCandidate {
            clip_id: id,
            current_in_s: c.in_point,
            current_out_s: c.out_point,
            source_duration_s: c.metadata.duration_seconds,
        })
    }

    /// True when [`trim_candidate_at_seq_ms`] would return `Some` — used to gate the
    /// **Edit → Trim Clip…** menu item.
    pub fn trim_enabled(&self, playhead_ms: f64) -> bool {
        self.trim_candidate_at_seq_ms(playhead_ms).is_some()
    }

    /// Set `clip_id.in_point` and `out_point` to the given source-file seconds. Undoable.
    ///
    /// Validates:
    /// - clip exists in the project
    /// - `0 <= new_in_s < new_out_s`
    /// - `new_out_s <= source_duration_s`
    /// - resulting duration `>= MIN_TRIM_DURATION_S` (50 ms — guards against zero-length clips
    ///   and the epsilon-rejection edge case in split / timeline math).
    ///
    /// No undo snapshot is pushed when validation fails, matching [`rotate_playhead_clip_right`]'s
    /// "failed op doesn't pollute undo" policy.
    pub fn trim_clip(
        &mut self,
        clip_id: Uuid,
        new_in_s: f64,
        new_out_s: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        if !new_in_s.is_finite() || !new_out_s.is_finite() {
            anyhow::bail!("trim values must be finite");
        }
        if new_in_s < 0.0 {
            anyhow::bail!("trim begin must be >= 0");
        }
        if new_in_s >= new_out_s {
            anyhow::bail!("trim begin must be < trim end");
        }
        if new_out_s - new_in_s < MIN_TRIM_DURATION_S {
            anyhow::bail!("clip duration must be >= {:.3} s", MIN_TRIM_DURATION_S);
        }
        let source_duration_s = {
            let p = self.project.as_ref().expect("checked");
            let c = p
                .clips
                .iter()
                .find(|c| c.id == clip_id)
                .context("clip missing from project")?;
            c.metadata.duration_seconds
        };
        // Source duration of `0` means probe didn't report it — treat as "unknown", skip upper bound.
        if source_duration_s > 0.0 && new_out_s > source_duration_s + SEQ_MS_EPS {
            anyhow::bail!(
                "trim end ({new_out_s:.3}) exceeds source duration ({source_duration_s:.3})"
            );
        }

        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        let clip = proj
            .clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .expect("clip existed a moment ago");
        clip.in_point = new_in_s;
        clip.out_point = new_out_s;
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// U2-c per-clip edge-drag trim. `edge` is `0` (left / in-point) or `1`
    /// (right / out-point). `delta_ratio` is the drag distance as a fraction of
    /// the chip's current duration (what the Slint handle emits), so:
    ///
    /// - left edge: `new_in  = in  + delta_ratio * (out - in)` (`out` unchanged)
    /// - right edge: `new_out = out + delta_ratio * (out - in)` (`in` unchanged)
    ///
    /// Delegates to [`EditSession::trim_clip`] for invariant checking — so
    /// rejecting trim past the source bounds, sub-50-ms duration, or a flipped
    /// in/out doesn't pollute undo. Ripple is automatic: the project has no
    /// absolute timeline positions, so shortening a clip pulls downstream
    /// clips forward by the same delta without any extra bookkeeping here.
    pub fn trim_clip_by_edge_drag(
        &mut self,
        clip_id: Uuid,
        edge: u8,
        delta_ratio: f64,
    ) -> anyhow::Result<()> {
        if !delta_ratio.is_finite() {
            anyhow::bail!("delta_ratio must be finite");
        }
        let (cur_in, cur_out) = {
            let p = self
                .project
                .as_ref()
                .context("no project — open a file first")?;
            let c = p
                .clips
                .iter()
                .find(|c| c.id == clip_id)
                .context("clip missing from project")?;
            (c.in_point, c.out_point)
        };
        let dur = cur_out - cur_in;
        let delta_s = delta_ratio * dur;
        let (new_in, new_out) = match edge {
            0 => (cur_in + delta_s, cur_out),
            1 => (cur_in, cur_out + delta_s),
            _ => anyhow::bail!("edge must be 0 (left) or 1 (right)"),
        };
        self.trim_clip(clip_id, new_in, new_out)
    }

    /// Snapshot needed to populate the **Resize Video…** sheet for the clip at `seq_ms`.
    /// `None` when no project is loaded or the playhead isn't on a primary-track clip.
    pub fn resize_candidate_at_seq_ms(&self, seq_ms: f64) -> Option<ResizeCandidate> {
        let p = self.project.as_ref()?;
        let id = crate::timeline::primary_clip_id_at_seq_ms(p, seq_ms)?;
        let c = p.clips.iter().find(|c| c.id == id)?;
        let (w, h) = c
            .metadata
            .video
            .as_ref()
            .map(|v| (v.width, v.height))
            .unwrap_or((0, 0));
        Some(ResizeCandidate {
            clip_id: id,
            current_percent: c.scale.display_percent(),
            source_width: w,
            source_height: h,
        })
    }

    /// True when [`resize_candidate_at_seq_ms`] would return `Some` — used to gate the
    /// **Edit → Resize Video…** menu item. Same policy as rotate/trim: project state only,
    /// so decode readiness doesn't have to be wired through.
    pub fn resize_enabled(&self, playhead_ms: f64) -> bool {
        self.resize_candidate_at_seq_ms(playhead_ms).is_some()
    }

    /// Set `clip_id.scale` to the given percent (clamped in [`ClipScale::set_percent`]).
    /// Undoable. Fails without pushing undo when no project is loaded or the clip is missing.
    ///
    /// `100` (identity) is accepted and restores the stream-copy fast path on export.
    pub fn resize_clip(&mut self, clip_id: Uuid, percent: u32) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }
        {
            let p = self.project.as_ref().expect("checked");
            p.clips
                .iter()
                .find(|c| c.id == clip_id)
                .context("clip missing from project")?;
        }
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        let clip = proj
            .clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .expect("clip existed a moment ago");
        let mut new_scale = clip.scale;
        new_scale.set_percent(percent);
        clip.scale = new_scale;
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Current `audio_mute` state of the primary-track clip at `seq_ms`, if any.
    /// Returns `None` when no project is loaded or no clip sits under the playhead —
    /// the caller uses this to gate/uncheck the **Edit → Mute Clip Audio** menu item.
    pub fn audio_mute_state_at_seq_ms(&self, seq_ms: f64) -> Option<bool> {
        let p = self.project.as_ref()?;
        let id = crate::timeline::primary_clip_id_at_seq_ms(p, seq_ms)?;
        p.clips.iter().find(|c| c.id == id).map(|c| c.audio_mute)
    }

    /// Convenience gate for the **Edit → Mute Clip Audio** menu item.
    pub fn audio_mute_enabled(&self, playhead_ms: f64) -> bool {
        self.audio_mute_state_at_seq_ms(playhead_ms).is_some()
    }

    /// Toggle the `audio_mute` flag on the primary-track clip at `seq_ms`. Undoable.
    /// Fails without pushing undo when no project is loaded or no clip sits under
    /// the playhead.
    pub fn toggle_audio_mute_at_seq_ms(&mut self, seq_ms: f64) -> anyhow::Result<()> {
        let id = {
            let p = self
                .project
                .as_ref()
                .context("no project — open a file first")?;
            crate::timeline::primary_clip_id_at_seq_ms(p, seq_ms)
                .context("no clip at playhead to mute")?
        };
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        let clip = proj
            .clips
            .iter_mut()
            .find(|c| c.id == id)
            .expect("clip existed a moment ago");
        clip.audio_mute = !clip.audio_mute;
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Set the per-clip audio stream selection (Edit → Audio Track) for the
    /// primary-track clip at `seq_ms`. `Some(i)` picks container stream `i`;
    /// `None` resets to "first decodable" (matches legacy player behavior).
    /// Undoable. No-ops when the value matches today's (keeps undo tidy for
    /// rapid menu clicks that end on the current selection).
    pub fn set_audio_stream_index_at_seq_ms(
        &mut self,
        seq_ms: f64,
        stream_index: Option<u32>,
    ) -> anyhow::Result<()> {
        let id = {
            let p = self
                .project
                .as_ref()
                .context("no project — open a file first")?;
            crate::timeline::primary_clip_id_at_seq_ms(p, seq_ms)
                .context("no clip at playhead to change audio track on")?
        };
        let current = self
            .project
            .as_ref()
            .and_then(|p| p.clips.iter().find(|c| c.id == id))
            .and_then(|c| c.audio_stream_index);
        if current == stream_index {
            return Ok(());
        }
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("checked");
        let clip = proj
            .clips
            .iter_mut()
            .find(|c| c.id == id)
            .expect("clip existed a moment ago");
        clip.audio_stream_index = stream_index;
        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Currently-selected audio stream index on the primary-track clip at
    /// `seq_ms`. `None` when the clip uses the default ("first decodable") —
    /// the menu shows that as "Default (stream 0)" so users aren't surprised
    /// by a phantom selected row on files with one audio track.
    pub fn audio_stream_index_at_seq_ms(&self, seq_ms: f64) -> Option<u32> {
        let p = self.project.as_ref()?;
        let id = crate::timeline::primary_clip_id_at_seq_ms(p, seq_ms)?;
        p.clips
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.audio_stream_index)
    }

    /// Snapshot of the audio streams on the source file backing the primary-
    /// track clip at `seq_ms`. Returns an empty vec when there's no clip or
    /// the file pre-dates the multi-stream probe path (`audio_streams` empty).
    /// The Audio Track menu is gated on `len() >= 2` — for single-stream
    /// files the picker has nothing to choose between.
    pub fn audio_streams_at_seq_ms(&self, seq_ms: f64) -> Vec<reel_core::AudioStreamInfo> {
        let Some(p) = self.project.as_ref() else {
            return Vec::new();
        };
        let Some(id) = crate::timeline::primary_clip_id_at_seq_ms(p, seq_ms) else {
            return Vec::new();
        };
        p.clips
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.metadata.audio_streams.clone())
            .unwrap_or_default()
    }

    /// True when every clip on the primary video track has `audio_mute = true`.
    /// Used by the export pipeline to decide whether to pass `-an` for the
    /// single-track embedded-audio case.
    pub fn all_primary_clips_audio_muted(&self) -> bool {
        let Some(p) = self.project.as_ref() else {
            return false;
        };
        let Some(track) = p.tracks.iter().find(|t| t.kind == TrackKind::Video) else {
            return false;
        };
        if track.clip_ids.is_empty() {
            return false;
        }
        track.clip_ids.iter().all(|id| {
            p.clips
                .iter()
                .find(|c| c.id == *id)
                .map(|c| c.audio_mute)
                .unwrap_or(false)
        })
    }

    /// True when at least one primary-track clip has `audio_mute = true` and at
    /// least one does not — the mixed case that would need silence substitution.
    pub fn any_primary_clip_audio_muted(&self) -> bool {
        let Some(p) = self.project.as_ref() else {
            return false;
        };
        let Some(track) = p.tracks.iter().find(|t| t.kind == TrackKind::Video) else {
            return false;
        };
        track.clip_ids.iter().any(|id| {
            p.clips
                .iter()
                .find(|c| c.id == *id)
                .map(|c| c.audio_mute)
                .unwrap_or(false)
        })
    }

    /// Current seek-bar **In** marker in sequence ms, or `None` when unset.
    pub fn in_marker_ms(&self) -> Option<f64> {
        self.in_marker_ms
    }

    /// Current seek-bar **Out** marker in sequence ms, or `None` when unset.
    pub fn out_marker_ms(&self) -> Option<f64> {
        self.out_marker_ms
    }

    /// Set the **In** marker at the given playhead position (ms). Rejects non-finite or
    /// negative values. If an **Out** marker is already set and the new in is `>= out`,
    /// the existing out is **cleared** (user is re-anchoring the range).
    ///
    /// Markers are ephemeral session state — **not** undoable and **not** persisted.
    pub fn set_in_marker_ms(&mut self, seq_ms: f64) -> anyhow::Result<()> {
        if !seq_ms.is_finite() || seq_ms < 0.0 {
            anyhow::bail!("in marker must be >= 0");
        }
        self.in_marker_ms = Some(seq_ms);
        if let Some(out) = self.out_marker_ms {
            if seq_ms + SEQ_MS_EPS >= out {
                self.out_marker_ms = None;
            }
        }
        Ok(())
    }

    /// Set the **Out** marker at the given playhead position (ms). Rejects non-finite or
    /// negative values. If an **In** marker is already set and the new out is `<= in`,
    /// the existing in is **cleared** (user is re-anchoring the range).
    pub fn set_out_marker_ms(&mut self, seq_ms: f64) -> anyhow::Result<()> {
        if !seq_ms.is_finite() || seq_ms < 0.0 {
            anyhow::bail!("out marker must be >= 0");
        }
        self.out_marker_ms = Some(seq_ms);
        if let Some(in_ms) = self.in_marker_ms {
            if seq_ms <= in_ms + SEQ_MS_EPS {
                self.in_marker_ms = None;
            }
        }
        Ok(())
    }

    /// Clears both markers. No-op when neither is set.
    pub fn clear_markers(&mut self) {
        self.in_marker_ms = None;
        self.out_marker_ms = None;
    }

    /// True when at least one marker is set — used to gate **Edit → Clear Range Markers**.
    pub fn has_any_marker(&self) -> bool {
        self.in_marker_ms.is_some() || self.out_marker_ms.is_some()
    }

    /// Returns `(in_ms, out_ms)` when **both** markers are set and the in/out ordering
    /// is valid — the caller should slice export / operations to this range.
    /// `None` when either marker is unset or the ordering is invalid.
    pub fn marker_range_ms(&self) -> Option<(f64, f64)> {
        match (self.in_marker_ms, self.out_marker_ms) {
            (Some(i), Some(o)) if o > i + SEQ_MS_EPS => Some((i, o)),
            _ => None,
        }
    }

    /// Load media or a saved **`.reel` / `.json` project**; clears undo/redo for a new media open,
    /// or establishes a save baseline when opening a project file. Uses the real ffmpeg probe.
    pub fn open_media(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let probe = FfmpegProbe::new();
        self.open_media_with_probe(&probe, path)
    }

    /// Open-media variant that takes an injected `&dyn MediaProbe`. Used by UI
    /// tests that want to exercise the Open flow without ffmpeg. Project-file
    /// opens (`.reel` / `.json`) skip the probe entirely and ignore the
    /// injected dependency, matching [`Self::open_media`] behavior.
    pub fn open_media_with_probe(
        &mut self,
        probe: &dyn MediaProbe,
        path: PathBuf,
    ) -> anyhow::Result<()> {
        if is_project_document_path(&path) {
            let p = load_project_file(&path)?;
            self.current_media = primary_video_source_path(&p);
            self.project = Some(p);
            self.opened_from_project_document = true;
            self.mark_saved_to_path(path);
            self.clear_markers();
            return Ok(());
        }
        let p = crate::project_io::project_from_media_path_with_probe(probe, &path)?;
        self.current_media = Some(path);
        self.opened_from_project_document = false;
        self.project = Some(p);
        self.saved_baseline = None;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.clear_markers();
        Ok(())
    }

    pub fn clear_media(&mut self) {
        self.current_media = None;
        self.opened_from_project_document = false;
        self.project = None;
        self.saved_baseline = None;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.clear_markers();
    }

    pub fn has_media(&self) -> bool {
        self.project.is_some()
    }

    /// Insert a new clip from disk at the timeline position indicated by `playhead_ms`
    /// (milliseconds on the concatenated sequence). Uses the real ffmpeg probe.
    pub fn insert_clip_at_playhead(
        &mut self,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        let probe = FfmpegProbe::new();
        self.insert_clip_at_playhead_with_probe(&probe, path, playhead_ms)
    }

    /// Insert variant that takes an injected `&dyn MediaProbe`. Tests pass a
    /// fake probe here so the edit flow exercises its split / append logic
    /// without spinning up ffmpeg. See `docs/phases-ui-test.md` Phase 1b.
    pub fn insert_clip_at_playhead_with_probe(
        &mut self,
        probe: &dyn MediaProbe,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        self.push_undo_snapshot();

        let proj = self.project.as_mut().expect("project checked above");

        let plan = insert_plan_for_playhead_ms(proj, playhead_ms).context("no video track")?;

        let md = probe.probe(&path).context("probe insert")?;
        let new_id = Uuid::new_v4();
        let dur = md.duration_seconds;
        let new_clip = Clip {
            id: new_id,
            source_path: path,
            metadata: md,
            in_point: 0.0,
            out_point: dur,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            audio_stream_index: None,
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
                    orientation: old.orientation,
                    scale: old.scale,
                    audio_mute: old.audio_mute,
                    audio_stream_index: old.audio_stream_index,
                    extensions: Default::default(),
                };
                let right = Clip {
                    id: right_id,
                    source_path: old.source_path.clone(),
                    metadata: old.metadata.clone(),
                    in_point: split_sec,
                    out_point: old.out_point,
                    orientation: old.orientation,
                    scale: old.scale,
                    audio_mute: old.audio_mute,
                    audio_stream_index: old.audio_stream_index,
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
    /// Requires **File → New Audio Track** (or an existing audio lane) first. Uses the real ffmpeg probe.
    pub fn insert_audio_clip_at_playhead(
        &mut self,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        let probe = FfmpegProbe::new();
        self.insert_audio_clip_at_playhead_with_probe(&probe, path, playhead_ms)
    }

    /// Audio-insert variant that takes an injected `&dyn MediaProbe`. See
    /// [`EditSession::insert_clip_at_playhead_with_probe`] for the rationale.
    pub fn insert_audio_clip_at_playhead_with_probe(
        &mut self,
        probe: &dyn MediaProbe,
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

        let md = probe.probe(&path).context("probe insert")?;
        let new_id = Uuid::new_v4();
        let dur = md.duration_seconds;
        let new_clip = Clip {
            id: new_id,
            source_path: path,
            metadata: md,
            in_point: 0.0,
            out_point: dur,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            audio_stream_index: None,
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
                    orientation: old.orientation,
                    scale: old.scale,
                    audio_mute: old.audio_mute,
                    audio_stream_index: old.audio_stream_index,
                    extensions: Default::default(),
                };
                let right = Clip {
                    id: right_id,
                    source_path: old.source_path.clone(),
                    metadata: old.metadata.clone(),
                    in_point: split_sec,
                    out_point: old.out_point,
                    orientation: old.orientation,
                    scale: old.scale,
                    audio_mute: old.audio_mute,
                    audio_stream_index: old.audio_stream_index,
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

    /// **Edit → Overlay Audio…**: append a fresh [`TrackKind::Audio`] lane
    /// and insert `path` at sequence time 0 on that new lane.
    ///
    /// Unlike **File → Insert Audio…** (which targets the *first* existing
    /// audio lane), overlay always creates a *new* lane — every invocation
    /// stacks another parallel audio source that the export `amix` path mixes
    /// alongside the others. The U2-b-export dispatcher already handles
    /// N≥2 lanes via `-filter_complex amix`, so overlay clips mix into the
    /// export without further plumbing.
    ///
    /// One undo snapshot covers both the lane creation and the clip insert —
    /// `Edit → Undo` removes the whole overlay in a single step.
    ///
    /// **Preview-side** mix is still first-lane-only until the audio-thread
    /// rewrite lands, so added overlays are audible only after export. The
    /// UI copy in the menu makes that caveat explicit.
    pub fn insert_overlay_audio_clip_at_playhead(
        &mut self,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        let probe = FfmpegProbe::new();
        self.insert_overlay_audio_clip_at_playhead_with_probe(&probe, path, playhead_ms)
    }

    /// Probe-injected variant of
    /// [`EditSession::insert_overlay_audio_clip_at_playhead`]. See the
    /// non-probe version's doc comment for semantics; this one exists so
    /// tests can inject a [`crate::MockMediaProbe`] without hitting ffmpeg.
    pub fn insert_overlay_audio_clip_at_playhead_with_probe(
        &mut self,
        probe: &dyn MediaProbe,
        path: PathBuf,
        _playhead_ms: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        // Probe first so a bad file fails *before* we mutate the project or
        // push an undo snapshot — matches the other insert helpers.
        let md = probe.probe(&path).context("probe overlay audio")?;
        let dur = md.duration_seconds;

        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("project checked above");

        let new_track_id = Uuid::new_v4();
        let new_clip_id = Uuid::new_v4();
        let new_clip = Clip {
            id: new_clip_id,
            source_path: path,
            metadata: md,
            in_point: 0.0,
            out_point: dur,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            audio_stream_index: None,
            extensions: Default::default(),
        };
        proj.clips.push(new_clip);
        proj.tracks.push(Track {
            id: new_track_id,
            kind: TrackKind::Audio,
            clip_ids: vec![new_clip_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });

        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// **Edit → Replace Audio…**: atomically mute every primary-track clip
    /// **and** append a fresh [`TrackKind::Audio`] lane containing `path`.
    ///
    /// This is the "voice-over" companion to **Overlay Audio…**: overlay
    /// *adds* a new source alongside the existing video's own audio; replace
    /// *silences* the primary track and substitutes the new clip. Conceptually
    /// equivalent to running **Mute Clip Audio** on every primary clip in
    /// sequence and then **Overlay Audio…**, but collapsed into a single undo
    /// step so **Edit → Undo** unwinds the entire substitution in one click.
    ///
    /// ### Why one big snapshot instead of chaining the helpers
    ///
    /// A naïve `self.toggle_audio_mute_at_seq_ms(..)` loop followed by
    /// `self.insert_overlay_audio_clip_at_playhead_with_probe(..)` would push
    /// **N + 1** undo snapshots — the user would have to tap Undo N+1 times to
    /// get back to the pre-replace state, which violates the "one atomic edit"
    /// contract this helper promises. We also probe *before* pushing undo so a
    /// bad audio file fails cleanly (no half-applied mute state, no wasted
    /// undo slot).
    ///
    /// ### Semantics
    ///
    /// * All clips on the **primary video track** have `audio_mute` set to
    ///   `true`. Clips that were already muted stay muted (idempotent).
    /// * A **new** `TrackKind::Audio` lane is appended with a single clip
    ///   spanning the probed duration — same shape as
    ///   [`EditSession::insert_overlay_audio_clip_at_playhead_with_probe`].
    /// * Secondary audio lanes are untouched — use
    ///   [`EditSession::replace_audio_clip_at_playhead_clear_others`] if you
    ///   want the swap to drop existing audio lanes too.
    /// * Preview-side mix sums all `TrackKind::Audio` lanes with per-lane
    ///   gain (see `player::AudioLane`), so the replacement clip is audible
    ///   immediately alongside any existing overlays.
    pub fn replace_audio_clip_at_playhead(
        &mut self,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        let probe = FfmpegProbe::new();
        self.replace_audio_clip_at_playhead_with_probe(&probe, path, playhead_ms)
    }

    /// **Edit → Replace & Clear Other Audio…**: like
    /// [`EditSession::replace_audio_clip_at_playhead`] but first **removes every
    /// existing `TrackKind::Audio` lane** so the project is left with exactly
    /// one audio source — the new clip. Every operation lives under a single
    /// undo snapshot so one **Edit → Undo** restores the pre-call state
    /// (existing audio lanes and their clips, all original mute states)
    /// exactly.
    ///
    /// Use this when the intent is "swap the soundtrack" rather than
    /// "voice-over on top of what's there". `Replace Audio…` is the stacking
    /// variant and is usually the safer default; this one is destructive
    /// toward existing audio lanes.
    pub fn replace_audio_clip_at_playhead_clear_others(
        &mut self,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        let probe = FfmpegProbe::new();
        self.replace_audio_clip_at_playhead_clear_others_with_probe(&probe, path, playhead_ms)
    }

    /// Probe-injected variant of
    /// [`EditSession::replace_audio_clip_at_playhead_clear_others`]. See the
    /// non-probe version for semantics.
    pub fn replace_audio_clip_at_playhead_clear_others_with_probe(
        &mut self,
        probe: &dyn MediaProbe,
        path: PathBuf,
        _playhead_ms: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        let md = probe.probe(&path).context("probe replace audio")?;
        let dur = md.duration_seconds;

        // One snapshot covers the mute pass, every lane removal, and the
        // replacement-lane append — so one Undo unwinds the whole
        // "swap soundtrack" atomically.
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("project checked above");

        let primary_clip_ids: Vec<Uuid> = proj
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .map(|t| t.clip_ids.clone())
            .unwrap_or_default();
        for clip in proj.clips.iter_mut() {
            if primary_clip_ids.contains(&clip.id) {
                clip.audio_mute = true;
            }
        }

        // Collect audio-lane indices descending so `remove_track_at` doesn't
        // invalidate later ones as it mutates `proj.tracks`. `remove_track_at`
        // also drops any orphaned clips, matching `remove_audio_track_lane`.
        let audio_indices: Vec<usize> = proj
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == TrackKind::Audio)
            .map(|(i, _)| i)
            .rev()
            .collect();
        for idx in audio_indices {
            remove_track_at(proj, idx)?;
        }

        let new_track_id = Uuid::new_v4();
        let new_clip_id = Uuid::new_v4();
        let new_clip = Clip {
            id: new_clip_id,
            source_path: path,
            metadata: md,
            in_point: 0.0,
            out_point: dur,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            audio_stream_index: None,
            extensions: Default::default(),
        };
        proj.clips.push(new_clip);
        proj.tracks.push(Track {
            id: new_track_id,
            kind: TrackKind::Audio,
            clip_ids: vec![new_clip_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });

        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Probe-injected variant of
    /// [`EditSession::replace_audio_clip_at_playhead`]. See the non-probe
    /// version's doc comment for semantics; this one exists so tests can inject
    /// a [`crate::MockMediaProbe`] without hitting ffmpeg.
    pub fn replace_audio_clip_at_playhead_with_probe(
        &mut self,
        probe: &dyn MediaProbe,
        path: PathBuf,
        _playhead_ms: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        // Probe first so a bad file fails *before* we mutate the project or
        // push an undo snapshot — matches the other insert helpers.
        let md = probe.probe(&path).context("probe replace audio")?;
        let dur = md.duration_seconds;

        // Single snapshot covers BOTH the mute pass and the lane append, so
        // one Undo unwinds the whole replace atomically.
        self.push_undo_snapshot();
        let proj = self.project.as_mut().expect("project checked above");

        // Mute every clip on the primary video track. We collect IDs first
        // because the borrow on `proj.tracks` would otherwise conflict with
        // the mutable iteration through `proj.clips`.
        let primary_clip_ids: Vec<Uuid> = proj
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .map(|t| t.clip_ids.clone())
            .unwrap_or_default();
        for clip in proj.clips.iter_mut() {
            if primary_clip_ids.contains(&clip.id) {
                clip.audio_mute = true;
            }
        }

        // Append the replacement lane — same shape as Overlay Audio….
        let new_track_id = Uuid::new_v4();
        let new_clip_id = Uuid::new_v4();
        let new_clip = Clip {
            id: new_clip_id,
            source_path: path,
            metadata: md,
            in_point: 0.0,
            out_point: dur,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            audio_stream_index: None,
            extensions: Default::default(),
        };
        proj.clips.push(new_clip);
        proj.tracks.push(Track {
            id: new_track_id,
            kind: TrackKind::Audio,
            clip_ids: vec![new_clip_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });

        proj.touch();
        self.redo.clear();
        self.recompute_dirty();
        Ok(())
    }

    /// Insert a **subtitle clip** (SubRip `.srt`, WebVTT `.vtt`, or TTML
    /// `.ttml` / `.dfxp` / `.xml`) at the playhead on the first
    /// [`TrackKind::Subtitle`] lane. The lane must exist first
    /// (**File → New Subtitle Track**).
    ///
    /// Duration comes from [`reel_core::probe_subtitle_file`] — the max cue
    /// end time across whichever parser the extension routes to. Burn-in on
    /// export is handled by [`crate::session::subtitle_burn_in_path`], which
    /// delegates to ffmpeg's `subtitles=` filter. SRT and WebVTT burn-in via
    /// libass; TTML burn-in depends on the local ffmpeg build recognising
    /// TTML through the same filter (libass ≥ 0.14 handles basic TTML via
    /// an internal converter — files with heavy styling may fall back to an
    /// error at export time, in which case the preview overlay still works).
    pub fn insert_subtitle_clip_at_playhead(
        &mut self,
        path: PathBuf,
        playhead_ms: f64,
    ) -> anyhow::Result<()> {
        if self.project.is_none() {
            anyhow::bail!("no project — open a file first");
        }

        let probe = reel_core::probe_subtitle_file(&path)
            .with_context(|| format!("read subtitle file {}", path.display()))?;
        if probe.cue_count == 0 {
            anyhow::bail!("subtitle file has no cues: {}", path.display());
        }

        self.push_undo_snapshot();

        let proj = self.project.as_mut().expect("project checked above");

        let plan = insert_plan_for_first_subtitle_track_ms(proj, playhead_ms)
            .context("add a subtitle track first (File → New Subtitle Track)")?;

        let container = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_else(|| "srt".into());
        let md = MediaMetadata {
            path: path.clone(),
            duration_seconds: probe.duration_seconds,
            container,
            video: None,
            audio: None,
            audio_disabled: false,
            video_stream_count: 0,
            audio_stream_count: 0,
            subtitle_stream_count: 1,
            audio_streams: Vec::new(),
        };
        let new_id = Uuid::new_v4();
        let new_clip = Clip {
            id: new_id,
            source_path: path,
            metadata: md,
            in_point: 0.0,
            out_point: probe.duration_seconds,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            audio_stream_index: None,
            extensions: Default::default(),
        };

        let track = proj
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Subtitle)
            .context("no subtitle track")?;

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
                let left_id = Uuid::new_v4();
                let right_id = Uuid::new_v4();
                let left = Clip {
                    id: left_id,
                    source_path: old.source_path.clone(),
                    metadata: old.metadata.clone(),
                    in_point: old.in_point,
                    out_point: split_sec,
                    orientation: old.orientation,
                    scale: old.scale,
                    audio_mute: old.audio_mute,
                    audio_stream_index: old.audio_stream_index,
                    extensions: Default::default(),
                };
                let right = Clip {
                    id: right_id,
                    source_path: old.source_path.clone(),
                    metadata: old.metadata.clone(),
                    in_point: split_sec,
                    out_point: old.out_point,
                    orientation: old.orientation,
                    scale: old.scale,
                    audio_mute: old.audio_mute,
                    audio_stream_index: old.audio_stream_index,
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

    /// Absolute path to the **first subtitle clip** on the first
    /// [`TrackKind::Subtitle`] lane, if any — used by export to decide whether
    /// to burn captions into the video with ffmpeg's `subtitles=` filter.
    ///
    /// Returns `None` when there is no subtitle track, the track is empty, or
    /// the referenced clip can't be canonicalised.
    pub fn primary_subtitle_path(&self) -> Option<PathBuf> {
        let proj = self.project.as_ref()?;
        let track = proj.tracks.iter().find(|t| t.kind == TrackKind::Subtitle)?;
        let first_id = *track.clip_ids.first()?;
        let clip = proj.clips.iter().find(|c| c.id == first_id)?;
        Some(clip.source_path.clone())
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
            opened_from_project_document: true,
            project: Some(p),
            saved_baseline: None,
            undo: vec![],
            redo: vec![],
            dirty: true,
            in_marker_ms: None,
            out_marker_ms: None,
        }
    }
}

/// True when `path`'s lowercase extension matches the container of `fmt`. Accepts `.m4v`
/// alongside `.mp4` for every MP4 preset (remux, H.264/AAC, HEVC/AAC).
pub fn path_matches_export_format(path: &Path, fmt: reel_core::WebExportFormat) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    match fmt {
        reel_core::WebExportFormat::Mp4Remux
        | reel_core::WebExportFormat::Mp4H264Aac
        | reel_core::WebExportFormat::Mp4H265Aac => ext == "mp4" || ext == "m4v",
        reel_core::WebExportFormat::WebmVp8Opus
        | reel_core::WebExportFormat::WebmVp9Opus
        | reel_core::WebExportFormat::WebmAv1Opus => ext == "webm",
        reel_core::WebExportFormat::MkvRemux | reel_core::WebExportFormat::MkvDnxhrHq => {
            ext == "mkv"
        }
        reel_core::WebExportFormat::MovRemux | reel_core::WebExportFormat::MovProResHq => {
            ext == "mov"
        }
        reel_core::WebExportFormat::GifSharp
        | reel_core::WebExportFormat::GifGood
        | reel_core::WebExportFormat::GifShare
        | reel_core::WebExportFormat::GifTiny => ext == "gif",
    }
}

/// For the **remux** presets (`Mp4Remux`, `MkvRemux`, `MovRemux`) return a
/// short hint the status line appends to the ffmpeg failure message, pointing
/// the user at a transcode preset that doesn't require codec/container
/// compatibility.
///
/// Non-remux presets already transcode, so their failures are not recoverable
/// by switching presets — returns `None` there and the status stays terse.
pub fn remux_failure_hint(fmt: reel_core::WebExportFormat) -> Option<&'static str> {
    match fmt {
        reel_core::WebExportFormat::Mp4Remux => Some(
            "Remux requires H.264/HEVC + AAC in an MP4 container. \
             Try the MP4 — H.264 + AAC or MP4 — HEVC + AAC preset.",
        ),
        reel_core::WebExportFormat::MkvRemux => Some(
            "Remux copies the source streams unchanged. \
             If the source codecs are incompatible, try a WebM preset \
             (VP9 + Opus is a good default).",
        ),
        reel_core::WebExportFormat::MovRemux => Some(
            "MOV remux accepts H.264/HEVC + AAC/PCM. \
             For incompatible sources, transcode via MP4 — H.264 + AAC \
             then rename, or use an MP4 preset directly.",
        ),
        _ => None,
    }
}

/// Preset row from **Export** dialog.
///
/// Order groups by container then codec (MP4 → WebM → MKV → MOV → intermediates → GIF):
/// `0` MP4 remux, `1` MP4 H.264+AAC, `2` MP4 H.265+AAC, `3` WebM VP8+Opus,
/// `4` WebM VP9+Opus, `5` WebM AV1+Opus, `6` MKV remux, `7` MOV remux,
/// `8` MOV ProRes 422 HQ + PCM (pro intermediate), `9` MKV DNxHR HQ + PCM
/// (Avid-style intermediate), `10` GIF Sharp, `11` GIF Good, `12` GIF Share,
/// `13` GIF Tiny (animated GIF — no audio). See `docs/SUPPORTED_FORMATS.md`.
pub fn web_export_format_from_preset_index(index: i32) -> Option<reel_core::WebExportFormat> {
    match index {
        0 => Some(reel_core::WebExportFormat::Mp4Remux),
        1 => Some(reel_core::WebExportFormat::Mp4H264Aac),
        2 => Some(reel_core::WebExportFormat::Mp4H265Aac),
        3 => Some(reel_core::WebExportFormat::WebmVp8Opus),
        4 => Some(reel_core::WebExportFormat::WebmVp9Opus),
        5 => Some(reel_core::WebExportFormat::WebmAv1Opus),
        6 => Some(reel_core::WebExportFormat::MkvRemux),
        7 => Some(reel_core::WebExportFormat::MovRemux),
        8 => Some(reel_core::WebExportFormat::MovProResHq),
        9 => Some(reel_core::WebExportFormat::MkvDnxhrHq),
        10 => Some(reel_core::WebExportFormat::GifSharp),
        11 => Some(reel_core::WebExportFormat::GifGood),
        12 => Some(reel_core::WebExportFormat::GifShare),
        13 => Some(reel_core::WebExportFormat::GifTiny),
        _ => None,
    }
}

/// Test-only helpers exposed to sibling `#[cfg(test)]` modules in this crate
/// (notably `main::ui_smoke_tests`). Kept in a separate module so `FakeProbe`
/// is reachable from any `crate::session::tests_fake_probe::FakeProbe` path.
#[cfg(test)]
pub(crate) mod tests_fake_probe {
    use super::*;
    use reel_core::{MediaMetadata, VideoStreamInfo};
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Fake probe for Phase 1b UI/unit tests: returns canned [`MediaMetadata`]
    /// for any path, so Open / Insert flows exercise the edit model without
    /// ffmpeg on disk. See `docs/phases-ui-test.md` Phase 1b.
    ///
    /// Tests that want a specific duration (e.g. to land a split at a known
    /// ms boundary) should use [`FakeProbe::with_duration`].
    pub(crate) struct FakeProbe {
        duration_seconds: f64,
        /// Paths passed to `probe`, in call order — tests assert on this to
        /// confirm the seam is actually invoked from the expected call site.
        calls: Mutex<Vec<PathBuf>>,
    }

    impl FakeProbe {
        pub(crate) fn with_duration(duration_seconds: f64) -> Self {
            Self {
                duration_seconds,
                calls: Mutex::new(Vec::new()),
            }
        }
        pub(crate) fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl MediaProbe for FakeProbe {
        fn probe(
            &self,
            path: &std::path::Path,
        ) -> Result<MediaMetadata, reel_core::error::ProbeError> {
            self.calls.lock().unwrap().push(path.to_path_buf());
            Ok(MediaMetadata {
                path: path.to_path_buf(),
                duration_seconds: self.duration_seconds,
                container: "fake".into(),
                video: Some(VideoStreamInfo {
                    codec: "h264".into(),
                    width: 16,
                    height: 16,
                    frame_rate: 24.0,
                    pixel_format: "YUV420P".into(),
                    rotation: 0,
                }),
                audio: None,
                audio_disabled: false,
                video_stream_count: 1,
                audio_stream_count: 0,
                subtitle_stream_count: 0,
                audio_streams: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_fake_probe::FakeProbe;
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

    /// Exercises the Phase 1b seam: open + insert at the tail of the
    /// timeline without touching ffmpeg. Guards against regressions where a
    /// future refactor hard-codes `FfmpegProbe::new()` back into the flow —
    /// that would make this test panic inside `FakeProbe` call counting.
    #[test]
    fn open_and_insert_via_fake_probe_no_ffmpeg() {
        let probe = FakeProbe::with_duration(4.0);
        let mut s = EditSession::default();
        let fake = PathBuf::from("/tmp/fake-video-1.mp4");
        s.open_media_with_probe(&probe, fake.clone())
            .expect("open with fake probe");
        let p = s.project().expect("project after open");
        assert_eq!(p.clips.len(), 1);
        assert_eq!(p.clips[0].out_point, 4.0, "clip out-point = fake duration");

        // Insert a second clip at the tail.
        let tail_ms = timeline_end_ms_for_tests(p).unwrap_or(0.0);
        let second = PathBuf::from("/tmp/fake-video-2.mp4");
        s.insert_clip_at_playhead_with_probe(&probe, second.clone(), tail_ms)
            .expect("insert with fake probe");

        let p = s.project().unwrap();
        assert_eq!(p.clips.len(), 2);
        let track = p
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .expect("video track");
        assert_eq!(track.clip_ids.len(), 2);
        assert_eq!(p.clips[0].source_path, fake);
        assert_eq!(p.clips[1].source_path, second);
        // Both flows must call through the injected probe — 1 for open, 1 for insert.
        assert_eq!(probe.call_count(), 2);

        // Undo / redo still intact once we've bypassed ffmpeg.
        assert!(s.undo());
        assert_eq!(s.project().unwrap().clips.len(), 1);
        assert!(s.redo());
        assert_eq!(s.project().unwrap().clips.len(), 2);
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
    fn rotate_playhead_clip_right_pushes_undo_and_marks_dirty() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f.clone()).unwrap();
        s.mark_saved_to_path(PathBuf::from("/tmp/rot.reel"));
        assert!(!s.dirty);
        s.rotate_playhead_clip_right(0.0).expect("rotate right");
        assert_eq!(
            s.project().unwrap().clips[0]
                .orientation
                .rotation_quarter_turns,
            1
        );
        assert!(s.dirty);
        assert!(s.undo_enabled());
        s.undo();
        assert_eq!(
            s.project().unwrap().clips[0]
                .orientation
                .rotation_quarter_turns,
            0
        );
        assert!(s.redo_enabled());
    }

    #[test]
    fn flip_playhead_clip_toggles_independently() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f.clone()).unwrap();
        s.flip_playhead_clip_horizontal(0.0).unwrap();
        assert!(s.project().unwrap().clips[0].orientation.flip_h);
        assert!(!s.project().unwrap().clips[0].orientation.flip_v);
        s.flip_playhead_clip_vertical(0.0).unwrap();
        assert!(s.project().unwrap().clips[0].orientation.flip_h);
        assert!(s.project().unwrap().clips[0].orientation.flip_v);
        s.flip_playhead_clip_horizontal(0.0).unwrap();
        assert!(!s.project().unwrap().clips[0].orientation.flip_h);
        assert!(s.project().unwrap().clips[0].orientation.flip_v);
    }

    #[test]
    fn rotate_without_project_errors() {
        let mut s = EditSession::default();
        assert!(s.rotate_playhead_clip_right(0.0).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn rotate_in_gap_errors_without_pushing_undo() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f.clone()).unwrap();
        // Past the end of the tiny fixture's single clip.
        let far = 60.0 * 60.0 * 1000.0;
        assert!(s.rotate_playhead_clip_right(far).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn set_audio_stream_index_is_undoable_and_noops_when_unchanged() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f.clone()).unwrap();
        s.mark_saved_to_path(PathBuf::from("/tmp/aud.reel"));
        assert!(!s.dirty);
        assert_eq!(s.audio_stream_index_at_seq_ms(0.0), None);

        // First selection: marks dirty, pushes undo.
        s.set_audio_stream_index_at_seq_ms(0.0, Some(1))
            .expect("set stream 1");
        assert_eq!(s.audio_stream_index_at_seq_ms(0.0), Some(1));
        assert!(s.dirty);
        assert!(s.undo_enabled());

        // Re-selecting the same value is a no-op: no new undo frame,
        // dirty state unchanged from before the call.
        let undo_depth_before = s.undo.len();
        s.set_audio_stream_index_at_seq_ms(0.0, Some(1)).unwrap();
        assert_eq!(s.undo.len(), undo_depth_before);
        assert_eq!(s.audio_stream_index_at_seq_ms(0.0), Some(1));

        // Undo returns to default ("first decodable").
        assert!(s.undo());
        assert_eq!(s.audio_stream_index_at_seq_ms(0.0), None);
        assert!(s.redo_enabled());
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
    fn remux_presets_get_transcode_hint_non_remux_dont() {
        use reel_core::WebExportFormat as F;
        // Remux presets fail when source codecs don't match the container,
        // so their status should steer the user to a transcode alternative.
        assert!(remux_failure_hint(F::Mp4Remux)
            .map(|s| s.contains("H.264"))
            .unwrap_or(false));
        assert!(remux_failure_hint(F::MkvRemux)
            .map(|s| s.contains("WebM"))
            .unwrap_or(false));
        // Transcode presets already re-encode, so ffmpeg errors there aren't
        // a preset-choice problem — no hint.
        assert!(remux_failure_hint(F::Mp4H264Aac).is_none());
        assert!(remux_failure_hint(F::Mp4H265Aac).is_none());
        assert!(remux_failure_hint(F::WebmVp8Opus).is_none());
        assert!(remux_failure_hint(F::WebmVp9Opus).is_none());
        assert!(remux_failure_hint(F::WebmAv1Opus).is_none());
        // MOV remux is a fourth remux preset — it also gets a pointer to a
        // transcode alternative when ffmpeg refuses the source streams.
        assert!(remux_failure_hint(F::MovRemux)
            .map(|s| s.contains("MP4"))
            .unwrap_or(false));
    }

    #[test]
    fn path_matches_export_format_by_extension() {
        use reel_core::WebExportFormat as F;
        assert!(path_matches_export_format(Path::new("x.mp4"), F::Mp4Remux));
        assert!(path_matches_export_format(Path::new("x.m4v"), F::Mp4Remux));
        assert!(path_matches_export_format(
            Path::new("x.mp4"),
            F::Mp4H264Aac
        ));
        assert!(path_matches_export_format(
            Path::new("x.m4v"),
            F::Mp4H264Aac
        ));
        assert!(path_matches_export_format(
            Path::new("x.webm"),
            F::WebmVp8Opus
        ));
        assert!(path_matches_export_format(Path::new("x.mkv"), F::MkvRemux));
        assert!(path_matches_export_format(Path::new("x.mov"), F::MovRemux));
        assert!(!path_matches_export_format(
            Path::new("x.webm"),
            F::Mp4Remux
        ));
        assert!(!path_matches_export_format(Path::new("x.mp4"), F::MkvRemux));
        // MOV is **not** an MP4 alias for the remux path — container magic
        // differs (ftyp vs moov), and forcing a .mov into a .mp4 preset
        // would mislabel the output on disk.
        assert!(!path_matches_export_format(Path::new("x.mov"), F::Mp4Remux));
        assert!(!path_matches_export_format(
            Path::new("no_ext"),
            F::Mp4Remux
        ));
        // Animated GIF presets all accept only `.gif`. Guard against a
        // future preset getting added and accidentally inheriting some
        // other container's arm via a missed `|` pattern.
        for fmt in [F::GifSharp, F::GifGood, F::GifShare, F::GifTiny] {
            assert!(path_matches_export_format(Path::new("x.gif"), fmt));
            assert!(path_matches_export_format(Path::new("X.GIF"), fmt));
            assert!(!path_matches_export_format(Path::new("x.mp4"), fmt));
            assert!(!path_matches_export_format(Path::new("x.webm"), fmt));
        }
    }

    #[test]
    fn export_preset_index_maps_web_formats() {
        assert_eq!(
            web_export_format_from_preset_index(0),
            Some(reel_core::WebExportFormat::Mp4Remux),
        );
        assert_eq!(
            web_export_format_from_preset_index(1),
            Some(reel_core::WebExportFormat::Mp4H264Aac),
        );
        assert_eq!(
            web_export_format_from_preset_index(2),
            Some(reel_core::WebExportFormat::Mp4H265Aac),
        );
        assert_eq!(
            web_export_format_from_preset_index(3),
            Some(reel_core::WebExportFormat::WebmVp8Opus),
        );
        assert_eq!(
            web_export_format_from_preset_index(4),
            Some(reel_core::WebExportFormat::WebmVp9Opus),
        );
        assert_eq!(
            web_export_format_from_preset_index(5),
            Some(reel_core::WebExportFormat::WebmAv1Opus),
        );
        assert_eq!(
            web_export_format_from_preset_index(6),
            Some(reel_core::WebExportFormat::MkvRemux),
        );
        assert_eq!(
            web_export_format_from_preset_index(7),
            Some(reel_core::WebExportFormat::MovRemux),
        );
        assert_eq!(
            web_export_format_from_preset_index(8),
            Some(reel_core::WebExportFormat::MovProResHq),
        );
        assert_eq!(
            web_export_format_from_preset_index(9),
            Some(reel_core::WebExportFormat::MkvDnxhrHq),
        );
        // Animated GIF presets (no audio — see `is_gif()`).
        assert_eq!(
            web_export_format_from_preset_index(10),
            Some(reel_core::WebExportFormat::GifSharp),
        );
        assert_eq!(
            web_export_format_from_preset_index(11),
            Some(reel_core::WebExportFormat::GifGood),
        );
        assert_eq!(
            web_export_format_from_preset_index(12),
            Some(reel_core::WebExportFormat::GifShare),
        );
        assert_eq!(
            web_export_format_from_preset_index(13),
            Some(reel_core::WebExportFormat::GifTiny),
        );
        assert_eq!(web_export_format_from_preset_index(-1), None);
        assert_eq!(web_export_format_from_preset_index(14), None);
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
                video_stream_count: 0,
                audio_stream_count: 0,
                subtitle_stream_count: 0,
                audio_streams: Vec::new(),
            },
            in_point: 0.0,
            out_point: sec,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            audio_stream_index: None,
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
            gain_db: 0.0,
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
            gain_db: 0.0,
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
            gain_db: 0.0,
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
        assert!(rows[0].contains("0:05.0"));
    }

    #[test]
    fn overlay_audio_creates_new_lane_and_inserts_clip_in_one_undo_step() {
        let probe = FakeProbe::with_duration(1.25);
        let mut s = EditSession::default();
        s.open_media_with_probe(&probe, PathBuf::from("/tmp/fake-video-ov.mp4"))
            .unwrap();
        // Baseline: opened media ⇒ 1 video lane, 0 audio lanes.
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            0
        );
        let overlay_path = PathBuf::from("/tmp/fake-overlay.wav");
        s.insert_overlay_audio_clip_at_playhead_with_probe(&probe, overlay_path.clone(), 500.0)
            .expect("overlay insert");

        let p = s.project().unwrap();
        let a: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .collect();
        assert_eq!(a.len(), 1, "overlay appends a fresh audio lane");
        assert_eq!(a[0].clip_ids.len(), 1, "new lane holds the overlay clip");
        let clip_id = a[0].clip_ids[0];
        let clip = p.clips.iter().find(|c| c.id == clip_id).expect("clip");
        assert_eq!(clip.source_path, overlay_path);
        assert!((clip.out_point - 1.25).abs() < 1e-9, "duration from probe");

        // Undo must reverse BOTH the lane creation and the clip insert in a
        // single step so "Overlay → Undo" feels atomic.
        assert!(s.undo(), "undoable");
        let p = s.project().unwrap();
        assert_eq!(
            p.tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            0,
            "undo removed the lane"
        );
        assert!(
            !p.clips.iter().any(|c| c.source_path == overlay_path),
            "undo removed the overlay clip"
        );
    }

    #[test]
    fn overlay_audio_stacks_on_new_lane_each_invocation() {
        // Each overlay call appends its OWN lane — multiple overlays produce
        // N separate lanes so the export amix mixes them in parallel.
        let probe = FakeProbe::with_duration(2.0);
        let mut s = EditSession::default();
        s.open_media_with_probe(&probe, PathBuf::from("/tmp/fake-video-ov2.mp4"))
            .unwrap();

        s.insert_overlay_audio_clip_at_playhead_with_probe(
            &probe,
            PathBuf::from("/tmp/ov-a.wav"),
            0.0,
        )
        .unwrap();
        s.insert_overlay_audio_clip_at_playhead_with_probe(
            &probe,
            PathBuf::from("/tmp/ov-b.wav"),
            0.0,
        )
        .unwrap();

        let p = s.project().unwrap();
        let a: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .collect();
        assert_eq!(a.len(), 2, "two overlays ⇒ two separate lanes");
        assert_eq!(a[0].clip_ids.len(), 1);
        assert_eq!(a[1].clip_ids.len(), 1);
        let lane_paths: Vec<_> = a
            .iter()
            .map(|t| {
                let cid = t.clip_ids[0];
                p.clips
                    .iter()
                    .find(|c| c.id == cid)
                    .unwrap()
                    .source_path
                    .clone()
            })
            .collect();
        assert!(lane_paths.contains(&PathBuf::from("/tmp/ov-a.wav")));
        assert!(lane_paths.contains(&PathBuf::from("/tmp/ov-b.wav")));
    }

    #[test]
    fn overlay_audio_without_project_errors_without_mutation() {
        let probe = FakeProbe::with_duration(1.0);
        let mut s = EditSession::default();
        // No project open — must refuse cleanly, not panic or mutate.
        let err = s
            .insert_overlay_audio_clip_at_playhead_with_probe(
                &probe,
                PathBuf::from("/tmp/ov.wav"),
                0.0,
            )
            .expect_err("no-project path must error");
        assert!(err.to_string().contains("no project"));
        assert!(s.project().is_none());
    }

    #[test]
    fn replace_audio_mutes_every_primary_clip_and_appends_new_lane() {
        // Single replace should (a) set `audio_mute = true` on every clip of
        // the primary video track and (b) append exactly one new audio lane
        // whose sole clip is the picked file. Guards the core contract of
        // Edit → Replace Audio….
        let probe = FakeProbe::with_duration(4.0);
        let mut s = EditSession::default();
        s.open_media_with_probe(&probe, PathBuf::from("/tmp/fake-video-rep.mp4"))
            .unwrap();
        // Grow the primary track to two clips so "every primary clip" is a
        // non-trivial assertion (single-clip would be indistinguishable from
        // a one-shot toggle).
        let tail_ms = timeline_end_ms_for_tests(s.project().unwrap()).unwrap_or(0.0);
        s.insert_clip_at_playhead_with_probe(
            &probe,
            PathBuf::from("/tmp/fake-video-rep-b.mp4"),
            tail_ms,
        )
        .unwrap();

        // Sanity: baseline has no muted clips and no audio lane.
        assert!(!s.any_primary_clip_audio_muted());
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            0
        );

        let replace_path = PathBuf::from("/tmp/fake-replace.wav");
        s.replace_audio_clip_at_playhead_with_probe(&probe, replace_path.clone(), 500.0)
            .expect("replace audio ok");

        let p = s.project().unwrap();

        // All primary clips muted.
        let vt = p
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .expect("video track");
        for cid in &vt.clip_ids {
            let c = p.clips.iter().find(|c| c.id == *cid).expect("clip");
            assert!(
                c.audio_mute,
                "primary clip {} should be muted after replace",
                c.source_path.display()
            );
        }
        assert!(s.all_primary_clips_audio_muted());

        // Exactly one new audio lane, carrying the replacement clip.
        let a: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .collect();
        assert_eq!(a.len(), 1, "replace appends one fresh audio lane");
        assert_eq!(a[0].clip_ids.len(), 1);
        let clip_id = a[0].clip_ids[0];
        let clip = p
            .clips
            .iter()
            .find(|c| c.id == clip_id)
            .expect("replacement clip");
        assert_eq!(clip.source_path, replace_path);
        assert!((clip.out_point - 4.0).abs() < 1e-9, "duration from probe");
        assert!(
            !clip.audio_mute,
            "replacement clip itself must not be muted"
        );
    }

    #[test]
    fn replace_audio_single_undo_step_unwinds_everything() {
        // Replace collapses "mute every primary clip + append lane + insert
        // clip" into ONE undo snapshot — a single Edit → Undo must restore
        // the original mute states AND drop the new lane. If this regresses
        // the user would need to press Undo N+1 times, which violates the
        // "atomic edit" contract.
        let probe = FakeProbe::with_duration(3.0);
        let mut s = EditSession::default();
        s.open_media_with_probe(&probe, PathBuf::from("/tmp/fake-video-repu.mp4"))
            .unwrap();
        let tail_ms = timeline_end_ms_for_tests(s.project().unwrap()).unwrap_or(0.0);
        s.insert_clip_at_playhead_with_probe(
            &probe,
            PathBuf::from("/tmp/fake-video-repu-b.mp4"),
            tail_ms,
        )
        .unwrap();

        // Mute the *first* primary clip manually so we can verify replace
        // preserves the pre-existing mute under undo (not just "unmutes all").
        s.toggle_audio_mute_at_seq_ms(100.0).unwrap();
        let pre_replace_mute_states: Vec<(Uuid, bool)> = {
            let p = s.project().unwrap();
            let vt = p
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Video)
                .unwrap();
            vt.clip_ids
                .iter()
                .map(|id| {
                    let c = p.clips.iter().find(|c| c.id == *id).unwrap();
                    (*id, c.audio_mute)
                })
                .collect()
        };

        let replace_path = PathBuf::from("/tmp/fake-replace2.wav");
        s.replace_audio_clip_at_playhead_with_probe(&probe, replace_path.clone(), 0.0)
            .unwrap();

        // Single undo — not a loop.
        assert!(s.undo(), "replace is undoable");

        let p = s.project().unwrap();
        // Lane gone.
        assert_eq!(
            p.tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            0,
            "undo removed the replacement lane"
        );
        // Replacement clip gone.
        assert!(
            !p.clips.iter().any(|c| c.source_path == replace_path),
            "undo removed the replacement clip"
        );
        // Mute states restored exactly — first clip still muted, second not.
        let vt = p
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .unwrap();
        let restored: Vec<(Uuid, bool)> = vt
            .clip_ids
            .iter()
            .map(|id| {
                let c = p.clips.iter().find(|c| c.id == *id).unwrap();
                (*id, c.audio_mute)
            })
            .collect();
        assert_eq!(
            restored, pre_replace_mute_states,
            "undo restored per-clip mute states to pre-replace snapshot"
        );
    }

    #[test]
    fn replace_audio_is_idempotent_on_already_muted_primary() {
        // Running replace twice shouldn't produce two replacement lanes
        // stacking inside one snapshot or double-mute anything — each call is
        // just "ensure everything muted + append another lane". The second
        // call's mute pass must be a no-op on already-muted clips.
        let probe = FakeProbe::with_duration(2.0);
        let mut s = EditSession::default();
        s.open_media_with_probe(&probe, PathBuf::from("/tmp/fake-video-repi.mp4"))
            .unwrap();

        s.replace_audio_clip_at_playhead_with_probe(&probe, PathBuf::from("/tmp/ra-1.wav"), 0.0)
            .unwrap();
        s.replace_audio_clip_at_playhead_with_probe(&probe, PathBuf::from("/tmp/ra-2.wav"), 0.0)
            .unwrap();

        let p = s.project().unwrap();
        assert!(s.all_primary_clips_audio_muted());
        // Two replace calls ⇒ two lanes (same stacking behaviour as overlay).
        let a: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .collect();
        assert_eq!(a.len(), 2, "each replace appends its own lane");
    }

    #[test]
    fn replace_audio_without_project_errors_without_mutation() {
        let probe = FakeProbe::with_duration(1.0);
        let mut s = EditSession::default();
        let err = s
            .replace_audio_clip_at_playhead_with_probe(&probe, PathBuf::from("/tmp/ra.wav"), 0.0)
            .expect_err("no-project path must error");
        assert!(err.to_string().contains("no project"));
        assert!(s.project().is_none());
    }

    #[test]
    fn replace_audio_clear_others_drops_existing_lanes_and_leaves_one() {
        // After overlaying two audio lanes, replace-and-clear must result in
        // exactly one audio lane — the replacement — and every primary-track
        // clip must be muted. This is the headline contract of the clear-
        // others variant; regressing it reverts the "swap soundtrack" intent
        // into the stacking behaviour of plain Replace Audio.
        let probe = FakeProbe::with_duration(2.5);
        let mut s = EditSession::default();
        s.open_media_with_probe(&probe, PathBuf::from("/tmp/fake-video-rco.mp4"))
            .unwrap();
        s.insert_overlay_audio_clip_at_playhead_with_probe(
            &probe,
            PathBuf::from("/tmp/overlay-1.wav"),
            0.0,
        )
        .unwrap();
        s.insert_overlay_audio_clip_at_playhead_with_probe(
            &probe,
            PathBuf::from("/tmp/overlay-2.wav"),
            0.0,
        )
        .unwrap();
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            2,
            "two overlays ⇒ two audio lanes pre-clear"
        );

        let replace_path = PathBuf::from("/tmp/fake-swap.wav");
        s.replace_audio_clip_at_playhead_clear_others_with_probe(&probe, replace_path.clone(), 0.0)
            .expect("clear-others replace ok");

        let p = s.project().unwrap();
        let lanes: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .collect();
        assert_eq!(lanes.len(), 1, "exactly one audio lane after clear-others");
        assert_eq!(lanes[0].clip_ids.len(), 1);
        let clip_id = lanes[0].clip_ids[0];
        let clip = p
            .clips
            .iter()
            .find(|c| c.id == clip_id)
            .expect("replacement clip");
        assert_eq!(clip.source_path, replace_path);
        // Overlay clips' source paths must no longer live in the project
        // (remove_track_at drops orphaned clips).
        assert!(
            !p.clips
                .iter()
                .any(|c| c.source_path == PathBuf::from("/tmp/overlay-1.wav")),
            "overlay-1 clip should be gone"
        );
        assert!(
            !p.clips
                .iter()
                .any(|c| c.source_path == PathBuf::from("/tmp/overlay-2.wav")),
            "overlay-2 clip should be gone"
        );
        assert!(s.all_primary_clips_audio_muted(), "primary track muted");
    }

    #[test]
    fn replace_audio_clear_others_is_one_undo_step() {
        // The whole mute + drop-existing-lanes + append-new-lane sequence
        // must live under a single undo snapshot so one Edit → Undo restores
        // the pre-call project byte-exact. Two overlays pre-call ⇒ after
        // Undo those two lanes must be back and the replacement gone, with
        // primary clips un-muted.
        let probe = FakeProbe::with_duration(1.5);
        let mut s = EditSession::default();
        s.open_media_with_probe(&probe, PathBuf::from("/tmp/fake-video-rcou.mp4"))
            .unwrap();
        s.insert_overlay_audio_clip_at_playhead_with_probe(
            &probe,
            PathBuf::from("/tmp/ov-a.wav"),
            0.0,
        )
        .unwrap();
        s.insert_overlay_audio_clip_at_playhead_with_probe(
            &probe,
            PathBuf::from("/tmp/ov-b.wav"),
            0.0,
        )
        .unwrap();
        let pre_snapshot = s.project().cloned().unwrap();

        s.replace_audio_clip_at_playhead_clear_others_with_probe(
            &probe,
            PathBuf::from("/tmp/swap.wav"),
            0.0,
        )
        .unwrap();

        assert!(s.undo(), "one undo must be available");
        let after = s.project().cloned().unwrap();
        assert_eq!(
            after.tracks.len(),
            pre_snapshot.tracks.len(),
            "tracks fully restored"
        );
        assert_eq!(
            after.clips.len(),
            pre_snapshot.clips.len(),
            "clips fully restored"
        );
        assert!(!s.any_primary_clip_audio_muted(), "mute pass undone");
    }

    #[test]
    fn replace_audio_clear_others_handles_no_existing_audio_lanes() {
        // With zero existing audio lanes the clear step is a no-op and this
        // call must behave identically to plain Replace Audio: primary
        // muted, exactly one new audio lane with the replacement. Exercises
        // the boundary where `audio_indices` is empty so `remove_track_at`
        // is never called.
        let probe = FakeProbe::with_duration(2.0);
        let mut s = EditSession::default();
        s.open_media_with_probe(&probe, PathBuf::from("/tmp/fake-video-rcoe.mp4"))
            .unwrap();
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            0
        );

        let replace_path = PathBuf::from("/tmp/fake-only.wav");
        s.replace_audio_clip_at_playhead_clear_others_with_probe(&probe, replace_path.clone(), 0.0)
            .unwrap();

        let p = s.project().unwrap();
        let lanes: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .collect();
        assert_eq!(lanes.len(), 1);
        assert!(s.all_primary_clips_audio_muted());
    }

    #[test]
    fn replace_audio_clear_others_without_project_errors_without_mutation() {
        let probe = FakeProbe::with_duration(1.0);
        let mut s = EditSession::default();
        let err = s
            .replace_audio_clip_at_playhead_clear_others_with_probe(
                &probe,
                PathBuf::from("/tmp/rc.wav"),
                0.0,
            )
            .expect_err("no-project path must error");
        assert!(err.to_string().contains("no project"));
        assert!(s.project().is_none());
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

    /// Build a project with one video track and two empty audio tracks so the
    /// gain-setter/reader tests can target lane 0 and lane 1 without having to
    /// drive ffmpeg.
    fn project_with_two_audio_lanes() -> Project {
        let mut p = two_clip_project();
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Audio,
            clip_ids: vec![],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Audio,
            clip_ids: vec![],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        p
    }

    #[test]
    fn set_audio_track_gain_db_writes_and_reads_back() {
        let mut s = EditSession::from_project_for_tests(project_with_two_audio_lanes());
        s.set_audio_track_gain_db(0, 6.0).expect("set lane 0");
        s.set_audio_track_gain_db(1, -3.5).expect("set lane 1");
        assert_eq!(s.audio_track_gain_db(0), Some(6.0));
        assert_eq!(s.audio_track_gain_db(1), Some(-3.5));
        // Bulk accessor returns lanes in project order.
        assert_eq!(s.audio_track_gains_db(), vec![6.0, -3.5]);
    }

    #[test]
    fn set_audio_track_gain_db_clamps_out_of_range_values() {
        let mut s = EditSession::from_project_for_tests(project_with_two_audio_lanes());
        s.set_audio_track_gain_db(0, 1_000.0).unwrap();
        assert_eq!(
            s.audio_track_gain_db(0),
            Some(EditSession::AUDIO_GAIN_DB_MAX)
        );
        s.set_audio_track_gain_db(0, -1_000.0).unwrap();
        assert_eq!(
            s.audio_track_gain_db(0),
            Some(EditSession::AUDIO_GAIN_DB_MIN)
        );
    }

    #[test]
    fn set_audio_track_gain_db_falls_back_to_zero_on_nan() {
        let mut s = EditSession::from_project_for_tests(project_with_two_audio_lanes());
        // Seed a non-zero baseline so the NaN→0 write actually mutates.
        s.set_audio_track_gain_db(0, 12.0).unwrap();
        s.set_audio_track_gain_db(0, f32::NAN).unwrap();
        assert_eq!(s.audio_track_gain_db(0), Some(0.0));
    }

    #[test]
    fn set_audio_track_gain_db_errors_on_out_of_range_lane() {
        let mut s = EditSession::from_project_for_tests(project_with_two_audio_lanes());
        // Two audio lanes exist (indices 0 and 1). Lane 2 must fail without
        // mutating anything.
        assert!(s.set_audio_track_gain_db(2, 6.0).is_err());
        assert_eq!(s.audio_track_gains_db(), vec![0.0, 0.0]);
    }

    #[test]
    fn set_audio_track_gain_db_is_noop_when_value_unchanged() {
        let mut s = EditSession::from_project_for_tests(project_with_two_audio_lanes());
        s.set_audio_track_gain_db(0, 4.0).unwrap();
        // Re-writing the same value must not push undo, so a single `undo()`
        // rolls straight back past the redundant write to the original state.
        s.set_audio_track_gain_db(0, 4.0).unwrap();
        assert!(s.undo());
        assert_eq!(s.audio_track_gain_db(0), Some(0.0));
    }

    #[test]
    fn set_audio_track_gain_db_is_undoable() {
        let mut s = EditSession::from_project_for_tests(project_with_two_audio_lanes());
        s.set_audio_track_gain_db(1, -12.0).unwrap();
        assert_eq!(s.audio_track_gain_db(1), Some(-12.0));
        assert!(s.undo());
        assert_eq!(s.audio_track_gain_db(1), Some(0.0));
        assert!(s.redo());
        assert_eq!(s.audio_track_gain_db(1), Some(-12.0));
    }

    #[test]
    fn audio_track_row_label_suffixes_with_non_unity_gain() {
        let mut s = EditSession::from_project_for_tests(project_with_two_audio_lanes());
        // Unity by default — labels stay clean.
        let rows = s.audio_track_row_labels();
        assert_eq!(rows.len(), 2);
        assert!(
            !rows[0].contains("dB") && !rows[1].contains("dB"),
            "unity gain must not bloat the row label"
        );
        // After a boost, lane 0's label gains a `· +6.0 dB` suffix; lane 1 stays clean.
        s.set_audio_track_gain_db(0, 6.0).unwrap();
        let rows = s.audio_track_row_labels();
        assert!(
            rows[0].ends_with("· +6.0 dB"),
            "expected '· +6.0 dB' suffix, got: {}",
            rows[0]
        );
        assert!(
            !rows[1].contains("dB"),
            "unmodified lane must not gain a suffix: {}",
            rows[1]
        );
        // Negative gains render with an explicit sign too.
        s.set_audio_track_gain_db(1, -3.5).unwrap();
        let rows = s.audio_track_row_labels();
        assert!(
            rows[1].ends_with("· -3.5 dB"),
            "expected '· -3.5 dB' suffix, got: {}",
            rows[1]
        );
    }

    #[test]
    fn audio_track_gains_db_empty_without_project() {
        let s = EditSession::default();
        assert!(s.audio_track_gains_db().is_empty());
        assert_eq!(s.audio_track_gain_db(0), None);
    }

    #[test]
    fn add_subtitle_track_appends_empty_lane() {
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
                .filter(|t| t.kind == TrackKind::Subtitle)
                .count(),
            0
        );
        s.add_subtitle_track().unwrap();
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Subtitle)
                .count(),
            1
        );
        assert!(s.undo());
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Subtitle)
                .count(),
            0
        );
    }

    #[test]
    fn insert_subtitle_fails_without_any_track() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let srt = dir.path().join("cap.srt");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:02,000\nHello\n").unwrap();
        let mut s = EditSession::default();
        s.open_media(f).unwrap();
        let err = s
            .insert_subtitle_clip_at_playhead(srt, 0.0)
            .expect_err("no subtitle track yet");
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("subtitle"),
            "expected subtitle-track error, got {msg}"
        );
    }

    #[test]
    fn insert_subtitle_appends_clip_and_is_undoable() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let srt = dir.path().join("cap.srt");
        std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:02,250\nHi\n").unwrap();
        let mut s = EditSession::default();
        s.open_media(f).unwrap();
        s.add_subtitle_track().unwrap();
        s.insert_subtitle_clip_at_playhead(srt.clone(), 0.0)
            .expect("happy path");
        assert_eq!(s.primary_subtitle_path().as_deref(), Some(srt.as_path()));
        let p = s.project().unwrap();
        let sub_track = p
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Subtitle)
            .unwrap();
        assert_eq!(sub_track.clip_ids.len(), 1);
        assert!(s.undo());
        assert!(s.primary_subtitle_path().is_none());
    }

    #[test]
    fn insert_subtitle_accepts_webvtt_file() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let vtt = dir.path().join("cap.vtt");
        std::fs::write(&vtt, "WEBVTT\n\n00:00:00.500 --> 00:00:02.000\nHi\n").unwrap();
        let mut s = EditSession::default();
        s.open_media(f).unwrap();
        s.add_subtitle_track().unwrap();
        s.insert_subtitle_clip_at_playhead(vtt.clone(), 0.0)
            .expect("WebVTT happy path");
        assert_eq!(s.primary_subtitle_path().as_deref(), Some(vtt.as_path()));
        let p = s.project().unwrap();
        let clip = p
            .clips
            .iter()
            .find(|c| c.source_path == vtt)
            .expect("inserted clip present");
        assert_eq!(clip.metadata.container, "vtt");
    }

    #[test]
    fn insert_subtitle_rejects_empty_srt() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let srt = dir.path().join("empty.srt");
        std::fs::write(&srt, "").unwrap();
        let mut s = EditSession::default();
        s.open_media(f).unwrap();
        s.add_subtitle_track().unwrap();
        let err = s
            .insert_subtitle_clip_at_playhead(srt, 0.0)
            .expect_err("empty srt should be rejected");
        assert!(format!("{err}").to_lowercase().contains("no cues"));
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

    #[test]
    fn remove_video_track_lane_drops_empty_secondary() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f).unwrap();
        s.add_video_track().unwrap();
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video)
                .count(),
            2
        );
        s.remove_video_track_lane(1).unwrap();
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video)
                .count(),
            1
        );
        assert!(s.remove_video_track_lane(0).is_err());
    }

    #[test]
    fn remove_audio_track_lane_clears_lane() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.open_media(f).unwrap();
        s.add_audio_track().unwrap();
        assert_eq!(
            s.project()
                .unwrap()
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Audio)
                .count(),
            1
        );
        s.remove_audio_track_lane(0).unwrap();
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

    fn timeline_end_ms_for_tests(project: &Project) -> Option<f64> {
        let track = project.tracks.iter().find(|t| t.kind == TrackKind::Video)?;
        let mut ms = 0.0_f64;
        for cid in &track.clip_ids {
            let clip = project.clips.iter().find(|c| c.id == *cid)?;
            ms += (clip.out_point - clip.in_point) * 1000.0;
        }
        Some(ms)
    }

    /// Build an EditSession pre-populated with `two_clip_project()`, bypassing the
    /// fixture-file requirement so trim tests don't depend on ffmpeg on CI.
    fn session_with_two_clip_project() -> EditSession {
        EditSession {
            project: Some(two_clip_project()),
            ..Default::default()
        }
    }

    #[test]
    fn trim_candidate_none_without_project() {
        let s = EditSession::default();
        assert!(s.trim_candidate_at_seq_ms(0.0).is_none());
        assert!(!s.trim_enabled(0.0));
    }

    #[test]
    fn trim_candidate_on_first_clip_returns_bounds() {
        let s = session_with_two_clip_project();
        let p = s.project().unwrap();
        let first_id = p.tracks[0].clip_ids[0];
        // 500ms is inside first clip (spans 0–2000 ms)
        let c = s
            .trim_candidate_at_seq_ms(500.0)
            .expect("candidate on first clip");
        assert_eq!(c.clip_id, first_id);
        assert!((c.current_in_s - 0.0).abs() < 1e-9);
        assert!((c.current_out_s - 2.0).abs() < 1e-9);
        assert!((c.source_duration_s - 2.0).abs() < 1e-9);
        assert!(s.trim_enabled(500.0));
    }

    #[test]
    fn trim_candidate_past_end_returns_none() {
        let s = session_with_two_clip_project();
        // two_clip_project totals 5s = 5000ms; well past end:
        assert!(s.trim_candidate_at_seq_ms(999_999.0).is_none());
        assert!(!s.trim_enabled(999_999.0));
    }

    #[test]
    fn trim_clip_happy_path_updates_bounds_and_marks_dirty() {
        let mut s = session_with_two_clip_project();
        // Establish a saved baseline == current state so `dirty` tracks subsequent edits.
        s.saved_baseline = s.project().cloned();
        s.dirty = false;
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        s.trim_clip(first_id, 0.2, 1.5).expect("trim ok");
        let c = s
            .project()
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == first_id)
            .unwrap();
        assert!((c.in_point - 0.2).abs() < 1e-9);
        assert!((c.out_point - 1.5).abs() < 1e-9);
        assert!(s.dirty);
        assert!(s.undo_enabled());
    }

    #[test]
    fn trim_clip_undo_restores_original_bounds() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        s.trim_clip(first_id, 0.3, 1.7).unwrap();
        assert!(s.undo());
        let c = s
            .project()
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == first_id)
            .unwrap();
        assert!((c.in_point - 0.0).abs() < 1e-9);
        assert!((c.out_point - 2.0).abs() < 1e-9);
    }

    #[test]
    fn trim_clip_rejects_begin_ge_end_without_pushing_undo() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        assert!(!s.undo_enabled());
        assert!(s.trim_clip(first_id, 1.0, 1.0).is_err());
        assert!(s.trim_clip(first_id, 1.5, 1.0).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn trim_clip_rejects_negative_begin() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        assert!(s.trim_clip(first_id, -0.1, 1.0).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn trim_clip_rejects_duration_below_minimum() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        // Below MIN_TRIM_DURATION_S (0.05s)
        assert!(s.trim_clip(first_id, 0.5, 0.52).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn trim_clip_rejects_out_exceeding_source_duration() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        // Source duration is 2.0s; 2.5s is clearly out of range.
        assert!(s.trim_clip(first_id, 0.0, 2.5).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn trim_clip_rejects_unknown_clip_id() {
        let mut s = session_with_two_clip_project();
        assert!(s.trim_clip(Uuid::new_v4(), 0.0, 1.0).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn trim_clip_rejects_non_finite_values() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        assert!(s.trim_clip(first_id, f64::NAN, 1.0).is_err());
        assert!(s.trim_clip(first_id, 0.0, f64::INFINITY).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn trim_edge_drag_left_moves_in_point_by_ratio_of_duration() {
        // Clip starts at [0.0, 2.0]s (2.0s duration). Left-edge drag by +0.25
        // of the chip width → new_in = 0.0 + 0.25 * 2.0 = 0.5s.
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        s.trim_clip_by_edge_drag(first_id, 0, 0.25).expect("ok");
        let c = s
            .project()
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == first_id)
            .unwrap();
        assert!((c.in_point - 0.5).abs() < 1e-9);
        assert!((c.out_point - 2.0).abs() < 1e-9);
    }

    #[test]
    fn trim_edge_drag_right_moves_out_point_by_negative_ratio() {
        // Clip [0.0, 2.0]s; right-edge drag by -0.25 of the chip width shrinks
        // the tail: new_out = 2.0 + (-0.25) * 2.0 = 1.5s.
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        s.trim_clip_by_edge_drag(first_id, 1, -0.25).expect("ok");
        let c = s
            .project()
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == first_id)
            .unwrap();
        assert!((c.in_point - 0.0).abs() < 1e-9);
        assert!((c.out_point - 1.5).abs() < 1e-9);
    }

    #[test]
    fn trim_edge_drag_rejects_invalid_edge_and_non_finite_ratio() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        assert!(s.trim_clip_by_edge_drag(first_id, 2, 0.1).is_err());
        assert!(s.trim_clip_by_edge_drag(first_id, 0, f64::NAN).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn trim_edge_drag_rejects_over_extension_past_source_without_undo() {
        // Source duration is 2.0s; dragging the right edge out by +1.0 of the
        // current width would push out_point to 4.0s — reject via trim_clip's
        // upper-bound check and leave the clip untouched.
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        assert!(s.trim_clip_by_edge_drag(first_id, 1, 1.0).is_err());
        let c = s
            .project()
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == first_id)
            .unwrap();
        assert!((c.out_point - 2.0).abs() < 1e-9);
        assert!(!s.undo_enabled());
    }

    #[test]
    fn trim_edge_drag_rejects_unknown_clip_id() {
        let mut s = session_with_two_clip_project();
        assert!(s.trim_clip_by_edge_drag(Uuid::new_v4(), 0, 0.1).is_err());
        assert!(!s.undo_enabled());
    }

    /// Regression: shortening the **first** primary-track clip ripples the sequence
    /// automatically — there's no explicit ripple because the project model
    /// has no absolute timeline positions. Total duration after the trim equals
    /// the new clip-1 duration + original clip-2 duration.
    #[test]
    fn trim_edge_drag_shrinks_first_clip_and_total_sequence_ripples() {
        let mut s = session_with_two_clip_project();
        let clip_ids = s.project().unwrap().tracks[0].clip_ids.clone();
        let (first_id, second_id) = (clip_ids[0], clip_ids[1]);
        // Pre-trim: clip-1 is [0.0, 2.0]s (2.0s), clip-2 is [0.0, 3.0]s (3.0s).
        let second_dur_s = {
            let p = s.project().unwrap();
            let c = p.clips.iter().find(|c| c.id == second_id).unwrap();
            c.out_point - c.in_point
        };
        // Drag clip-1's right edge by -0.25 → new clip-1 duration = 1.5s.
        s.trim_clip_by_edge_drag(first_id, 1, -0.25).expect("ok");
        let p = s.project().unwrap();
        let c1 = p.clips.iter().find(|c| c.id == first_id).unwrap();
        let new_first_dur_s = c1.out_point - c1.in_point;
        assert!((new_first_dur_s - 1.5).abs() < 1e-9);
        // Clip-2 is unchanged (no absolute position → no ripple math needed).
        let c2 = p.clips.iter().find(|c| c.id == second_id).unwrap();
        assert!((c2.out_point - c2.in_point - second_dur_s).abs() < 1e-9);
    }

    #[test]
    fn resize_candidate_reports_current_percent_and_source_dims() {
        let s = session_with_two_clip_project();
        let cand = s.resize_candidate_at_seq_ms(500.0).expect("cand");
        assert_eq!(cand.current_percent, 100); // default == identity
        assert!(s.resize_enabled(500.0));
        // two_clip_project totals 5s; past the end returns None.
        assert!(s.resize_candidate_at_seq_ms(999_999.0).is_none());
        assert!(!s.resize_enabled(999_999.0));
    }

    #[test]
    fn resize_clip_happy_path_updates_scale_and_marks_dirty() {
        let mut s = session_with_two_clip_project();
        s.saved_baseline = s.project().cloned();
        s.dirty = false;
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        s.resize_clip(first_id, 50).expect("resize ok");
        let c = s
            .project()
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == first_id)
            .unwrap();
        assert_eq!(c.scale.display_percent(), 50);
        assert!(s.dirty);
        assert!(s.undo_enabled());
    }

    #[test]
    fn resize_clip_100_percent_is_identity() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        s.resize_clip(first_id, 100).unwrap();
        let c = s
            .project()
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == first_id)
            .unwrap();
        assert!(c.scale.is_identity());
    }

    #[test]
    fn resize_clip_undo_restores_original_scale() {
        let mut s = session_with_two_clip_project();
        let first_id = s.project().unwrap().tracks[0].clip_ids[0];
        s.resize_clip(first_id, 200).unwrap();
        assert!(s.undo());
        let c = s
            .project()
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == first_id)
            .unwrap();
        assert!(c.scale.is_identity());
    }

    #[test]
    fn resize_clip_rejects_unknown_clip_id() {
        let mut s = session_with_two_clip_project();
        assert!(s.resize_clip(Uuid::new_v4(), 75).is_err());
        assert!(!s.undo_enabled());
    }

    #[test]
    fn audio_mute_defaults_to_false_and_toggles() {
        let mut s = session_with_two_clip_project();
        s.saved_baseline = s.project().cloned();
        s.dirty = false;
        assert_eq!(s.audio_mute_state_at_seq_ms(500.0), Some(false));
        s.toggle_audio_mute_at_seq_ms(500.0).expect("toggle ok");
        assert_eq!(s.audio_mute_state_at_seq_ms(500.0), Some(true));
        assert!(s.dirty);
        assert!(s.undo_enabled());
        // Toggle back.
        s.toggle_audio_mute_at_seq_ms(500.0).unwrap();
        assert_eq!(s.audio_mute_state_at_seq_ms(500.0), Some(false));
    }

    #[test]
    fn audio_mute_undo_restores_previous_state() {
        let mut s = session_with_two_clip_project();
        s.toggle_audio_mute_at_seq_ms(500.0).unwrap();
        assert!(s.undo());
        assert_eq!(s.audio_mute_state_at_seq_ms(500.0), Some(false));
    }

    #[test]
    fn audio_mute_past_end_returns_none() {
        let s = session_with_two_clip_project();
        // two_clip_project totals 5s; past end no clip.
        assert!(s.audio_mute_state_at_seq_ms(999_999.0).is_none());
        assert!(!s.audio_mute_enabled(999_999.0));
    }

    #[test]
    fn audio_mute_all_and_any_reflect_per_clip_state() {
        let mut s = session_with_two_clip_project();
        // two_clip_project has clips at [0..2s, 2..5s] on the primary track.
        assert!(!s.all_primary_clips_audio_muted());
        assert!(!s.any_primary_clip_audio_muted());

        s.toggle_audio_mute_at_seq_ms(500.0).unwrap(); // first clip only
        assert!(!s.all_primary_clips_audio_muted());
        assert!(s.any_primary_clip_audio_muted());

        s.toggle_audio_mute_at_seq_ms(3_500.0).unwrap(); // second clip
        assert!(s.all_primary_clips_audio_muted());
        assert!(s.any_primary_clip_audio_muted());
    }

    #[test]
    fn markers_default_to_none_and_no_range() {
        let s = EditSession::default();
        assert_eq!(s.in_marker_ms(), None);
        assert_eq!(s.out_marker_ms(), None);
        assert!(!s.has_any_marker());
        assert_eq!(s.marker_range_ms(), None);
    }

    #[test]
    fn set_in_then_out_builds_valid_range() {
        let mut s = EditSession::default();
        s.set_in_marker_ms(100.0).unwrap();
        assert_eq!(s.in_marker_ms(), Some(100.0));
        assert_eq!(s.marker_range_ms(), None); // out still unset
        s.set_out_marker_ms(500.0).unwrap();
        assert_eq!(s.marker_range_ms(), Some((100.0, 500.0)));
        assert!(s.has_any_marker());
    }

    #[test]
    fn set_in_past_existing_out_clears_out() {
        let mut s = EditSession::default();
        s.set_out_marker_ms(200.0).unwrap();
        s.set_in_marker_ms(300.0).unwrap(); // past out → out cleared
        assert_eq!(s.in_marker_ms(), Some(300.0));
        assert_eq!(s.out_marker_ms(), None);
        assert_eq!(s.marker_range_ms(), None);
    }

    #[test]
    fn set_out_before_existing_in_clears_in() {
        let mut s = EditSession::default();
        s.set_in_marker_ms(500.0).unwrap();
        s.set_out_marker_ms(100.0).unwrap(); // before in → in cleared
        assert_eq!(s.in_marker_ms(), None);
        assert_eq!(s.out_marker_ms(), Some(100.0));
        assert_eq!(s.marker_range_ms(), None);
    }

    #[test]
    fn set_in_equal_to_out_clears_out() {
        let mut s = EditSession::default();
        s.set_out_marker_ms(200.0).unwrap();
        s.set_in_marker_ms(200.0).unwrap(); // equal → out cleared (no zero-length range)
        assert_eq!(s.out_marker_ms(), None);
    }

    #[test]
    fn markers_reject_non_finite_and_negative() {
        let mut s = EditSession::default();
        assert!(s.set_in_marker_ms(-1.0).is_err());
        assert!(s.set_in_marker_ms(f64::NAN).is_err());
        assert!(s.set_out_marker_ms(-1.0).is_err());
        assert!(s.set_out_marker_ms(f64::INFINITY).is_err());
        assert!(!s.has_any_marker());
    }

    #[test]
    fn clear_markers_removes_both() {
        let mut s = EditSession::default();
        s.set_in_marker_ms(100.0).unwrap();
        s.set_out_marker_ms(500.0).unwrap();
        s.clear_markers();
        assert!(!s.has_any_marker());
        assert_eq!(s.marker_range_ms(), None);
    }

    #[test]
    fn clear_markers_is_noop_when_none_set() {
        let mut s = EditSession::default();
        s.clear_markers();
        assert!(!s.has_any_marker());
    }

    #[test]
    fn clear_media_clears_markers() {
        let mut s = EditSession::default();
        s.set_in_marker_ms(100.0).unwrap();
        s.set_out_marker_ms(500.0).unwrap();
        s.clear_media();
        assert!(!s.has_any_marker());
    }

    #[test]
    fn open_media_clears_markers() {
        let f = tiny_fixture();
        if !f.is_file() {
            return;
        }
        let mut s = EditSession::default();
        s.set_in_marker_ms(100.0).unwrap();
        s.set_out_marker_ms(500.0).unwrap();
        s.open_media(f).unwrap();
        assert!(!s.has_any_marker());
    }
}
