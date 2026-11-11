//! The archive tree's key map and pointer controller, on C1e's seam.
//!
//! One member is highlighted at a time, so the map has no Shift ranges, no Ctrl+A or Ctrl+Space
//! and no Escape (that's the host's: Columns closes the preview). Left and Right have no
//! neighbour in a list, so C1c reports them as edges, and the view opens a folder or goes up.

use rune::PointerButton;

use crate::collection::element::{
    Activation, Ctx, Effect, Flow, Intent, KeyController, KeyInput, PointerController,
    PointerInput, PointerPhase,
};
use crate::collection::keyboard::{
    Command, KeyMap, KeySettings, Keyboard, Letters, Target, Vertical,
};
use crate::contract::Direction;

/// How far (points) a release may land from its press and still click: Rune's click slop.
const CLICK_SLOP: f32 = 4.0;

/// C1c's default map less everything that selects more than one member or clears the
/// highlight: the arrows, Home, End, the page keys, Ctrl+Up and Ctrl+Down, and Enter. `h j k l`
/// stay aliases, which act with [`Letters::Arrows`] (Type to search off).
pub fn key_map() -> KeyMap {
    let mut map = KeyMap::default_map();
    let targets = [
        Target::Step(Direction::Up),
        Target::Step(Direction::Down),
        Target::Step(Direction::Left),
        Target::Step(Direction::Right),
        Target::First,
        Target::Last,
        Target::Page(Vertical::Up),
        Target::Page(Vertical::Down),
    ];
    for t in targets {
        map.unbind(Command::Extend(t));
    }
    map.unbind(Command::Nudge(Direction::Left))
        .unbind(Command::Nudge(Direction::Right))
        .unbind(Command::Toggle)
        .unbind(Command::SelectAll)
        .unbind(Command::Clear);
    map
}

/// The keyboard settings for the archive tree: its map, with `h j k l` moving when `letters`.
pub fn key_settings(letters: bool) -> KeySettings {
    KeySettings {
        map: key_map(),
        ..KeySettings::default()
    }
    .letters(if letters {
        Letters::Arrows
    } else {
        Letters::Pass
    })
}

/// Members highlight on the release of a click, never on a press or a hover, as Strata's
/// archive list does (lgse/strata#1570): a click on a member highlights it alone, whatever
/// modifiers are held, and a click on a folder also opens it ([`Intent::Activate`] with
/// [`Activation::Click`]). Only a run's first click counts: opening a folder replaces every
/// row, so a double click's second press lands on a row of the folder just opened, and must not
/// open it too. A drag selects nothing, and starts no marquee and no drag out.
#[derive(Debug, Default)]
pub struct ArchivePointer {
    pressed: Option<usize>,
}

impl PointerController for ArchivePointer {
    fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
        if input.button != PointerButton::Primary {
            return Flow::Pass;
        }
        match input.phase {
            PointerPhase::Down => {
                self.pressed = input.hit.index;
            }
            PointerPhase::Move => {}
            PointerPhase::Up => {
                let pressed = self.pressed.take();
                let still =
                    input.travel.x.abs() <= CLICK_SLOP && input.travel.y.abs() <= CLICK_SLOP;
                if let Some(i) = input.hit.index
                    && pressed == Some(i)
                    && still
                    && !input.cancelled
                    && input.clicks == 1
                    && i < cx.source.len()
                {
                    cx.apply(|sel, src| sel.select_only(src, i));
                    let entry = cx.source.entry(i);
                    if entry.kind.is_folder_like() {
                        cx.emit(Effect::Intent(Intent::Activate {
                            id: entry.id,
                            how: Activation::Click,
                        }));
                    }
                }
            }
        }
        Flow::Handled
    }

    fn reset(&mut self) {
        self.pressed = None;
    }
}

/// A collection setting: Left at the folder shown goes on past the tree rather than to C1c. The
/// view sets it at the root when the host has no `on_leave`, so the key reaches the host's own
/// key handling (Strata's 10xer `h` behaviour) instead of being swallowed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftPasses(pub bool);

/// C1c's keyboard, less Left where [`LeftPasses`] says so.
#[derive(Debug, Default)]
pub struct ArchiveKeys {
    inner: Keyboard,
}

impl ArchiveKeys {
    /// Whether the key is one of the map's Left chords. `chords_for` includes the `h` alias
    /// whatever [`Letters`] says: with `h` passed on ([`Letters::Pass`]) the inner keyboard
    /// passes it too, so passing it here changes nothing.
    fn left(input: &KeyInput<'_>, cx: &Ctx<'_>) -> bool {
        cx.settings::<KeySettings>().is_some_and(|k| {
            k.map
                .chords_for(Command::Select(Target::Step(Direction::Left)))
                .iter()
                .any(|c| input.matches(c))
        })
    }
}

impl KeyController for ArchiveKeys {
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
        if cx.settings::<LeftPasses>().is_some_and(|l| l.0) && Self::left(input, cx) {
            return Flow::Pass;
        }
        self.inner.key(input, cx)
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        self.inner.tick(cx)
    }

    fn pointer_seen(&mut self, input: &PointerInput) {
        self.inner.pointer_seen(input);
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}
