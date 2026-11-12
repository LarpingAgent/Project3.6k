use std::time::Duration;

use crate::EngineError;

/// Upper bound for per-frame simulation work, even with caller-provided config.
pub const MAX_FIXED_UPDATES_PER_FRAME: u32 = 32;
/// Upper bound for preallocated engine events, preventing accidental huge allocations.
pub const MAX_EVENT_QUEUE_CAPACITY: usize = 65_536;

/// Bounded timing and event limits for the engine runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    /// Simulation interval used by fixed updates.
    pub fixed_update_interval: Duration,
    /// Maximum elapsed time accepted by one rendered frame.
    pub max_frame_delta: Duration,
    /// Maximum fixed updates run in one frame before excess whole steps are dropped.
    pub max_fixed_updates_per_frame: u32,
    /// Maximum queued events before producers receive a typed error.
    pub event_queue_capacity: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            fixed_update_interval: Duration::from_nanos(16_666_667),
            max_frame_delta: Duration::from_millis(100),
            max_fixed_updates_per_frame: 5,
            event_queue_capacity: 2_048,
        }
    }
}

impl EngineConfig {
    pub(crate) fn validate(self) -> Result<Self, EngineError> {
        if self.fixed_update_interval.is_zero() {
            return Err(EngineError::InvalidConfiguration(
                "fixed_update_interval must be nonzero",
            ));
        }
        if self.max_frame_delta < self.fixed_update_interval {
            return Err(EngineError::InvalidConfiguration(
                "max_frame_delta must be at least one fixed update interval",
            ));
        }
        if self.max_fixed_updates_per_frame == 0 {
            return Err(EngineError::InvalidConfiguration(
                "max_fixed_updates_per_frame must be nonzero",
            ));
        }
        if self.max_fixed_updates_per_frame > MAX_FIXED_UPDATES_PER_FRAME {
            return Err(EngineError::InvalidConfiguration(
                "max_fixed_updates_per_frame exceeds the engine safety limit",
            ));
        }
        if self.event_queue_capacity == 0 {
            return Err(EngineError::InvalidConfiguration(
                "event_queue_capacity must be nonzero",
            ));
        }
        if self.event_queue_capacity > MAX_EVENT_QUEUE_CAPACITY {
            return Err(EngineError::InvalidConfiguration(
                "event_queue_capacity exceeds the engine allocation limit",
            ));
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{EngineConfig, MAX_EVENT_QUEUE_CAPACITY, MAX_FIXED_UPDATES_PER_FRAME};
    use crate::{EngineError, EngineRuntime};

    #[test]
    fn rejects_configurations_that_exceed_bounded_work_or_allocation_limits() {
        let too_many_ticks = EngineRuntime::new(EngineConfig {
            max_fixed_updates_per_frame: MAX_FIXED_UPDATES_PER_FRAME + 1,
            ..EngineConfig::default()
        });
        assert!(matches!(
            too_many_ticks,
            Err(EngineError::InvalidConfiguration(_))
        ));

        let oversized_queue = EngineRuntime::new(EngineConfig {
            event_queue_capacity: MAX_EVENT_QUEUE_CAPACITY + 1,
            ..EngineConfig::default()
        });
        assert!(matches!(
            oversized_queue,
            Err(EngineError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn rejects_frame_delta_shorter_than_fixed_interval() {
        let result = EngineRuntime::new(EngineConfig {
            fixed_update_interval: Duration::from_millis(20),
            max_frame_delta: Duration::from_millis(10),
            ..EngineConfig::default()
        });
        assert!(matches!(result, Err(EngineError::InvalidConfiguration(_))));
    }
}
