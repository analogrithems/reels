//! Most-recently-used paths for **File → Open Recent** (see `docs/phases-ui.md` U4).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 12;
const MENU_SLOTS: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecentFile {
    path: PathBuf,
    #[serde(default)]
    is_project: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecentFileList {
    entries: Vec<RecentFile>,
}

/// Tracks recently opened project (`.reel` / `.json`) and media files for the MRU menu.
#[derive(Debug, Default)]
pub struct RecentStore {
    inner: RecentFileList,
}

impl RecentStore {
    fn store_path() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("reel").join("recent.json"))
    }

    pub fn load() -> Self {
        let Some(p) = Self::store_path() else {
            return Self::default();
        };
        let Ok(bytes) = fs::read(&p) else {
            return Self::default();
        };
        match serde_json::from_slice::<RecentFileList>(&bytes) {
            Ok(inner) => Self { inner },
            Err(e) => {
                tracing::warn!(error = %e, path = %p.display(), "failed to parse recent.json");
                Self::default()
            }
        }
    }

    fn save(&self) {
        let Some(p) = Self::store_path() else {
            return;
        };
        if let Some(dir) = p.parent() {
            if let Err(e) = fs::create_dir_all(dir) {
                tracing::warn!(error = %e, "recent: create dir");
                return;
            }
        }
        let json = match serde_json::to_vec_pretty(&self.inner) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "recent: serialize");
                return;
            }
        };
        if let Err(e) = fs::write(&p, json) {
            tracing::warn!(error = %e, path = %p.display(), "recent: write");
        }
    }

    fn push_front(path: PathBuf, is_project: bool, list: &mut Vec<RecentFile>) {
        list.retain(|e| e.path != path);
        list.insert(0, RecentFile { path, is_project });
        list.truncate(MAX_ENTRIES);
    }

    /// Call after a successful **Open** or **Save** of a project file.
    pub fn record_project(&mut self, path: PathBuf) {
        Self::push_front(path, true, &mut self.inner.entries);
        self.save();
    }

    /// Call after opening media from **Open…** or inserting a clip from disk.
    pub fn record_media(&mut self, path: PathBuf) {
        Self::push_front(path, false, &mut self.inner.entries);
        self.save();
    }

    pub fn clear(&mut self) {
        self.inner.entries.clear();
        self.save();
    }

    pub fn remove_path(&mut self, path: &Path) {
        let before = self.inner.entries.len();
        self.inner.entries.retain(|e| e.path != path);
        if self.inner.entries.len() != before {
            self.save();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.entries.is_empty()
    }

    /// `MENU_SLOTS` lines for the Open Recent submenu; empty string = unused slot.
    pub fn menu_labels(&self) -> [String; MENU_SLOTS] {
        let mut out = std::array::from_fn(|_| String::new());
        for (i, e) in self.inner.entries.iter().take(MENU_SLOTS).enumerate() {
            out[i] = label_for_path(&e.path, e.is_project);
        }
        out
    }

    pub fn path_for_menu_index(&self, index: i32) -> Option<PathBuf> {
        if index < 0 {
            return None;
        }
        let i = index as usize;
        self.inner.entries.get(i).map(|e| e.path.clone())
    }
}

fn label_for_path(path: &Path, is_project: bool) -> String {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
    let tag = if is_project { "Project" } else { "Media" };
    let mut s = format!("{name} ({tag})");
    if s.len() > 64 {
        s.truncate(61);
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_dedupes_and_caps() {
        let mut st = RecentStore::default();
        st.record_media(PathBuf::from("/a/x.mp4"));
        st.record_project(PathBuf::from("/p/t.reel"));
        st.record_media(PathBuf::from("/a/x.mp4"));
        assert_eq!(st.inner.entries.len(), 2);
        assert_eq!(st.inner.entries[0].path, PathBuf::from("/a/x.mp4"));
        assert!(!st.inner.entries[0].is_project);
    }
}
