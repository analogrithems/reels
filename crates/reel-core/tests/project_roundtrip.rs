//! Round-trip `Project` through `serde_json` and through the on-disk golden
//! file. Both paths must produce the same value.

use std::path::PathBuf;

use pretty_assertions::assert_eq;
use reel_core::media::{MediaMetadata, VideoStreamInfo};
use reel_core::project::SCHEMA_VERSION;
use reel_core::{Clip, Project, Track, TrackKind};
use uuid::Uuid;

fn sample_project() -> Project {
    let clip_id = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0001);
    let track_id = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0002);
    Project {
        schema_version: SCHEMA_VERSION,
        name: "Golden".into(),
        path: None,
        clips: vec![Clip {
            id: clip_id,
            source_path: PathBuf::from("/media/footage.mp4"),
            metadata: MediaMetadata {
                path: PathBuf::from("/media/footage.mp4"),
                duration_seconds: 12.5,
                container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
                video: Some(VideoStreamInfo {
                    codec: "h264".into(),
                    width: 1280,
                    height: 720,
                    frame_rate: 30.0,
                    pixel_format: "YUV420P".into(),
                    rotation: 0,
                }),
                audio: None,
                audio_disabled: true, // exercise the graceful-degradation branch
            },
            in_point: 1.0,
            out_point: 10.0,
        }],
        tracks: vec![Track {
            id: track_id,
            kind: TrackKind::Video,
            clip_ids: vec![clip_id],
        }],
        created_at: "2026-04-16T12:00:00Z".into(),
        modified_at: "2026-04-16T12:00:00Z".into(),
    }
}

#[test]
fn project_roundtrip_via_serde_json() {
    let p = sample_project();
    let s = serde_json::to_string_pretty(&p).unwrap();
    let back: Project = serde_json::from_str(&s).unwrap();
    assert_eq!(p, back);
}

#[test]
fn project_matches_golden_snapshot() {
    let p = sample_project();
    insta::assert_json_snapshot!(p);
}

#[test]
fn project_rejects_unknown_fields() {
    let bad = r#"{
        "schema_version": 1,
        "name": "X",
        "clips": [],
        "tracks": [],
        "created_at": "2026-04-16T00:00:00Z",
        "modified_at": "2026-04-16T00:00:00Z",
        "extra_typo_field": true
    }"#;
    let r: Result<Project, _> = serde_json::from_str(bad);
    assert!(r.is_err(), "deny_unknown_fields should reject typos");
}
