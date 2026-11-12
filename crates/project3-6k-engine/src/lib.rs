//! Platform-independent lifecycle, events, input actions, and frame scheduling.
//!
//! The platform host owns the native event pump and calls [`EngineRuntime`] on
//! its event thread. This crate deliberately has no windowing or renderer
//! dependency.

mod config;
mod event;
mod input;
mod runtime;

pub use config::{EngineConfig, MAX_EVENT_QUEUE_CAPACITY, MAX_FIXED_UPDATES_PER_FRAME};
pub use event::EngineEvent;
pub use input::{ActionId, EngineInput};
pub use runtime::{
    EngineError, EngineRuntime, EngineState, FixedUpdate, FrameSchedule, VariableUpdate,
};
