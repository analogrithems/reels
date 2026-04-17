//! Human-readable time labels for the timeline transport strip.

use crate::AppWindow;

/// Clamp sequence playhead to **[0, duration]** (handles empty / zero-duration timelines).
#[must_use]
pub(crate) fn clamp_playhead_ms(playhead_ms: f32, duration_ms: f32) -> f32 {
    let d = duration_ms.max(0.0);
    playhead_ms.clamp(0.0, d)
}

/// `playhead / duration` as `M:SS.mmm` (or `H:MM:SS.mmm` when needed).
pub(crate) fn format_pair(playhead_ms: f32, duration_ms: f32) -> String {
    format!("{} / {}", fmt_ms(playhead_ms), fmt_ms(duration_ms))
}

/// Single time value as `M:SS.mmm` (shared with per-track lane labels).
pub(crate) fn format_ms_alone(ms: f32) -> String {
    fmt_ms(ms)
}

fn fmt_ms(ms: f32) -> String {
    let t = ms.round().clamp(0.0, u64::MAX as f32) as u64;
    let frac = (t % 1000) as u32;
    let total_s = t / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{frac:03}")
    } else {
        format!("{m}:{s:02}.{frac:03}")
    }
}

/// Update slider + timecode label together (player thread → UI thread).
pub(crate) fn apply_playhead_transport(w: &AppWindow, playhead_ms: f32) {
    let dur = w.get_duration_ms();
    let ph = clamp_playhead_ms(playhead_ms, dur);
    w.set_timecode(format_pair(ph, dur).into());
    w.set_playhead_ms(ph);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_subminute() {
        assert_eq!(format_pair(123.0, 12_345.0), "0:00.123 / 0:12.345");
    }

    #[test]
    fn formats_over_minute() {
        assert_eq!(format_pair(61_234.0, 61_234.0), "1:01.234 / 1:01.234");
    }

    #[test]
    fn clamp_playhead_never_negative() {
        assert_eq!(super::clamp_playhead_ms(-10.0, 5000.0), 0.0);
    }

    #[test]
    fn clamp_playhead_respects_duration() {
        assert_eq!(super::clamp_playhead_ms(9999.0, 5000.0), 5000.0);
    }

    #[test]
    fn clamp_playhead_zero_duration() {
        assert_eq!(super::clamp_playhead_ms(100.0, 0.0), 0.0);
    }

    #[test]
    fn clamp_playhead_identity() {
        assert_eq!(super::clamp_playhead_ms(1234.0, 5000.0), 1234.0);
    }
}
