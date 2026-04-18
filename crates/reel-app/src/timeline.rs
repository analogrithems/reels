//! Primary video track: concatenated sequence time ↔ media files (U2 sequence preview).

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use reel_core::project::ClipOrientation;
use reel_core::{Clip, Project, TrackKind};
use uuid::Uuid;

/// Epsilon for float boundaries (matches session insert math).
const SEQ_MS_EPS: f64 = 1e-3;

#[derive(Debug, Clone)]
pub(crate) struct PrimaryTimelineClip {
    pub path: PathBuf,
    pub media_in_s: f64,
    pub media_out_s: f64,
    pub seq_start_ms: f64,
    pub orientation: ClipOrientation,
}

/// First [`TrackKind::Audio`] track in project order (if any), concatenated like the primary video lane.
pub(crate) fn clips_from_first_audio_track(p: &Project) -> Option<Vec<PrimaryTimelineClip>> {
    let track = p.tracks.iter().find(|t| t.kind == TrackKind::Audio)?;
    let mut seq = 0.0_f64;
    let mut out = Vec::new();
    for cid in &track.clip_ids {
        let c = p.clips.iter().find(|x| x.id == *cid)?;
        let dur_ms = (c.out_point - c.in_point) * 1000.0;
        if dur_ms <= SEQ_MS_EPS {
            continue;
        }
        out.push(PrimaryTimelineClip {
            path: c.source_path.clone(),
            media_in_s: c.in_point,
            media_out_s: c.out_point,
            seq_start_ms: seq,
            orientation: c.orientation,
        });
        seq += dur_ms;
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

pub(crate) fn clips_from_project(p: &Project) -> Option<Vec<PrimaryTimelineClip>> {
    let track = p.tracks.iter().find(|t| t.kind == TrackKind::Video)?;
    let mut seq = 0.0_f64;
    let mut out = Vec::new();
    for cid in &track.clip_ids {
        let c = p.clips.iter().find(|x| x.id == *cid)?;
        let dur_ms = (c.out_point - c.in_point) * 1000.0;
        if dur_ms <= SEQ_MS_EPS {
            continue;
        }
        out.push(PrimaryTimelineClip {
            path: c.source_path.clone(),
            media_in_s: c.in_point,
            media_out_s: c.out_point,
            seq_start_ms: seq,
            orientation: c.orientation,
        });
        seq += dur_ms;
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

pub(crate) fn sequence_duration_ms(clips: &[PrimaryTimelineClip]) -> f64 {
    clips
        .last()
        .map(|c| c.seq_start_ms + (c.media_out_s - c.media_in_s) * 1000.0)
        .unwrap_or(0.0)
}

/// Slice `clips` to the `(range_in_ms, range_out_ms)` sequence range (e.g. from the
/// seek-bar In/Out markers). Clips entirely outside the range are dropped; partials are
/// trimmed in source-file seconds; `seq_start_ms` of the result is **rebased to 0** so the
/// sliced list stands alone as a new concat timeline (ffmpeg export treats it that way).
pub(crate) fn slice_clips_to_range_ms(
    clips: &[PrimaryTimelineClip],
    range_ms: (f64, f64),
) -> Vec<PrimaryTimelineClip> {
    let (range_in, range_out) = range_ms;
    if range_out <= range_in + SEQ_MS_EPS {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut new_seq = 0.0_f64;
    for c in clips {
        let span_ms = (c.media_out_s - c.media_in_s) * 1000.0;
        let seq_start = c.seq_start_ms;
        let seq_end = seq_start + span_ms;
        if seq_end <= range_in + SEQ_MS_EPS {
            continue;
        }
        if seq_start + SEQ_MS_EPS >= range_out {
            break;
        }
        let lead_ms = (range_in - seq_start).max(0.0);
        let trail_ms = (seq_end - range_out).max(0.0);
        let new_in_s = c.media_in_s + lead_ms / 1000.0;
        let new_out_s = c.media_out_s - trail_ms / 1000.0;
        let new_span_ms = (new_out_s - new_in_s) * 1000.0;
        if new_span_ms <= SEQ_MS_EPS {
            continue;
        }
        out.push(PrimaryTimelineClip {
            path: c.path.clone(),
            media_in_s: new_in_s,
            media_out_s: new_out_s,
            seq_start_ms: new_seq,
            orientation: c.orientation,
        });
        new_seq += new_span_ms;
    }
    out
}

/// Nudge sequence time slightly inward from the timeline end so [`primary_clip_id_at_seq_ms`]
/// still finds the last clip when the UI playhead is clamped to `duration_ms`.
///
/// Only applies when `sequence_ms` is **at** the real end (within ~1 ms past `d`). Values far past
/// the sequence (gaps / bogus seeks) are left unchanged so they still resolve to **no** clip.
pub(crate) fn sequence_ms_for_primary_clip_lookup(p: &Project, sequence_ms: f64) -> f64 {
    let Some(clips) = clips_from_project(p) else {
        return sequence_ms.max(0.0);
    };
    let d = sequence_duration_ms(&clips);
    if d <= 0.0 {
        return sequence_ms.max(0.0);
    }
    let mut seq = sequence_ms.max(0.0);
    if seq + 1e-6 >= d - 1e-3 && seq <= d + 1.0 {
        seq = (d - 0.5).max(0.0);
    }
    seq
}

/// Map concatenated timeline `sequence_ms` to a source file and ffmpeg seek position (ms in file).
pub(crate) fn resolve_sequence_media_ms(
    clips: &[PrimaryTimelineClip],
    sequence_ms: f64,
) -> Option<(PathBuf, u64)> {
    if clips.is_empty() {
        return None;
    }
    let seq = sequence_ms.max(0.0);
    let n = clips.len();
    for (i, c) in clips.iter().enumerate() {
        let span = (c.media_out_s - c.media_in_s) * 1000.0;
        let end = c.seq_start_ms + span;
        let last = i + 1 == n;
        if seq < end - SEQ_MS_EPS || (last && seq <= end + SEQ_MS_EPS) {
            let local = c.media_in_s * 1000.0 + (seq - c.seq_start_ms);
            let lo = c.media_in_s * 1000.0;
            let hi = c.media_out_s * 1000.0;
            let local = local.clamp(lo, (hi - SEQ_MS_EPS).max(lo));
            return Some((c.path.clone(), local.round() as u64));
        }
    }
    let last = clips.last()?;
    let lm = (last.media_out_s * 1000.0 - SEQ_MS_EPS).max(0.0);
    Some((last.path.clone(), lm.round() as u64))
}

/// One contiguous decode span on the primary track (sequence clock space).
#[derive(Debug, Clone)]
pub(crate) struct TimelineSegment {
    pub path: PathBuf,
    pub media_in_ms: u64,
    pub media_out_ms: u64,
    pub seq_start_ms: u64,
    pub orientation: ClipOrientation,
}

impl TimelineSegment {
    fn from_clip(c: &PrimaryTimelineClip) -> Self {
        Self {
            path: c.path.clone(),
            media_in_ms: (c.media_in_s * 1000.0).round() as u64,
            media_out_ms: (c.media_out_s * 1000.0).round() as u64,
            seq_start_ms: c.seq_start_ms.round() as u64,
            orientation: c.orientation,
        }
    }

    pub(crate) fn span_ms(&self) -> u64 {
        self.media_out_ms.saturating_sub(self.media_in_ms)
    }
}

/// Shared state for multi-clip playback: both decoder threads advance `active_index` from EOF on
/// the audio side; video follows when it runs out of frames.
pub(crate) struct TimelineSync {
    pub segments: Arc<Vec<TimelineSegment>>,
    /// Index of the segment currently being decoded (0..segments.len).
    pub active_index: AtomicUsize,
}

impl TimelineSync {
    pub(crate) fn from_clips(clips: &[PrimaryTimelineClip]) -> Option<Arc<Self>> {
        if clips.is_empty() {
            return None;
        }
        let v: Vec<TimelineSegment> = clips.iter().map(TimelineSegment::from_clip).collect();
        Some(Arc::new(Self {
            segments: Arc::new(v),
            active_index: AtomicUsize::new(0),
        }))
    }

    pub(crate) fn total_sequence_ms(&self) -> u64 {
        self.segments
            .last()
            .map(|s| s.seq_start_ms + s.span_ms())
            .unwrap_or(0)
    }

    /// Which segment contains `seq_ms`, and local ffmpeg seek target in that file.
    pub(crate) fn resolve_seek(&self, seq_ms: u64) -> Option<(usize, u64)> {
        let n = self.segments.len();
        for (i, s) in self.segments.iter().enumerate() {
            let end_seq = s.seq_start_ms + s.span_ms();
            let last = i + 1 == n;
            if seq_ms >= s.seq_start_ms && (seq_ms < end_seq || (last && seq_ms <= end_seq)) {
                let local = s.media_in_ms + (seq_ms - s.seq_start_ms);
                let cap = s.media_out_ms.saturating_sub(1);
                return Some((i, local.min(cap)));
            }
        }
        self.segments.last().map(|s| {
            let i = n.saturating_sub(1);
            (i, s.media_out_ms.saturating_sub(1))
        })
    }
}

pub(crate) fn timeline_sync_from_project(p: &Project) -> Option<Arc<TimelineSync>> {
    let clips = clips_from_project(p)?;
    TimelineSync::from_clips(&clips)
}

/// When the first **audio** track has clips, a separate concat timeline for the audio thread (preview uses this instead of embedded audio from video files).
pub(crate) fn dedicated_audio_timeline_sync_from_project(p: &Project) -> Option<Arc<TimelineSync>> {
    let clips = clips_from_first_audio_track(p)?;
    TimelineSync::from_clips(&clips)
}

pub(crate) fn resolve_for_project(p: &Project, sequence_ms: f64) -> Option<(PathBuf, u64)> {
    let clips = clips_from_project(p)?;
    resolve_sequence_media_ms(&clips, sequence_ms)
}

/// Clip on the **primary** (first) video track that contains `sequence_ms` in sequence time,
/// or `None` if the playhead is in a gap or past the end (with the same edge rules as playback).
pub(crate) fn primary_clip_id_at_seq_ms(p: &Project, sequence_ms: f64) -> Option<Uuid> {
    let track = p.tracks.iter().find(|t| t.kind == TrackKind::Video)?;
    let seq = sequence_ms.max(0.0);
    let mut spans: Vec<(Uuid, f64, f64)> = Vec::new();
    let mut t_ms = 0.0_f64;
    for cid in &track.clip_ids {
        let c = p.clips.iter().find(|x| x.id == *cid)?;
        let dur_ms = (c.out_point - c.in_point) * 1000.0;
        if dur_ms <= SEQ_MS_EPS {
            continue;
        }
        let start = t_ms;
        let end = t_ms + dur_ms;
        spans.push((*cid, start, end));
        t_ms = end;
    }
    if spans.is_empty() {
        return None;
    }
    let last_i = spans.len() - 1;
    for (i, &(id, start, end)) in spans.iter().enumerate() {
        let is_last = i == last_i;
        if seq + SEQ_MS_EPS >= start
            && (seq < end - SEQ_MS_EPS || (is_last && seq <= end + SEQ_MS_EPS))
        {
            return Some(id);
        }
    }
    None
}

/// Primary video [`Clip`] under the playhead (`sequence_ms` in ms on the concatenated sequence).
pub(crate) fn primary_clip_ref_at_seq_ms(p: &Project, sequence_ms: f64) -> Option<&Clip> {
    let id = primary_clip_id_at_seq_ms(p, sequence_ms)?;
    p.clips.iter().find(|c| c.id == id)
}

/// First **audio** track: clip id at `sequence_ms`, or `None` in a gap / past end / no audio lane.
pub(crate) fn first_audio_clip_id_at_seq_ms(p: &Project, sequence_ms: f64) -> Option<Uuid> {
    let track = p.tracks.iter().find(|t| t.kind == TrackKind::Audio)?;
    let seq = sequence_ms.max(0.0);
    let mut spans: Vec<(Uuid, f64, f64)> = Vec::new();
    let mut t_ms = 0.0_f64;
    for cid in &track.clip_ids {
        let c = p.clips.iter().find(|x| x.id == *cid)?;
        let dur_ms = (c.out_point - c.in_point) * 1000.0;
        if dur_ms <= SEQ_MS_EPS {
            continue;
        }
        let start = t_ms;
        let end = t_ms + dur_ms;
        spans.push((*cid, start, end));
        t_ms = end;
    }
    if spans.is_empty() {
        return None;
    }
    let last_i = spans.len() - 1;
    for (i, &(id, start, end)) in spans.iter().enumerate() {
        let is_last = i == last_i;
        if seq + SEQ_MS_EPS >= start
            && (seq < end - SEQ_MS_EPS || (is_last && seq <= end + SEQ_MS_EPS))
        {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::{Clip, MediaMetadata, Project, Track, TrackKind};
    use uuid::Uuid;

    fn clip(id: u128, path: &str, dur: f64) -> Clip {
        Clip {
            id: Uuid::from_u128(id),
            source_path: PathBuf::from(path),
            metadata: MediaMetadata {
                path: PathBuf::from(path),
                duration_seconds: dur,
                container: "mp4".into(),
                video: None,
                audio: None,
                audio_disabled: false,
                video_stream_count: 0,
                audio_stream_count: 0,
                subtitle_stream_count: 0,
            },
            in_point: 0.0,
            out_point: dur,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            extensions: Default::default(),
        }
    }

    #[test]
    fn primary_clip_id_mid_first_clip() {
        let a = clip(1, "/a.mp4", 2.0);
        let b = clip(2, "/b.mp4", 3.0);
        let tid = Uuid::from_u128(99);
        let mut p = Project::new("t");
        p.clips.push(a);
        p.clips.push(b);
        p.tracks.push(Track {
            id: tid,
            kind: TrackKind::Video,
            clip_ids: vec![p.clips[0].id, p.clips[1].id],
            extensions: Default::default(),
        });
        assert_eq!(primary_clip_id_at_seq_ms(&p, 500.0), Some(p.clips[0].id));
        assert_eq!(primary_clip_id_at_seq_ms(&p, 2500.0), Some(p.clips[1].id));
        assert_eq!(primary_clip_id_at_seq_ms(&p, 999_999.0), None);
    }

    #[test]
    fn clips_from_first_audio_track_concat() {
        let a = clip(1, "/a.wav", 2.0);
        let b = clip(2, "/b.wav", 3.0);
        let vid = Uuid::from_u128(10);
        let aid = Uuid::from_u128(11);
        let mut p = Project::new("t");
        p.clips.push(a);
        p.clips.push(b);
        p.tracks.push(Track {
            id: vid,
            kind: TrackKind::Video,
            clip_ids: vec![p.clips[0].id],
            extensions: Default::default(),
        });
        p.tracks.push(Track {
            id: aid,
            kind: TrackKind::Audio,
            clip_ids: vec![p.clips[0].id, p.clips[1].id],
            extensions: Default::default(),
        });
        let c = clips_from_first_audio_track(&p).unwrap();
        assert_eq!(c.len(), 2);
        assert!((sequence_duration_ms(&c) - 5000.0).abs() < 0.1);
        assert!(dedicated_audio_timeline_sync_from_project(&p).is_some());
    }

    #[test]
    fn resolve_spans_two_files() {
        let a = clip(1, "/a.mp4", 2.0);
        let b = clip(2, "/b.mp4", 3.0);
        let tid = Uuid::from_u128(99);
        let mut p = Project::new("t");
        p.clips.push(a);
        p.clips.push(b);
        p.tracks.push(Track {
            id: tid,
            kind: TrackKind::Video,
            clip_ids: vec![p.clips[0].id, p.clips[1].id],
            extensions: Default::default(),
        });
        let c = clips_from_project(&p).unwrap();
        assert!((sequence_duration_ms(&c) - 5000.0).abs() < 0.1);
        let (pth, ms) = resolve_sequence_media_ms(&c, 2500.0).unwrap();
        assert_eq!(pth, PathBuf::from("/b.mp4"));
        assert!((ms as f64 - 500.0).abs() < 2.0);
    }

    fn timeline_clip(path: &str, in_s: f64, out_s: f64, seq_start_ms: f64) -> PrimaryTimelineClip {
        PrimaryTimelineClip {
            path: PathBuf::from(path),
            media_in_s: in_s,
            media_out_s: out_s,
            seq_start_ms,
            orientation: Default::default(),
        }
    }

    #[test]
    fn slice_keeps_clip_fully_inside_range() {
        // Clip 0..2 s on sequence; range 500..1500 → one clip, media 0.5..1.5 s, rebased to 0.
        let c = timeline_clip("/a.mp4", 0.0, 2.0, 0.0);
        let out = slice_clips_to_range_ms(&[c], (500.0, 1500.0));
        assert_eq!(out.len(), 1);
        assert!((out[0].media_in_s - 0.5).abs() < 1e-6);
        assert!((out[0].media_out_s - 1.5).abs() < 1e-6);
        assert!((out[0].seq_start_ms - 0.0).abs() < 1e-6);
        assert!((sequence_duration_ms(&out) - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn slice_drops_clips_outside_range() {
        // Two 2 s clips (sequence 0..2 s, 2..4 s). Range 2500..3500 should keep only the second,
        // trimmed to its middle 1 s (media 0.5..1.5 s), rebased to sequence start 0.
        let a = timeline_clip("/a.mp4", 0.0, 2.0, 0.0);
        let b = timeline_clip("/b.mp4", 0.0, 2.0, 2000.0);
        let out = slice_clips_to_range_ms(&[a, b], (2500.0, 3500.0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, PathBuf::from("/b.mp4"));
        assert!((out[0].media_in_s - 0.5).abs() < 1e-6);
        assert!((out[0].media_out_s - 1.5).abs() < 1e-6);
        assert!((out[0].seq_start_ms - 0.0).abs() < 1e-6);
    }

    #[test]
    fn slice_spans_two_clips_trims_edges_and_rebases() {
        // Clips 0..2 s and 2..5 s on sequence. Range 1500..3500 should keep the tail of the first
        // (media 1.5..2.0 = 0.5 s) followed by the head of the second (media 0.0..1.5 = 1.5 s).
        let a = timeline_clip("/a.mp4", 0.0, 2.0, 0.0);
        let b = timeline_clip("/b.mp4", 0.0, 3.0, 2000.0);
        let out = slice_clips_to_range_ms(&[a, b], (1500.0, 3500.0));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].path, PathBuf::from("/a.mp4"));
        assert!((out[0].media_in_s - 1.5).abs() < 1e-6);
        assert!((out[0].media_out_s - 2.0).abs() < 1e-6);
        assert!((out[0].seq_start_ms - 0.0).abs() < 1e-6);
        assert_eq!(out[1].path, PathBuf::from("/b.mp4"));
        assert!((out[1].media_in_s - 0.0).abs() < 1e-6);
        assert!((out[1].media_out_s - 1.5).abs() < 1e-6);
        assert!((out[1].seq_start_ms - 500.0).abs() < 1e-6);
        assert!((sequence_duration_ms(&out) - 2000.0).abs() < 1e-3);
    }

    #[test]
    fn slice_empty_when_range_outside_all_clips() {
        let a = timeline_clip("/a.mp4", 0.0, 2.0, 0.0);
        let out = slice_clips_to_range_ms(&[a], (5000.0, 6000.0));
        assert!(out.is_empty());
    }

    #[test]
    fn slice_empty_when_range_degenerate() {
        let a = timeline_clip("/a.mp4", 0.0, 2.0, 0.0);
        assert!(slice_clips_to_range_ms(std::slice::from_ref(&a), (500.0, 500.0)).is_empty());
        assert!(slice_clips_to_range_ms(&[a], (1500.0, 500.0)).is_empty());
    }

    #[test]
    fn slice_handles_trimmed_source_clip() {
        // Source media_in 10..20 s (clip already trimmed); sequence span 10 s starting at 0.
        // Range 2000..5000 ms → keep 3 s in source, media 12..15 s.
        let c = timeline_clip("/trimmed.mp4", 10.0, 20.0, 0.0);
        let out = slice_clips_to_range_ms(&[c], (2000.0, 5000.0));
        assert_eq!(out.len(), 1);
        assert!((out[0].media_in_s - 12.0).abs() < 1e-6);
        assert!((out[0].media_out_s - 15.0).abs() < 1e-6);
    }
}
