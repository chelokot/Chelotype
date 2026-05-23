use std::time::Duration;

const SCROLL_ANIMATION_LENGTH: Duration = Duration::from_millis(100);
const SCROLL_SETTLE_LINES: f64 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmoothScrollFrame {
    pub line_delta: i32,
    pub offset_px: f64,
    pub remaining_px: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmoothScroll {
    pending_px: f64,
    offset_px: f64,
    velocity_px: f64,
    fresh: bool,
}

impl SmoothScroll {
    pub fn enqueue_lines(&mut self, lines: i32, line_height: f64) {
        self.enqueue_pixels(f64::from(lines) * line_height);
    }

    pub fn enqueue_pixels(&mut self, pixels: f64) {
        let was_active = self.is_active();
        self.pending_px += pixels;
        if !was_active && self.is_active() {
            self.fresh = true;
        }
    }

    pub fn enqueue_pixels_clamped(
        &mut self,
        pixels: f64,
        line_height: f64,
        available_lines: usize,
    ) -> f64 {
        if pixels == 0.0 || line_height <= 0.0 {
            return 0.0;
        }
        let candidate_unapplied_px = self.pending_px + pixels;
        let clamped_unapplied_px = if candidate_unapplied_px.signum() == pixels.signum() {
            let available_px = available_lines as f64 * line_height;
            candidate_unapplied_px
                .abs()
                .min(available_px)
                .copysign(candidate_unapplied_px)
        } else {
            candidate_unapplied_px
        };
        let was_active = self.is_active();
        let enqueued_px = clamped_unapplied_px - self.pending_px;
        self.pending_px = clamped_unapplied_px;
        if !was_active && self.is_active() {
            self.fresh = true;
        }
        enqueued_px
    }

    pub fn advance(
        &mut self,
        frame_duration: Duration,
        line_height: f64,
    ) -> Option<SmoothScrollFrame> {
        if !self.is_active() || line_height <= 0.0 {
            return None;
        }
        let frame_duration = if self.fresh {
            self.fresh = false;
            crate::frame_timing::TARGET_FRAME_DURATION
        } else {
            frame_duration
        };

        let line_delta = (self.pending_px / line_height).trunc() as i32;
        if line_delta != 0 {
            let applied_px = f64::from(line_delta) * line_height;
            self.pending_px -= applied_px;
            self.offset_px -= applied_px;
        } else if self.offset_px == 0.0 {
            self.offset_px += self.pending_px;
            self.pending_px = 0.0;
        }

        self.advance_spring(frame_duration, line_height);

        let remaining_px = self.remaining_px();
        Some(SmoothScrollFrame {
            line_delta,
            offset_px: self.offset_px,
            remaining_px,
        })
    }

    pub fn remaining_px(&self) -> f64 {
        self.pending_px.abs() + self.offset_px.abs()
    }

    pub fn visual_px(&self) -> f64 {
        self.offset_px
    }

    pub fn is_active(&self) -> bool {
        self.pending_px.abs() >= 0.5 || self.offset_px != 0.0
    }

    pub fn cancel(&mut self) {
        self.pending_px = 0.0;
        self.offset_px = 0.0;
        self.velocity_px = 0.0;
        self.fresh = false;
    }

    fn advance_spring(&mut self, frame_duration: Duration, line_height: f64) {
        if self.offset_px == 0.0 {
            self.velocity_px = 0.0;
            return;
        }

        if SCROLL_ANIMATION_LENGTH <= frame_duration {
            self.offset_px = 0.0;
            self.velocity_px = 0.0;
            return;
        }

        let dt = frame_duration.as_secs_f64();
        let animation_length = SCROLL_ANIMATION_LENGTH.as_secs_f64();
        let omega = 4.0 / animation_length;
        let position = self.offset_px;
        let velocity = self.velocity_px;
        let a = position;
        let b = position * omega + velocity;
        let decay = (-omega * dt).exp();

        self.offset_px = (a + b * dt) * decay;
        self.velocity_px = decay * (-a * omega - b * dt * omega + b);

        if self.offset_px.abs() < line_height * SCROLL_SETTLE_LINES {
            self.offset_px = 0.0;
            self.velocity_px = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animates_positive_scroll_with_fractional_pixel_steps() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(3, 20.0);

        let frame = scroll
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("smooth scroll frame");
        assert_eq!(frame.line_delta, 3);
        assert!((-59.3..-59.2).contains(&frame.offset_px), "{frame:?}");
        assert!((59.2..59.3).contains(&frame.remaining_px), "{frame:?}");
    }

    #[test]
    fn coalesces_opposite_scroll_directions() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(3, 20.0);
        scroll.enqueue_lines(-1, 20.0);

        let frame = scroll
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("smooth scroll frame");
        assert_eq!(frame.line_delta, 2);
        assert!((-39.6..-39.5).contains(&frame.offset_px), "{frame:?}");
        assert!((39.5..39.6).contains(&frame.remaining_px), "{frame:?}");
    }

    #[test]
    fn accepts_fractional_pixel_scroll_without_quantizing_to_lines() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_pixels(2.5);

        let frame = scroll
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("smooth scroll frame");

        assert_eq!(frame.line_delta, 0);
        assert!((2.4..2.5).contains(&frame.offset_px), "{frame:?}");
        assert!((2.4..2.5).contains(&frame.remaining_px), "{frame:?}");
    }

    #[test]
    fn clamps_enqueued_pixels_to_available_scrollback_distance() {
        let mut scroll = SmoothScroll::default();

        let enqueued = scroll.enqueue_pixels_clamped(60.0, 20.0, 2);

        assert_eq!(enqueued, 40.0);
        let frame = scroll
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("clamped scroll frame");
        assert_eq!(frame.line_delta, 2);
        assert!((-39.6..-39.5).contains(&frame.offset_px), "{frame:?}");
        assert!((39.5..39.6).contains(&frame.remaining_px), "{frame:?}");
    }

    #[test]
    fn clamps_repeated_enqueue_against_existing_pending_distance() {
        let mut scroll = SmoothScroll::default();

        assert_eq!(scroll.enqueue_pixels_clamped(60.0, 20.0, 2), 40.0);
        assert_eq!(scroll.enqueue_pixels_clamped(60.0, 20.0, 2), 0.0);
        assert_eq!(scroll.remaining_px(), 40.0);
    }

    #[test]
    fn opposite_enqueue_can_reduce_pending_distance_at_scroll_limit() {
        let mut scroll = SmoothScroll::default();

        assert_eq!(scroll.enqueue_pixels_clamped(60.0, 20.0, 3), 60.0);
        assert_eq!(scroll.enqueue_pixels_clamped(-20.0, 20.0, 0), -20.0);
        assert_eq!(scroll.remaining_px(), 40.0);
    }

    #[test]
    fn starts_positive_scroll_by_revealing_the_next_viewport_line() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(1, 20.0);

        let frame = scroll
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("scroll start frame");
        assert_eq!(frame.line_delta, 1);
        assert!((-19.8..-19.7).contains(&frame.offset_px), "{frame:?}");
    }

    #[test]
    fn starts_negative_scroll_by_revealing_the_previous_viewport_line() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(-1, 20.0);

        let frame = scroll
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("scroll start frame");
        assert_eq!(frame.line_delta, -1);
        assert!((19.7..19.8).contains(&frame.offset_px), "{frame:?}");
    }

    #[test]
    fn preapplies_incoming_logical_lines_as_visual_distance_crosses_rows() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(2, 20.0);

        let first = scroll
            .advance(Duration::from_millis(8), 20.0)
            .expect("first partial-line frame");
        assert_eq!(first.line_delta, 2);
        assert!((-39.6..-39.5).contains(&first.offset_px), "{first:?}");

        let second = scroll
            .advance(Duration::from_millis(8), 20.0)
            .expect("second partial-line frame");
        assert_eq!(second.line_delta, 0);
        assert!((-36.6..-36.5).contains(&second.offset_px), "{second:?}");

        let final_frame = scroll
            .advance(Duration::from_millis(100), 20.0)
            .expect("final frame");
        assert_eq!(final_frame.line_delta, 0);
        assert_eq!(final_frame.offset_px, 0.0);
        assert_eq!(final_frame.remaining_px, 0.0);
    }

    #[test]
    fn settles_when_remaining_offset_is_subpixel() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(-1, 20.0);

        for _ in 0..80 {
            let _ = scroll.advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0);
        }
        assert!(!scroll.is_active());
        assert_eq!(scroll.remaining_px(), 0.0);
    }

    #[test]
    fn cancel_clears_pending_visual_scroll() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(3, 20.0);
        let _ = scroll.advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0);

        scroll.cancel();

        assert!(!scroll.is_active());
        assert_eq!(scroll.remaining_px(), 0.0);
    }

    #[test]
    fn first_active_frame_ignores_idle_gap_duration() {
        let mut target_frame_scroll = SmoothScroll::default();
        target_frame_scroll.enqueue_lines(1, 20.0);
        let target_frame = target_frame_scroll
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("target frame");

        let mut delayed_scroll = SmoothScroll::default();
        delayed_scroll.enqueue_lines(1, 20.0);
        let delayed_frame = delayed_scroll
            .advance(Duration::from_millis(100), 20.0)
            .expect("delayed first frame");

        assert_eq!(delayed_frame, target_frame);
    }

    #[test]
    fn actual_longer_frame_duration_advances_farther_after_animation_starts() {
        let mut target_frame_scroll = SmoothScroll::default();
        target_frame_scroll.enqueue_lines(1, 20.0);
        let _ = target_frame_scroll.advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0);
        let target_frame = target_frame_scroll
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("target frame");

        let mut longer_frame_scroll = SmoothScroll::default();
        longer_frame_scroll.enqueue_lines(1, 20.0);
        let _ = longer_frame_scroll.advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0);
        let longer_frame = longer_frame_scroll
            .advance(Duration::from_millis(8), 20.0)
            .expect("longer frame");

        assert!(longer_frame.remaining_px < target_frame.remaining_px);
        assert!(longer_frame.offset_px.abs() < target_frame.offset_px.abs());
    }

    #[test]
    fn clamps_long_frames_to_remaining_target_without_overshooting() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(1, 20.0);
        let _ = scroll.advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0);

        let frame = scroll
            .advance(Duration::from_millis(100), 20.0)
            .expect("long frame");

        assert_eq!(frame.line_delta, 0);
        assert_eq!(frame.offset_px, 0.0);
        assert_eq!(frame.remaining_px, 0.0);
        assert!(!scroll.is_active());
    }

    #[test]
    fn accelerates_large_backlogs_by_distance_without_speed_cap() {
        let mut single_tick = SmoothScroll::default();
        single_tick.enqueue_lines(3, 20.0);
        let single_frame = single_tick
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("single tick frame");

        let mut backlog = SmoothScroll::default();
        backlog.enqueue_lines(24, 20.0);
        let backlog_frame = backlog
            .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
            .expect("backlog frame");

        assert!((-59.3..-59.2).contains(&single_frame.offset_px));
        assert_eq!(backlog_frame.line_delta, 24);
        assert!(
            (-474.1..-474.0).contains(&backlog_frame.offset_px),
            "{backlog_frame:?}"
        );
        assert!(
            (474.0..474.1).contains(&backlog_frame.remaining_px),
            "{backlog_frame:?}"
        );
    }

    #[test]
    fn produces_pixel_level_frames_at_240hz_until_settled() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue_lines(10, 20.0);

        let mut frames = Vec::new();
        while scroll.is_active() {
            frames.push(
                scroll
                    .advance(crate::frame_timing::TARGET_FRAME_DURATION, 20.0)
                    .expect("smooth scroll frame"),
            );
        }

        assert!(frames.len() >= 8, "{frames:?}");
        assert_eq!(frames.last().expect("final frame").remaining_px, 0.0);
        assert_eq!(frames.last().expect("final frame").offset_px, 0.0);
        assert!(frames.iter().all(|frame| frame.remaining_px >= 0.0));
        assert!(
            frames
                .windows(2)
                .all(|window| window[1].remaining_px <= window[0].remaining_px),
            "{frames:?}"
        );
        assert!(
            frames
                .iter()
                .any(|frame| frame.line_delta == 0 && frame.offset_px.abs() > 0.5),
            "{frames:?}"
        );
        assert!(
            frames
                .iter()
                .any(|frame| frame.line_delta != 0 && frame.offset_px.abs() > 0.5),
            "{frames:?}"
        );
    }
}
