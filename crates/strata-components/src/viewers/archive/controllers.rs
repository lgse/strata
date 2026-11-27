// SPDX-License-Identifier: MIT

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftPasses(pub bool);

#[derive(Debug, Default)]
pub struct ArchiveKeys {
    inner: Keyboard,
}

impl ArchiveKeys {
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
