use std::time::Duration;

pub const TARGET_FRAME_RATE: u32 = 240;
pub const TARGET_FRAME_DURATION: Duration =
    Duration::from_micros(1_000_000 / TARGET_FRAME_RATE as u64);
pub const MAX_ANIMATION_FRAME_DURATION: Duration = Duration::from_millis(16);

pub fn animation_frame_duration(elapsed: Option<Duration>) -> Duration {
    elapsed
        .unwrap_or(TARGET_FRAME_DURATION)
        .clamp(TARGET_FRAME_DURATION, MAX_ANIMATION_FRAME_DURATION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_frame_duration_tracks_240_hz_budget() {
        assert_eq!(TARGET_FRAME_RATE, 240);
        assert_eq!(TARGET_FRAME_DURATION.as_micros(), 4166);
    }

    #[test]
    fn animation_frame_duration_uses_real_elapsed_time_with_bounds() {
        assert_eq!(animation_frame_duration(None), TARGET_FRAME_DURATION);
        assert_eq!(
            animation_frame_duration(Some(Duration::from_micros(500))),
            TARGET_FRAME_DURATION
        );
        assert_eq!(
            animation_frame_duration(Some(Duration::from_millis(9))),
            Duration::from_millis(9)
        );
        assert_eq!(
            animation_frame_duration(Some(Duration::from_millis(40))),
            MAX_ANIMATION_FRAME_DURATION
        );
    }
}
