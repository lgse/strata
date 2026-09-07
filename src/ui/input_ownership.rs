// SPDX-License-Identifier: GPL-3.0-or-later

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum NavigationInput {
    #[default]
    Keyboard,
    Pointer,
}

#[derive(Debug, Default)]
pub(super) struct InputOwnership {
    pub last_navigation: NavigationInput,
    pointer_position: Option<(f64, f64)>,
}

impl InputOwnership {
    pub fn keyboard_navigation(&mut self) {
        self.last_navigation = NavigationInput::Keyboard;
    }

    /// Surface coordinates stay stable when columns scroll underneath a parked pointer.
    pub fn pointer_motion(&mut self, position: (f64, f64)) -> bool {
        if self.pointer_position == Some(position) {
            return false;
        }
        self.pointer_position = Some(position);
        self.pointer_action();
        true
    }

    pub fn pointer_action(&mut self) {
        self.last_navigation = NavigationInput::Pointer;
    }

    pub fn destination(
        &self,
        hovered: Option<usize>,
        focused: Option<usize>,
        active: Option<usize>,
        pane_count: usize,
    ) -> Option<usize> {
        let valid = |depth: &usize| *depth < pane_count;
        let pointer = (self.last_navigation == NavigationInput::Pointer)
            .then_some(hovered)
            .flatten()
            .filter(valid);
        pointer
            .or_else(|| focused.filter(valid))
            .or_else(|| active.filter(valid))
            .or_else(|| pane_count.checked_sub(1))
    }
}

#[cfg(test)]
mod tests;
