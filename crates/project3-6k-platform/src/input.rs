use std::collections::HashSet;

use winit::keyboard::KeyCode;

/// Current keyboard state, updated from native window events.
#[derive(Debug, Default)]
pub struct InputState {
    keys_down: HashSet<KeyCode>,
    mouse_delta: [f64; 2],
}

impl InputState {
    /// Returns whether a physical key is currently held.
    pub fn is_key_down(&self, key: KeyCode) -> bool {
        self.keys_down.contains(&key)
    }

    /// Returns the number of currently held physical keys.
    pub fn keys_down_count(&self) -> usize {
        self.keys_down.len()
    }

    /// Returns and clears the accumulated raw mouse movement since the previous call.
    pub fn take_mouse_delta(&mut self) -> [f64; 2] {
        std::mem::take(&mut self.mouse_delta)
    }

    pub(crate) fn set_key_down(&mut self, key: KeyCode, is_down: bool) {
        if is_down {
            self.keys_down.insert(key);
        } else {
            self.keys_down.remove(&key);
        }
    }

    pub(crate) fn add_mouse_motion(&mut self, delta: (f64, f64)) {
        if delta.0.is_finite() && delta.1.is_finite() {
            self.mouse_delta[0] += delta.0;
            self.mouse_delta[1] += delta.1;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.keys_down.clear();
        self.mouse_delta = [0.0; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_key_press_release_and_focus_clear() {
        let mut input = InputState::default();
        input.set_key_down(KeyCode::KeyW, true);
        input.set_key_down(KeyCode::KeyA, true);

        assert!(input.is_key_down(KeyCode::KeyW));
        assert_eq!(input.keys_down_count(), 2);

        input.set_key_down(KeyCode::KeyW, false);
        assert!(!input.is_key_down(KeyCode::KeyW));
        assert!(input.is_key_down(KeyCode::KeyA));

        input.clear();
        assert_eq!(input.keys_down_count(), 0);
    }

    #[test]
    fn accumulates_mouse_delta_until_read_and_clears_on_focus_loss() {
        let mut input = InputState::default();
        input.add_mouse_motion((2.5, -1.0));
        input.add_mouse_motion((0.5, 3.0));
        assert_eq!(input.take_mouse_delta(), [3.0, 2.0]);
        assert_eq!(input.take_mouse_delta(), [0.0, 0.0]);

        input.add_mouse_motion((4.0, 5.0));
        input.clear();
        assert_eq!(input.take_mouse_delta(), [0.0, 0.0]);
    }
}
