//! End-to-end tests for [`reel_core::media::scan::scan_file`] against the
//! same tiny fixtures used by `probe_fixtures.rs`. Missing fixtures are
//! treated as a skip so a fresh clone without `make fixtures` is still
//! green.

use std::path::{Path, PathBuf};

use reel_core::media::scan::{scan_file, ScanSeverity};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn maybe_skip(path: &Path) -> bool {
    if !path.exists() {
        eprintln!("fixture missing: {} — run `make fixtures`", path.display());
        return true;
    }
    false
}

#[test]
fn clean_mp4_reports_ok_verdict_and_decodes_all_frames() {
    let p = fixtures_dir().join("tiny_h264_aac.mp4");
    if maybe_skip(&p) {
        return;
    }
    let report = scan_file(&p, |_| {}).expect("scan opens the fixture");

    assert_eq!(
        report.verdict,
        ScanSeverity::Ok,
        "tiny_h264_aac.mp4 is a freshly-muxed fixture; expected zero errors. headline={}",
        report.headline()
    );
    assert_eq!(report.error_count, 0);
    assert_eq!(report.warning_count, 0);
    assert!(report.issues.is_empty(), "issues: {:?}", report.issues);
    assert!(
        report.video_frames_decoded > 0,
        "expected >0 video frames decoded"
    );
    assert!(
        report.audio_frames_decoded > 0,
        "expected >0 audio frames decoded"
    );
    assert!(report.packets_read > 0);
    assert!(!report.repair_recommended());
}

#[test]
fn scan_reports_progress_monotonically() {
    let p = fixtures_dir().join("tiny_h264_aac.mp4");
    if maybe_skip(&p) {
        return;
    }
    let seen: std::sync::Mutex<Vec<f64>> = std::sync::Mutex::new(Vec::new());
    scan_file(&p, |r| seen.lock().unwrap().push(r)).expect("scan ok");
    let values = seen.into_inner().unwrap();
    // Ratios must never go backwards — the UI progress bar would jitter.
    for w in values.windows(2) {
        assert!(
            w[1] >= w[0] - f64::EPSILON,
            "progress went backwards: {} → {} (all: {:?})",
            w[0],
            w[1],
            values
        );
    }
    if let Some(last) = values.last() {
        assert!(
            (*last - 1.0).abs() < 1e-6,
            "last progress tick must be 1.0, got {last}"
        );
    }
}

#[test]
fn scan_no_audio_fixture_reports_only_video_frames() {
    let p = fixtures_dir().join("no_audio.mp4");
    if maybe_skip(&p) {
        return;
    }
    let report = scan_file(&p, |_| {}).expect("scan opens the fixture");
    assert_eq!(report.audio_frames_decoded, 0);
    assert!(report.video_frames_decoded > 0);
    // A video-only file is still a valid, healthy file — verdict should be Ok.
    assert_eq!(report.verdict, ScanSeverity::Ok);
}
