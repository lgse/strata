//! The adapter on C1e's seam: what [`Keyboard`] reads from a `Ctx` and sends back to the
//! element. Each test names the bullet whose path through the element it carries.

use std::rc::Rc;

use rune::{Key, KeyEvent, Mods, PointerButton};

use super::super::*;
use super::source;
use crate::collection::element::{
    Activation, Ctx, Effect, Flow, Hit, Intent, KeyController, KeyInput, PointerInput,
    PointerPhase, Region, Settings,
};
use crate::collection::selection::Selection;
use crate::contract::{Direction, EntryId, ListSource, Point, Rect, UniformRows};

/// A ten-row list, its selection, and the settings the host passes.
struct Host {
    src: ListSource,
    geo: UniformRows,
    sel: Selection,
    settings: Rc<Settings>,
}

impl Host {
    fn new(settings: Option<KeySettings>) -> Self {
        let names: Vec<String> = (0..10).map(|i| format!("e{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut s = Settings::default();
        if let Some(k) = settings {
            s.insert(k);
        }
        Self {
            src: source(&refs),
            geo: UniformRows::new(10, 20.0, 400.0).viewport_height(200.0),
            sel: Selection::new(),
            settings: Rc::new(s),
        }
    }

    /// Runs `f` with a fresh `Ctx`, as the element does for one input, and returns its flow and
    /// effects.
    fn with(&mut self, f: impl FnOnce(&mut Ctx<'_>) -> Flow) -> (Flow, Vec<Effect>) {
        let mut cx = Ctx::new(&self.src, &self.geo, Rect::default(), 0.0, &mut self.sel)
            .with_settings(self.settings.clone());
        let flow = f(&mut cx);
        (flow, cx.effects().to_vec())
    }

    fn key(&mut self, k: &mut Keyboard, key: Key, mods: Mods) -> (Flow, Vec<Effect>) {
        let event = KeyEvent { key, mods };
        let input = KeyInput::new(&event);
        self.with(|cx| k.key(&input, cx))
    }

    fn cursor(&self) -> Option<usize> {
        self.sel.cursor_index(&self.src)
    }
}

fn plain() -> Mods {
    Mods::default()
}

fn press() -> PointerInput {
    let hit = Hit {
        index: Some(0),
        region: Region::Content,
    };
    PointerInput::new(
        PointerPhase::Down,
        PointerButton::Primary,
        Point::default(),
        hit,
    )
}

#[test]
fn moves_reveal_and_edges_become_intents() {
    // browser/selection/keyboard-navigation: Up from the first Icons row or first List entry focuses the pane header, including in an empty directory.
    let mut h = Host::new(None);
    let mut k = Keyboard::default();
    let (flow, effects) = h.key(&mut k, Key::Down, plain());
    assert_eq!(flow, Flow::Handled);
    assert!(matches!(effects[..], [Effect::Reveal(0)]));
    let (flow, effects) = h.key(&mut k, Key::Up, plain());
    assert_eq!(flow, Flow::Handled);
    assert!(matches!(
        effects[..],
        [Effect::Intent(Intent::Edge(Direction::Up))]
    ));
}

#[test]
fn enter_becomes_a_key_activation() {
    // browser/selection: Enter after Escape opens the folder under the keyboard cursor.
    let mut h = Host::new(None);
    let mut k = Keyboard::default();
    h.sel.select_only(&h.src, 4);
    h.key(&mut k, Key::Escape, plain());
    let (_, effects) = h.key(&mut k, Key::Enter, plain());
    assert!(matches!(
        effects[..],
        [Effect::Intent(Intent::Activate { id, how: Activation::Key })] if id == EntryId::new(4)
    ));
}

#[test]
fn nothing_to_select_reaches_the_host_as_its_own_type() {
    // integration/10xer-mode: In an empty folder, Space, `v`, `V`, Ctrl+A, and Ctrl+R flash `Nothing to select`.
    let mut h = Host::new(Some(KeySettings::tenxer()));
    h.src = ListSource::new(Vec::new());
    h.geo = UniformRows::new(0, 20.0, 400.0);
    let mut k = Keyboard::default();
    let (flow, effects) = h.key(&mut k, Key::Text(" ".into()), plain());
    assert_eq!(flow, Flow::Handled);
    let [Effect::Intent(Intent::Other(value))] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(
        value.downcast_ref::<NothingToSelect>(),
        Some(&NothingToSelect)
    );
}

#[test]
fn the_host_settings_choose_the_map() {
    // browser/selection/keyboard-navigation: With Type to search off, `h`, `j`, `k`, and `l` act as Left, Down, Up, and Right in every view and the file chooser.
    // browser/selection/keyboard-navigation: With Type to search on, `h`, `j`, `k`, and `l` pressed in the file list start a search instead of moving.
    let mut on = Host::new(None);
    let mut k = Keyboard::default();
    on.sel.select_only(&on.src, 2);
    let (flow, _) = on.key(&mut k, Key::Text("j".into()), plain());
    assert_eq!(
        (flow, on.cursor()),
        (Flow::Pass, Some(2)),
        "no settings: the defaults"
    );

    let mut off = Host::new(Some(KeySettings::default().letters(Letters::Arrows)));
    let mut k = Keyboard::default();
    off.sel.select_only(&off.src, 2);
    let (flow, _) = off.key(&mut k, Key::Text("j".into()), plain());
    assert_eq!((flow, off.cursor()), (Flow::Handled, Some(3)));
}

#[test]
fn a_queued_command_runs_at_tick_and_turns_keyboard_mode_on() {
    // integration/10xer-mode: In Columns, `g g` and `G` in a column with no cursor land on its first and last entry.
    let mut h = Host::new(Some(KeySettings::tenxer()));
    let handle = KeyHandle::new();
    let mut k = Keyboard::new(handle.clone());
    let (flow, effects) = h.with(|cx| k.tick(cx));
    assert_eq!(flow, Flow::Pass);
    assert!(effects.is_empty(), "nothing queued: nothing to commit");

    handle.run(Command::Cursor(Target::Last));
    let (flow, effects) = h.with(|cx| k.tick(cx));
    assert_eq!((flow, h.cursor()), (Flow::Handled, Some(9)));
    assert!(matches!(
        effects[..],
        [Effect::Reveal(9), Effect::KeyboardMode(true)]
    ));

    // A command that moves no cursor asks for no mode change.
    handle.run(Command::Cursor(Target::Last));
    let (_, effects) = h.with(|cx| k.tick(cx));
    assert!(matches!(effects[..], [Effect::Reveal(9)]));
}

#[test]
fn a_press_ends_a_range_and_a_folder_change_resets() {
    // integration/10xer-mode: `v` starts a range from the cursor and `V` starts an unselect range; the footer shows `VISUAL` or `UNSET` while it is active.
    let mut h = Host::new(Some(KeySettings::tenxer()));
    let handle = KeyHandle::new();
    let mut k = Keyboard::new(handle.clone());
    h.sel.select_only(&h.src, 1);
    h.key(&mut k, Key::Text("v".into()), plain());
    assert_eq!(handle.range(), Some(RangeKind::Select));
    let mut moved = press();
    moved.phase = PointerPhase::Move;
    k.pointer_seen(&moved);
    let mut secondary = press();
    secondary.button = PointerButton::Secondary;
    k.pointer_seen(&secondary);
    assert_eq!(
        handle.range(),
        Some(RangeKind::Select),
        "only a primary press ends it; a context menu doesn't"
    );
    k.pointer_seen(&press());
    assert_eq!(
        handle.range(),
        None,
        "the footer is right before the next key"
    );

    h.key(
        &mut k,
        Key::Text("V".into()),
        Mods {
            shift: true,
            ..plain()
        },
    );
    assert_eq!(handle.range(), Some(RangeKind::Unset));
    k.reset();
    assert_eq!(handle.range(), None);
}

#[test]
fn reset_drops_the_type_ahead_prefix() {
    // C1c scope: type-ahead finds by a typed prefix, with a timeout.
    let names = ["bat", "bee", "cat"];
    let mut h = Host::new(Some(KeySettings::default().letters(Letters::TypeAhead)));
    h.src = source(&names);
    h.geo = UniformRows::new(3, 20.0, 400.0);
    let mut k = Keyboard::default();
    h.key(&mut k, Key::Text("b".into()), plain());
    h.key(&mut k, Key::Text("e".into()), plain());
    assert_eq!(h.cursor(), Some(1));
    k.reset();
    // A fresh prefix: "c" finds "cat". Without the reset, "bec" would match nothing.
    h.key(&mut k, Key::Text("c".into()), plain());
    assert_eq!(h.cursor(), Some(2));
}

#[test]
fn ctrl_a_and_escape_turn_keyboard_mode_on_once() {
    // browser/selection: With no transient UI open, Escape clears the active pane's selection and keeps the keyboard cursor and directory.
    // Neither moves the cursor, so the element wouldn't turn keyboard mode on by itself.
    let mut h = Host::new(None);
    let mut k = Keyboard::default();
    h.sel.select_only(&h.src, 3);
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..plain()
    };
    let (_, effects) = h.key(&mut k, Key::Command('a'), ctrl);
    assert!(
        matches!(effects[..], [Effect::KeyboardMode(true)]),
        "{effects:?}"
    );
    let (_, effects) = h.key(&mut k, Key::Escape, plain());
    assert!(
        matches!(effects[..], [Effect::KeyboardMode(true)]),
        "{effects:?}"
    );
    // A queued Ctrl+A that also places the cursor sends the mode once, not twice.
    let mut fresh = Host::new(None);
    let handle = KeyHandle::new();
    let mut k = Keyboard::new(handle.clone());
    handle.run(Command::SelectAll);
    let (_, effects) = fresh.with(|cx| k.tick(cx));
    let modes = effects
        .iter()
        .filter(|e| matches!(e, Effect::KeyboardMode(true)))
        .count();
    assert_eq!(modes, 1, "{effects:?}");
}
