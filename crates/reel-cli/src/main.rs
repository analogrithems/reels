//! Headless CLI for Reel.
//!
//! Usage:
//!   reel-cli probe <path>
//!   reel-cli swap  <path> --out <png> [--frame-ms N] [--model identity|invert]
//!                        [--sidecar-dir <dir>] [--timeout-ms N]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reel_core::{grab_frame, FfmpegProbe, MediaProbe, SidecarClient};

#[derive(Parser)]
#[command(name = "reel-cli", version, about = "Reel headless operations")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print media metadata for a file as JSON.
    Probe {
        /// Path to a media file.
        path: PathBuf,
    },
    /// Grab one frame, push it through the FaceFusion sidecar, and write the
    /// returned RGBA as a PNG.
    Swap {
        /// Input video file.
        path: PathBuf,

        /// Output PNG path.
        #[arg(long)]
        out: PathBuf,

        /// Frame position in milliseconds. Defaults to 0.
        #[arg(long, default_value_t = 0)]
        frame_ms: u64,

        /// Transform name understood by the sidecar (`identity` | `invert`).
        #[arg(long, default_value = "identity")]
        model: String,

        /// Path to the sidecar directory (`pyproject.toml`, `facefusion_bridge.py`).
        /// Spawned via `uv run python facefusion_bridge.py` from this directory.
        /// Defaults to `./sidecar`.
        #[arg(long)]
        sidecar_dir: Option<PathBuf>,

        /// Per-request sidecar timeout in milliseconds.
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
    },
}

fn main() -> ExitCode {
    let _guard = match reel_core::logging::init() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("logging init failed: {e}");
            return ExitCode::from(2);
        }
    };

    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Probe { path } => {
            let probe = FfmpegProbe::new();
            let md = probe.probe(&path)?;
            let json = serde_json::to_string_pretty(&md)?;
            println!("{json}");
            Ok(())
        }
        Command::Swap {
            path,
            out,
            frame_ms,
            model,
            sidecar_dir,
            timeout_ms,
        } => do_swap(path, out, frame_ms, model, sidecar_dir, timeout_ms),
    }
}

fn do_swap(
    path: PathBuf,
    out: PathBuf,
    frame_ms: u64,
    model: String,
    sidecar_dir: Option<PathBuf>,
    timeout_ms: u64,
) -> Result<()> {
    let sidecar_dir = sidecar_dir
        .map(Ok::<_, anyhow::Error>)
        .unwrap_or_else(default_sidecar_dir)?;
    tracing::info!(path = %path.display(), frame_ms, %model, "grabbing frame");
    let frame = grab_frame(&path, frame_ms).context("grab frame")?;
    tracing::info!(
        width = frame.width,
        height = frame.height,
        pts_ms = frame.pts_ms,
        "frame decoded; spawning sidecar"
    );

    let client = SidecarClient::spawn_python(&sidecar_dir)
        .with_context(|| format!("spawn sidecar at {}", sidecar_dir.display()))?;
    client.set_timeout(Duration::from_millis(timeout_ms));
    // Sanity-check the sidecar before sending a large payload.
    client.ping().context("sidecar ping")?;

    let params = serde_json::json!({ "model": model });
    let swapped = client
        .swap_frame(&frame.rgba, frame.width, frame.height, params)
        .context("sidecar swap")?;

    let img = image::RgbaImage::from_raw(frame.width, frame.height, swapped)
        .context("RGBA buffer did not match declared dimensions")?;
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    img.save(&out)
        .with_context(|| format!("write {}", out.display()))?;
    println!("wrote {} ({}x{})", out.display(), frame.width, frame.height);
    Ok(())
}

/// Resolve `sidecar/` relative to the current working directory.
fn default_sidecar_dir() -> Result<PathBuf> {
    let p = std::env::current_dir()?.join("sidecar");
    Ok(p)
}
