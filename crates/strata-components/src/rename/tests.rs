use rune::{
    Cx, El, Env, Key, KeyEvent, Mods, PointerButton, Runtime, TextEdit, Tx, col, text_input,
};
use strata_kit::look::Look;

use super::*;
use crate::collection::element::{Collection, CollectionState};
use crate::contract::{EntryId, EntryKind, EntrySource, LinkTarget, ListEntry, ListSource};

const KEY: &str = "files";
const FRAME: f64 = 1.0 / 60.0;

// The first-selection rules.

#[test]
fn a_files_stem_runs_to_its_last_dot_and_a_folder_selects_everything() {
    fn stem(name: &str, kind: EntryKind) -> &str {
        let (a, b) = initial_selection(name, kind, Initial::Stem);
        &name[a..b]
    }
    let file = EntryKind::File;
    assert_eq!(stem("Notes.md", file), "Notes");
    assert_eq!(stem("archive.tar.gz", file), "archive.tar");
    assert_eq!(stem("README", file), "README");
    assert_eq!(stem("a.", file), "a");
    assert_eq!(stem("café.txt", file), "café");
    assert_eq!(stem("photos.2024", EntryKind::Folder), "photos.2024");
    assert_eq!(
        stem("link.d", EntryKind::Symlink(LinkTarget::Folder)),
        "link.d"
    );
}

#[test]
fn a_name_whose_only_dot_leads_it_selects_whole() {
    let (a, b) = initial_selection(".bashrc", EntryKind::File, Initial::Stem);
    assert_eq!((a, b), (0, ".bashrc".len()));
}

#[test]
fn a_new_items_default_name_selects_whole() {
    let (a, b) = initial_selection("new file.txt", EntryKind::File, Initial::All);
    assert_eq!((a, b), (0, "new file.txt".len()));
}

// How an edit ends, by the name in the field.

fn opened(name: &str) -> RenameState {
    let mut st = RenameState::new();
    let _ = st.begin(KEY, EntryId::new(7), name, EntryKind::File, Initial::Stem);
    st
}

fn ended(st: &mut RenameState, polled: Polled) -> RenameIntent {
    assert!(!st.is_open(), "the edit closes");
    assert!(polled.changed);
    let [intent] = <[RenameIntent; 1]>::try_from(polled.intents).expect("one intent");
    intent
}

fn typed(name: &str, text: &str) -> RenameState {
    let mut st = opened(name);
    st.edit.text = text.to_owned();
    st
}

#[test]
fn begin_selects_the_stem_and_asks_for_the_keyboard() {
    let st = opened("Notes.md");
    assert_eq!(st.edit.selected(), "Notes");
    assert!(st.edit.focused);
    assert_eq!(st.renaming_in(KEY), Some(EntryId::new(7)));
    assert_eq!(st.renaming_in("other"), None);
}

#[test]
fn a_changed_valid_name_renames_keeping_its_spaces() {
    let mut st = typed("Notes.md", " notes.md ");
    let polled = st.commit(By::Enter);
    assert_eq!(
        ended(&mut st, polled),
        RenameIntent::Rename {
            id: EntryId::new(7),
            name: " notes.md ".into(),
            by: By::Enter
        }
    );
}

#[test]
fn a_name_with_a_slash_the_validator_accepted_is_a_move() {
    let mut st = typed("Notes.md", "notes/todo.txt");
    let polled = st.commit(By::Enter);
    assert_eq!(
        ended(&mut st, polled),
        RenameIntent::Move {
            id: EntryId::new(7),
            path: "notes/todo.txt".into(),
            by: By::Enter
        }
    );
}

#[test]
fn an_empty_an_unchanged_or_a_refused_name_keeps_the_old_one() {
    let keep = |why| RenameIntent::Keep {
        id: EntryId::new(7),
        why,
        by: By::Blur,
    };
    let mut st = typed("Notes.md", "");
    let polled = st.commit(By::Blur);
    assert_eq!(ended(&mut st, polled), keep(Kept::Empty));

    let mut st = opened("Notes.md");
    let polled = st.commit(By::Blur);
    assert_eq!(ended(&mut st, polled), keep(Kept::Unchanged));

    let mut st = typed("Notes.md", "a//b");
    st.validated(Some("Enter a name".into()));
    let polled = st.commit(By::Blur);
    assert_eq!(ended(&mut st, polled), keep(Kept::Invalid));
}

#[test]
fn an_empty_field_is_never_an_error() {
    let mut st = typed("Notes.md", "");
    st.validated(Some("Enter a name".into()));
    assert_eq!(st.error(), None);
}

#[test]
fn begin_while_an_edit_is_open_cancels_it_first() {
    let mut st = opened("Notes.md");
    let polled = st.begin(
        KEY,
        EntryId::new(8),
        "Other",
        EntryKind::Folder,
        Initial::Stem,
    );
    assert_eq!(
        polled.intents,
        vec![RenameIntent::Keep {
            id: EntryId::new(7),
            why: Kept::Cancelled,
            by: By::Host
        }]
    );
    assert_eq!(st.renaming(), Some(EntryId::new(8)));
    assert_eq!(st.edit.selected(), "Other");
}

// Keys.

fn press(st: &mut RenameState, key: Key, mods: Mods, repeat: bool) -> Option<Polled> {
    st.key(&KeyEvent { key, mods }, repeat)
}

#[test]
fn enter_and_shift_enter_commit_and_chorded_enter_passes_on() {
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    for mods in [Mods::default(), shift] {
        let mut st = typed("Notes.md", "x.md");
        let polled = press(&mut st, Key::Enter, mods, false).expect("taken");
        assert!(matches!(
            ended(&mut st, polled),
            RenameIntent::Rename { by: By::Enter, .. }
        ));
    }
    for mods in [
        Mods {
            ctrl: true,
            ..Mods::default()
        },
        Mods {
            alt: true,
            ..Mods::default()
        },
        Mods {
            logo: true,
            ..Mods::default()
        },
    ] {
        let mut st = typed("Notes.md", "x.md");
        assert!(press(&mut st, Key::Enter, mods, false).is_none());
        assert!(st.is_open());
    }
}

#[test]
fn escape_cancels_and_tab_and_shift_tab_commit_toward_a_neighbour() {
    let mut st = typed("Notes.md", "x.md");
    let polled = press(&mut st, Key::Escape, Mods::default(), false).unwrap();
    assert!(matches!(
        ended(&mut st, polled),
        RenameIntent::Keep {
            why: Kept::Cancelled,
            by: By::Escape,
            ..
        }
    ));

    let mut st = typed("Notes.md", "x.md");
    let polled = press(&mut st, Key::Tab, Mods::default(), true).unwrap();
    let next = ended(&mut st, polled);
    assert_eq!(
        next.by().and_then(By::step),
        Some(1),
        "a held Tab walks on: {next:?}"
    );

    let mut st = opened("Notes.md");
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    let polled = press(&mut st, Key::Tab, shift, false).unwrap();
    assert_eq!(ended(&mut st, polled).by(), Some(By::Previous));
}

#[test]
fn shift_escape_cancels_as_escape_does() {
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    let mut st = typed("Notes.md", "x.md");
    let polled = press(&mut st, Key::Escape, shift, false).expect("taken");
    assert!(matches!(
        ended(&mut st, polled),
        RenameIntent::Keep {
            why: Kept::Cancelled,
            by: By::Escape,
            ..
        }
    ));
}

#[test]
fn a_menu_that_closes_without_giving_the_keyboard_back_gets_it_from_menu_closed() {
    // A fallback: Rune's modal restore normally gives the keyboard back first.
    let mut st = opened("Notes.md");
    st.menu_opened();
    st.edit.focused = false;
    let polled = st.menu_closed();
    assert!(polled.changed);
    assert!(st.edit.focused, "the field asks for the keyboard back");
    assert!(st.is_open());
}

#[test]
fn a_held_enter_ends_one_edit() {
    let mut st = typed("Notes.md", "x.md");
    let polled = press(&mut st, Key::Enter, Mods::default(), true).expect("taken");
    assert!(polled.intents.is_empty());
    assert!(st.is_open());
}

#[test]
fn keys_stay_the_input_methods_while_it_composes() {
    let mut st = typed("Notes.md", "x.md");
    st.edit.ime(&rune::Ime::Preedit("か".into(), None));
    for key in [Key::Enter, Key::Escape, Key::Tab] {
        assert!(press(&mut st, key, Mods::default(), false).is_none());
    }
    assert!(st.is_open());
}

// The editor in a collection, headless.

struct App {
    listing: ListSource,
    coll: CollectionState,
    look: Look,
    rename: RenameState,
    other: TextEdit,
    heard: Vec<RenameIntent>,
    menu: HostMenu,
    menu_at: Option<rune::PointerPress>,
    /// What the collection heard: its presses (`on_press`) and its controllers' intents.
    collection_heard: Vec<String>,
}

/// What the host does with a [`RenameIntent::FieldMenu`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum HostMenu {
    /// Nothing.
    #[default]
    None,
    /// Opens a menu (rune-kit's, as C21's), which takes the keyboard, and says when it
    /// closes.
    Real,
    /// Opens that menu, and never calls `menu_closed`.
    Forgetful,
    /// Calls `menu_opened`, and opens nothing.
    HoldOnly,
}

fn listing(n: usize) -> ListSource {
    ListSource::new(
        (1..=n as u64)
            .map(|i| {
                let (name, kind) = match i {
                    1 => ("Notes.md".to_owned(), EntryKind::File),
                    3 => ("Projects".to_owned(), EntryKind::Folder),
                    _ => (format!("item {i:03}.txt"), EntryKind::File),
                };
                ListEntry::new(EntryId::new(i), name, kind, "text")
            })
            .collect(),
    )
}

fn editor() -> RenameEditor<App> {
    RenameEditor::new(|s: &App| &s.rename, |s: &mut App| &mut s.rename)
        .validate(|_: &App, _, name: &str| name.contains("//").then(|| "Enter a name".to_owned()))
        .on_intent(|s: &mut App, intent, _: &mut Tx| {
            if let RenameIntent::FieldMenu { at, .. } = &intent
                && s.menu != HostMenu::None
            {
                s.rename.menu_opened();
                if s.menu != HostMenu::HoldOnly {
                    s.menu_at = Some(rune::PointerPress { x: at.x, y: at.y });
                }
            }
            s.heard.push(intent);
        })
}

fn view(s: &App, _: &mut Cx<App>) -> El {
    let list = Collection::new(
        KEY,
        |s: &App| (&s.listing, &s.coll),
        |s: &mut App| (&s.listing, &mut s.coll),
        |s: &App| &s.look,
        |_: &App| 0,
    )
    .rename(|s: &App| s.rename.renaming_in(KEY), editor().slot())
    .on_press(|s: &mut App, p, _| s.collection_heard.push(format!("press {:?}", p.button)))
    .on_intent(|s: &mut App, i, _| s.collection_heard.push(format!("{i:?}")))
    .build(s);
    let page = col()
        .w_full()
        .h_full()
        .child(
            text_input(&s.other, s.look.kit.ty.ui_compact)
                .single_line()
                .key("other")
                .w(120.0)
                .on_edit(|s: &mut App| &mut s.other),
        )
        .child(col().w(300.0).grow(1.0).min_h(0.0).child(list));
    let mut layer: Vec<El> = editor().message(s, &s.look).into_iter().collect();
    let menu = rune_kit::context_menu("menu", rune::el().key("menu-trigger"), s.menu_at)
        .item(rune_kit::MenuItem::new("Copy"))
        .on_close(|s: &mut App| {
            s.menu_at = None;
            if s.menu == HostMenu::Real {
                let _ = s.rename.menu_closed();
            }
        })
        .build(&s.look.kit, &mut layer);
    rune_kit::root(page.child(menu), layer)
}

fn mount(n: usize) -> Runtime<App> {
    let app = App {
        listing: listing(n),
        coll: CollectionState::new(),
        look: Look::fallback(false),
        rename: RenameState::new(),
        other: TextEdit::new(""),
        heard: Vec::new(),
        menu: HostMenu::None,
        menu_at: None,
        collection_heard: Vec::new(),
    };
    let mut rt = Runtime::new(app, view, (400.0, 300.0), Env::new(false, 1.0));
    rt.poll = Some(|s: &mut App, _, _| {
        let polled = s.rename.tick(KEY, &s.listing, &s.coll);
        s.heard.extend(polled.intents);
        polled.changed
    });
    rt.frame(0.0);
    frames(&mut rt, 2);
    rt
}

fn frames(rt: &mut Runtime<App>, n: usize) {
    for _ in 0..n {
        rt.frame(FRAME);
    }
}

fn begin(rt: &mut Runtime<App>, id: u64) {
    rt.update(|s, tx| {
        let id = EntryId::new(id);
        let index = s.listing.index_of(id).unwrap();
        let name = s.listing.entry(index).name.to_owned();
        let polled = s
            .rename
            .begin(KEY, id, &name, EntryKind::File, Initial::Stem);
        s.heard.extend(polled.intents);
        s.coll.reveal(KEY, &s.listing, index, tx);
    });
    frames(rt, 4);
}

fn rect(rt: &Runtime<App>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no {key}"))
        .rect
}

fn focused(rt: &Runtime<App>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn tap(rt: &mut Runtime<App>, key: &str) {
    let [x, y, w, h] = rect(rt, key);
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    frames(rt, 1);
    rt.pointer_up(x, y);
    frames(rt, 3);
}

fn last(rt: &Runtime<App>) -> Option<&RenameIntent> {
    rt.state.heard.last()
}

#[test]
fn a_press_on_another_field_commits_through_the_blur() {
    let mut rt = mount(20);
    begin(&mut rt, 1);
    assert_eq!(focused(&rt).as_deref(), Some(keys::FIELD));
    rt.key(KeyEvent {
        key: Key::Text("x".into()),
        mods: Mods::default(),
    });
    frames(&mut rt, 2);
    tap(&mut rt, "other");
    assert_eq!(
        last(&rt),
        Some(&RenameIntent::Rename {
            id: EntryId::new(1),
            name: "x.md".into(),
            by: By::Blur
        })
    );
    assert_eq!(focused(&rt).as_deref(), Some("other"));
}

#[test]
fn closing_from_the_field_gives_the_keyboard_back_to_the_collection() {
    for key in [Key::Enter, Key::Escape] {
        let mut rt = mount(20);
        begin(&mut rt, 1);
        rt.key(KeyEvent {
            key: key.clone(),
            mods: Mods::default(),
        });
        frames(&mut rt, 3);
        assert!(!rt.state.rename.is_open());
        assert_eq!(focused(&rt).as_deref(), Some(KEY), "after {key:?}");
    }
}

#[test]
fn a_row_scrolled_out_of_view_cancels_its_edit_once_it_had_shown() {
    let mut rt = mount(300);
    // Revealing a row far down: until the reveal lands, the row's absence isn't a scroll out.
    begin(&mut rt, 150);
    assert!(rt.state.rename.is_open(), "{:?}", rt.state.heard);
    let [x, y, w, h] = rect(&rt, "row-150");
    rt.pointer_move(x + w / 2.0, y + h / 2.0);
    for _ in 0..20 {
        rt.wheel_event(
            rune::WheelEvent::new(0.0, 200.0)
                .at(x + w / 2.0, y)
                .precise(false),
        );
        frames(&mut rt, 2);
    }
    frames(&mut rt, 5);
    assert_eq!(
        last(&rt),
        Some(&RenameIntent::Keep {
            id: EntryId::new(150),
            why: Kept::Cancelled,
            by: By::Scrolled
        })
    );
}

#[test]
fn an_entry_leaving_the_listing_or_renamed_elsewhere_cancels_its_edit() {
    let mut rt = mount(20);
    begin(&mut rt, 3);
    rt.update(|s, _| s.listing.retain(|e| e.id != EntryId::new(3)));
    frames(&mut rt, 3);
    assert_eq!(last(&rt).and_then(RenameIntent::by), Some(By::Listing));

    let mut rt = mount(20);
    begin(&mut rt, 3);
    rt.update(|s, _| {
        let mut entries = s.listing.entries().to_vec();
        entries[2].name = "elsewhere.txt".into();
        s.listing.replace(entries);
    });
    frames(&mut rt, 3);
    assert_eq!(last(&rt).and_then(RenameIntent::by), Some(By::Listing));
}

#[test]
fn a_right_click_on_the_field_asks_the_host_for_a_menu_and_keeps_editing() {
    let mut rt = mount(20);
    begin(&mut rt, 1);
    let [x, y, w, h] = rect(&rt, keys::FIELD);
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    rt.pointer_move(x, y);
    rt.secondary_down(x, y);
    frames(&mut rt, 1);
    rt.secondary_up(x, y);
    frames(&mut rt, 3);
    // Rune keeps its popup menus out of the accessibility tree: ask the runtime.
    assert!(rt.popup_menu().is_none(), "no native field menu");
    assert!(matches!(last(&rt), Some(RenameIntent::FieldMenu { .. })));
    assert!(rt.state.rename.has_keyboard());
}

#[test]
fn a_press_in_the_editors_box_off_the_text_line_keeps_editing() {
    let mut rt = mount(20);
    begin(&mut rt, 1);
    let [bx, by, _, bh] = rect(&rt, keys::EDITOR);
    let [fx, fy, fw, fh] = rect(&rt, keys::FIELD);
    // The box's margins, around the text line: above it, below it, and before it.
    let points = [
        (fx + fw / 2.0, by + 1.0),
        (fx + fw / 2.0, by + bh - 1.0),
        (bx + 1.0, by + bh / 2.0),
    ];
    for (x, y) in points {
        assert!(
            !(x >= fx && x <= fx + fw && y >= fy && y <= fy + fh),
            "({x}, {y}) is off the text line {:?}",
            [fx, fy, fw, fh]
        );
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        frames(&mut rt, 1);
        rt.pointer_up(x, y);
        frames(&mut rt, 3);
        assert!(
            rt.state.rename.has_keyboard(),
            "a press at ({x}, {y}) in the box kept the edit: {:?}",
            rt.state.heard
        );
    }
    assert!(rt.state.heard.is_empty(), "{:?}", rt.state.heard);
    // A right-click there is the field's too.
    let (x, y) = points[2];
    rt.secondary_down(x, y);
    frames(&mut rt, 1);
    rt.secondary_up(x, y);
    frames(&mut rt, 3);
    assert!(
        matches!(last(&rt), Some(RenameIntent::FieldMenu { .. })),
        "{:?}",
        rt.state.heard
    );
    assert!(rt.popup_menu().is_none());
}

fn right_click_field(rt: &mut Runtime<App>) {
    let [x, y, w, h] = rect(rt, keys::FIELD);
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    rt.pointer_move(x, y);
    rt.secondary_down(x, y);
    frames(rt, 1);
    rt.secondary_up(x, y);
    frames(rt, 3);
}

fn menu_key(rt: &mut Runtime<App>) {
    rt.key(KeyEvent {
        key: Key::ContextMenu,
        mods: Mods::default(),
    });
    frames(rt, 3);
}

#[test]
fn a_host_menu_holds_the_edit_open_while_it_has_the_keyboard() {
    for open in [menu_key, right_click_field] {
        let mut rt = mount(20);
        rt.state.menu = HostMenu::Real;
        begin(&mut rt, 1);
        open(&mut rt);
        assert!(matches!(last(&rt), Some(RenameIntent::FieldMenu { .. })));
        assert!(!rt.state.rename.edit.focused, "the menu has the keyboard");
        assert!(rt.state.rename.is_open(), "{:?}", rt.state.heard);
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        frames(&mut rt, 3);
        assert_eq!(focused(&rt).as_deref(), Some(keys::FIELD));
        assert!(rt.state.rename.is_open());
        tap(&mut rt, "other");
        assert_eq!(last(&rt).and_then(RenameIntent::by), Some(By::Blur));
    }
}

#[test]
fn page_keys_stay_in_the_field() {
    // Rune would scroll the listing a page with them, the row would leave the viewport, and
    // the edit would end as scrolled out, dropping the typed name.
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    for mods in [Mods::default(), shift] {
        for key in [Key::PageUp, Key::PageDown] {
            let mut rt = mount(60);
            begin(&mut rt, 12);
            rt.key(KeyEvent {
                key: Key::Text("Memo".into()),
                mods: Mods::default(),
            });
            frames(&mut rt, 2);
            assert_eq!(rt.state.rename.edit.text, "Memo.txt");
            rt.key(KeyEvent {
                key: key.clone(),
                mods,
            });
            frames(&mut rt, 6);
            assert!(
                rt.state.rename.has_keyboard(),
                "{key:?} (shift {}) ended the edit: {:?}",
                mods.shift,
                rt.state.heard
            );
            assert_eq!(rt.state.rename.edit.text, "Memo.txt");
        }
    }
}

#[test]
fn a_middle_press_in_the_editor_is_not_the_rows() {
    let mut rt = mount(20);
    rt.state.coll.controllers.pointer =
        Some(Box::new(crate::collection::pointer::PointerIntent::new()));
    // A folder's row: a middle press on it opens it in a new tab.
    begin(&mut rt, 3);
    let [bx, by, _, bh] = rect(&rt, keys::EDITOR);
    let [fx, fy, fw, fh] = rect(&rt, keys::FIELD);
    let middle = |rt: &mut Runtime<App>, x: f32, y: f32| {
        rt.pointer_move(x, y);
        rt.button_down(x, y, PointerButton::Middle);
        frames(rt, 1);
        rt.button_up(x, y, PointerButton::Middle);
        frames(rt, 3);
    };
    // The text line, then the box's margin before it.
    for (x, y) in [(fx + fw / 2.0, fy + fh / 2.0), (bx + 1.0, by + bh / 2.0)] {
        middle(&mut rt, x, y);
        assert!(
            rt.state.collection_heard.is_empty(),
            "a middle press at ({x}, {y}) reached the collection: {:?}",
            rt.state.collection_heard
        );
        assert!(rt.state.rename.has_keyboard());
    }
    // Not vacuous: a middle press on another row is the collection's.
    let [x, y, w, h] = rect(&rt, "row-6");
    middle(&mut rt, x + w / 2.0, y + h / 2.0);
    assert!(!rt.state.collection_heard.is_empty());
}

#[test]
fn a_menu_hold_ends_once_the_field_has_the_keyboard_back() {
    // The host's menu closed and the host never said so: the keyboard came back to the
    // field, so the next press away commits.
    let mut rt = mount(20);
    rt.state.menu = HostMenu::Forgetful;
    begin(&mut rt, 1);
    right_click_field(&mut rt);
    assert!(!rt.state.rename.edit.focused, "the menu has the keyboard");
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    frames(&mut rt, 3);
    assert!(rt.state.rename.has_keyboard());
    tap(&mut rt, "other");
    assert!(!rt.state.rename.is_open(), "{:?}", rt.state.heard);
    assert_eq!(last(&rt).and_then(RenameIntent::by), Some(By::Blur));
}

#[test]
fn a_hold_whose_menu_never_takes_the_keyboard_ends_at_once() {
    // `menu_opened` with no menu that takes the keyboard, and no `menu_closed`: the field
    // keeps the keyboard, and a press away still commits.
    let mut rt = mount(20);
    rt.state.menu = HostMenu::HoldOnly;
    begin(&mut rt, 1);
    menu_key(&mut rt);
    assert!(matches!(last(&rt), Some(RenameIntent::FieldMenu { .. })));
    assert!(rt.state.rename.has_keyboard());
    tap(&mut rt, "other");
    assert!(!rt.state.rename.is_open(), "{:?}", rt.state.heard);
    assert_eq!(last(&rt).and_then(RenameIntent::by), Some(By::Blur));
}

#[test]
fn the_validators_message_is_a_status_and_a_press_on_it_is_a_press_outside() {
    let mut rt = mount(20);
    begin(&mut rt, 1);
    for c in ["a", "/", "/"] {
        rt.key(KeyEvent {
            key: Key::Text(c.into()),
            mods: Mods::default(),
        });
        frames(&mut rt, 1);
    }
    frames(&mut rt, 2);
    assert_eq!(rt.state.rename.error(), Some("Enter a name"));
    // Walk the tree (a `serde_json::Value`, which this crate doesn't name) for the message.
    let mut stack = vec![rt.accessibility_json()];
    let mut found = None;
    while let Some(node) = stack.pop() {
        if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            stack.extend(children.iter().cloned());
        }
        if node["key"] == keys::MESSAGE {
            found = Some(node);
        }
    }
    let node = found.expect("the message's node");
    assert_eq!(node["role"], "status");
    assert!(node.get("actions").is_none(), "nothing to press: {node}");
    // The press goes to the row under the message, as any press outside the field.
    tap(&mut rt, keys::MESSAGE);
    assert!(!rt.state.rename.is_open());
    assert_eq!(
        last(&rt),
        Some(&RenameIntent::Keep {
            id: EntryId::new(1),
            why: Kept::Invalid,
            by: By::Blur
        })
    );
    assert_eq!(
        rt.state.coll.selection.selected_ids(&rt.state.listing),
        vec![EntryId::new(2)]
    );
}

#[test]
fn a_field_the_host_closes_with_a_blur_survives_runes_focus_sync() {
    // Upstream #180: Rune syncs focus into every field of the last view, also the one the
    // host just closed; the spare edit takes it.
    let mut rt = mount(20);
    begin(&mut rt, 1);
    rt.update(|s, tx| {
        let _ = s.rename.commit(By::Host);
        tx.blur();
    });
    frames(&mut rt, 3);
    assert!(!rt.state.rename.is_open());
    assert!(!rt.state.rename.edit.focused);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "the edit's scope")]
fn a_tick_with_another_collections_state_is_a_host_bug() {
    let mut st = opened("Notes.md");
    let _ = st.tick("elsewhere", &listing(3), &CollectionState::new());
}

#[test]
fn a_composition_ending_in_a_blur_keeps_only_the_committed_text() {
    let mut rt = mount(20);
    begin(&mut rt, 1);
    rt.ime(rune::Ime::Commit("か".into()));
    frames(&mut rt, 1);
    rt.ime(rune::Ime::Preedit("き".into(), Some((0, 3))));
    frames(&mut rt, 1);
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    frames(&mut rt, 1);
    assert!(rt.state.rename.is_open(), "Escape went to the composition");
    tap(&mut rt, "other");
    assert_eq!(
        last(&rt),
        Some(&RenameIntent::Rename {
            id: EntryId::new(1),
            name: "か.md".into(),
            by: By::Blur
        })
    );
}

#[test]
fn a_pasted_line_break_or_tab_is_dropped() {
    let mut rt = mount(20);
    begin(&mut rt, 1);
    rt.paste("a\nb\tc");
    frames(&mut rt, 2);
    assert_eq!(rt.state.rename.edit.text, "abc.md");
}
