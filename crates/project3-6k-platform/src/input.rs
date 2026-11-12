use project3_6k_engine::{ActionId, EngineInput};
use winit::keyboard::KeyCode;

pub(crate) const MOVE_FORWARD: ActionId = ActionId::new(1);
pub(crate) const MOVE_BACKWARD: ActionId = ActionId::new(2);
pub(crate) const MOVE_LEFT: ActionId = ActionId::new(3);
pub(crate) const MOVE_RIGHT: ActionId = ActionId::new(4);
pub(crate) const MOVE_UP: ActionId = ActionId::new(5);
pub(crate) const MOVE_DOWN_LEFT_SHIFT: ActionId = ActionId::new(6);
pub(crate) const CANCEL: ActionId = ActionId::new(7);
pub(crate) const MOVE_DOWN_RIGHT_SHIFT: ActionId = ActionId::new(8);

/// Maps native physical keys to logical engine actions used by the test app.
pub(crate) fn action_for_key(key: KeyCode) -> Option<ActionId> {
    match key {
        KeyCode::KeyW => Some(MOVE_FORWARD),
        KeyCode::KeyS => Some(MOVE_BACKWARD),
        KeyCode::KeyA => Some(MOVE_LEFT),
        KeyCode::KeyD => Some(MOVE_RIGHT),
        KeyCode::Space => Some(MOVE_UP),
        KeyCode::ShiftLeft => Some(MOVE_DOWN_LEFT_SHIFT),
        KeyCode::ShiftRight => Some(MOVE_DOWN_RIGHT_SHIFT),
        KeyCode::Escape => Some(CANCEL),
        _ => None,
    }
}

pub(crate) fn is_descending(input: &EngineInput) -> bool {
    input.is_active(MOVE_DOWN_LEFT_SHIFT) || input.is_active(MOVE_DOWN_RIGHT_SHIFT)
}

#[cfg(test)]
mod tests {
    use project3_6k_engine::{ActionId, EngineEvent, EngineRuntime};
    use winit::keyboard::KeyCode;

    use super::{
        CANCEL, MOVE_DOWN_LEFT_SHIFT, MOVE_DOWN_RIGHT_SHIFT, MOVE_FORWARD, action_for_key,
        is_descending,
    };

    #[test]
    fn maps_native_keys_to_stable_logical_actions() {
        assert_eq!(action_for_key(KeyCode::KeyW), Some(MOVE_FORWARD));
        assert_eq!(
            action_for_key(KeyCode::ShiftLeft),
            Some(MOVE_DOWN_LEFT_SHIFT)
        );
        assert_eq!(
            action_for_key(KeyCode::ShiftRight),
            Some(MOVE_DOWN_RIGHT_SHIFT)
        );
        assert_eq!(action_for_key(KeyCode::Escape), Some(CANCEL));
        assert_eq!(action_for_key(KeyCode::F12), None);
        assert_eq!(MOVE_FORWARD, ActionId::new(1));
    }

    #[test]
    fn releasing_one_shift_key_keeps_descent_active_while_the_other_is_held() {
        let mut engine = EngineRuntime::default();
        engine.start().unwrap();
        engine
            .queue_event(EngineEvent::ActionChanged {
                action: MOVE_DOWN_LEFT_SHIFT,
                pressed: true,
            })
            .unwrap();
        engine
            .queue_event(EngineEvent::ActionChanged {
                action: MOVE_DOWN_RIGHT_SHIFT,
                pressed: true,
            })
            .unwrap();
        engine
            .queue_event(EngineEvent::ActionChanged {
                action: MOVE_DOWN_LEFT_SHIFT,
                pressed: false,
            })
            .unwrap();

        assert!(is_descending(engine.input()));

        engine
            .queue_event(EngineEvent::ActionChanged {
                action: MOVE_DOWN_RIGHT_SHIFT,
                pressed: false,
            })
            .unwrap();
        assert!(!is_descending(engine.input()));
        engine.shutdown();
    }
}
