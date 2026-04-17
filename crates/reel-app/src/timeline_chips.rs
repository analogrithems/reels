//! Timeline “filmstrip” chips: clip **file names** and proportional widths from sequence duration.
//! For **single-media** opens (not a `.reel` file), lane counts follow container streams from probe metadata.

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

fn chips_for_track(p: &Project, track_idx: usize, is_video: bool) -> Vec<TlChip> {
    let kind = if is_video {
        TrackKind::Video
    } else {
        TrackKind::Audio
    };
    let tracks: Vec<_> = p
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == kind)
        .collect();
    let Some((_, track)) = tracks.get(track_idx) else {
        return Vec::new();
    };
    let mut spans: Vec<(f64, String)> = Vec::new();
    for id in &track.clip_ids {
        if let Some(c) = p.clips.iter().find(|c| c.id == *id) {
            let d_ms = (c.out_point - c.in_point) * 1000.0;
            if d_ms > 0.0 {
                spans.push((d_ms, clip_label(c)));
            }
        }
    }
    let total: f64 = spans.iter().map(|(d, _)| *d).sum();
    if spans.is_empty() {
        return Vec::new();
    }
    let total = total.max(1.0e-9);
    spans
        .into_iter()
        .enumerate()
        .map(|(i, (d_ms, label))| TlChip {
            label: label.into(),
            width_weight: (d_ms / total) as f32,
            chip_idx: i as i32,
            is_video,
        })
        .collect()
}

/// First clip on the primary (first) video track — used for single-media stream lanes.
fn primary_video_clip<'a>(p: &'a Project) -> Option<&'a reel_core::project::Clip> {
    let tid = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)?
        .clip_ids
        .first()?;
    p.clips.iter().find(|c| c.id == *tid)
}

fn synthetic_full_width_chip(label: String, is_video: bool) -> Vec<TlChip> {
    vec![TlChip {
        label: label.into(),
        width_weight: 1.0,
        chip_idx: 0,
        is_video,
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
}

pub(crate) fn timeline_chip_sync(p: &Project, opened_from_project_document: bool) -> TimelineChipSync {
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

    if opened_from_project_document {
        let mut video = [
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ];
        let mut audio = [
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ];
        for i in 0..vn_proj {
            video[i] = chips_for_track(p, i, true);
        }
        for i in 0..an_proj {
            audio[i] = chips_for_track(p, i, false);
        }
        let vn = vn_proj as i32;
        let an = an_proj as i32;
        return TimelineChipSync {
            video,
            video_display_n: vn,
            video_project_n: vn,
            audio,
            audio_display_n: an,
            audio_project_n: an,
            subtitle: Default::default(),
            subtitle_display_n: 0,
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
        };
    };
    let md = &pc.metadata;
    let vs = md.video_streams_display().min(MAX_ROWS as u32) as usize;
    let aus = md.audio_streams_display().min(MAX_ROWS as u32) as usize;
    let subs = md.subtitle_streams_display().min(MAX_ROWS as u32) as usize;

    let vn_disp = vn_proj.max(vs).min(MAX_ROWS);
    let an_disp = an_proj.max(aus).min(MAX_ROWS);

    let mut video = [
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ];
    for i in 0..vn_disp {
        if i < vn_proj {
            video[i] = chips_for_track(p, i, true);
        }
    }

    let base = clip_label(pc);
    let mut audio = [
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ];
    for i in 0..an_disp {
        if i < an_proj {
            audio[i] = chips_for_track(p, i, false);
        } else {
            let label = if aus <= 1 {
                format!("{base} (audio)")
            } else {
                format!("{base} (audio {})", i + 1)
            };
            audio[i] = synthetic_full_width_chip(label, false);
        }
    }

    let mut subtitle = [
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ];
    for i in 0..subs {
        let label = if subs <= 1 {
            format!("{base} (subtitles)")
        } else {
            format!("{base} (subtitles {})", i + 1)
        };
        subtitle[i] = synthetic_full_width_chip(label, false);
    }

    TimelineChipSync {
        video,
        video_display_n: vn_disp as i32,
        video_project_n: vn_proj as i32,
        audio,
        audio_display_n: an_disp as i32,
        audio_project_n: an_proj as i32,
        subtitle,
        subtitle_display_n: subs as i32,
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
}
