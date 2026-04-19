//! Timeline “filmstrip” chips: clip **file names** and proportional widths from sequence duration.
//! For **single-media** opens (not a `.reel` file), lane counts follow container streams from probe metadata.
//! When the file has only one video and/or audio stream, each corresponding lane shows a **single**
//! full-width chip (ignoring per-clip splits on the timeline) so the strip matches the media layout.

use reel_core::project::{Project, TrackKind};

use crate::TlChip;

const MAX_ROWS: usize = 4;

fn clip_label(c: &reel_core::project::Clip) -> String {
    c.source_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "clip".into())
}

fn chips_for_track(p: &Project, track_idx: usize, kind: TrackKind) -> Vec<TlChip> {
    let tracks: Vec<_> = p
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == kind)
        .collect();
    let Some((_, track)) = tracks.get(track_idx) else {
        return Vec::new();
    };
    let is_video = kind == TrackKind::Video;
    let is_subtitle = kind == TrackKind::Subtitle;
    // (d_ms, label, clip_id, begin_ms, end_ms, source_duration_ms)
    let mut spans: Vec<(f64, String, String, i32, i32, i32)> = Vec::new();
    for id in &track.clip_ids {
        if let Some(c) = p.clips.iter().find(|c| c.id == *id) {
            let d_ms = (c.out_point - c.in_point) * 1000.0;
            if d_ms > 0.0 {
                let begin_ms = (c.in_point * 1000.0).round() as i32;
                let end_ms = (c.out_point * 1000.0).round() as i32;
                let src_ms = (c.metadata.duration_seconds * 1000.0).round() as i32;
                spans.push((d_ms, clip_label(c), c.id.to_string(), begin_ms, end_ms, src_ms));
            }
        }
    }
    let total: f64 = spans.iter().map(|(d, ..)| *d).sum();
    if spans.is_empty() {
        return Vec::new();
    }
    let total = total.max(1.0e-9);
    spans
        .into_iter()
        .enumerate()
        .map(
            |(i, (d_ms, label, clip_id, begin_ms, end_ms, src_ms))| TlChip {
                label: label.into(),
                width_weight: (d_ms / total) as f32,
                chip_idx: i as i32,
                is_video,
                is_subtitle,
                clip_id: clip_id.into(),
                begin_ms,
                end_ms,
                source_duration_ms: src_ms,
                // Waveform / thumbnail start empty; the AssetCache layer
                // (see `asset_cache.rs`) fills them in asynchronously and
                // the chip list is rebuilt when a job completes. Until
                // then `waveform_ready == false` keeps the placeholder
                // stripe visible so the chip shows up instantly.
                waveform: slint::Image::default(),
                waveform_ready: false,
            },
        )
        .collect()
}

/// First clip on the primary (first) video track — used for single-media stream lanes.
fn primary_video_clip(p: &Project) -> Option<&reel_core::project::Clip> {
    let tid = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)?
        .clip_ids
        .first()?;
    p.clips.iter().find(|c| c.id == *tid)
}

fn synthetic_full_width_chip(label: String, is_video: bool) -> Vec<TlChip> {
    // Synthetic chips cover single-media-mode container-stream lanes, which
    // are **never** subtitle lanes today (subtitles always come from project
    // `TrackKind::Subtitle` rows), so `is_subtitle: false` is correct here.
    // Empty `clip_id` signals "no backing clip" — the trim-drag handles in
    // Slint gate on this, since synthetic chips don't correspond to an
    // editable `reel_core::project::Clip`.
    vec![TlChip {
        label: label.into(),
        width_weight: 1.0,
        chip_idx: 0,
        is_video,
        is_subtitle: false,
        clip_id: "".into(),
        begin_ms: 0,
        end_ms: 0,
        source_duration_ms: 0,
        // Synthetic single-media chips have no backing clip, so they can't
        // be keyed in the AssetCache. Leave `waveform_ready: false`; the
        // FilmstripLane renders them as flat color (no placeholder shown
        // for synthetic chips — they represent a whole stream, not a
        // decodeable segment).
        waveform: slint::Image::default(),
        waveform_ready: false,
    }]
}

/// Aggregated chip rows + how many lanes to show vs how many are real project tracks (for delete affordances).
pub(crate) struct TimelineChipSync {
    pub video: [Vec<TlChip>; MAX_ROWS],
    pub video_display_n: i32,
    pub video_project_n: i32,
    pub audio: [Vec<TlChip>; MAX_ROWS],
    pub audio_display_n: i32,
    pub audio_project_n: i32,
    pub subtitle: [Vec<TlChip>; MAX_ROWS],
    pub subtitle_display_n: i32,
    pub subtitle_project_n: i32,
}

pub(crate) fn timeline_chip_sync(
    p: &Project,
    opened_from_project_document: bool,
) -> TimelineChipSync {
    let vn_proj = p
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .count()
        .min(MAX_ROWS);
    let an_proj = p
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .count()
        .min(MAX_ROWS);
    let sn_proj = p
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Subtitle)
        .count()
        .min(MAX_ROWS);

    if opened_from_project_document {
        let mut video = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let mut audio = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let mut subtitle = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for (i, row) in video.iter_mut().enumerate().take(vn_proj) {
            *row = chips_for_track(p, i, TrackKind::Video);
        }
        for (i, row) in audio.iter_mut().enumerate().take(an_proj) {
            *row = chips_for_track(p, i, TrackKind::Audio);
        }
        for (i, row) in subtitle.iter_mut().enumerate().take(sn_proj) {
            *row = chips_for_track(p, i, TrackKind::Subtitle);
        }
        let vn = vn_proj as i32;
        let an = an_proj as i32;
        let sn = sn_proj as i32;
        return TimelineChipSync {
            video,
            video_display_n: vn,
            video_project_n: vn,
            audio,
            audio_display_n: an,
            audio_project_n: an,
            subtitle,
            subtitle_display_n: sn,
            subtitle_project_n: sn,
        };
    }

    // Single-media: merge project tracks with container stream counts from primary clip metadata.
    let Some(pc) = primary_video_clip(p) else {
        return TimelineChipSync {
            video: Default::default(),
            video_display_n: 0,
            video_project_n: 0,
            audio: Default::default(),
            audio_display_n: 0,
            audio_project_n: 0,
            subtitle: Default::default(),
            subtitle_display_n: 0,
            subtitle_project_n: 0,
        };
    };
    let md = &pc.metadata;
    let vs = md.video_streams_display().min(MAX_ROWS as u32) as usize;
    let aus = md.audio_streams_display().min(MAX_ROWS as u32) as usize;
    let subs = md.subtitle_streams_display().min(MAX_ROWS as u32) as usize;

    let vn_disp = vn_proj.max(vs).min(MAX_ROWS);
    let an_disp = an_proj.max(aus).min(MAX_ROWS);
    let sn_disp = sn_proj.max(subs).min(MAX_ROWS);

    let base = clip_label(pc);
    let base_label = base.clone();

    let mut video = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for (i, row) in video.iter_mut().enumerate().take(vn_disp) {
        if i < vn_proj {
            *row = if vs <= 1 && i == 0 {
                synthetic_full_width_chip(base_label.clone(), true)
            } else {
                chips_for_track(p, i, TrackKind::Video)
            };
        }
    }
    let mut audio = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for (i, row) in audio.iter_mut().enumerate().take(an_disp) {
        if i < an_proj {
            *row = if aus <= 1 && i == 0 {
                synthetic_full_width_chip(format!("{base} (audio)"), false)
            } else {
                chips_for_track(p, i, TrackKind::Audio)
            };
        } else {
            let label = if aus <= 1 {
                format!("{base} (audio)")
            } else {
                format!("{base} (audio {})", i + 1)
            };
            *row = synthetic_full_width_chip(label, false);
        }
    }

    let mut subtitle = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for (i, row) in subtitle.iter_mut().enumerate().take(sn_disp) {
        if i < sn_proj {
            *row = if subs <= 1 && i == 0 {
                synthetic_full_width_chip(format!("{base} (subtitles)"), false)
            } else {
                chips_for_track(p, i, TrackKind::Subtitle)
            };
        } else {
            let label = if subs <= 1 {
                format!("{base} (subtitles)")
            } else {
                format!("{base} (subtitles {})", i + 1)
            };
            *row = synthetic_full_width_chip(label, false);
        }
    }

    TimelineChipSync {
        video,
        video_display_n: vn_disp as i32,
        video_project_n: vn_proj as i32,
        audio,
        audio_display_n: an_disp as i32,
        audio_project_n: an_proj as i32,
        subtitle,
        subtitle_display_n: sn_disp as i32,
        subtitle_project_n: sn_proj as i32,
    }
}

/// Returns up to [`MAX_ROWS`] chip lists for video tracks, and the number of video tracks in the project (capped).
#[cfg(test)]
pub(crate) fn video_chip_rows(p: &Project) -> ([Vec<TlChip>; MAX_ROWS], i32) {
    let s = timeline_chip_sync(p, true);
    (s.video, s.video_display_n)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use reel_core::project::{Track, TrackKind};
    use uuid::Uuid;

    use crate::project_io::project_from_media_path;

    use super::*;

    #[test]
    fn single_clip_project_has_one_video_chip() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../reel-core/tests/fixtures/tiny_h264_aac.mp4");
        assert!(path.is_file(), "fixture {}", path.display());
        let p = project_from_media_path(&path).expect("probe");
        let (rows, n) = video_chip_rows(&p);
        assert_eq!(n, 1);
        assert_eq!(rows[0].len(), 1);
        assert!(rows[0][0].label.as_str().contains("tiny"));
    }

    #[test]
    fn single_media_mode_shows_embedded_audio_lane_from_probe() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../reel-core/tests/fixtures/tiny_h264_aac.mp4");
        assert!(path.is_file(), "fixture {}", path.display());
        let p = project_from_media_path(&path).expect("probe");
        let s = timeline_chip_sync(&p, false);
        assert!(
            s.audio_display_n >= 1,
            "single-media UI should list at least one audio lane when the file has audio"
        );
        assert!(
            !s.audio[0].is_empty(),
            "embedded audio lane should have a chip"
        );
        assert_eq!(s.audio_project_n, 0);
    }

    /// **`.reel` / project-document** mode: subtitle rows come only from `TrackKind::Subtitle` (no container merge).
    #[test]
    fn project_document_mode_lists_subtitle_tracks() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../reel-core/tests/fixtures/tiny_h264_aac.mp4");
        assert!(path.is_file(), "fixture {}", path.display());
        let mut p = project_from_media_path(&path).expect("probe");
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Subtitle,
            clip_ids: Vec::new(),
            gain_db: 0.0,
            extensions: Default::default(),
        });
        let s = timeline_chip_sync(&p, true);
        assert_eq!(s.subtitle_project_n, 1);
        assert_eq!(s.subtitle_display_n, 1);
    }

    /// Single-media with one video stream: timeline strip shows one chip even if the edit has multiple clips.
    #[test]
    fn single_media_one_video_stream_collapses_timeline_clips_to_one_chip() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../reel-core/tests/fixtures/tiny_h264_aac.mp4");
        assert!(path.is_file(), "fixture {}", path.display());
        let mut p = project_from_media_path(&path).expect("probe");
        assert_eq!(p.clips.len(), 1);
        let c0 = &p.clips[0];
        let dur = c0.out_point - c0.in_point;
        assert!(dur > 0.0, "fixture duration");
        let mid = c0.in_point + dur * 0.5;
        let id2 = Uuid::new_v4();
        let mut second = c0.clone();
        second.id = id2;
        second.in_point = mid;
        p.clips[0].out_point = mid;
        p.clips.push(second);
        let vtrack = p
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Video)
            .expect("video track");
        vtrack.clip_ids.push(id2);

        let s = timeline_chip_sync(&p, false);
        assert_eq!(
            s.video[0].len(),
            1,
            "one container video stream => one filmstrip chip"
        );
        assert_eq!(s.video[0][0].width_weight, 1.0);
    }

    /// U2-c: real (project-backed) chips expose a non-empty `clip_id` plus
    /// `begin_ms` / `end_ms` / `source_duration_ms` so the Slint trim handles
    /// can clamp against source bounds. Synthetic single-media container-stream
    /// chips leave `clip_id` empty — handles are gated on that.
    #[test]
    fn chip_surfaces_clip_id_and_source_bounds_for_project_backed_clip() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../reel-core/tests/fixtures/tiny_h264_aac.mp4");
        assert!(path.is_file(), "fixture {}", path.display());
        let p = project_from_media_path(&path).expect("probe");
        // `.reel` / project-document mode uses real clip chips.
        let s = timeline_chip_sync(&p, true);
        let chip = s.video[0].first().expect("primary video chip");
        assert!(!chip.clip_id.is_empty(), "real clip must expose a uuid");
        assert_eq!(chip.begin_ms, 0, "fresh probe in_point is 0 s");
        assert!(chip.end_ms > 0, "end_ms must come from out_point");
        // Fixture probes a known duration — source_duration_ms mirrors end_ms
        // for a fresh un-trimmed clip.
        assert_eq!(chip.end_ms, chip.source_duration_ms);
    }

    /// U2-c: synthetic full-width chips for single-media container-stream lanes
    /// must **not** expose a `clip_id` — they have no backing `Clip` in the
    /// project, so the Slint edge handles stay hidden.
    #[test]
    fn synthetic_container_stream_chip_has_empty_clip_id() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../reel-core/tests/fixtures/tiny_h264_aac.mp4");
        assert!(path.is_file(), "fixture {}", path.display());
        let p = project_from_media_path(&path).expect("probe");
        let s = timeline_chip_sync(&p, false);
        // Single-media mode with 1 video stream → synthetic full-width chip.
        let vchip = s.video[0].first().expect("synthetic video chip");
        assert!(
            vchip.clip_id.is_empty(),
            "synthetic single-media chip must not expose clip_id"
        );
        // And the audio lane too — synthetic "(audio)" chip has no backing clip.
        let achip = s.audio[0].first().expect("synthetic audio chip");
        assert!(achip.clip_id.is_empty());
    }

    /// **Single-media** mode: project subtitle lanes count toward display `max(project, container streams)`.
    #[test]
    fn single_media_merges_subtitle_project_count_with_probe_streams() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../reel-core/tests/fixtures/tiny_h264_aac.mp4");
        assert!(path.is_file(), "fixture {}", path.display());
        let mut p = project_from_media_path(&path).expect("probe");
        p.tracks.push(Track {
            id: Uuid::new_v4(),
            kind: TrackKind::Subtitle,
            clip_ids: Vec::new(),
            gain_db: 0.0,
            extensions: Default::default(),
        });
        let s = timeline_chip_sync(&p, false);
        assert_eq!(s.subtitle_project_n, 1);
        // Fixture has no subtitle streams; display count still shows one project lane.
        assert_eq!(s.subtitle_display_n, 1);
    }
}
