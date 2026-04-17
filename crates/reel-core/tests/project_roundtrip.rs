//! Round-trip `Project` through `serde_json` and through the on-disk golden
//! file. Both paths must produce the same value.

use std::path::PathBuf;

use pretty_assertions::assert_eq;
use reel_core::media::{MediaMetadata, VideoStreamInfo};
use reel_core::project::{migrate, ClipOrientation, SCHEMA_VERSION};
use reel_core::{Clip, Project, Track, TrackKind};
use serde_json::json;
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
            orientation: Default::default(),
            extensions: Default::default(),
        }],
        tracks: vec![Track {
            id: track_id,
            kind: TrackKind::Video,
            clip_ids: vec![clip_id],
            extensions: Default::default(),
        }],
        created_at: "2026-04-16T12:00:00Z".into(),
        modified_at: "2026-04-16T12:00:00Z".into(),
        extensions: Default::default(),
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
fn project_preserves_unknown_top_level_keys() {
    let raw = r#"{
        "schema_version": 2,
        "name": "X",
        "clips": [],
        "tracks": [],
        "created_at": "2026-04-16T00:00:00Z",
        "modified_at": "2026-04-16T00:00:00Z",
        "future_filter_defaults": { "facefusion": { "enabled": true } }
    }"#;
    let p: Project = serde_json::from_str(raw).unwrap();
    assert_eq!(
        p.extensions.get("future_filter_defaults"),
        Some(&json!({ "facefusion": { "enabled": true } }))
    );
    let s = serde_json::to_string(&p).unwrap();
    let back: Project = serde_json::from_str(&s).unwrap();
    assert_eq!(p, back);
}

#[test]
fn clip_preserves_extension_keys() {
    let raw = r#"{
        "schema_version": 2,
        "name": "E",
        "clips": [{
            "id": "00000000-0000-0000-0000-000000000001",
            "source_path": "/a.mp4",
            "metadata": {
                "path": "/a.mp4",
                "duration_seconds": 1.0,
                "container": "mp4",
                "video": null,
                "audio": null,
                "audio_disabled": false
            },
            "in_point": 0.0,
            "out_point": 1.0,
            "filters": { "rvm": { "strength": 0.9 } }
        }],
        "tracks": [],
        "created_at": "2026-04-16T12:00:00Z",
        "modified_at": "2026-04-16T12:00:00Z"
    }"#;
    let p: Project = serde_json::from_str(raw).unwrap();
    assert_eq!(
        p.clips[0].extensions.get("filters"),
        Some(&json!({ "rvm": { "strength": 0.9 } }))
    );
}

#[test]
fn clip_orientation_roundtrips_non_identity() {
    let mut p = sample_project();
    p.clips[0].orientation = ClipOrientation {
        rotation_quarter_turns: 1,
        flip_h: true,
        flip_v: false,
    };
    let s = serde_json::to_string_pretty(&p).unwrap();
    assert!(
        s.contains("\"orientation\""),
        "non-identity orientation must serialize"
    );
    let back: Project = serde_json::from_str(&s).unwrap();
    assert_eq!(p, back);
}

#[test]
fn clip_orientation_default_is_omitted_from_json() {
    let p = sample_project();
    let s = serde_json::to_string(&p).unwrap();
    assert!(
        !s.contains("orientation"),
        "identity orientation must not bloat the JSON"
    );
}

#[test]
fn clip_orientation_loads_without_field_in_json() {
    let raw = r#"{
        "schema_version": 2,
        "name": "NoOrient",
        "clips": [{
            "id": "00000000-0000-0000-0000-000000000001",
            "source_path": "/a.mp4",
            "metadata": {
                "path": "/a.mp4",
                "duration_seconds": 1.0,
                "container": "mp4",
                "video": null,
                "audio": null,
                "audio_disabled": false
            },
            "in_point": 0.0,
            "out_point": 1.0
        }],
        "tracks": [],
        "created_at": "2026-04-16T12:00:00Z",
        "modified_at": "2026-04-16T12:00:00Z"
    }"#;
    let p: Project = serde_json::from_str(raw).unwrap();
    assert_eq!(p.clips[0].orientation, ClipOrientation::default());
}

#[test]
fn migrate_v1_project_loads_as_v2() {
    let v1 = r#"{
        "schema_version": 1,
        "name": "Legacy",
        "clips": [],
        "tracks": [],
        "created_at": "2026-04-16T00:00:00Z",
        "modified_at": "2026-04-16T00:00:00Z"
    }"#;
    let mut value: serde_json::Value = serde_json::from_str(v1).unwrap();
    migrate(&mut value).unwrap();
    assert_eq!(value["schema_version"], SCHEMA_VERSION);
    let p: Project = serde_json::from_value(value).unwrap();
    assert_eq!(p.schema_version, SCHEMA_VERSION);
    assert_eq!(p.name, "Legacy");
    assert!(p.extensions.is_empty());
}
