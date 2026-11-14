// SPDX-License-Identifier: MIT

//! The offscreen host as the QA backend drives it: this thread owns the `Runtime`, and input
//! goes in at Rune's input layer with no display server between.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use rune::{KeyEvent, PhysicalInput, PointerButton, WheelEvent, accesskit::Action};
use strata_components::contract::EntryId;
use strata_qa::{Chord, Keymap, Modifier};

use super::{
    host::{Activated, Host},
    input_log::{InputLog, modifier_name},
    keyboard::Keyboard,
    replaced::Replacements,
    stage::Stage,
    tree::Snapshot,
};
use crate::{App, host::offscreen::Offscreen};

/// How far one click of the wheel scrolls, in points.
const WHEEL_CLICK: f32 = 40.0;

pub(super) struct Staged {
    stage: RefCell<Stage>,
    keyboard: RefCell<Keyboard>,
    /// Where the last motion put the pointer, in window points.
    pointer: Cell<(i16, i16)>,
    log: InputLog,
    replacements: Rc<Replacements>,
}

impl Staged {
    pub(super) fn new(host: Offscreen, log: InputLog) -> Self {
        let replacements = Replacements::watch(&host.app().browser);
        Self {
            replacements,
            stage: RefCell::new(Stage::new(host)),
            keyboard: RefCell::default(),
            pointer: Cell::new((0, 0)),
            log,
        }
    }

    /// The window's state, for the idle loop and a test to reach behind the steps.
    pub(super) fn app<T>(&self, read: impl FnOnce(&App) -> T) -> T {
        read(self.stage.borrow().app())
    }

    /// One turn of the host while no step runs. Returns whether it did anything.
    pub(super) fn turn(&self) -> bool {
        self.stage.borrow_mut().turn()
    }

    /// Tells Rune which modifiers are down, as a keyboard does before the key they modify.
    /// The Columns adapter reads them to tell a Shift range from a move.
    fn show_modifiers(&self, stage: &mut Stage) {
        let mods = self.keyboard.borrow().mods();
        let runtime = stage.host().runtime();
        runtime.physical_input(PhysicalInput::Modifiers(mods));
    }

    /// A key going down and up with its modifiers held around it, then a settle. With no
    /// `event` the key is one a window hears nothing of, a dead key: only the log has it.
    fn tap(&self, name: &str, event: Option<KeyEvent>) {
        let stage = &mut *self.stage.borrow_mut();
        if let Some(event) = event {
            let runtime = stage.host().runtime();
            runtime.physical_input(PhysicalInput::Modifiers(event.mods));
            runtime.key(event.clone());
            runtime.key_up(event);
        }
        self.log.key(name, true);
        self.log.key(name, false);
        self.show_modifiers(stage);
        stage.settle();
    }

    /// One click of the wheel at (x, y), as Rune's window host delivers it off macOS: with
    /// the held modifiers, and with Shift turning it sideways.
    fn wheel_click(&self, x: f32, y: f32, delta: f32) -> WheelEvent {
        let mods = self.keyboard.borrow().mods();
        let (dx, dy) = if mods.shift {
            (delta, 0.0)
        } else {
            (0.0, delta)
        };
        WheelEvent::new(dx, dy).at(x, y).mods(mods)
    }

    fn set_modifier(&self, modifier: Modifier, pressed: bool) {
        let mut keyboard = self.keyboard.borrow_mut();
        if pressed {
            keyboard.hold(modifier);
        } else {
            keyboard.release(modifier);
        }
        self.log.key(modifier_name(modifier), pressed);
    }
}

impl Host for Staged {
    fn snapshot(&self) -> Rc<Snapshot> {
        self.stage.borrow_mut().snapshot()
    }

    fn with_app<T: Send + 'static>(
        &self,
        read: impl FnOnce(&App) -> T + Send + 'static,
    ) -> Result<T, String> {
        Ok(self.app(read))
    }

    fn replacements(&self) -> u64 {
        self.replacements.now()
    }

    fn replaced_since(&self, entry: EntryId, since: u64) -> bool {
        let stage = self.stage.borrow();
        self.replacements.replaced_since(stage.app(), entry, since)
    }

    async fn settle(&self) {
        self.stage.borrow_mut().settle();
    }

    async fn sleep(&self, time: Duration) {
        self.stage.borrow_mut().sleep(time);
    }

    fn pause(&self, limit: Duration) {
        self.stage.borrow_mut().wait(limit);
    }

    /// The host has one surface, at the origin: a box in the window is a box on the screen.
    fn origin(&self) -> Result<(f64, f64), String> {
        Ok((0.0, 0.0))
    }

    async fn resize(&self, width: f32, height: f32) -> Result<(), String> {
        let stage = &mut *self.stage.borrow_mut();
        stage.host().runtime().set_viewport((width, height), 1.0);
        Ok(())
    }

    fn set_layout(&self, keymap: &Keymap) -> Result<(), String> {
        self.keyboard.borrow_mut().set_layout(keymap)
    }

    async fn chord(&self, chord: &Chord) -> Result<(), String> {
        let event = self
            .keyboard
            .borrow_mut()
            .strike(&chord.key, &chord.modifiers)?;
        for modifier in &chord.modifiers {
            self.log.key(modifier_name(*modifier), true);
        }
        self.tap(&chord.key, event);
        for modifier in chord.modifiers.iter().rev() {
            self.log.key(modifier_name(*modifier), false);
        }
        Ok(())
    }

    async fn text_input(&self, character: char) -> Result<(), String> {
        let event = self.keyboard.borrow_mut().typed(character);
        self.tap(&character.to_string(), event);
        Ok(())
    }

    async fn hold(&self, modifiers: &[Modifier]) -> Result<(), String> {
        for modifier in modifiers {
            self.set_modifier(*modifier, true);
        }
        let stage = &mut *self.stage.borrow_mut();
        self.show_modifiers(stage);
        stage.settle();
        Ok(())
    }

    async fn release(&self, modifiers: &[Modifier]) {
        for modifier in modifiers.iter().rev() {
            self.set_modifier(*modifier, false);
        }
        let stage = &mut *self.stage.borrow_mut();
        self.show_modifiers(stage);
        stage.settle();
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        let stage = &mut *self.stage.borrow_mut();
        if let Ok(modifier) = Modifier::parse(name) {
            self.set_modifier(modifier, pressed);
            self.show_modifiers(stage);
            return Ok(());
        }
        // A release composes nothing, and a dead key's is a release a window hears nothing of.
        let event = match pressed {
            true => self.keyboard.borrow_mut().strike(name, &[])?,
            false => self.keyboard.borrow().press(name, &[]).ok(),
        };
        self.log.key(name, pressed);
        let Some(event) = event else {
            return Ok(());
        };
        let runtime = stage.host().runtime();
        if pressed {
            runtime.physical_input(PhysicalInput::Modifiers(event.mods));
            runtime.key(event);
        } else {
            runtime.key_up(event);
            self.show_modifiers(stage);
        }
        Ok(())
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        self.log.motion((x, y));
        self.pointer.set((x, y));
        let stage = &mut *self.stage.borrow_mut();
        stage
            .host()
            .runtime()
            .pointer_move(f32::from(x), f32::from(y));
        Ok(())
    }

    fn pointer_position(&self) -> (i16, i16) {
        self.pointer.get()
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        let at = self.pointer.get();
        let (x, y) = (f32::from(at.0), f32::from(at.1));
        let stage = &mut *self.stage.borrow_mut();
        let runtime = stage.host().runtime();
        match (number, pressed) {
            (1, true) => runtime.pointer_down(x, y),
            (1, false) => runtime.pointer_up(x, y),
            (2, true) => runtime.button_down(x, y, PointerButton::Middle),
            (2, false) => runtime.button_up(x, y, PointerButton::Middle),
            (3, true) => runtime.secondary_down(x, y),
            (3, false) => runtime.secondary_up(x, y),
            // A wheel click is a press and release of button 4 or 5, as X numbers them.
            // Positive moves the content down, which is the wheel turning up.
            (4, true) => runtime.wheel_event(self.wheel_click(x, y, WHEEL_CLICK)),
            (5, true) => runtime.wheel_event(self.wheel_click(x, y, -WHEEL_CLICK)),
            (4 | 5, false) => {}
            _ => return Err(format!("the pointer has no button {number}")),
        }
        self.log.button(at, number, pressed);
        Ok(())
    }

    /// The element's press handler, through the click an assistive client asks for.
    fn activate(&self, target: &Activated) -> bool {
        let stage = &mut *self.stage.borrow_mut();
        stage
            .host()
            .runtime()
            .accessibility_action(&target.key, Action::Click, None)
    }
}
