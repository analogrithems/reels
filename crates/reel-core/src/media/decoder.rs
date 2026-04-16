//! Video decode loop feeding the Slint `SharedPixelBuffer` path.
//!
//! Filled in during Phase 2 (player window). For now this module only
//! exposes type aliases the `reel-app` crate will consume, so adding real
//! logic later does not trigger a visible API break.

use std::path::PathBuf;

/// Commands the UI sends to the decoder thread.
#[derive(Debug, Clone)]
pub enum DecodeCmd {
    /// Swap to a new source file.
    Open(PathBuf),
    /// Resume decode → present at wall-clock pace.
    Play,
    /// Pause decode; hold last frame.
    Pause,
    /// Jump to the given position in milliseconds.
    Seek { pts_ms: u64 },
    /// Stop and release resources.
    Stop,
}

/// Frames handed back to the UI thread.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub pts_ms: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA8, tightly packed (`width * height * 4` bytes, row stride `width * 4`).
    pub rgba: std::sync::Arc<[u8]>,
}
