//! `tracing` initialization and a helper for piping child-process stdio into
//! the Rust log stream.
//!
//! Environment:
//! - `REEL_LOG` (fallback `RUST_LOG`): env-filter directive.
//! - `REEL_LOG_FORMAT` = `pretty` (default) | `json`.
//! - `REEL_LOG_FILE` = path to a log file (optional; non-blocking rolling `never`).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Guard returned from [`init`]. Keep it alive for the duration of the
/// process — dropping flushes the non-blocking log writers.
#[must_use = "dropping the LogGuard will flush pending log writes"]
pub struct LogGuard(#[allow(dead_code)] Option<WorkerGuard>);

/// Initialize the global `tracing` subscriber.
///
/// Safe to call multiple times: subsequent calls return a no-op guard if a
/// subscriber is already installed (this keeps tests well-behaved).
pub fn init() -> Result<LogGuard> {
    let filter = EnvFilter::try_from_env("REEL_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("reel_core=info,reel_app=info,reel_cli=info,warn"));

    let format = std::env::var("REEL_LOG_FORMAT").unwrap_or_else(|_| "pretty".into());
    let log_file = std::env::var("REEL_LOG_FILE").ok();

    let (file_layer, guard) = if let Some(path) = log_file.as_deref() {
        let path = std::path::Path::new(path);
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let file_name = path
            .file_name()
            .map(|f| f.to_owned())
            .context("REEL_LOG_FILE has no filename component")?;
        std::fs::create_dir_all(&dir).ok();
        let appender = tracing_appender::rolling::never(dir, file_name);
        let (writer, guard) = tracing_appender::non_blocking(appender);
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(writer);
        (Some(layer.boxed()), Some(guard))
    } else {
        (None, None)
    };

    let fmt_layer = match format.as_str() {
        "json" => tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .boxed(),
        _ => tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_thread_names(true)
            .boxed(),
    };

    let registry = tracing_subscriber::registry().with(filter).with(fmt_layer);
    let init_result = if let Some(fl) = file_layer {
        registry.with(fl).try_init()
    } else {
        registry.try_init()
    };

    // `try_init` returns Err if a subscriber is already installed — that's
    // fine, we just don't own the global one. Drop the file guard in that
    // case so we don't hold open a file against another test's writer.
    match init_result {
        Ok(()) => Ok(LogGuard(guard)),
        Err(_) => Ok(LogGuard(None)),
    }
}

/// Spawn a child process and forward its `stdout`/`stderr` line-by-line into
/// the `tracing` stream.
///
/// - `stdout` lines → `INFO` at the given `target`.
/// - `stderr` lines → `WARN` at the given `target`.
///
/// The returned [`Child`] still owns `wait()`; the caller is responsible for
/// reaping it. Readers run on dedicated threads and terminate when the child
/// closes its pipes.
pub fn spawn_logged_child(mut cmd: Command, target: &'static str) -> std::io::Result<Child> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        let _g1 = init().expect("first init");
        let _g2 = init().expect("second init is a no-op");
    }

    #[test]
    fn spawn_logged_child_echoes_and_exits() {
        let _g = init().expect("init");
        let mut cmd = Command::new("/bin/echo");
        cmd.arg("hello-from-child");
        let mut child = spawn_logged_child(cmd, "test.echo").expect("spawn");
        let status = child.wait().expect("wait");
        assert!(status.success(), "echo should exit 0");
    }

    #[test]
    fn spawn_logged_child_captures_stderr() {
        let _g = init().expect("init");
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("echo to-stdout; echo to-stderr 1>&2");
        let mut child = spawn_logged_child(cmd, "test.sh").expect("spawn");
        let status = child.wait().expect("wait");
        assert!(status.success());
    }
}
