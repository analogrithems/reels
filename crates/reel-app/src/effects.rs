//! Effects menu: grab current frame → sidecar transform → save PNG.

use std::path::{Path, PathBuf};

use image::{ImageBuffer, RgbaImage};
use reel_core::{grab_frame, SidecarClient};
use serde_json::json;

/// Resolve `sidecar/` for `uv run python facefusion_bridge.py`.
///
/// 1. `REEL_SIDECAR_DIR` if set and exists  
/// 2. `../sidecar` from the executable (typical `cargo run` cwd = workspace root)  
/// 3. Workspace `sidecar/` relative to this crate (`crates/reel-app/../../sidecar`)
pub fn resolve_sidecar_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("REEL_SIDECAR_DIR") {
        let pb = PathBuf::from(p);
        if pb.is_dir() {
            return Some(pb);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let s = cwd.join("sidecar");
        if s.is_dir() {
            return Some(s);
        }
    }
    let manifest_adjacent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("sidecar");
    if manifest_adjacent.is_dir() {
        return Some(manifest_adjacent);
    }
    None
}

#[derive(Debug, Clone, Copy)]
pub enum EffectKind {
    FaceFusion,
    FaceEnhance,
    RvmBackground,
}

impl EffectKind {
    fn model_param(self) -> &'static str {
        match self {
            EffectKind::FaceFusion => "facefusion",
            EffectKind::FaceEnhance => "face_enhance",
            EffectKind::RvmBackground => "rvm_chroma",
        }
    }

    fn label(self) -> &'static str {
        match self {
            EffectKind::FaceFusion => "Face swap (FaceFusion)",
            EffectKind::FaceEnhance => "Face enhance",
            EffectKind::RvmBackground => "Remove background (RVM-style)",
        }
    }
}

/// Decode one frame at `pts_ms`, run sidecar, write PNG to `out_path`.
pub fn apply_effect_to_png(
    media: &Path,
    pts_ms: u64,
    effect: EffectKind,
    sidecar_dir: &Path,
    out_path: &Path,
) -> anyhow::Result<()> {
    let frame = grab_frame(media, pts_ms)?;
    let w = frame.width;
    let h = frame.height;
    let rgba = frame.rgba.as_ref();

    let client = SidecarClient::spawn_python(sidecar_dir)?;
    client.set_timeout(std::time::Duration::from_secs(120));
    client.ping()?;
    let params = json!({ "model": effect.model_param() });
    let out_bytes = client.swap_frame(rgba, w, h, params)?;

    let img: RgbaImage = ImageBuffer::from_raw(w, h, out_bytes)
        .ok_or_else(|| anyhow::anyhow!("effect output size mismatch"))?;
    img.save(out_path)?;
    tracing::info!(
        effect = effect.label(),
        path = %out_path.display(),
        "wrote effect PNG"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_sidecar_finds_repo_bridge() {
        let p = resolve_sidecar_dir().expect("sidecar next to workspace");
        assert!(
            p.join("facefusion_bridge.py").is_file(),
            "expected {}",
            p.display()
        );
    }
}
