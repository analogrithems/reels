//! Headless CLI for Reel.
//!
//! Usage:
//!   reel-cli probe <path>
//!   reel-cli swap  <path> --out <png> [--frame-ms N] [--model …]
//!                        (identity, invert, facefusion, face_enhance, rvm_chroma, …)
//!                        [--sidecar-dir <dir>] [--timeout-ms N]
//!   reel-cli plugins install <name> [--accept-license]

use std::path::PathBuf;
use std::process::{Command as ProcessCommand, ExitCode};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
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
    /// Manage Reel plugins (install / list).
    #[command(subcommand)]
    Plugins(PluginsCommand),
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

        /// Sidecar transform (`identity`, `invert`, `facefusion`, `face_enhance`, `rvm_chroma`, …).
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

#[derive(Subcommand)]
enum PluginsCommand {
    /// Install a plugin (clones upstream + sets up a per-user venv). Pinned
    /// versions live in `build/deps.toml` under `[plugins.<name>]`.
    Install {
        /// Plugin name (e.g. `facefusion`).
        name: String,
        /// Skip the interactive license prompt and accept the plugin's
        /// license non-interactively. Required in CI.
        #[arg(long)]
        accept_license: bool,
    },
}

fn main() -> ExitCode {
    let _log = match reel_core::logging::init() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("logging init failed: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(ref p) = _log.session_log_path {
        tracing::info!(session_log = %p.display(), "reel-cli starting");
    }

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
        Command::Plugins(cmd) => run_plugins(cmd),
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

fn run_plugins(cmd: PluginsCommand) -> Result<()> {
    match cmd {
        PluginsCommand::Install {
            name,
            accept_license,
        } => {
            let script = locate_install_plugin_script()?;
            let mut c = ProcessCommand::new(&script);
            c.arg(&name);
            if accept_license {
                c.arg("--accept-license");
            }
            let status = c
                .status()
                .with_context(|| format!("spawn {}", script.display()))?;
            if !status.success() {
                return Err(anyhow!(
                    "plugin install failed (exit {})",
                    status.code().unwrap_or(-1)
                ));
            }
            Ok(())
        }
    }
}

/// Find `scripts/install_plugin.sh`. In dev we walk up from `CARGO_MANIFEST_DIR`;
/// in a shipped binary we look next to the executable (packaging copies the
/// script into Reel.app/Contents/Resources/scripts/).
fn locate_install_plugin_script() -> Result<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("scripts/install_plugin.sh")),
        std::env::current_exe().ok().and_then(|p| {
            p.parent()
                .map(|d| d.join("../Resources/scripts/install_plugin.sh"))
        }),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .map(|d| d.join("scripts/install_plugin.sh")),
    ];
    for c in candidates.into_iter().flatten() {
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(anyhow!(
        "install_plugin.sh not found; expected at scripts/install_plugin.sh relative to the repo or the binary"
    ))
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
