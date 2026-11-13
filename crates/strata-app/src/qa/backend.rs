// SPDX-License-Identifier: MIT

//! Rune's side of the QA mode: what `strata_qa::Backend` asks, answered from the
//! accessibility tree, with input sent at Rune's input layer.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use rune::{KeyEvent, PhysicalInput, PointerButton, WheelEvent, accesskit::Action};
use strata_core::{i18n::tr, logging::criticals, storage};
use strata_qa::{
    Backend, Chord, Command, FieldState, Labels, Mark, Modifier, Orientation, Outcome, Rect, Scope,
    Scroll, States,
};

use super::{
    field,
    input_log::InputLog,
    keyboard::Keyboard,
    stage::Stage,
    tree::{Kept, Node, Seen, Snapshot},
};
use crate::{host::offscreen::Offscreen, shell};

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_POLL: Duration = Duration::from_millis(5);
/// How far one click of the wheel scrolls, in points.
const WHEEL_CLICK: f32 = 40.0;
/// The names of the preview drawer, the folder peek and the search results, as the GTK
/// application's `PREVIEW_LABEL`, `PEEK_LABEL` and `SEARCH_RESULTS_LABEL` spell them. The
/// slice draws none of the three.
const PREVIEW: &str = "Preview";
const PEEK: &str = "Folder peek";
const SEARCH_RESULTS: &str = "Search results";
pub(super) struct Rune {
    stage: RefCell<Stage>,
    keyboard: RefCell<Keyboard>,
    /// Where the last motion put the pointer, in window points.
    pointer: Cell<(i16, i16)>,
    quitting: Cell<bool>,
    log: InputLog,
}

fn modifier_name(modifier: Modifier) -> &'static str {
    match modifier {
        Modifier::Ctrl => "ctrl",
        Modifier::Shift => "shift",
        Modifier::Alt => "alt",
        Modifier::AltGr => "altgr",
    }
}

impl Rune {
    pub(super) fn new(host: Offscreen, log: InputLog) -> Self {
        Self {
            stage: RefCell::new(Stage::new(host)),
            keyboard: RefCell::default(),
            pointer: Cell::new((0, 0)),
            quitting: Cell::new(false),
            log,
        }
    }

    /// Opens a folder, as the launch does with its argument.
    #[cfg(test)]
    pub(super) fn open(&self, folder: &std::path::Path) {
        let location = strata_core::model::Location::local(folder);
        self.stage.borrow().app().open(location);
    }

    pub(super) fn quitting(&self) -> bool {
        self.quitting.get()
    }

    /// One turn of the host while no step runs. Returns whether it did anything.
    pub(super) fn turn(&self) -> bool {
        self.stage.borrow_mut().turn()
    }

    /// Blocks until the host has work, and for `limit` at most.
    pub(super) fn wait(&self, limit: Duration) {
        self.stage.borrow_mut().wait(limit);
    }

    fn snapshot(&self) -> Rc<Snapshot> {
        self.stage.borrow_mut().snapshot()
    }

    fn read<T>(&self, node: &Node, read: impl FnOnce(&Snapshot, &Seen) -> T) -> Option<T> {
        let snapshot = self.snapshot();
        snapshot.get(node).map(|seen| read(&snapshot, seen))
    }

    /// Whether the folder the launch opened has been listed, or could not be.
    fn listed(stage: &Stage) -> bool {
        let app = stage.app();
        shell::shown_view(app).is_some_and(|view| (view.probe.listed)(app))
    }

    /// Whether the view shown is the one the saved `browser_mode` names, which the runner
    /// writes for the run's mode. The shell shows the first view when none answers to it, so a
    /// run of a view not mounted would otherwise pass on that one.
    fn shows_saved_mode(stage: &Stage) -> Result<(), String> {
        let app = stage.app();
        let mode = &shell::preferences(app).browser_mode;
        match shell::shown_view(app) {
            Some(view) if (view.shows)(mode) => Ok(()),
            Some(view) => Err(format!(
                "the run starts in the {mode:?} view, and the window shows the {} view instead: \
                 no view mounted answers to {mode:?}",
                view.probe.name
            )),
            None => Err("the Strata window shows no view".to_owned()),
        }
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

impl Backend for Rune {
    type Node = Node;
    type Kept = Kept;

    async fn ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + READY_TIMEOUT;
        let stage = &mut *self.stage.borrow_mut();
        // The view is chosen when the window is built, so a fallback is named at once rather
        // than waited on until it lists.
        Self::shows_saved_mode(stage)?;
        loop {
            stage.settle();
            if Self::listed(stage) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("the Strata window never showed an entry list".to_owned());
            }
            stage.wait(READY_POLL);
        }
    }

    fn quit(&self) {
        self.quitting.set(true);
    }

    fn criticals(&self) -> Vec<String> {
        criticals::take()
    }

    fn window(&self) -> Option<Node> {
        self.snapshot().root().and_then(|root| root.node.clone())
    }

    /// The one window is the only toplevel.
    fn toplevels(&self) -> Vec<Node> {
        self.window().into_iter().collect()
    }

    fn title(&self, _window: &Node) -> Option<String> {
        Some(self.stage.borrow().app().title().to_owned())
    }

    fn window_width(&self, window: &Node) -> i32 {
        let width = self.frame(window).map_or(0.0, |frame| frame.width);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a window is far narrower than i32::MAX points"
        )]
        let width = width.round() as i32;
        width
    }

    fn focus(&self, _window: &Node) -> Option<Node> {
        self.snapshot().focus().and_then(|focus| focus.node.clone())
    }

    fn parent(&self, node: &Node) -> Option<Node> {
        let snapshot = self.snapshot();
        let parent = snapshot.at(snapshot.get(node)?.parent?)?;
        parent.node.clone()
    }

    fn children(&self, node: &Node) -> Vec<Node> {
        let snapshot = self.snapshot();
        let Some(seen) = snapshot.get(node) else {
            return Vec::new();
        };
        let children = seen.children.iter();
        children
            .filter_map(|child| snapshot.at(*child)?.node.clone())
            .collect()
    }

    /// The tree holds no hidden element: Rune leaves those out.
    fn visible(&self, node: &Node) -> bool {
        self.attached(node)
    }

    fn attached(&self, node: &Node) -> bool {
        self.snapshot().get(node).is_some()
    }

    fn role(&self, node: &Node) -> &'static str {
        self.read(node, |_, seen| seen.role()).unwrap_or("filler")
    }

    fn name(&self, node: &Node) -> String {
        self.read(node, |_, seen| seen.name.clone())
            .unwrap_or_default()
    }

    fn description(&self, node: &Node) -> String {
        self.read(node, |_, seen| seen.description.clone())
            .unwrap_or_default()
    }

    fn mark(&self, node: &Node) -> Option<Mark> {
        self.read(node, |_, seen| seen.mark).flatten()
    }

    fn states(&self, node: &Node) -> States {
        self.read(node, |snapshot, seen| States {
            selected: seen.has_state("selected"),
            focused: snapshot.is_focus(seen),
            checked: seen.has_state("checked"),
            pressed: false,
            sensitive: !seen.has_state("disabled"),
            selectable: seen.has_state("selected") || seen.has_state("unselected"),
            editable: field::is_one(&seen.role) && !seen.has_state("read-only"),
            focusable: seen.has_action("focus"),
            busy: false,
            marked: false,
        })
        .unwrap_or_default()
    }

    fn orientation(&self, _node: &Node) -> Option<Orientation> {
        None
    }

    fn text(&self, node: &Node) -> Option<String> {
        self.read(node, |_, seen| match seen.role.as_str() {
            "label" => Some(seen.name.clone()),
            role if field::is_one(role) => seen.value.clone(),
            _ => None,
        })
        .flatten()
    }

    fn field(&self, node: &Node) -> Option<FieldState> {
        self.read(node, |snapshot, seen| {
            field::state(seen, snapshot.is_focus(seen))
        })
        .flatten()
    }

    fn value(&self, node: &Node) -> Option<f64> {
        self.read(node, |_, seen| seen.number).flatten()
    }

    fn frame(&self, node: &Node) -> Option<Rect> {
        self.read(node, |_, seen| seen.bounds).flatten()
    }

    /// The host has one surface, at the origin: a box in the window is a box on the screen.
    fn origin(&self, _node: &Node) -> Result<(f64, f64), String> {
        Ok((0.0, 0.0))
    }

    fn layout(&self, node: &Node, _window: &Node) -> Option<Rect> {
        self.frame(node)
    }

    /// A scroll view's, as Rune's tree reports it (`tree::Scrolled`).
    fn scroll(&self, node: &Node, along: Orientation) -> Option<Scroll> {
        let scrolled = self.read(node, |_, seen| seen.scroll).flatten()?;
        Some(scrolled.along(along))
    }

    fn scroll_position(&self, scroller: &Node) -> Option<f64> {
        let scrolled = self.read(scroller, |_, seen| seen.scroll).flatten()?;
        scrolled.bar()
    }

    fn labels(&self) -> Labels {
        Labels {
            preview: tr(PREVIEW),
            peek: tr(PEEK),
            search_results: tr(SEARCH_RESULTS),
        }
    }

    fn settings_file(&self) -> PathBuf {
        storage::config_directory().join("settings.toml")
    }

    fn keep(&self, node: &Node) -> Kept {
        self.snapshot().keep(node)
    }

    fn recall(&self, kept: &Kept) -> Option<Node> {
        self.snapshot().recall(kept)
    }

    fn dump(&self, _window: &Node) -> String {
        self.snapshot().dump()
    }

    async fn settle(&self, _window: &Node) {
        self.stage.borrow_mut().settle();
    }

    async fn sleep(&self, time: Duration) {
        self.stage.borrow_mut().sleep(time);
    }

    /// There is one window, and it is the one the steps act on already.
    fn adopt(&self, _window: &Node) {}

    async fn resize(&self, window: &Node, width: u16, height: Option<u16>) -> Result<(), String> {
        let frame = self.frame(window).ok_or("the window is not drawn")?;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a window's height in points fits f32"
        )]
        let height = height.map_or(frame.height as f32, f32::from);
        let stage = &mut *self.stage.borrow_mut();
        stage
            .host()
            .runtime()
            .set_viewport((f32::from(width), height), 1.0);
        Ok(())
    }

    async fn chord(&self, _window: &Node, chord: &Chord) -> Result<(), String> {
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

    async fn text_input(&self, _window: &Node, character: char) -> Result<(), String> {
        let event = self.keyboard.borrow_mut().typed(character);
        self.tap(&character.to_string(), event);
        Ok(())
    }

    async fn hold(&self, _window: &Node, modifiers: &[Modifier]) -> Result<(), String> {
        for modifier in modifiers {
            self.set_modifier(*modifier, true);
        }
        let stage = &mut *self.stage.borrow_mut();
        self.show_modifiers(stage);
        stage.settle();
        Ok(())
    }

    async fn release(&self, _window: &Node, modifiers: &[Modifier]) {
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

    fn pointer_position(&self) -> Result<(i16, i16), String> {
        Ok(self.pointer.get())
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
    fn activate(&self, node: &Node) -> bool {
        let key = self.read(node, |_, seen| {
            seen.key.clone().filter(|_| seen.has_action("click"))
        });
        let Some(key) = key.flatten() else {
            return false;
        };
        let stage = &mut *self.stage.borrow_mut();
        stage
            .host()
            .runtime()
            .accessibility_action(&key, Action::Click, None)
    }

    /// The one step of Rune's own: the keyboard layout, which the GTK application reads from
    /// its display server.
    async fn command(&self, _scope: &Scope<'_, Self>, command: &Command) -> Option<Outcome> {
        let Command::Keymap(keymap) = command else {
            return None;
        };
        Some(Outcome::from(self.keyboard.borrow_mut().set_layout(keymap)))
    }
}
