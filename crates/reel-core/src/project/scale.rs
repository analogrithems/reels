//! Per-clip output scale (Edit → Resize Video…).
//!
//! Scale is a **project-level output** setting, stored on [`crate::Clip`] and applied
//! only to **export** via an ffmpeg `-vf scale=…` filter. The preview is unchanged;
//! the preview viewport already scales decoded frames to fit the window, so showing
//! a 50 % reduction in preview would duplicate that work without adding information.
//! Mixed scales across primary-track clips are rejected at export time (same policy
//! as [`crate::project::ClipOrientation`]).

use serde::{Deserialize, Serialize};

/// Identity = unscaled (100 %).
///
/// `percent` is stored as an integer to keep the JSON diff stable and avoid float
/// equality edge cases when comparing against the identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipScale {
    /// Percent of the source resolution. `0` is normalized to identity; anything
    /// outside [`MIN_PERCENT`]..=[`MAX_PERCENT`] is clamped by [`Self::set_percent`].
    #[serde(default)]
    pub percent: u32,
}

/// Hard-clamp so the sheet and ffmpeg both see sane values.
pub const MIN_PERCENT: u32 = 10;
pub const MAX_PERCENT: u32 = 400;
pub const IDENTITY_PERCENT: u32 = 100;

impl ClipScale {
    pub const IDENTITY: ClipScale = ClipScale { percent: 0 };

    pub fn is_identity(&self) -> bool {
        self.percent == 0 || self.percent == IDENTITY_PERCENT
    }

    /// Clamp and store `percent`, normalizing identity (`100`) to `0` so the
    /// `skip_serializing_if` on `Clip.scale` keeps unchanged projects off disk.
    pub fn set_percent(&mut self, percent: u32) {
        let clamped = percent.clamp(MIN_PERCENT, MAX_PERCENT);
        self.percent = if clamped == IDENTITY_PERCENT { 0 } else { clamped };
    }

    /// Current display value for the sheet — identity reads as `100`.
    pub fn display_percent(&self) -> u32 {
        if self.is_identity() {
            IDENTITY_PERCENT
        } else {
            self.percent
        }
    }

    /// ffmpeg `-vf` fragment, e.g. `"scale=trunc(iw*0.500/2)*2:trunc(ih*0.500/2)*2"`.
    /// Returns `None` for identity so the caller can skip `-vf` and keep stream-copy.
    ///
    /// Dimensions are forced to even numbers (`trunc(·/2)*2`) because H.264 / HEVC
    /// `yuv420p` require both width and height to be divisible by 2.
    pub fn ffmpeg_filter_chain(&self) -> Option<String> {
        if self.is_identity() {
            return None;
        }
        let factor = self.percent as f64 / 100.0;
        Some(format!(
            "scale=trunc(iw*{factor:.3}/2)*2:trunc(ih*{factor:.3}/2)*2"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_returns_none() {
        let s = ClipScale::default();
        assert!(s.is_identity());
        assert_eq!(s.ffmpeg_filter_chain(), None);
    }

    #[test]
    fn one_hundred_percent_normalizes_to_identity() {
        let mut s = ClipScale::default();
        s.set_percent(100);
        assert!(s.is_identity());
        assert_eq!(s.ffmpeg_filter_chain(), None);
        assert_eq!(s.display_percent(), 100);
    }

    #[test]
    fn half_scale_emits_even_dims() {
        let mut s = ClipScale::default();
        s.set_percent(50);
        let chain = s.ffmpeg_filter_chain().unwrap();
        assert!(chain.starts_with("scale="));
        assert!(chain.contains("trunc(iw*0.500/2)*2"));
        assert!(chain.contains("trunc(ih*0.500/2)*2"));
    }

    #[test]
    fn clamps_below_min() {
        let mut s = ClipScale::default();
        s.set_percent(1);
        assert_eq!(s.percent, MIN_PERCENT);
    }

    #[test]
    fn clamps_above_max() {
        let mut s = ClipScale::default();
        s.set_percent(9999);
        assert_eq!(s.percent, MAX_PERCENT);
    }

    #[test]
    fn display_percent_identity() {
        let s = ClipScale::default();
        assert_eq!(s.display_percent(), 100);
    }

    #[test]
    fn serde_roundtrips_nondefault() {
        let mut s = ClipScale::default();
        s.set_percent(75);
        let json = serde_json::to_string(&s).unwrap();
        let back: ClipScale = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
