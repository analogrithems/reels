//! Ad-hoc manual scanner. `cargo run --example scan_file -p reel-core -- <path>`
//!
//! Kept as a dev aid for validating [`reel_core::media::scan::scan_file`]
//! against real files without booting the desktop app. Not a unit test —
//! requires an ambient FFmpeg install and real media on disk.

use std::env;
use std::path::Path;

use reel_core::media::scan::scan_file;

fn main() {
    let args: Vec<String> = env::args().collect();
    let Some(raw) = args.get(1) else {
        eprintln!("usage: scan_file <path>");
        std::process::exit(2);
    };
    let path = Path::new(raw);
    let r = match scan_file(path, |ratio| eprintln!("progress={ratio:.2}")) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("scan error: {e}");
            std::process::exit(1);
        }
    };
    println!("verdict:  {}", r.verdict.as_str());
    println!("headline: {}", r.headline());
    println!(
        "packets={} vframes={} aframes={} errors={} warnings={}",
        r.packets_read,
        r.video_frames_decoded,
        r.audio_frames_decoded,
        r.error_count,
        r.warning_count
    );
    for i in &r.issues {
        println!("  [{}] {}", i.kind, i.message);
    }
    if r.repair_recommended() {
        std::process::exit(1);
    }
}
