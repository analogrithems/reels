//! Debounced, atomic autosave for `Project`.
//!
//! `ProjectStore` owns the in-memory `Project` behind an `RwLock` and runs a
//! background worker thread that coalesces mutation signals and writes
//! `project.json` atomically (tmp + rename). All mutations MUST go through
//! [`ProjectStore::mutate`]; direct writes to the `RwLock` won't trigger a
//! save.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::RwLock;

use super::Project;

/// Debounce window between the last mutation and an atomic disk write.
const DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug)]
enum Msg {
    Dirty,
    Shutdown,
}

/// Owns a `Project` and autosaves it on mutation.
///
/// Construct via [`ProjectStore::open`] (loads or creates) or
/// [`ProjectStore::new_in_memory`] (no autosave thread — useful for tests).
pub struct ProjectStore {
    inner: Arc<RwLock<Project>>,
    tx: Sender<Msg>,
    worker: Option<JoinHandle<()>>,
    path: Option<PathBuf>,
}

impl ProjectStore {
    /// Create a store with no disk backing; [`mutate`](Self::mutate) won't
    /// trigger any writes.
    pub fn new_in_memory(project: Project) -> Self {
        let (tx, _rx) = crossbeam_channel::unbounded();
        Self {
            inner: Arc::new(RwLock::new(project)),
            tx,
            worker: None,
            path: None,
        }
    }

    /// Open or create the project file at `path`.
    ///
    /// If `path` exists: read, schema-migrate, and return the store.
    /// Otherwise: create an empty project with `name`, write it immediately,
    /// and return the store.
    pub fn open(path: impl AsRef<Path>, default_name: &str) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let project = if path.exists() {
            let bytes = std::fs::read(&path)?;
            let mut value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            super::schema::migrate(&mut value)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let mut p: Project = serde_json::from_value(value)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            p.path = Some(path.clone());
            p
        } else {
            let mut p = Project::new(default_name);
            p.path = Some(path.clone());
            write_atomic(&path, &p)?;
            p
        };

        let inner = Arc::new(RwLock::new(project));
        let (tx, rx) = crossbeam_channel::unbounded();
        let worker = spawn_worker(inner.clone(), path.clone(), rx);

        Ok(Self {
            inner,
            tx,
            worker: Some(worker),
            path: Some(path),
        })
    }

    /// Borrow the project for reading.
    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, Project> {
        self.inner.read()
    }

    /// Apply `f` to the project, touch `modified_at`, and signal the autosave
    /// worker. Returns `f`'s result.
    pub fn mutate<R>(&self, f: impl FnOnce(&mut Project) -> R) -> R {
        let r = {
            let mut guard = self.inner.write();
            let r = f(&mut guard);
            guard.touch();
            r
        };
        let _ = self.tx.send(Msg::Dirty);
        r
    }

    /// Force an immediate synchronous flush (skips debounce).
    pub fn flush(&self) -> std::io::Result<()> {
        if let Some(path) = &self.path {
            let guard = self.inner.read();
            write_atomic(path, &guard)?;
        }
        Ok(())
    }

    /// Disk path, if this store is disk-backed.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl Drop for ProjectStore {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

fn spawn_worker(inner: Arc<RwLock<Project>>, path: PathBuf, rx: Receiver<Msg>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("reel-autosave".into())
        .spawn(move || worker_loop(inner, path, rx))
        .expect("spawn autosave worker")
}

fn worker_loop(inner: Arc<RwLock<Project>>, path: PathBuf, rx: Receiver<Msg>) {
    let mut pending_since: Option<Instant> = None;
    loop {
        let timeout = match pending_since {
            Some(t) => DEBOUNCE.saturating_sub(t.elapsed()),
            None => Duration::from_secs(3600),
        };
        match rx.recv_timeout(timeout) {
            Ok(Msg::Dirty) => {
                pending_since = Some(Instant::now());
            }
            Ok(Msg::Shutdown) => {
                if pending_since.is_some() {
                    let guard = inner.read();
                    if let Err(e) = write_atomic(&path, &guard) {
                        tracing::error!(error = %e, path = %path.display(), "final autosave failed");
                    }
                }
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                if pending_since.is_some() {
                    let guard = inner.read();
                    if let Err(e) = write_atomic(&path, &guard) {
                        tracing::error!(error = %e, path = %path.display(), "autosave failed");
                    }
                    pending_since = None;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn write_atomic(final_path: &Path, project: &Project) -> std::io::Result<()> {
    let dir = final_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = final_path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp_path = PathBuf::from(tmp);
    let json = serde_json::to_vec_pretty(project)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, final_path)?;
    tracing::debug!(path = %final_path.display(), bytes = json.len(), "autosaved project");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_new_project_file() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("project.json");
        let store = ProjectStore::open(&p, "First").unwrap();
        assert!(p.exists(), "open() should create the file");
        assert_eq!(store.read().name, "First");
    }

    #[test]
    fn mutate_triggers_debounced_write() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("project.json");
        let store = ProjectStore::open(&p, "Start").unwrap();

        store.mutate(|pr| pr.name = "Renamed".into());

        // Wait past debounce.
        std::thread::sleep(DEBOUNCE + Duration::from_millis(250));
        let bytes = std::fs::read(&p).unwrap();
        let parsed: Project = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.name, "Renamed");
    }

    #[test]
    fn flush_is_synchronous() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("project.json");
        let store = ProjectStore::open(&p, "Sync").unwrap();
        store.mutate(|pr| pr.name = "Flushed".into());
        store.flush().unwrap();
        let parsed: Project = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(parsed.name, "Flushed");
    }
}
