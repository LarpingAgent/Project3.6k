//! Small engine-wide primitives that have no platform or renderer dependency.

use std::time::{Duration, Instant};

mod camera;
mod scene;

pub use camera::Camera;
pub use scene::{MeshHandle, Scene, SceneError, SceneNode, SceneNodeId, Transform};

/// Monotonic timing information for one application frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTime {
    /// Time since the previous call to [`FrameClock::tick`].
    pub delta: Duration,
    /// Time since this clock was created.
    pub elapsed: Duration,
    /// Zero-based frame number.
    pub frame_index: u64,
}

/// Tracks frame timing without allocating or reading a wall clock.
#[derive(Debug)]
pub struct FrameClock {
    started_at: Instant,
    last_tick: Instant,
    frame_index: u64,
}

impl Default for FrameClock {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl FrameClock {
    /// Creates a frame clock with a caller-provided monotonic timestamp.
    pub fn new(now: Instant) -> Self {
        Self {
            started_at: now,
            last_tick: now,
            frame_index: 0,
        }
    }

    /// Records a frame using the current monotonic time.
    pub fn tick(&mut self) -> FrameTime {
        self.tick_at(Instant::now())
    }

    /// Records a frame at `now`. This form also makes timing behavior testable.
    pub fn tick_at(&mut self, now: Instant) -> FrameTime {
        let frame_time = FrameTime {
            delta: now.saturating_duration_since(self.last_tick),
            elapsed: now.saturating_duration_since(self.started_at),
            frame_index: self.frame_index,
        };
        self.last_tick = now;
        self.frame_index = self.frame_index.saturating_add(1);
        frame_time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_monotonic_deltas_and_indices() {
        let start = Instant::now();
        let mut clock = FrameClock::new(start);

        let first = clock.tick_at(start + Duration::from_millis(16));
        let second = clock.tick_at(start + Duration::from_millis(41));

        assert_eq!(first.delta, Duration::from_millis(16));
        assert_eq!(first.elapsed, Duration::from_millis(16));
        assert_eq!(first.frame_index, 0);
        assert_eq!(second.delta, Duration::from_millis(25));
        assert_eq!(second.elapsed, Duration::from_millis(41));
        assert_eq!(second.frame_index, 1);
    }

    #[test]
    fn backwards_timestamp_does_not_underflow() {
        let start = Instant::now();
        let mut clock = FrameClock::new(start);
        let earlier = start.checked_sub(Duration::from_millis(1)).unwrap_or(start);

        assert_eq!(clock.tick_at(earlier).delta, Duration::ZERO);
    }
}
