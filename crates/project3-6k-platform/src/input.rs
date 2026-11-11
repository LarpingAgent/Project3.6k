use std::collections::HashSet;

use winit::keyboard::KeyCode;

/// Current keyboard state, updated from native window events.
#[derive(Debug, Default)]
pub struct InputState {
    keys_down: HashSet<KeyCode>,
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

    pub(crate) fn set_key_down(&mut self, key: KeyCode, is_down: bool) {
        if is_down {
            self.keys_down.insert(key);
        } else {
            self.keys_down.remove(&key);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.keys_down.clear();
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
}
