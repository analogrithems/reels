//! Human-readable time labels for the timeline transport strip.

use crate::AppWindow;

/// Clamp sequence playhead to **[0, duration]** (handles empty / zero-duration timelines).
#[must_use]
pub(crate) fn clamp_playhead_ms(playhead_ms: f32, duration_ms: f32) -> f32 {
    let d = duration_ms.max(0.0);
    playhead_ms.clamp(0.0, d)
}

/// Timeline elapsed + total labels (playhead clamped to duration).
pub(crate) fn refresh_time_labels(w: &AppWindow, playhead_ms: f32, duration_ms: f32) {
    let ph = clamp_playhead_ms(playhead_ms, duration_ms);
    w.set_time_elapsed(fmt_ms(ph).into());
    w.set_time_total(fmt_ms(duration_ms).into());
}

/// Single time value (shared with per-track lane labels).
pub(crate) fn format_ms_alone(ms: f32) -> String {
    fmt_ms(ms)
}

fn fmt_ms(ms: f32) -> String {
    let ms_u = ms.round().clamp(0.0, u64::MAX as f32) as u64;
    let total_tenths = (ms_u + 50) / 100;
    let whole_sec = total_tenths / 10;
    let tenth = (total_tenths % 10) as u32;
    let h = whole_sec / 3600;
    let rem = whole_sec % 3600;
    let m = rem / 60;
    let s = rem % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{tenth}")
    } else {
        format!("{m}:{s:02}.{tenth}")
    }
}

/// Update slider + elapsed/total labels together (player thread → UI thread).
pub(crate) fn apply_playhead_transport(w: &AppWindow, playhead_ms: f32) {
    let dur = w.get_duration_ms();
    let ph = clamp_playhead_ms(playhead_ms, dur);
    refresh_time_labels(w, ph, dur);
    w.set_playhead_ms(ph);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_subminute_tenths() {
        assert_eq!(format_ms_alone(123.0), "0:00.1");
        assert_eq!(format_ms_alone(12_345.0), "0:12.3");
    }

    #[test]
    fn formats_over_minute_tenths() {
        assert_eq!(format_ms_alone(61_234.0), "1:01.2");
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
