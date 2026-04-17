//! Primary video track: concatenated sequence time ↔ media files (U2 sequence preview).

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use reel_core::{Project, TrackKind};

/// Epsilon for float boundaries (matches session insert math).
const SEQ_MS_EPS: f64 = 1e-3;

#[derive(Debug, Clone)]
pub(crate) struct PrimaryTimelineClip {
    pub path: PathBuf,
    pub media_in_s: f64,
    pub media_out_s: f64,
    pub seq_start_ms: f64,
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
}

impl TimelineSegment {
    fn from_clip(c: &PrimaryTimelineClip) -> Self {
        Self {
            path: c.path.clone(),
            media_in_ms: (c.media_in_s * 1000.0).round() as u64,
            media_out_ms: (c.media_out_s * 1000.0).round() as u64,
            seq_start_ms: c.seq_start_ms.round() as u64,
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

pub(crate) fn resolve_for_project(p: &Project, sequence_ms: f64) -> Option<(PathBuf, u64)> {
    let clips = clips_from_project(p)?;
    resolve_sequence_media_ms(&clips, sequence_ms)
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
            },
            in_point: 0.0,
            out_point: dur,
            extensions: Default::default(),
        }
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
}
