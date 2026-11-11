use rune::{Mods, PointerButton};

use super::controllers::ArchivePointer;
use super::{ArchiveDir, ArchiveItem, ArchiveTree, file_size};
use crate::collection::element::{
    Activation, Ctx, Effect, Hit, Intent, PointerController, PointerInput, PointerPhase, Region,
};
use crate::collection::selection::Selection;
use crate::contract::{EntryId, EntrySource, Point, Rect, UniformRows};

fn file(name: &str, size: u64) -> ArchiveItem {
    ArchiveItem::File {
        name: name.into(),
        size,
        icon: String::new(),
    }
}

fn dir(name: &str, children: Vec<ArchiveItem>) -> ArchiveItem {
    ArchiveItem::Dir(ArchiveDir {
        name: name.into(),
        children,
    })
}

/// `docs/` (with `api/` inside), `src/`, `README.md`.
fn tree() -> ArchiveTree {
    let mut next = 7_000_000u64;
    ArchiveTree::new(
        ArchiveDir {
            name: String::new(),
            children: vec![
                dir(
                    "docs",
                    vec![
                        dir("api", vec![file("index.html", 2048)]),
                        file("guide.md", 900),
                    ],
                ),
                dir("src", vec![]),
                file("README.md", 12_400),
            ],
        },
        move || {
            next += 1;
            EntryId::new(next)
        },
    )
}

#[test]
fn the_listing_shows_one_folder_and_finds_ids_only_in_it() {
    let mut l = super::ArchiveListing::new(tree());
    assert_eq!(l.len(), 3);
    assert_eq!(l.entry(0).name, "docs/");
    assert_eq!(l.entry(2).name, "README.md");
    assert_eq!(l.size(2), Some(12_400));
    assert_eq!(l.size(0), None);
    assert_eq!(l.counts(), (1, 2));
    let docs = l.id_at(0);
    assert!(l.open(0));
    assert_eq!(l.crumbs(), ["docs"]);
    // The folder left isn't in the folder shown.
    assert_eq!(l.index_of(docs), None);
    assert_eq!(l.entry(0).name, "api/");
    assert!(l.open(0));
    assert_eq!(l.depth(), 2);
    // Back to the root through its first crumb: the cursor goes to the folder came through.
    assert_eq!(l.go_to(0), Some(0));
    assert_eq!(l.depth(), 0);
    assert_eq!(l.index_of(docs), Some(0));
    assert!(!l.open(2), "a file doesn't open");
}

/// Every member's id, at every level, by walking the folders.
fn all_ids(l: &mut super::ArchiveListing, out: &mut Vec<u64>) {
    let depth = l.depth();
    for i in 0..l.len() {
        out.push(l.id_at(i).get());
        if l.is_dir(i) {
            assert!(l.open(i));
            all_ids(l, out);
            l.go_to(depth);
        }
    }
}

#[test]
fn every_member_gets_its_own_id_from_the_host() {
    let mut l = super::ArchiveListing::new(tree());
    let mut ids = Vec::new();
    all_ids(&mut l, &mut ids);
    // docs/, api/, index.html, guide.md, src/, README.md.
    assert_eq!(ids.len(), 6);
    assert!(ids.iter().all(|&id| id > 7_000_000));
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
}

#[test]
fn sizes_read_as_strata_writes_them() {
    assert_eq!(file_size(900), "900 B");
    assert_eq!(file_size(12_400), "12.4 kB");
    assert_eq!(file_size(2_000), "2 kB");
    assert_eq!(file_size(999_960), "1 MB");
}

/// A press then a release on `index` of three 24 pt rows, with `mods`, as click `clicks` of a
/// run; what the controller asked for.
fn click(
    p: &mut ArchivePointer,
    sel: &mut Selection,
    index: usize,
    mods: Mods,
    clicks: u32,
    travel: f32,
) -> Vec<Effect> {
    let l = super::ArchiveListing::new(tree());
    let g = UniformRows::new(l.len(), 24.0, 300.0);
    let at = Point::new(40.0, 24.0 * index as f32 + 12.0);
    let hit = Hit {
        index: Some(index),
        region: Region::Name,
    };
    let mut down = PointerInput::new(PointerPhase::Down, PointerButton::Primary, at, hit);
    down.mods = mods;
    down.clicks = clicks;
    let before = sel.selected_ids(&l);
    let mut cx = Ctx::new(&l, &g, Rect::new(0.0, 0.0, 300.0, 72.0), 0.0, sel);
    p.pointer(&down, &mut cx);
    assert_eq!(
        cx.selection().selected_ids(&l),
        before,
        "a press changes nothing"
    );
    let mut up = PointerInput::new(PointerPhase::Up, PointerButton::Primary, at, hit);
    up.mods = mods;
    up.clicks = clicks;
    up.travel = Point::new(travel, 0.0);
    p.pointer(&up, &mut cx);
    cx.effects().to_vec()
}

fn opens(effects: &[Effect]) -> bool {
    effects.iter().any(|e| {
        matches!(
            e,
            Effect::Intent(Intent::Activate {
                how: Activation::Click,
                ..
            })
        )
    })
}

#[test]
fn a_release_highlights_and_a_folder_click_opens() {
    let l = super::ArchiveListing::new(tree());
    let (mut p, mut sel) = (ArchivePointer::default(), Selection::new());
    let fx = click(&mut p, &mut sel, 2, Mods::default(), 1, 0.0);
    assert_eq!(sel.cursor(), Some(l.id_at(2)));
    assert!(!opens(&fx), "a file doesn't open");
    let fx = click(&mut p, &mut sel, 0, Mods::default(), 1, 0.0);
    assert_eq!(sel.selected_ids(&l), [l.id_at(0)]);
    assert!(opens(&fx));
}

/// `operations/archives/preview`: "Ctrl+click and Shift+click act as a plain click; one member
/// is highlighted at a time."
#[test]
fn modified_clicks_act_as_plain_ones() {
    let l = super::ArchiveListing::new(tree());
    let (mut p, mut sel) = (ArchivePointer::default(), Selection::new());
    click(&mut p, &mut sel, 2, Mods::default(), 1, 0.0);
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    click(&mut p, &mut sel, 1, ctrl, 1, 0.0);
    assert_eq!(sel.selected_ids(&l), [l.id_at(1)]);
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    let fx = click(&mut p, &mut sel, 0, shift, 1, 0.0);
    assert_eq!(sel.selected_ids(&l), [l.id_at(0)]);
    assert!(opens(&fx), "a modified click on a folder opens it too");
}

#[test]
fn a_double_clicks_second_press_and_a_drag_do_nothing() {
    let (mut p, mut sel) = (ArchivePointer::default(), Selection::new());
    // The second press of a double click lands on the folder just opened: no second level.
    let fx = click(&mut p, &mut sel, 0, Mods::default(), 2, 0.0);
    assert!(!opens(&fx) && sel.is_empty());
    let fx = click(&mut p, &mut sel, 0, Mods::default(), 1, 20.0);
    assert!(!opens(&fx) && sel.is_empty(), "a drag isn't a click");
}

#[test]
fn the_key_map_keeps_one_member_highlighted() {
    use crate::collection::keyboard::{Command, Target};
    use crate::contract::Direction;
    let map = super::controllers::key_map();
    assert!(
        map.chords_for(Command::Extend(Target::Step(Direction::Down)))
            .is_empty()
    );
    assert!(map.chords_for(Command::SelectAll).is_empty());
    assert!(map.chords_for(Command::Clear).is_empty());
    assert!(!map.chords_for(Command::Activate).is_empty());
    assert!(
        !map.chords_for(Command::Select(Target::Step(Direction::Left)))
            .is_empty()
    );
}

/// `operations/archives/preview`: "A wrong password keeps the prompt open with "The password is
/// incorrect."": with the field empty again, as Strata's prompt comes back.
#[test]
fn an_error_clears_the_password_typed() {
    let mut prompt = super::PromptState::default();
    prompt.edit.replace_text("hunter2");
    prompt.set_error(Some("The password is incorrect.".into()));
    assert_eq!(prompt.edit.text, "");
    assert_eq!(prompt.error(), Some("The password is incorrect."));
}

// The element, in a runtime.

mod prompt {
    use rune::accesskit::{Action, Live};
    use rune::{Cx, El, Env, Runtime};
    use strata_kit::look::Look;

    use super::super::{ArchivePrompt, PromptState};

    struct App {
        prompt: PromptState,
        look: Look,
    }

    fn view(s: &App, _cx: &mut Cx<App>) -> El {
        ArchivePrompt::new(&s.prompt, |s: &mut App| &mut s.prompt)
            .on_unlock(|s: &mut App, _, _| {
                s.prompt
                    .set_error(Some("The password is incorrect.".into()));
            })
            .build(&s.look)
    }

    /// The AccessKit ids of the live nodes in the tree.
    fn live_nodes(rt: &Runtime<App>) -> Vec<u64> {
        rt.accessibility_tree()
            .nodes
            .iter()
            .filter(|(_, n)| n.live() == Some(Live::Polite))
            .map(|(id, _)| id.0)
            .collect()
    }

    fn unlock(rt: &mut Runtime<App>) -> Vec<u64> {
        assert!(rt.accessibility_action("archive-prompt-unlock", Action::Click, None));
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        live_nodes(rt)
    }

    /// `operations/archives/preview`: "A wrong password keeps the prompt open with "The
    /// password is incorrect."", each time. AT-SPI announces a live region only when its node
    /// is added or its name changes, and a second error has the same words: it must be a new
    /// node to be heard.
    #[test]
    fn each_wrong_password_is_a_new_live_node() {
        let app = App {
            prompt: PromptState::default(),
            look: Look::fallback(false),
        };
        let mut rt = Runtime::new(app, view, (420.0, 400.0), Env::new(false, 1.0));
        rt.frame(1.0 / 60.0);
        assert!(live_nodes(&rt).is_empty(), "no error before an attempt");
        let first = unlock(&mut rt);
        assert_eq!(first.len(), 1, "one error after the first attempt");
        let second = unlock(&mut rt);
        assert_eq!(second.len(), 1, "one error after the second attempt");
        assert_ne!(
            first, second,
            "the second error is a new node, so it is announced"
        );
    }
}

mod element {
    use std::cell::Cell;

    use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, Tx};
    use strata_kit::look::Look;

    use super::super::{ArchiveState, ArchiveView};
    use super::tree;

    thread_local! {
        static PASSED: Cell<u32> = const { Cell::new(0) };
        static LEFT: Cell<u32> = const { Cell::new(0) };
    }

    struct App {
        archive: ArchiveState,
        look: Look,
        leave: bool,
        letters: bool,
    }

    fn view(s: &App, _cx: &mut Cx<App>) -> El {
        let mut v = ArchiveView::new(
            |s: &App| &s.archive,
            |s: &mut App| &mut s.archive,
            |s: &App| &s.look,
        )
        .letters(s.letters);
        if s.leave {
            v = v.on_leave(|_: &mut App, _: &mut Tx| LEFT.with(|l| l.set(l.get() + 1)));
        }
        v.build(s)
    }

    fn on_key(_: &mut App, _: &KeyEvent, _: &mut Tx) -> bool {
        PASSED.with(|p| p.set(p.get() + 1));
        true
    }

    /// Where Left is pressed.
    #[derive(Clone, Copy, PartialEq)]
    enum At {
        Root,
        /// In `docs/`.
        Docs,
    }

    /// What one Left did.
    #[derive(Debug, PartialEq)]
    struct Heard {
        /// Keys the window heard.
        passed: u32,
        /// `on_leave` calls.
        left: u32,
        /// At the root afterwards.
        root: bool,
    }

    fn click(rt: &mut Runtime<App>, label: &str) {
        let r = rt
            .elements()
            .into_iter()
            .find(|e| e.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("{label}"))
            .rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }

    /// `key` (Left, or `h` with the letters on) at `at`, with the tree focused by a click on a
    /// file, with an `on_leave` when `leave`.
    fn left(at: At, leave: bool, key: Key) -> Heard {
        PASSED.with(|p| p.set(0));
        LEFT.with(|l| l.set(0));
        let app = App {
            archive: ArchiveState::new("archive", tree()),
            look: Look::fallback(false),
            leave,
            letters: matches!(key, Key::Text(_)),
        };
        let mut rt = Runtime::new(app, view, (420.0, 400.0), Env::new(false, 1.0));
        rt.on_key = Some(on_key);
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
        if at == At::Docs {
            // A click on a folder opens it.
            click(&mut rt, "docs/");
            rt.frame(1.0 / 60.0);
            assert_eq!(rt.state.archive.listing().crumbs(), ["docs"]);
            click(&mut rt, "guide.md");
        } else {
            click(&mut rt, "README.md");
        }
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        rt.frame(1.0 / 60.0);
        Heard {
            passed: PASSED.with(Cell::get),
            left: LEFT.with(Cell::get),
            root: rt.state.archive.at_root(),
        }
    }

    fn heard(passed: u32, left: u32) -> Heard {
        Heard {
            passed,
            left,
            root: true,
        }
    }

    /// The description of the option named `name`, or none; panics with no such option.
    fn description(rt: &Runtime<App>, name: &str) -> Option<String> {
        let mut stack = vec![rt.accessibility_json()];
        while let Some(v) = stack.pop() {
            if v["role"] == "list-box-option" && v["name"] == name {
                return v["description"].as_str().map(str::to_owned);
            }
            if let Some(c) = v["children"].as_array() {
                stack.extend(c.iter().cloned());
            }
        }
        panic!("no option {name}");
    }

    /// Strata's archive rows read a file's size after its name; a folder's name ends in "/"
    /// and says no more.
    #[test]
    fn a_file_row_reads_its_size_after_its_name() {
        let app = App {
            archive: ArchiveState::new("archive", tree()),
            look: Look::fallback(false),
            leave: false,
            letters: false,
        };
        let mut rt = Runtime::new(app, view, (420.0, 400.0), Env::new(false, 1.0));
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(description(&rt, "README.md").as_deref(), Some("12.4 kB"));
        assert_eq!(description(&rt, "docs/"), None);
    }

    /// Left at the root goes to the host's `on_leave`; with none, it goes on past the tree to
    /// the window's keys, as Strata's 10xer `h` behaviour does, and so does `h` with the
    /// letters on. In a folder, Left goes up either way, and the host hears nothing.
    #[test]
    fn left_at_the_root_reaches_the_host() {
        let text = || Key::Text("h".into());
        assert_eq!(
            left(At::Root, true, Key::Left),
            heard(0, 1),
            "taken: on_leave"
        );
        assert_eq!(
            left(At::Root, false, Key::Left),
            heard(1, 0),
            "passed to the window"
        );
        assert_eq!(
            left(At::Root, false, text()),
            heard(1, 0),
            "h passed to the window"
        );
        assert_eq!(
            left(At::Docs, false, Key::Left),
            heard(0, 0),
            "up, with no on_leave"
        );
        assert_eq!(
            left(At::Docs, true, Key::Left),
            heard(0, 0),
            "up, with on_leave"
        );
    }
}
