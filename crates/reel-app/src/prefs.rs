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
}

fn default_master_volume() -> f32 {
    1.0
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self { master_volume: 1.0 }
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
