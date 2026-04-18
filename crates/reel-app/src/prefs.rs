//! Persisted app preferences (master volume, etc.). Stored next to MRU under the OS data dir.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPrefs {
    /// Linear master gain **0.0..=1.0** for preview audio.
    #[serde(default = "default_master_volume")]
    pub master_volume: f32,
    /// When true, playback seeks to the start and continues at sequence end.
    #[serde(default)]
    pub playback_loop: bool,
    /// Preview zoom **25..=400** as a percent of **fit** (contain/cover) size; ignored when `preview_zoom_actual`.
    #[serde(default = "default_preview_zoom_percent")]
    pub preview_zoom_percent: f32,
    /// When true, preview draws the frame at **1:1** logical pixels.
    #[serde(default)]
    pub preview_zoom_actual: bool,
    /// Codec / path / save line under the preview (View → Show Status).
    #[serde(default)]
    pub show_footer_status: bool,
    /// Keep the floating transport visible instead of fading after idle.
    #[serde(default)]
    pub controls_overlay_always_visible: bool,
    /// Timeline: show video lane rows (header + filmstrips).
    #[serde(default = "default_true")]
    pub show_video_tracks: bool,
    /// Timeline: show audio lane rows.
    #[serde(default = "default_true")]
    pub show_audio_tracks: bool,
    /// Timeline: show subtitle lane rows.
    #[serde(default = "default_true")]
    pub show_subtitle_tracks: bool,
}

fn default_true() -> bool {
    true
}

fn default_master_volume() -> f32 {
    1.0
}

fn default_preview_zoom_percent() -> f32 {
    100.0
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            master_volume: 1.0,
            playback_loop: false,
            preview_zoom_percent: 100.0,
            preview_zoom_actual: false,
            show_footer_status: false,
            controls_overlay_always_visible: false,
            show_video_tracks: true,
            show_audio_tracks: true,
            show_subtitle_tracks: true,
        }
    }
}

impl AppPrefs {
    fn store_path() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("reel").join("prefs.json"))
    }

    pub fn load() -> Self {
        let Some(p) = Self::store_path() else {
            return Self::default();
        };
        let Ok(bytes) = fs::read(&p) else {
            return Self::default();
        };
        match serde_json::from_slice::<AppPrefs>(&bytes) {
            Ok(mut v) => {
                v.master_volume = v.master_volume.clamp(0.0, 1.0);
                v.preview_zoom_percent = v.preview_zoom_percent.clamp(25.0, 400.0);
                v
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %p.display(), "prefs.json parse failed");
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(p) = Self::store_path() else {
            return;
        };
        if let Some(dir) = p.parent() {
            if let Err(e) = fs::create_dir_all(dir) {
                tracing::warn!(error = %e, "prefs: create dir");
                return;
            }
        }
        let json = match serde_json::to_vec_pretty(self) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "prefs: serialize");
                return;
            }
        };
        if let Err(e) = fs::write(&p, json) {
            tracing::warn!(error = %e, path = %p.display(), "prefs: write");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_master_is_full() {
        let p = AppPrefs::default();
        assert!((p.master_volume - 1.0).abs() < f32::EPSILON);
    }
}
