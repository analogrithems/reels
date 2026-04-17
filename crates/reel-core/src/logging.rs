//! `tracing` initialization and helpers for piping child-process stdio into the Rust log stream.
//!
//! ## Session log file (always)
//!
//! Unless a subscriber is already installed (tests), every process writes a **session log file** so
//! failures are reviewable without a terminal attached (e.g. GUI launches from Finder).
//! Default path:
//!
//! `{data_local_dir}/reel/logs/reels.session.<UTC timestamp>.log`
//!
//! Override the **directory** with `REEL_LOG_SESSION_DIR`, or set `REEL_LOG_FILE` to a full file
//! path to write that file instead of a timestamped name (advanced / CI).
//!
//! ## Stdout (optional)
//!
//! By default logs go **only** to the session file. When **`stdout` is a TTY** (e.g. `cargo run`),
//! logs are **also** mirrored to the terminal. Set `REEL_LOG_STDOUT=0` to disable mirroring, or
//! `REEL_LOG_STDOUT=1` to force mirroring even when stdout is not a TTY.
//!
//! ## Session file vs terminal
//!
//! The **session log file is always JSON** (newline-delimited JSON). That keeps every record
//! machine-parseable and gives a stable place for **fields** from `tracing` (`key = value` in the
//! macro become JSON properties alongside `target`, `level`, `file`, `line`, etc.).
//!
//! The optional **stdout** mirror uses **`REEL_LOG_FORMAT`**: `pretty` (default) or `json`. Use
//! pretty when running under a TTY; set `json` if you want the same NDJSON shape in the terminal.
//!
//! ## Other environment
//!
//! - `REEL_LOG` (fallback `RUST_LOG`): [`tracing_subscriber::EnvFilter`] directives.
//! - `REEL_LOG_FORMAT` = `pretty` (default) | `json` — **stdout mirror only**; the session file is
//!   always JSON.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Path chosen for the first successful [`init`] (used when later calls are no-ops).
static SESSION_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Guard returned from [`init`]. Keep it alive for the duration of the process — dropping flushes
/// the non-blocking log writers.
#[must_use = "dropping the LogGuard will flush pending log writes"]
pub struct LogGuard(#[allow(dead_code)] Option<WorkerGuard>);

/// Result of [`init`]: holds the flush guard and, when this invocation installed the subscriber,
/// the session log path.
#[must_use = "dropping LogInit flushes the log writer"]
pub struct LogInit {
    guard: LogGuard,
    /// Absolute path to the session log file when this call installed `tracing` (or a cached path
    /// on idempotent follow-up calls after a successful first init).
    pub session_log_path: Option<PathBuf>,
}

impl LogInit {
    /// For callers that only need the underlying flush guard.
    pub fn into_guard(self) -> LogGuard {
        self.guard
    }
}

/// Initialize the global `tracing` subscriber.
///
/// - Writes a **session log file** on first successful init (see module docs).
/// - Safe to call multiple times: later calls return a no-op guard and the same cached
///   [`LogInit::session_log_path`] as the first successful init.
pub fn init() -> Result<LogInit> {
    if let Some(p) = SESSION_LOG_PATH.get() {
        return Ok(LogInit {
            guard: LogGuard(None),
            session_log_path: Some(p.clone()),
        });
    }

    let filter = EnvFilter::try_from_env("REEL_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("reel_core=info,reel_app=info,reel_cli=info,warn"));

    let stdout_format = std::env::var("REEL_LOG_FORMAT").unwrap_or_else(|_| "pretty".into());

    let session_path = resolve_session_log_path()?;
    if let Some(parent) = session_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory {}", parent.display()))?;
    }

    let file_name = session_path
        .file_name()
        .map(|f| f.to_owned())
        .context("session log path has no filename")?;
    let dir = session_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let appender = tracing_appender::rolling::never(&dir, file_name);
    let (file_writer, file_guard) = tracing_appender::non_blocking(appender);

    let mirror_stdout = mirror_stdout_enabled();

    // Session file is always JSON (structured fields). Stdout mirror: pretty or json per REEL_LOG_FORMAT.
    // Layers are inlined per arm so each `fmt::layer()` stacks on the correct inner subscriber type.
    let init_result = match (mirror_stdout, stdout_format.as_str()) {
        (true, "json") => tracing_subscriber::registry()
            .with(filter.clone())
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_current_span(false)
                    .with_writer(file_writer.clone())
                    .with_ansi(false),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_current_span(false)
                    .with_writer(std::io::stdout)
                    .with_ansi(true),
            )
            .try_init(),
        (true, _) => tracing_subscriber::registry()
            .with(filter.clone())
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_current_span(false)
                    .with_writer(file_writer.clone())
                    .with_ansi(false),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_thread_names(true)
                    .with_writer(std::io::stdout)
                    .with_ansi(true),
            )
            .try_init(),
        (false, _) => tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_current_span(false)
                    .with_writer(file_writer)
                    .with_ansi(false),
            )
            .try_init(),
    };

    match init_result {
        Ok(()) => {
            let _ = SESSION_LOG_PATH.set(session_path.clone());
            Ok(LogInit {
                guard: LogGuard(Some(file_guard)),
                session_log_path: Some(session_path),
            })
        }
        Err(_) => {
            // Another test (or embedder) already installed a global subscriber.
            drop(file_guard);
            Ok(LogInit {
                guard: LogGuard(None),
                session_log_path: SESSION_LOG_PATH.get().cloned(),
            })
        }
    }
}

/// `REEL_LOG_STDOUT=0` / `false` → never mirror. `REEL_LOG_STDOUT=1` / `true` → always mirror.
/// Otherwise mirror when stdout is a terminal (handy for `cargo run` without extra env).
fn mirror_stdout_enabled() -> bool {
    match std::env::var("REEL_LOG_STDOUT").as_deref() {
        Ok("0") | Ok("false") => false,
        Ok("1") | Ok("true") => true,
        _ => std::io::stdout().is_terminal(),
    }
}

fn resolve_session_log_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("REEL_LOG_FILE") {
        let path = PathBuf::from(p);
        if path.file_name().is_none() {
            anyhow::bail!("REEL_LOG_FILE must include a filename");
        }
        return Ok(path);
    }

    let dir = session_log_dir();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let name = format!("reels.session.{stamp}.log");
    Ok(dir.join(name))
}

fn session_log_dir() -> PathBuf {
    std::env::var("REEL_LOG_SESSION_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|d| d.join("reel").join("logs")))
        .unwrap_or_else(|| PathBuf::from(".reel_logs"))
}

/// Spawn a child process and forward its `stdout`/`stderr` line-by-line into
/// the `tracing` stream.
///
/// - `stdout` lines → `INFO` at the given `target`.
/// - `stderr` lines → `WARN` at the given `target`.
///
/// The returned [`std::process::Child`] still owns `wait()`; the caller is responsible for
/// reaping it. Readers run on dedicated threads and terminate when the child
/// closes its pipes.
pub fn spawn_logged_child(
    mut cmd: std::process::Command,
    target: &'static str,
) -> std::io::Result<std::process::Child> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        std::thread::Builder::new()
            .name(format!("{target}-stdout"))
            .spawn(move || {
                for line in reader.lines().map_while(Result::ok) {
                    tracing::info!(target: "reel_core::sidecar", sidecar = target, line = %line);
                }
            })?;
    }

    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        std::thread::Builder::new()
            .name(format!("{target}-stderr"))
            .spawn(move || {
                for line in reader.lines().map_while(Result::ok) {
                    tracing::warn!(target: "reel_core::sidecar", sidecar = target, line = %line);
                }
            })?;
    }

    Ok(child)
}

/// Spawn a child process for IPC use: `stdin`/`stdout` are piped and returned
/// to the caller (used by [`crate::sidecar::SidecarClient`] for line-delimited
/// JSON); `stderr` is forwarded into `tracing` at `WARN` under `target`.
///
/// Unlike [`spawn_logged_child`], the child's `stdout` is **not** consumed by
/// this helper — the caller owns the read side.
pub fn spawn_child_with_logged_stderr(
    mut cmd: std::process::Command,
    target: &'static str,
) -> std::io::Result<std::process::Child> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;

    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        std::thread::Builder::new()
            .name(format!("{target}-stderr"))
            .spawn(move || {
                for line in reader.lines().map_while(Result::ok) {
                    tracing::warn!(target: "reel_core::sidecar", sidecar = target, line = %line);
                }
            })?;
    }

    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        let a = init().expect("first init");
        let p1 = a.session_log_path.clone();
        let b = init().expect("second init is a no-op");
        let p2 = b.session_log_path;
        assert_eq!(p1, p2);
    }

    #[test]
    fn spawn_logged_child_echoes_and_exits() {
        let _g = init().expect("init");
        let mut cmd = std::process::Command::new("/bin/echo");
        cmd.arg("hello-from-child");
        let mut child = spawn_logged_child(cmd, "test.echo").expect("spawn");
        let status = child.wait().expect("wait");
        assert!(status.success(), "echo should exit 0");
    }

    #[test]
    fn spawn_logged_child_captures_stderr() {
        let _g = init().expect("init");
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg("echo to-stdout; echo to-stderr 1>&2");
        let mut child = spawn_logged_child(cmd, "test.sh").expect("spawn");
        let status = child.wait().expect("wait");
        assert!(status.success());
    }
}
