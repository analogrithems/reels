//! Per-clip rotate / flip state (QuickTime-style Edit menu).
//!
//! Orientation is stored on [`crate::Clip`] and applied both to the **preview**
//! (post-scaler RGBA transform) and to **export** (ffmpeg `-vf` filter chain).
//! The operations form an **affine on the unit square** and compose
//! cleanly under the rules below; rotation is the outer operation applied
//! *after* flips (matching how ffmpeg `transpose` + `hflip`/`vflip` compose).

use serde::{Deserialize, Serialize};

/// Identity = unrotated, no flips. Rotation is expressed as quarter-turns
/// clockwise (0..=3). Compact (three fields) so it round-trips as a stable
/// JSON object and keeps diffs small.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipOrientation {
    /// Quarter-turns **clockwise**, `0..=3` after normalization.
    #[serde(default)]
    pub rotation_quarter_turns: u8,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
}

impl ClipOrientation {
    pub const IDENTITY: ClipOrientation = ClipOrientation {
        rotation_quarter_turns: 0,
        flip_h: false,
        flip_v: false,
    };

    pub fn is_identity(&self) -> bool {
        self.rotation_quarter_turns % 4 == 0 && !self.flip_h && !self.flip_v
    }

    /// Add one 90° **clockwise** step (normalized to 0..=3).
    pub fn rotate_right(&mut self) {
        self.rotation_quarter_turns = (self.rotation_quarter_turns + 1) % 4;
    }

    /// Add one 90° **counter-clockwise** step (normalized to 0..=3).
    pub fn rotate_left(&mut self) {
        self.rotation_quarter_turns = (self.rotation_quarter_turns + 3) % 4;
    }

    pub fn toggle_flip_h(&mut self) {
        self.flip_h = !self.flip_h;
    }

    pub fn toggle_flip_v(&mut self) {
        self.flip_v = !self.flip_v;
    }

    /// ffmpeg `-vf` filter chain, e.g. `"hflip,transpose=1"`. Returns `None`
    /// for identity (no filter → stream-copy fast path can stay).
    ///
    /// Flip filters come first so `transpose=1/2` rotates the flipped image
    /// and the result matches how [`apply_rgba`] composes the transforms.
    pub fn ffmpeg_filter_chain(&self) -> Option<String> {
        if self.is_identity() {
            return None;
        }
        let mut parts: Vec<&'static str> = Vec::new();
        if self.flip_h {
            parts.push("hflip");
        }
        if self.flip_v {
            parts.push("vflip");
        }
        match self.rotation_quarter_turns % 4 {
            1 => parts.push("transpose=1"),             // 90° CW
            2 => parts.push("transpose=2,transpose=2"), // 180°
            3 => parts.push("transpose=2"),             // 90° CCW
            _ => {}
        }
        if parts.is_empty() {
            return None;
        }
        Some(parts.join(","))
    }

    /// Apply the flip+rotation to a tightly-packed RGBA buffer of `w × h`
    /// pixels and return `(rgba, new_w, new_h)`. Returns `None` when the input
    /// buffer length doesn't match `w * h * 4` (caller keeps the original).
    pub fn apply_rgba(&self, src: &[u8], w: u32, h: u32) -> Option<(Vec<u8>, u32, u32)> {
        if self.is_identity() {
            return None;
        }
        let (wu, hu) = (w as usize, h as usize);
        if src.len() != wu.checked_mul(hu)?.checked_mul(4)? {
            return None;
        }

        // Stage 1: flips in place into a working buffer.
        let mut stage = vec![0u8; src.len()];
        for y in 0..hu {
            let sy = if self.flip_v { hu - 1 - y } else { y };
            for x in 0..wu {
                let sx = if self.flip_h { wu - 1 - x } else { x };
                let si = (sy * wu + sx) * 4;
                let di = (y * wu + x) * 4;
                stage[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }

        // Stage 2: rotation.
        match self.rotation_quarter_turns % 4 {
            0 => Some((stage, w, h)),
            1 => {
                // 90° CW: dst[x_new, y_new] = src[y_new, h_src - 1 - x_new].
                let new_w = h;
                let new_h = w;
                let (nw, nh) = (new_w as usize, new_h as usize);
                let mut out = vec![0u8; nw * nh * 4];
                for y in 0..nh {
                    for x in 0..nw {
                        let sx = y;
                        let sy = hu - 1 - x;
                        let si = (sy * wu + sx) * 4;
                        let di = (y * nw + x) * 4;
                        out[di..di + 4].copy_from_slice(&stage[si..si + 4]);
                    }
                }
                Some((out, new_w, new_h))
            }
            2 => {
                // 180°: reverse rows and columns.
                let mut out = vec![0u8; stage.len()];
                for y in 0..hu {
                    for x in 0..wu {
                        let sx = wu - 1 - x;
                        let sy = hu - 1 - y;
                        let si = (sy * wu + sx) * 4;
                        let di = (y * wu + x) * 4;
                        out[di..di + 4].copy_from_slice(&stage[si..si + 4]);
                    }
                }
                Some((out, w, h))
            }
            3 => {
                // 90° CCW.
                let new_w = h;
                let new_h = w;
                let (nw, nh) = (new_w as usize, new_h as usize);
                let mut out = vec![0u8; nw * nh * 4];
                for y in 0..nh {
                    for x in 0..nw {
                        let sx = wu - 1 - y;
                        let sy = x;
                        let si = (sy * wu + sx) * 4;
                        let di = (y * nw + x) * 4;
                        out[di..di + 4].copy_from_slice(&stage[si..si + 4]);
                    }
                }
                Some((out, new_w, new_h))
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_default() {
        let o = ClipOrientation::default();
        assert!(o.is_identity());
        assert_eq!(o.ffmpeg_filter_chain(), None);
    }

    #[test]
    fn rotate_right_cycles() {
        let mut o = ClipOrientation::default();
        o.rotate_right();
        assert_eq!(o.rotation_quarter_turns, 1);
        o.rotate_right();
        o.rotate_right();
        o.rotate_right();
        assert_eq!(o.rotation_quarter_turns, 0);
    }

    #[test]
    fn rotate_left_inverts_right() {
        let mut o = ClipOrientation::default();
        o.rotate_right();
        o.rotate_left();
        assert!(o.is_identity());
    }

    #[test]
    fn filter_chain_examples() {
        let o = ClipOrientation {
            rotation_quarter_turns: 1,
            flip_h: false,
            flip_v: false,
        };
        assert_eq!(o.ffmpeg_filter_chain().as_deref(), Some("transpose=1"));

        let o = ClipOrientation {
            rotation_quarter_turns: 3,
            flip_h: true,
            flip_v: false,
        };
        assert_eq!(
            o.ffmpeg_filter_chain().as_deref(),
            Some("hflip,transpose=2")
        );

        let o = ClipOrientation {
            rotation_quarter_turns: 2,
            flip_h: false,
            flip_v: false,
        };
        assert_eq!(
            o.ffmpeg_filter_chain().as_deref(),
            Some("transpose=2,transpose=2")
        );

        let o = ClipOrientation {
            rotation_quarter_turns: 0,
            flip_h: false,
            flip_v: true,
        };
        assert_eq!(o.ffmpeg_filter_chain().as_deref(), Some("vflip"));
    }

    /// A 2×2 image encoding pixel coordinates as the R channel:
    /// `(0,0)→0, (1,0)→1, (0,1)→10, (1,1)→11`.
    fn sample_2x2() -> Vec<u8> {
        let mut buf = vec![0u8; 2 * 2 * 4];
        for y in 0..2 {
            for x in 0..2 {
                let i = (y * 2 + x) * 4;
                buf[i] = (y * 10 + x) as u8; // R encodes (x, y)
                buf[i + 3] = 255; // A
            }
        }
        buf
    }

    fn red_at(buf: &[u8], w: u32, x: u32, y: u32) -> u8 {
        buf[((y * w + x) * 4) as usize]
    }

    #[test]
    fn rotate_right_2x2_moves_top_left_to_top_right() {
        let src = sample_2x2();
        let mut o = ClipOrientation::default();
        o.rotate_right();
        let (out, w, h) = o.apply_rgba(&src, 2, 2).expect("applied");
        assert_eq!((w, h), (2, 2));
        // Original (0,0) = 0 should move to (1,0); (1,0) = 1 → (1,1); (1,1) = 11 → (0,1); (0,1) = 10 → (0,0).
        assert_eq!(red_at(&out, w, 1, 0), 0);
        assert_eq!(red_at(&out, w, 1, 1), 1);
        assert_eq!(red_at(&out, w, 0, 1), 11);
        assert_eq!(red_at(&out, w, 0, 0), 10);
    }

    #[test]
    fn rotate_left_is_inverse_of_right() {
        let src = sample_2x2();
        let mut o = ClipOrientation::default();
        o.rotate_right();
        let (mid, w, h) = o.apply_rgba(&src, 2, 2).unwrap();
        o.rotate_left();
        // Orientation back to identity → apply_rgba returns None; mid after rotate_left of a single step:
        let mut undo = ClipOrientation::default();
        undo.rotate_left();
        let (back, bw, bh) = undo.apply_rgba(&mid, w, h).unwrap();
        assert_eq!((bw, bh), (2, 2));
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(red_at(&back, bw, x, y), red_at(&src, 2, x, y));
            }
        }
    }

    #[test]
    fn flip_h_swaps_columns() {
        let src = sample_2x2();
        let mut o = ClipOrientation::default();
        o.toggle_flip_h();
        let (out, w, h) = o.apply_rgba(&src, 2, 2).expect("applied");
        assert_eq!((w, h), (2, 2));
        assert_eq!(red_at(&out, w, 0, 0), 1);
        assert_eq!(red_at(&out, w, 1, 0), 0);
        assert_eq!(red_at(&out, w, 0, 1), 11);
        assert_eq!(red_at(&out, w, 1, 1), 10);
    }

    #[test]
    fn apply_rgba_rejects_wrong_length() {
        let o = ClipOrientation {
            rotation_quarter_turns: 1,
            flip_h: false,
            flip_v: false,
        };
        let short = vec![0u8; 4];
        assert!(o.apply_rgba(&short, 4, 4).is_none());
    }

    #[test]
    fn rotate_right_3x2_swaps_dimensions() {
        // 3-wide, 2-tall. R channel = y*10 + x.
        let w: u32 = 3;
        let h: u32 = 2;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                buf[i] = (y * 10 + x) as u8;
                buf[i + 3] = 255;
            }
        }
        let mut o = ClipOrientation::default();
        o.rotate_right();
        let (out, nw, nh) = o.apply_rgba(&buf, w, h).unwrap();
        assert_eq!((nw, nh), (2, 3));
        // Spot check: top-left of the source (0,0, R=0) ends up at top-right of the result (nw-1, 0).
        assert_eq!(red_at(&out, nw, nw - 1, 0), 0);
    }
}
