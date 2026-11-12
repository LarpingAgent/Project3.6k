use std::collections::VecDeque;

use crate::{ActionId, EngineError, EngineInput};

/// Owned, platform-neutral events consumed by the engine on its owner thread.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    ExitRequested,
    WindowResized { width: u32, height: u32 },
    FocusChanged { focused: bool },
    ActionChanged { action: ActionId, pressed: bool },
    PointerMotion { delta: [f64; 2] },
}

#[derive(Debug)]
pub(crate) struct EventQueue {
    pending: VecDeque<EngineEvent>,
    capacity: usize,
}

impl EventQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            pending: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub(crate) fn push(
        &mut self,
        event: EngineEvent,
        input: &mut EngineInput,
    ) -> Result<(), EngineError> {
        if self.pending.len() == self.capacity {
            return Err(EngineError::EventQueueFull {
                capacity: self.capacity,
            });
        }
        if let EngineEvent::PointerMotion { delta } = event
            && (!delta[0].is_finite() || !delta[1].is_finite())
        {
            return Err(EngineError::NonFinitePointerMotion);
        }

        input.apply(&event);
        self.pending.push_back(event);
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<EngineEvent> {
        self.pending.pop_front()
    }

    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }
}
