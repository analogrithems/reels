//! Integration tests for `reel_core::sidecar::SidecarClient` against the real
//! Python bridge.
//!
//! These tests require `make setup` (which runs `uv sync` in `sidecar/`) to
//! have been run first — same contract as the ffmpeg fixture tests. If the
//! venv is missing, each test is skipped with an explanatory message.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reel_core::sidecar::{SidecarClient, SidecarError};

fn sidecar_dir() -> PathBuf {
    // crates/reel-core/tests → ../../../sidecar
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("sidecar")
        .canonicalize()
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("sidecar")
        })
}

fn uv_on_path() -> bool {
    std::process::Command::new("uv")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn maybe_skip(dir: &Path) -> bool {
    if !uv_on_path() {
        eprintln!("`uv` not found on PATH — install uv (see Makefile check-tools)");
        return true;
    }
    let bridge = dir.join("facefusion_bridge.py");
    if !bridge.exists() {
        eprintln!(
            "sidecar bridge missing at {} — run `make setup` from repo root",
            bridge.display()
        );
        return true;
    }
    false
}

fn client() -> Option<SidecarClient> {
    let dir = sidecar_dir();
    if maybe_skip(&dir) {
        return None;
    }
    Some(SidecarClient::spawn_python(&dir).expect("spawn sidecar"))
}

fn checkerboard(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let on = (x + y) & 1 == 0;
            let c = if on { 200 } else { 40 };
            v.extend_from_slice(&[c, c / 2, c / 3, 255]);
        }
    }
    v
}

#[test]
fn ping_round_trip() {
    let Some(c) = client() else {
        return;
    };
    c.ping().expect("ping");
}

#[test]
fn swap_identity_returns_input_bytes() {
    let Some(c) = client() else {
        return;
    };
    let (w, h) = (8u32, 6u32);
    let rgba = checkerboard(w, h);
    let out = c
        .swap_frame(&rgba, w, h, serde_json::json!({"model": "identity"}))
        .expect("swap identity");
    assert_eq!(out, rgba, "identity transform must preserve bytes");
}

#[test]
fn swap_invert_flips_rgb_keeps_alpha() {
    let Some(c) = client() else {
        return;
    };
    let (w, h) = (4u32, 4u32);
    let rgba = checkerboard(w, h);
    let out = c
        .swap_frame(&rgba, w, h, serde_json::json!({"model": "invert"}))
        .expect("swap invert");
    assert_eq!(out.len(), rgba.len());
    for (i, (a, b)) in rgba.chunks(4).zip(out.chunks(4)).enumerate() {
        assert_eq!(a[3], b[3], "alpha must be preserved at px {i}");
        assert_eq!(255 - a[0], b[0], "R inverted at px {i}");
        assert_eq!(255 - a[1], b[1], "G inverted at px {i}");
        assert_eq!(255 - a[2], b[2], "B inverted at px {i}");
    }
}

#[test]
fn swap_timeout_surfaces_timeout_error() {
    let Some(c) = client() else {
        return;
    };
    c.set_timeout(Duration::from_millis(200));
    let (w, h) = (2u32, 2u32);
    let rgba = checkerboard(w, h);
    let err = c
        .swap_frame(
            &rgba,
            w,
            h,
            serde_json::json!({"model": "identity", "sleep_ms": 2000}),
        )
        .expect_err("expected timeout error");
    match err {
        SidecarError::Timeout { .. } => {}
        other => panic!("expected Timeout, got {other:?}"),
    }
}

#[test]
fn swap_crash_surfaces_crashed_error() {
    let Some(c) = client() else {
        return;
    };
    let (w, h) = (2u32, 2u32);
    let rgba = checkerboard(w, h);
    let err = c
        .swap_frame(
            &rgba,
            w,
            h,
            serde_json::json!({"model": "identity", "crash": true}),
        )
        .expect_err("expected crashed error");
    match err {
        SidecarError::Crashed(_) => {}
        other => panic!("expected Crashed, got {other:?}"),
    }
}

#[test]
fn swap_unknown_model_returns_protocol_error() {
    let Some(c) = client() else {
        return;
    };
    let (w, h) = (2u32, 2u32);
    let rgba = checkerboard(w, h);
    let err = c
        .swap_frame(&rgba, w, h, serde_json::json!({"model": "nope"}))
        .expect_err("expected protocol error");
    match err {
        SidecarError::Protocol(msg) => {
            assert!(
                msg.contains("nope"),
                "error should name the bad model: {msg}"
            );
        }
        other => panic!("expected Protocol, got {other:?}"),
    }
}

#[test]
fn swap_mismatched_rgba_length_is_rejected_locally() {
    let Some(c) = client() else {
        return;
    };
    // Claim 4x4 (= 64 bytes) but only hand over 16 bytes.
    let err = c
        .swap_frame(&[0u8; 16], 4, 4, serde_json::json!({"model": "identity"}))
        .expect_err("expected local length validation");
    match err {
        SidecarError::Protocol(msg) => assert!(msg.contains("16")),
        other => panic!("expected Protocol, got {other:?}"),
    }
}
