//! Rust half of the FaceFusion stdio IPC bridge.
//!
//! The Python sidecar (see `sidecar/facefusion_bridge.py`) speaks a simple
//! line-delimited JSON protocol over stdin/stdout; `stderr` is consumed by
//! [`crate::logging::spawn_child_with_logged_stderr`].
//!
//! # Protocol
//!
//! Requests from Rust → Python (one JSON object per line):
//! ```json
//! {"id": 42, "op": "swap", "in_path": "/tmp/xxx.rgba",
//!  "width": 1920, "height": 1080, "params": {"model": "identity"}}
//! {"id": 43, "op": "ping"}
//! {"id": 44, "op": "shutdown"}
//! ```
//!
//! Responses Python → Rust:
//! ```json
//! {"id": 42, "status": "ok", "out_path": "/tmp/yyy.rgba"}
//! {"id": 43, "status": "ok"}
//! {"id": 42, "status": "err", "reason": "unknown model"}
//! ```
//!
//! Pixel payloads travel via tempfiles (raw RGBA8, `w*h*4` bytes) — keeping
//! the JSON tiny and avoiding per-frame base64 overhead. The client owns a
//! private tempdir for its lifetime; on drop, everything under it is GC'd
//! by [`tempfile::TempDir`].
//!
//! # Failure modes
//!
//! - Sidecar panics/exits: in-flight requests receive [`SidecarError::Crashed`];
//!   subsequent sends fail with [`SidecarError::Io`].
//! - Sidecar too slow: [`SidecarError::Timeout`] after the configured deadline
//!   (default 10 s, see [`SidecarClient::set_timeout`]).
//! - Malformed response line: logged, but does not kill the reader — the
//!   corresponding request will eventually time out.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::TempDir;
use thiserror::Error;

use crate::logging::spawn_child_with_logged_stderr;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors from the sidecar IPC layer.
#[derive(Debug, Error)]
pub enum SidecarError {
    #[error("sidecar crashed: {0}")]
    Crashed(String),

    #[error("sidecar timed out after {timeout:?} waiting for id={id}")]
    Timeout { id: u64, timeout: Duration },

    #[error("sidecar protocol error: {0}")]
    Protocol(String),

    #[error("sidecar i/o: {0}")]
    Io(#[from] std::io::Error),

    #[error("sidecar serde: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Serialize)]
struct Request<'a> {
    id: u64,
    op: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    in_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Value::is_null")]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct Response {
    id: u64,
    status: String,
    #[serde(default)]
    out_path: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

type Pending = Arc<Mutex<HashMap<u64, Sender<Result<Response, SidecarError>>>>>;

/// Long-lived handle to a running Python sidecar.
///
/// Cheap to clone pointers live inside; methods take `&self` and are safe to
/// call from multiple threads. Dropping the client sends a `shutdown`, closes
/// stdin, waits briefly for a graceful exit, and then kills the child.
pub struct SidecarClient {
    child: Mutex<Option<Child>>,
    /// `Option` so [`Drop`] can [`Option::take`] it and close stdin by
    /// dropping the writer, which is what signals the Python loop to exit.
    writer: Mutex<Option<BufWriter<ChildStdin>>>,
    pending: Pending,
    next_id: AtomicU64,
    #[allow(dead_code)] // held for its Drop (deletes the temp dir)
    tempdir: TempDir,
    tempdir_path: PathBuf,
    timeout: Mutex<Duration>,
}

impl SidecarClient {
    /// Spawn `uv run python facefusion_bridge.py` with [`Command::current_dir`]
    /// set to `sidecar_dir` so `uv` resolves deps from that tree's `pyproject.toml`.
    ///
    /// Requires `uv` on `PATH` (see `make check-tools`). The environment is
    /// created/updated by `uv` on demand; [`crate::logging`] still forwards
    /// stderr into `tracing`.
    pub fn spawn_python(sidecar_dir: &Path) -> Result<Self, SidecarError> {
        let bridge = sidecar_dir.join("facefusion_bridge.py");
        if !bridge.exists() {
            return Err(SidecarError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("sidecar bridge not found: {}", bridge.display()),
            )));
        }
        let mut cmd = Command::new("uv");
        cmd.current_dir(sidecar_dir)
            .args(["run", "python", "facefusion_bridge.py"]);
        Self::spawn_command(cmd)
    }

    /// Lower-level constructor — caller builds the `Command`. Useful for
    /// tests that want to point at a fake sidecar.
    pub fn spawn_command(cmd: Command) -> Result<Self, SidecarError> {
        let mut child = spawn_child_with_logged_stderr(cmd, "facefusion_bridge")?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SidecarError::Protocol("no stdin on child".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SidecarError::Protocol("no stdout on child".into()))?;
        let writer = BufWriter::new(stdin);

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_rx = pending.clone();
        std::thread::Builder::new()
            .name("reel-sidecar-reader".into())
            .spawn(move || reader_loop(stdout, pending_rx))?;

        let tempdir = tempfile::Builder::new().prefix("reel-sidecar-").tempdir()?;
        let tempdir_path = tempdir.path().to_path_buf();

        Ok(Self {
            child: Mutex::new(Some(child)),
            writer: Mutex::new(Some(writer)),
            pending,
            next_id: AtomicU64::new(1),
            tempdir,
            tempdir_path,
            timeout: Mutex::new(DEFAULT_TIMEOUT),
        })
    }

    /// Override the default per-request timeout (10 s).
    pub fn set_timeout(&self, d: Duration) {
        *self.timeout.lock() = d;
    }

    /// Round-trip health check. Returns `Ok(())` if the sidecar replied with
    /// `status: "ok"` within the current timeout.
    pub fn ping(&self) -> Result<(), SidecarError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let rx = self.register(id);
        self.send_request(&Request {
            id,
            op: "ping",
            in_path: None,
            width: None,
            height: None,
            params: Value::Null,
        })?;
        let resp = self.await_response(id, rx)?;
        if resp.status == "ok" {
            Ok(())
        } else {
            Err(SidecarError::Protocol(
                resp.reason.unwrap_or_else(|| "ping failed".into()),
            ))
        }
    }

    /// Send an RGBA frame through the sidecar and receive the transformed
    /// frame back. `params` is passed through verbatim (e.g.
    /// `json!({"model": "identity"})`).
    pub fn swap_frame(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        params: Value,
    ) -> Result<Vec<u8>, SidecarError> {
        let expected = (width as usize) * (height as usize) * 4;
        if rgba.len() != expected {
            return Err(SidecarError::Protocol(format!(
                "rgba input length {} != {} (w*h*4)",
                rgba.len(),
                expected
            )));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let in_path = self.tempdir_path.join(format!("in-{id}.rgba"));
        std::fs::write(&in_path, rgba)?;

        let rx = self.register(id);
        let in_path_str = in_path.to_string_lossy().into_owned();
        let send_result = self.send_request(&Request {
            id,
            op: "swap",
            in_path: Some(in_path_str),
            width: Some(width),
            height: Some(height),
            params,
        });
        if let Err(e) = send_result {
            self.pending.lock().remove(&id);
            let _ = std::fs::remove_file(&in_path);
            return Err(e);
        }

        let resp_result = self.await_response(id, rx);
        let _ = std::fs::remove_file(&in_path);
        let resp = resp_result?;
        if resp.status != "ok" {
            return Err(SidecarError::Protocol(
                resp.reason.unwrap_or_else(|| "swap failed".into()),
            ));
        }
        let out_path = resp
            .out_path
            .ok_or_else(|| SidecarError::Protocol("swap response missing out_path".into()))?;
        let bytes = std::fs::read(&out_path)?;
        let _ = std::fs::remove_file(&out_path);
        if bytes.len() != expected {
            return Err(SidecarError::Protocol(format!(
                "swap output length {} != {}",
                bytes.len(),
                expected
            )));
        }
        Ok(bytes)
    }

    fn register(&self, id: u64) -> Receiver<Result<Response, SidecarError>> {
        let (tx, rx) = bounded(1);
        self.pending.lock().insert(id, tx);
        rx
    }

    fn send_request(&self, req: &Request) -> Result<(), SidecarError> {
        let line = serde_json::to_string(req)?;
        let mut guard = self.writer.lock();
        let w = guard
            .as_mut()
            .ok_or_else(|| SidecarError::Crashed("writer closed".into()))?;
        w.write_all(line.as_bytes())?;
        w.write_all(b"\n")?;
        w.flush()?;
        Ok(())
    }

    fn await_response(
        &self,
        id: u64,
        rx: Receiver<Result<Response, SidecarError>>,
    ) -> Result<Response, SidecarError> {
        let timeout = *self.timeout.lock();
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => {
                self.pending.lock().remove(&id);
                Err(SidecarError::Timeout { id, timeout })
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(SidecarError::Crashed("reader channel closed".into()))
            }
        }
    }
}

fn reader_loop(stdout: std::process::ChildStdout, pending: Pending) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        match line {
            Ok(l) => {
                let trimmed = l.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Response>(trimmed) {
                    Ok(resp) => {
                        let slot = pending.lock().remove(&resp.id);
                        match slot {
                            Some(tx) => {
                                let _ = tx.send(Ok(resp));
                            }
                            None => {
                                tracing::warn!(id = resp.id, "unsolicited sidecar response");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, line = %trimmed, "invalid sidecar json");
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "sidecar stdout read error");
                break;
            }
        }
    }
    // EOF — drain any waiters with Crashed.
    let mut guard = pending.lock();
    let count = guard.len();
    if count > 0 {
        tracing::warn!(
            pending = count,
            "sidecar closed stdout with pending requests"
        );
    }
    for (_, tx) in guard.drain() {
        let _ = tx.send(Err(SidecarError::Crashed("sidecar closed stdout".into())));
    }
}

impl Drop for SidecarClient {
    fn drop(&mut self) {
        // Best-effort graceful shutdown: send `shutdown`, then close stdin
        // by dropping the writer. The Python loop sees EOF and exits.
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = Request {
            id,
            op: "shutdown",
            in_path: None,
            width: None,
            height: None,
            params: Value::Null,
        };
        {
            let mut guard = self.writer.lock();
            if let Some(w) = guard.as_mut() {
                if let Ok(line) = serde_json::to_string(&req) {
                    let _ = w.write_all(line.as_bytes());
                    let _ = w.write_all(b"\n");
                    let _ = w.flush();
                }
            }
            // Drop the writer → close stdin → Python loop exits on EOF.
            *guard = None;
        }

        // Give the child up to ~1 s to exit cleanly; then kill.
        let mut child_guard = self.child.lock();
        if let Some(mut child) = child_guard.take() {
            for _ in 0..20 {
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => break,
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
