//! Minimal `.reel` project writes — full timeline integration comes later.

use std::path::Path;

use anyhow::Context;
use reel_core::{Clip, FfmpegProbe, MediaProbe, Project, Track, TrackKind};
use uuid::Uuid;

/// Write a single-clip project pointing at `media_path`.
pub fn save_project_reel(out: &Path, media_path: &Path) -> anyhow::Result<()> {
    let probe = FfmpegProbe::new();
    let md = probe.probe(media_path).context("probe media for save")?;
    let clip_id = Uuid::new_v4();
    let track_id = Uuid::new_v4();
    let name = media_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Project");

    let mut p = Project::new(name);
    let dur = md.duration_seconds;
    p.clips.push(Clip {
        id: clip_id,
        source_path: media_path.to_path_buf(),
        metadata: md,
        in_point: 0.0,
        out_point: dur,
        extensions: Default::default(),
    });
    p.tracks.push(Track {
        id: track_id,
        kind: TrackKind::Video,
        clip_ids: vec![clip_id],
        extensions: Default::default(),
    });

    let json = serde_json::to_vec_pretty(&p).context("serialize project")?;
    std::fs::write(out, json).with_context(|| format!("write {}", out.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use tempfile::tempdir;

    #[test]
    fn save_writes_valid_json_roundtrip() {
        let dir = tempdir().unwrap();
        let reel = dir.path().join("t.reel");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("reel-core")
            .join("tests")
            .join("fixtures")
            .join("tiny_h264_aac.mp4");
        if !fixture.is_file() {
            eprintln!("skip: fixture missing");
            return;
        }
        save_project_reel(&reel, &fixture).expect("save");
        let bytes = std::fs::read(&reel).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["schema_version"], 2);
    }
}
