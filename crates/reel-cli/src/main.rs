//! Headless CLI for Reel.
//!
//! Usage:
//!   reel-cli probe <path>            # print metadata JSON to stdout

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use reel_core::{FfmpegProbe, MediaProbe};

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
    }
}
