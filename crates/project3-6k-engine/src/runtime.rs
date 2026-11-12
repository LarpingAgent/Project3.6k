use std::{
    marker::PhantomData,
    rc::Rc,
    time::{Duration, Instant},
};

use project3_6k_core::{FrameClock, FrameTime};
use thiserror::Error;
use tracing::{info, warn};

use crate::{EngineConfig, EngineEvent, EngineInput, event::EventQueue};

/// Lifecycle state owned by the engine runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineState {
    Created,
    Running,
    StopRequested,
    Stopped,
}

/// Failures from lifecycle, configuration, event dispatch, and frame scheduling.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EngineError {
    #[error("invalid engine configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("cannot {operation} while engine is {state:?}")]
    InvalidStateTransition {
        state: EngineState,
        operation: &'static str,
    },
    #[error("engine event queue reached its capacity of {capacity}")]
    EventQueueFull { capacity: usize },
    #[error("pointer motion must contain finite values")]
    NonFinitePointerMotion,
}

/// One deterministic fixed simulation update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedUpdate {
    pub tick_index: u64,
    pub delta: Duration,
}

/// One variable update after fixed updates have run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VariableUpdate {
    pub frame_time: FrameTime,
    pub delta: Duration,
    pub interpolation_alpha: f64,
}

/// Scheduling evidence for the most recent rendered frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSchedule {
    pub frame_time: FrameTime,
    pub variable_delta: Duration,
    pub fixed_updates: u32,
    pub interpolation_alpha: f64,
    pub dropped_simulation_time: Duration,
    pub events_dispatched: usize,
}

/// Main-thread engine state. It owns no window, worker, or GPU resources.
#[derive(Debug)]
pub struct EngineRuntime {
    config: EngineConfig,
    state: EngineState,
    events: EventQueue,
    input: EngineInput,
    clock: Option<FrameClock>,
    fixed_accumulator: Duration,
    fixed_tick_index: u64,
    _owner_thread_only: PhantomData<Rc<()>>,
}

impl Default for EngineRuntime {
    fn default() -> Self {
        Self::new(EngineConfig::default()).expect("default engine configuration is valid")
    }
}

impl EngineRuntime {
    pub fn new(config: EngineConfig) -> Result<Self, EngineError> {
        let config = config.validate()?;
        Ok(Self {
            config,
            state: EngineState::Created,
            events: EventQueue::new(config.event_queue_capacity),
            input: EngineInput::default(),
            clock: None,
            fixed_accumulator: Duration::ZERO,
            fixed_tick_index: 0,
            _owner_thread_only: PhantomData,
        })
    }

    pub fn config(&self) -> EngineConfig {
        self.config
    }

    pub fn state(&self) -> EngineState {
        self.state
    }

    pub fn input(&self) -> &EngineInput {
        &self.input
    }

    pub fn input_mut(&mut self) -> &mut EngineInput {
        &mut self.input
    }

    pub fn pending_event_count(&self) -> usize {
        self.events.len()
    }

    pub fn start(&mut self) -> Result<(), EngineError> {
        self.start_at(Instant::now())
    }

    pub fn start_at(&mut self, now: Instant) -> Result<(), EngineError> {
        if self.state != EngineState::Created {
            return Err(self.invalid_transition("start"));
        }
        self.clock = Some(FrameClock::new(now));
        self.fixed_accumulator = Duration::ZERO;
        self.fixed_tick_index = 0;
        self.state = EngineState::Running;
        info!("engine started");
        Ok(())
    }

    /// Enqueues a typed event and immediately updates action/pointer state.
    /// Event callbacks are invoked in FIFO order by `dispatch_pending_events`
    /// or `run_frame`.
    pub fn queue_event(&mut self, event: EngineEvent) -> Result<(), EngineError> {
        if self.state != EngineState::Running {
            return Err(self.invalid_transition("queue an event"));
        }
        self.events.push(event, &mut self.input)
    }

    /// Dispatches all currently queued events. Exit requests transition the
    /// runtime to `StopRequested` after the event is delivered to the handler.
    pub fn dispatch_pending_events(
        &mut self,
        mut handler: impl FnMut(&EngineEvent),
    ) -> Result<usize, EngineError> {
        if !matches!(
            self.state,
            EngineState::Running | EngineState::StopRequested
        ) {
            return Err(self.invalid_transition("dispatch events"));
        }
        let mut dispatched = 0;
        while let Some(event) = self.events.pop() {
            handler(&event);
            dispatched += 1;
            if event == EngineEvent::ExitRequested && self.state == EngineState::Running {
                self.state = EngineState::StopRequested;
            }
        }
        Ok(dispatched)
    }

    /// Runs event dispatch, fixed updates, and one variable update in order.
    /// The platform host renders after this call returns.
    pub fn run_frame(
        &mut self,
        now: Instant,
        mut on_event: impl FnMut(&EngineEvent),
        mut fixed_update: impl FnMut(FixedUpdate),
        mut variable_update: impl FnMut(VariableUpdate),
    ) -> Result<FrameSchedule, EngineError> {
        if self.state != EngineState::Running {
            return Err(self.invalid_transition("run a frame"));
        }
        let frame_time = self
            .clock
            .as_mut()
            .expect("running engine always has a frame clock")
            .tick_at(now);
        let events_dispatched = self.dispatch_pending_events(&mut on_event)?;
        let variable_delta = frame_time.delta.min(self.config.max_frame_delta);
        if self.state != EngineState::Running {
            return Ok(FrameSchedule {
                frame_time,
                variable_delta,
                fixed_updates: 0,
                interpolation_alpha: 0.0,
                dropped_simulation_time: Duration::ZERO,
                events_dispatched,
            });
        }

        self.fixed_accumulator = self.fixed_accumulator.saturating_add(variable_delta);
        let fixed_delta = self.config.fixed_update_interval;
        let mut fixed_updates = 0;
        while self.fixed_accumulator >= fixed_delta
            && fixed_updates < self.config.max_fixed_updates_per_frame
        {
            fixed_update(FixedUpdate {
                tick_index: self.fixed_tick_index,
                delta: fixed_delta,
            });
            self.fixed_tick_index = self.fixed_tick_index.saturating_add(1);
            self.fixed_accumulator -= fixed_delta;
            fixed_updates += 1;
        }

        let dropped_simulation_time = if self.fixed_accumulator >= fixed_delta {
            let fixed_nanos = fixed_delta.as_nanos();
            let remainder_nanos = self.fixed_accumulator.as_nanos() % fixed_nanos;
            let remainder = duration_from_nanos(remainder_nanos);
            let dropped = self.fixed_accumulator.saturating_sub(remainder);
            self.fixed_accumulator = remainder;
            warn!(
                dropped_ms = dropped.as_secs_f64() * 1000.0,
                "fixed update catch-up limit reached; dropping excess simulation time"
            );
            dropped
        } else {
            Duration::ZERO
        };
        let interpolation_alpha = self.fixed_accumulator.as_secs_f64() / fixed_delta.as_secs_f64();
        variable_update(VariableUpdate {
            frame_time,
            delta: variable_delta,
            interpolation_alpha,
        });

        Ok(FrameSchedule {
            frame_time,
            variable_delta,
            fixed_updates,
            interpolation_alpha,
            dropped_simulation_time,
            events_dispatched,
        })
    }

    pub fn request_shutdown(&mut self) -> Result<(), EngineError> {
        match self.state {
            EngineState::Running => {
                self.state = EngineState::StopRequested;
                info!("engine shutdown requested");
                Ok(())
            }
            EngineState::StopRequested => Ok(()),
            _ => Err(self.invalid_transition("request shutdown")),
        }
    }

    /// Marks runtime shutdown complete. Repeated calls are harmless.
    pub fn shutdown(&mut self) {
        if self.state == EngineState::Stopped {
            return;
        }
        if self.state == EngineState::Running {
            self.state = EngineState::StopRequested;
            warn!("engine shutdown began without an explicit stop request");
        }
        self.events.clear();
        self.input.clear();
        self.clock = None;
        self.fixed_accumulator = Duration::ZERO;
        self.state = EngineState::Stopped;
        info!("engine stopped");
    }

    fn invalid_transition(&self, operation: &'static str) -> EngineError {
        EngineError::InvalidStateTransition {
            state: self.state,
            operation,
        }
    }
}

fn duration_from_nanos(nanos: u128) -> Duration {
    Duration::new(
        (nanos / 1_000_000_000) as u64,
        (nanos % 1_000_000_000) as u32,
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use super::{
        EngineConfig, EngineError, EngineEvent, EngineRuntime, EngineState, FixedUpdate,
        VariableUpdate,
    };
    use crate::ActionId;

    fn config() -> EngineConfig {
        EngineConfig {
            fixed_update_interval: Duration::from_millis(10),
            max_frame_delta: Duration::from_millis(100),
            max_fixed_updates_per_frame: 4,
            event_queue_capacity: 8,
        }
    }

    #[test]
    fn lifecycle_transitions_are_checked_and_shutdown_is_idempotent() {
        let start = std::time::Instant::now();
        let mut engine = EngineRuntime::new(config()).unwrap();
        assert_eq!(engine.state(), EngineState::Created);
        assert!(matches!(
            engine.request_shutdown(),
            Err(EngineError::InvalidStateTransition { .. })
        ));
        engine.start_at(start).unwrap();
        assert_eq!(engine.state(), EngineState::Running);
        engine.request_shutdown().unwrap();
        assert_eq!(engine.state(), EngineState::StopRequested);
        engine.shutdown();
        engine.shutdown();
        assert_eq!(engine.state(), EngineState::Stopped);
        assert!(matches!(
            engine.start(),
            Err(EngineError::InvalidStateTransition { .. })
        ));
    }

    #[test]
    fn queue_is_bounded_and_rejected_events_do_not_change_input_state() {
        let start = std::time::Instant::now();
        let mut engine = EngineRuntime::new(EngineConfig {
            event_queue_capacity: 1,
            ..config()
        })
        .unwrap();
        engine.start_at(start).unwrap();
        let action = ActionId::new(5);
        engine
            .queue_event(EngineEvent::WindowResized {
                width: 640,
                height: 480,
            })
            .unwrap();
        let error = engine
            .queue_event(EngineEvent::ActionChanged {
                action,
                pressed: true,
            })
            .unwrap_err();
        assert_eq!(error, EngineError::EventQueueFull { capacity: 1 });
        assert!(!engine.input().is_active(action));
    }

    #[test]
    fn events_dispatch_fifo_before_fixed_and_variable_updates_then_host_render() {
        let start = std::time::Instant::now();
        let mut engine = EngineRuntime::new(config()).unwrap();
        engine.start_at(start).unwrap();
        engine
            .queue_event(EngineEvent::WindowResized {
                width: 800,
                height: 600,
            })
            .unwrap();
        engine
            .queue_event(EngineEvent::ActionChanged {
                action: ActionId::new(9),
                pressed: true,
            })
            .unwrap();
        let order = Rc::new(RefCell::new(Vec::new()));
        let events_order = order.clone();
        let fixed_order = order.clone();
        let variable_order = order.clone();
        let schedule = engine
            .run_frame(
                start + Duration::from_millis(20),
                move |_| events_order.borrow_mut().push("event"),
                move |_: FixedUpdate| fixed_order.borrow_mut().push("fixed"),
                move |_: VariableUpdate| variable_order.borrow_mut().push("update"),
            )
            .unwrap();
        order.borrow_mut().push("render");
        assert_eq!(schedule.events_dispatched, 2);
        assert_eq!(schedule.fixed_updates, 2);
        assert_eq!(
            *order.borrow(),
            ["event", "event", "fixed", "fixed", "update", "render"]
        );
    }

    #[test]
    fn fixed_updates_are_capped_and_excess_time_is_discarded_deterministically() {
        let start = std::time::Instant::now();
        let mut engine = EngineRuntime::new(EngineConfig {
            max_fixed_updates_per_frame: 3,
            ..config()
        })
        .unwrap();
        engine.start_at(start).unwrap();
        let schedule = engine
            .run_frame(start + Duration::from_millis(95), |_| {}, |_| {}, |_| {})
            .unwrap();
        assert_eq!(schedule.fixed_updates, 3);
        assert_eq!(schedule.dropped_simulation_time, Duration::from_millis(60));
        assert!((schedule.interpolation_alpha - 0.5).abs() < 1e-9);
    }

    #[test]
    fn variable_delta_is_clamped_and_fixed_tick_ids_increase() {
        let start = std::time::Instant::now();
        let mut engine = EngineRuntime::new(config()).unwrap();
        engine.start_at(start).unwrap();
        let ticks = Rc::new(RefCell::new(Vec::new()));
        let ticks_capture = ticks.clone();
        let updates = Rc::new(RefCell::new(Vec::new()));
        let updates_capture = updates.clone();
        let schedule = engine
            .run_frame(
                start + Duration::from_millis(250),
                |_| {},
                move |step| ticks_capture.borrow_mut().push(step.tick_index),
                move |step| updates_capture.borrow_mut().push(step.delta),
            )
            .unwrap();
        assert_eq!(schedule.variable_delta, Duration::from_millis(100));
        assert_eq!(schedule.fixed_updates, 4);
        assert_eq!(*ticks.borrow(), [0, 1, 2, 3]);
        assert_eq!(*updates.borrow(), [Duration::from_millis(100)]);
    }

    #[test]
    fn exit_event_is_delivered_and_stops_the_frame_schedule() {
        let start = std::time::Instant::now();
        let mut engine = EngineRuntime::new(config()).unwrap();
        engine.start_at(start).unwrap();
        engine.queue_event(EngineEvent::ExitRequested).unwrap();
        let mut fixed_called = false;
        let schedule = engine
            .run_frame(
                start + Duration::from_millis(20),
                |_| {},
                |_| fixed_called = true,
                |_| {},
            )
            .unwrap();
        assert_eq!(engine.state(), EngineState::StopRequested);
        assert_eq!(schedule.fixed_updates, 0);
        assert!(!fixed_called);
    }

    #[test]
    fn rejects_invalid_time_configuration() {
        let error = EngineRuntime::new(EngineConfig {
            fixed_update_interval: Duration::ZERO,
            ..config()
        })
        .err()
        .unwrap();
        assert!(matches!(error, EngineError::InvalidConfiguration(_)));
    }
}
