const ACTION_WORD_COUNT: usize = (u16::MAX as usize + 1).div_ceil(u64::BITS as usize);

/// Stable, platform-neutral identifier for an input action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionId(u16);

impl ActionId {
    /// Creates an action identifier. The full `u16` range is supported.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the numeric identifier for serialization or diagnostics.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Current logical input state, independent of native keyboard APIs.
#[derive(Clone, Debug)]
pub struct EngineInput {
    active_actions: [u64; ACTION_WORD_COUNT],
    pointer_delta: [f64; 2],
}

impl Default for EngineInput {
    fn default() -> Self {
        Self {
            active_actions: [0; ACTION_WORD_COUNT],
            pointer_delta: [0.0; 2],
        }
    }
}

impl EngineInput {
    /// Returns whether a logical action is currently held.
    pub fn is_active(&self, action: ActionId) -> bool {
        let index = action.0 as usize;
        self.active_actions[index / u64::BITS as usize] & (1_u64 << (index % u64::BITS as usize))
            != 0
    }

    /// Number of currently held logical actions.
    pub fn active_action_count(&self) -> usize {
        self.active_actions
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// Returns and clears pointer movement accumulated since the previous call.
    pub fn take_pointer_delta(&mut self) -> [f64; 2] {
        std::mem::take(&mut self.pointer_delta)
    }

    pub(crate) fn apply(&mut self, event: &crate::EngineEvent) {
        match *event {
            crate::EngineEvent::ActionChanged { action, pressed } => {
                let index = action.0 as usize;
                let mask = 1_u64 << (index % u64::BITS as usize);
                if pressed {
                    self.active_actions[index / u64::BITS as usize] |= mask;
                } else {
                    self.active_actions[index / u64::BITS as usize] &= !mask;
                }
            }
            crate::EngineEvent::PointerMotion { delta } => {
                self.pointer_delta[0] += delta[0];
                self.pointer_delta[1] += delta[1];
            }
            crate::EngineEvent::FocusChanged { focused: false } => self.clear(),
            _ => {}
        }
    }

    pub(crate) fn clear(&mut self) {
        self.active_actions.fill(0);
        self.pointer_delta = [0.0; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionId, EngineInput};
    use crate::EngineEvent;

    #[test]
    fn tracks_logical_action_press_release_and_focus_loss() {
        let mut input = EngineInput::default();
        let move_forward = ActionId::new(7);
        let jump = ActionId::new(u16::MAX);
        input.apply(&EngineEvent::ActionChanged {
            action: move_forward,
            pressed: true,
        });
        input.apply(&EngineEvent::ActionChanged {
            action: jump,
            pressed: true,
        });
        assert!(input.is_active(move_forward));
        assert!(input.is_active(jump));
        assert_eq!(input.active_action_count(), 2);

        input.apply(&EngineEvent::ActionChanged {
            action: move_forward,
            pressed: false,
        });
        assert!(!input.is_active(move_forward));
        assert!(input.is_active(jump));

        input.apply(&EngineEvent::FocusChanged { focused: false });
        assert_eq!(input.active_action_count(), 0);
    }

    #[test]
    fn pointer_delta_accumulates_and_focus_loss_clears_it() {
        let mut input = EngineInput::default();
        input.apply(&EngineEvent::PointerMotion { delta: [2.5, -1.0] });
        input.apply(&EngineEvent::PointerMotion { delta: [0.5, 3.0] });
        assert_eq!(input.take_pointer_delta(), [3.0, 2.0]);
        assert_eq!(input.take_pointer_delta(), [0.0, 0.0]);
        input.apply(&EngineEvent::PointerMotion { delta: [1.0, 2.0] });
        input.apply(&EngineEvent::FocusChanged { focused: false });
        assert_eq!(input.take_pointer_delta(), [0.0, 0.0]);
    }
}
