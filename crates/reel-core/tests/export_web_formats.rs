//! Integration tests: export the tiny H.264 fixture to web-family outputs.
//! Artifacts live under `target/reel-export-verify/` for manual inspection.

use std::path::PathBuf;

use reel_core::{export_with_ffmpeg, WebExportFormat};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tiny_h264_aac.mp4")
}

fn verify_dir() -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        PathBuf::from,
    );
    base.join("reel-export-verify")
}

#[test]
fn exports_fixture_to_each_web_format() {
    let input = fixture();
    assert!(
        input.is_file(),
        "missing fixture {} — run scripts/generate_fixtures.sh",
        input.display()
    );

    let out_dir = verify_dir();
    std::fs::create_dir_all(&out_dir).expect("create verify dir");

    for fmt in WebExportFormat::ALL {
        let name = format!("tiny_h264_aac.{}", fmt.file_extension());
        let output = out_dir.join(&name);
        export_with_ffmpeg(&input, &output, fmt).unwrap_or_else(|e| {
            panic!("export {:?} failed: {e}", output);
        });
        let meta = std::fs::metadata(&output).expect("output exists");
        assert!(
            meta.len() > 64,
            "exported file {} suspiciously small",
            output.display()
        );
    }
}
