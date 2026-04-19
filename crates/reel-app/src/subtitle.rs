//! Subtitle preview overlay: map a timeline sequence time to the active cue
//! text on any `TrackKind::Subtitle` lane.
//!
//! Today's rules (MVP — `docs/phase-status.md` "Subtitles — live preview"):
//!
//! - **First match wins** across subtitle lanes. A project with two subtitle
//!   tracks (e.g. EN + ES) will show whichever lane's cue we hit first. Full
//!   multi-lane selection (Edit → Subtitle Track, parallel to Edit → Audio
//!   Track) is a follow-up; until then, reorder the tracks to pick the
//!   language.
//! - Parsed cues are cached by source path so the 10 Hz preview-tick lookup
//!   only pays parse cost once per file. Cache is keyed on `PathBuf` (not
//!   `Clip.id`) — re-inserting the same file on a different clip reuses the
//!   already-parsed cues.
//! - Cache errors (missing file, I/O failure, unparseable body) fall through
//!   to an empty vec so a corrupt subtitle file doesn't crash playback. The
//!   probe step at **Insert Subtitle…** time already validates the format;
//!   this path is best-effort.
//! - Cue-at-timestamp matching is **inclusive on start, exclusive on end**
//!   (matches [`reel_core::find_srt_cue_at_seconds`] and standard subtitle
//!   renderer behaviour — avoids one-frame overlap on boundary seeks).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use reel_core::{find_srt_cue_at_seconds, parse_subtitle_file, Project, SrtCue, TrackKind};

/// Epsilon for sequence-ms boundaries — mirrors `timeline::SEQ_MS_EPS` and
/// `session::SEQ_MS_EPS` so subtitle span math lines up with clip span math
/// exactly (a cue must be active on the same sub-ms tick the audio/video
/// concat paths treat as "inside" the clip).
const SEQ_MS_EPS: f64 = 1e-3;

/// Lazy per-path cache of parsed SRT/WebVTT cues.
///
/// Interior mutability (`RefCell`) so lookups can populate the cache without
/// demanding `&mut` access — the subtitle refresh timer fires from UI
/// callbacks that only hold shared references. Wrap in `Rc` at the
/// application level to share one cache across every handler that wants to
/// look up cues or invalidate on project swap.
#[derive(Default)]
pub struct SubtitleCueCache {
    map: RefCell<HashMap<PathBuf, Arc<Vec<SrtCue>>>>,
}

impl SubtitleCueCache {
    /// Return parsed cues for `path`, loading from disk on first access.
    /// Parse / I/O failures cache an empty vec so we don't retry a broken
    /// file every tick.
    fn get_or_load(&self, path: &Path) -> Arc<Vec<SrtCue>> {
        if let Some(existing) = self.map.borrow().get(path) {
            return Arc::clone(existing);
        }
        // Extension-dispatched: `.ttml` / `.dfxp` / `.xml` routes to the
        // TTML parser; everything else through the SRT/WebVTT parser (both
        // of which tolerate malformed bodies). See
        // `reel_core::media::parse_subtitle_file`.
        let cues = parse_subtitle_file(path).unwrap_or_default();
        let arc = Arc::new(cues);
        self.map
            .borrow_mut()
            .insert(path.to_path_buf(), Arc::clone(&arc));
        arc
    }

    /// Drop every cached entry. Called on **File → Close** / project swap so a
    /// freshly-opened project doesn't serve cues from a prior file that
    /// happened to share a path.
    pub fn clear(&self) {
        self.map.borrow_mut().clear();
    }
}

/// Active subtitle cue text at `seq_ms` on the concatenated timeline, or
/// `None` when no subtitle track has a cue covering that instant (includes
/// "inside a subtitle clip but between cues" — the overlay should blank out
/// during those gaps, not hold the prior cue).
///
/// Walks `TrackKind::Subtitle` tracks in project order; the first lane with a
/// clip spanning `seq_ms` whose source file has an active cue wins. Empty
/// cue text is treated as no match (some authoring tools emit blank
/// placeholder cues — showing an empty black bar is worse than hiding the
/// overlay).
pub fn subtitle_text_at_seq_ms(
    project: &Project,
    seq_ms: f64,
    cache: &SubtitleCueCache,
) -> Option<String> {
    if seq_ms < -SEQ_MS_EPS {
        return None;
    }
    for track in project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Subtitle)
    {
        let mut t_ms = 0.0_f64;
        for cid in &track.clip_ids {
            let Some(clip) = project.clips.iter().find(|c| c.id == *cid) else {
                continue;
            };
            let dur_ms = (clip.out_point - clip.in_point) * 1000.0;
            if dur_ms <= SEQ_MS_EPS {
                continue;
            }
            let span_start = t_ms;
            let span_end = t_ms + dur_ms;
            t_ms = span_end;
            // Inclusive on start, exclusive on end — same rule cues use.
            if seq_ms + SEQ_MS_EPS < span_start || seq_ms >= span_end + SEQ_MS_EPS {
                continue;
            }
            // Map timeline seq-time → source-file seconds. `in_point` is the
            // clip's starting offset into the subtitle file itself (users can
            // trim the subtitle clip, which means the first rendered cue is
            // `in_point` seconds into the file, not cue 0).
            let local_ms = seq_ms - span_start;
            let source_seconds = clip.in_point + local_ms / 1000.0;
            let cues = cache.get_or_load(&clip.source_path);
            if let Some(cue) = find_srt_cue_at_seconds(&cues, source_seconds) {
                let text = cue.text.trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::{Clip, MediaMetadata, Project, Track};
    use std::io::Write;
    use uuid::Uuid;

    fn subtitle_clip(path: PathBuf, dur_s: f64, in_s: f64) -> Clip {
        Clip {
            id: Uuid::new_v4(),
            source_path: path.clone(),
            metadata: MediaMetadata {
                path,
                duration_seconds: dur_s + in_s,
                container: "srt".into(),
                video: None,
                audio: None,
                audio_disabled: true,
                video_stream_count: 0,
                audio_stream_count: 0,
                subtitle_stream_count: 1,
                audio_streams: Vec::new(),
            },
            in_point: in_s,
            out_point: in_s + dur_s,
            orientation: Default::default(),
            scale: Default::default(),
            audio_mute: false,
            audio_stream_index: None,
            extensions: Default::default(),
        }
    }

    fn write_tmp_srt(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn returns_cue_text_when_seq_ms_is_inside_a_cue() {
        let dir = tempfile::tempdir().unwrap();
        // Cue 1.0..3.5 ("Hello") and 4.0..7.25 ("World") in source seconds.
        let srt = write_tmp_srt(
            &dir,
            "demo.srt",
            "1\n00:00:01,000 --> 00:00:03,500\nHello\n\n\
             2\n00:00:04,000 --> 00:00:07,250\nWorld\n",
        );
        let mut p = Project::new("t");
        let clip = subtitle_clip(srt, 10.0, 0.0);
        let clip_id = clip.id;
        p.clips.push(clip);
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Subtitle,
            clip_ids: vec![clip_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        let cache = SubtitleCueCache::default();
        // seq 2000 ms → source 2.0 s → inside cue 1 ("Hello").
        assert_eq!(
            subtitle_text_at_seq_ms(&p, 2000.0, &cache).as_deref(),
            Some("Hello")
        );
        // seq 5000 ms → source 5.0 s → inside cue 2 ("World").
        assert_eq!(
            subtitle_text_at_seq_ms(&p, 5000.0, &cache).as_deref(),
            Some("World")
        );
    }

    #[test]
    fn returns_none_in_inter_cue_gap_and_past_clip_end() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write_tmp_srt(
            &dir,
            "gap.srt",
            "1\n00:00:01,000 --> 00:00:02,000\nA\n\n\
             2\n00:00:05,000 --> 00:00:06,000\nB\n",
        );
        let mut p = Project::new("t");
        let clip = subtitle_clip(srt, 8.0, 0.0);
        let clip_id = clip.id;
        p.clips.push(clip);
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Subtitle,
            clip_ids: vec![clip_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        let cache = SubtitleCueCache::default();
        // Gap between cue A (1–2) and cue B (5–6).
        assert!(subtitle_text_at_seq_ms(&p, 3000.0, &cache).is_none());
        // Past the end of the subtitle clip (8 s).
        assert!(subtitle_text_at_seq_ms(&p, 10_000.0, &cache).is_none());
        // Before the first cue.
        assert!(subtitle_text_at_seq_ms(&p, 500.0, &cache).is_none());
    }

    #[test]
    fn respects_clip_in_point_when_mapping_seq_to_source() {
        // Trim the subtitle clip so the first rendered cue is 2 s into the
        // file. seq_ms=0 should map to source=2.0 s (inside cue "Hello").
        let dir = tempfile::tempdir().unwrap();
        let srt = write_tmp_srt(
            &dir,
            "trimmed.srt",
            "1\n00:00:01,500 --> 00:00:03,000\nHello\n",
        );
        let mut p = Project::new("t");
        // in_point = 2 s → seq 0 maps to source 2.0 s which is inside cue 1 (1.5..3.0).
        let clip = subtitle_clip(srt, 1.0, 2.0);
        let clip_id = clip.id;
        p.clips.push(clip);
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Subtitle,
            clip_ids: vec![clip_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        let cache = SubtitleCueCache::default();
        assert_eq!(
            subtitle_text_at_seq_ms(&p, 0.0, &cache).as_deref(),
            Some("Hello")
        );
    }

    #[test]
    fn missing_subtitle_file_returns_none_without_panicking() {
        // Cache falls back to an empty cue list on I/O failure so a broken
        // subtitle file (moved / deleted since insert) doesn't crash preview.
        let mut p = Project::new("t");
        let clip = subtitle_clip(
            PathBuf::from("/definitely/does/not/exist-reel-subtitles.srt"),
            5.0,
            0.0,
        );
        let clip_id = clip.id;
        p.clips.push(clip);
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Subtitle,
            clip_ids: vec![clip_id],
            gain_db: 0.0,
            extensions: Default::default(),
        });
        let cache = SubtitleCueCache::default();
        assert!(subtitle_text_at_seq_ms(&p, 1000.0, &cache).is_none());
    }

    #[test]
    fn returns_none_when_project_has_no_subtitle_tracks() {
        let p = Project::new("t");
        let cache = SubtitleCueCache::default();
        assert!(subtitle_text_at_seq_ms(&p, 0.0, &cache).is_none());
        assert!(subtitle_text_at_seq_ms(&p, 9_999.0, &cache).is_none());
    }
}
