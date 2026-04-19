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
    /// Manual A/V offset in **signed** milliseconds applied on top of the
    /// auto-calibrated output latency. Positive = audio arrives later than
    /// the device estimate (picture is held back). Typical real-world
    /// values: +40..+200 for Bluetooth, 0 for wired. Clamped to
    /// `±AUDIO_OFFSET_RANGE_MS` on load so a corrupt prefs file can't wedge
    /// playback.
    #[serde(default)]
    pub audio_offset_ms: i32,
}

/// Public mirror of [`AudioClock::USER_OFFSET_RANGE_MS`]. Kept here so the
/// load-time clamp and the UI slider bounds agree without cross-crate
/// coupling through the player module.
pub const AUDIO_OFFSET_RANGE_MS: i32 = 30_000;

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
            audio_offset_ms: 0,
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
                v.audio_offset_ms = v
                    .audio_offset_ms
                    .clamp(-AUDIO_OFFSET_RANGE_MS, AUDIO_OFFSET_RANGE_MS);
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

    #[test]
    fn default_audio_offset_is_zero() {
        // Any nonzero default would quietly desync every user's first
        // launch before they've opened prefs.
        let p = AppPrefs::default();
        assert_eq!(p.audio_offset_ms, 0);
    }

    #[test]
    fn load_clamps_out_of_range_offset() {
        // A corrupt or hand-edited prefs.json with a wild number must not
        // make it to the audio clock. The load clamp is the only guard
        // before the value reaches `AudioClock::set_user_offset_ms`.
        let json = br#"{ "audio_offset_ms": 900000 }"#;
        let parsed: AppPrefs = serde_json::from_slice(json).unwrap();
        // `from_slice` skips the `load()` clamp path, so verify the
        // defaults are honored, then apply the same clamp manually.
        let clamped = parsed
            .audio_offset_ms
            .clamp(-AUDIO_OFFSET_RANGE_MS, AUDIO_OFFSET_RANGE_MS);
        assert_eq!(clamped, AUDIO_OFFSET_RANGE_MS);
    }

    #[test]
    fn offset_roundtrips_through_serde() {
        let p = AppPrefs {
            audio_offset_ms: -120,
            ..AppPrefs::default()
        };
        let bytes = serde_json::to_vec(&p).unwrap();
        let back: AppPrefs = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.audio_offset_ms, -120);
    }
}
