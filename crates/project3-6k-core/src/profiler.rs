use std::time::Duration;

/// Number of recent frame samples retained by [`FrameProfiler`].
pub const PROFILE_WINDOW_SIZE: usize = 120;

#[derive(Clone, Copy, Debug, Default)]
struct FrameSample {
    interval: Duration,
    renderer_cpu: Duration,
}

/// Rolling frame and renderer-CPU timing statistics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameProfileStats {
    /// Number of valid samples contributing to these statistics.
    pub sample_count: usize,
    /// Estimated frames per second from the mean observed frame interval.
    pub frames_per_second: f64,
    /// Mean time between application redraws, in milliseconds.
    pub average_frame_ms: f64,
    /// 95th percentile time between application redraws, in milliseconds.
    pub p95_frame_ms: f64,
    /// Longest time between application redraws, in milliseconds.
    pub maximum_frame_ms: f64,
    /// Mean wall time spent inside `VulkanRenderer::draw`, in milliseconds.
    pub average_renderer_cpu_ms: f64,
}

/// Fixed-size rolling profiler with no heap allocation while recording frames.
#[derive(Clone, Debug)]
pub struct FrameProfiler {
    samples: [FrameSample; PROFILE_WINDOW_SIZE],
    cursor: usize,
    sample_count: usize,
    report_count: usize,
}

impl Default for FrameProfiler {
    fn default() -> Self {
        Self {
            samples: [FrameSample::default(); PROFILE_WINDOW_SIZE],
            cursor: 0,
            sample_count: 0,
            report_count: 0,
        }
    }
}

impl FrameProfiler {
    /// Records one redraw interval and renderer call duration.
    ///
    /// Returns refreshed statistics after each full reporting window.
    pub fn record(
        &mut self,
        frame_interval: Duration,
        renderer_cpu: Duration,
    ) -> Option<FrameProfileStats> {
        self.samples[self.cursor] = FrameSample {
            interval: frame_interval,
            renderer_cpu,
        };
        self.cursor = (self.cursor + 1) % PROFILE_WINDOW_SIZE;
        self.sample_count = (self.sample_count + 1).min(PROFILE_WINDOW_SIZE);
        self.report_count += 1;
        if self.report_count == PROFILE_WINDOW_SIZE {
            self.report_count = 0;
            self.stats()
        } else {
            None
        }
    }

    /// Calculates statistics for all currently retained samples.
    pub fn stats(&self) -> Option<FrameProfileStats> {
        let count = self.sample_count;
        if count == 0 {
            return None;
        }

        let mut intervals = [Duration::ZERO; PROFILE_WINDOW_SIZE];
        let mut total_interval = Duration::ZERO;
        let mut total_renderer_cpu = Duration::ZERO;
        for (index, sample) in self.samples.iter().take(count).enumerate() {
            intervals[index] = sample.interval;
            total_interval = total_interval.saturating_add(sample.interval);
            total_renderer_cpu = total_renderer_cpu.saturating_add(sample.renderer_cpu);
        }
        intervals[..count].sort_unstable();
        let p95_rank = (count * 95).div_ceil(100).max(1);
        let interval_seconds = total_interval.as_secs_f64();
        let mean_interval = interval_seconds / count as f64;

        Some(FrameProfileStats {
            sample_count: count,
            frames_per_second: if mean_interval > 0.0 {
                1.0 / mean_interval
            } else {
                0.0
            },
            average_frame_ms: mean_interval * 1000.0,
            p95_frame_ms: intervals[p95_rank - 1].as_secs_f64() * 1000.0,
            maximum_frame_ms: intervals[count - 1].as_secs_f64() * 1000.0,
            average_renderer_cpu_ms: total_renderer_cpu.as_secs_f64() * 1000.0 / count as f64,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{FrameProfiler, PROFILE_WINDOW_SIZE};

    #[test]
    fn profiler_reports_rolling_interval_and_renderer_cpu_statistics() {
        let mut profiler = FrameProfiler::default();
        for index in 0..PROFILE_WINDOW_SIZE {
            let frame_interval = if index == PROFILE_WINDOW_SIZE - 1 {
                Duration::from_millis(100)
            } else {
                Duration::from_millis(16)
            };
            let report = profiler.record(frame_interval, Duration::from_millis(2));
            assert_eq!(report.is_some(), index == PROFILE_WINDOW_SIZE - 1);
        }

        let stats = profiler.stats().unwrap();
        assert_eq!(stats.sample_count, PROFILE_WINDOW_SIZE);
        assert!(stats.frames_per_second > 50.0);
        assert!(stats.p95_frame_ms < stats.maximum_frame_ms);
        assert!((stats.maximum_frame_ms - 100.0).abs() < 0.001);
        assert!((stats.average_renderer_cpu_ms - 2.0).abs() < 0.001);
    }

    #[test]
    fn profiler_overwrites_old_samples_without_growing_memory() {
        let mut profiler = FrameProfiler::default();
        for _ in 0..PROFILE_WINDOW_SIZE {
            profiler.record(Duration::from_millis(10), Duration::from_millis(1));
        }
        profiler.record(Duration::from_millis(100), Duration::from_millis(10));

        let before = profiler.stats().unwrap();
        assert_eq!(before.maximum_frame_ms, 100.0);

        for _ in 0..PROFILE_WINDOW_SIZE {
            profiler.record(Duration::from_millis(10), Duration::from_millis(1));
        }

        let stats = profiler.stats().unwrap();
        assert_eq!(stats.sample_count, PROFILE_WINDOW_SIZE);
        assert!(stats.maximum_frame_ms <= 10.0);
        assert!(stats.average_renderer_cpu_ms < 2.0);
    }
}
