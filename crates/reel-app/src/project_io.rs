//! Project file I/O (`.reel` JSON).

use std::path::Path;

use anyhow::Context;
use reel_core::migrate;
use reel_core::{Clip, FfmpegProbe, MediaProbe, Project, Track, TrackKind};
use uuid::Uuid;

/// True when **Open** should load a saved project JSON (vs probing as a single media file).
pub fn is_project_document_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("reel" | "json")
    )
}

/// Read a `.reel` / project `.json` file, migrate schema, and set [`Project::path`].
pub fn load_project_file(path: &Path) -> anyhow::Result<Project> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut v: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    migrate(&mut v).map_err(|e| anyhow::anyhow!("project migrate: {e}"))?;
    let mut p: Project =
        serde_json::from_value(v).with_context(|| format!("deserialize {}", path.display()))?;
    p.path = Some(path.to_path_buf());
    Ok(p)
}

/// Probe `media` with `probe` and build a single-clip, single-track project.
///
/// Taking `&dyn MediaProbe` is the injection seam for UI/unit tests (see
/// `docs/phases-ui-test.md` Phase 1b). Production code should keep calling
/// [`project_from_media_path`], which wraps this with the real
/// [`FfmpegProbe`].
pub fn project_from_media_path_with_probe(
    probe: &dyn MediaProbe,
    media: &Path,
) -> anyhow::Result<Project> {
    let md = probe.probe(media).context("probe media")?;
    let clip_id = Uuid::new_v4();
    let track_id = Uuid::new_v4();
    let name = media
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Project");

    let mut p = Project::new(name);
    let dur = md.duration_seconds;
    p.clips.push(Clip {
        id: clip_id,
        source_path: media.to_path_buf(),
        metadata: md,
        in_point: 0.0,
        out_point: dur,
        orientation: Default::default(),
        scale: Default::default(),
        audio_mute: false,
        extensions: Default::default(),
    });
    p.tracks.push(Track {
        id: track_id,
        kind: TrackKind::Video,
        clip_ids: vec![clip_id],
        gain_db: 0.0,
        extensions: Default::default(),
    });
    Ok(p)
}

/// Probe `media` (real ffmpeg) and build a single-clip, single-track project.
///
/// Thin wrapper over [`project_from_media_path_with_probe`] that supplies the
/// real ffmpeg-backed probe. Prefer the `_with_probe` variant from tests.
pub fn project_from_media_path(media: &Path) -> anyhow::Result<Project> {
    let probe = FfmpegProbe::new();
    project_from_media_path_with_probe(&probe, media)
}

/// Serialize `project` to `out` (pretty JSON).
pub fn save_project(out: &Path, project: &Project) -> anyhow::Result<()> {
    let json = serde_json::to_vec_pretty(project).context("serialize project")?;
    std::fs::write(out, json).with_context(|| format!("write {}", out.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use tempfile::tempdir;

    #[test]
    fn save_roundtrip_matches_probe_fixture() {
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
        let p = project_from_media_path(&fixture).expect("project");
        assert_eq!(p.clips.len(), 1);
        save_project(&reel, &p).expect("save");
        let bytes = std::fs::read(&reel).unwrap();
        let parsed: Project = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.clips.len(), 1);
        assert_eq!(parsed.schema_version, reel_core::project::SCHEMA_VERSION);
    }
}
