//! C21's unit tests, in a headless runtime where they touch Rune: what opens, where the
//! keyboard goes, what a choice reports, and what the field menu does to its field.

use rune::clipboard::MemoryClipboard;
use rune::runtime::ElementInfo;
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, TextEdit, col, row, text_input};
use strata_kit::look::Look;

use super::*;
use crate::collection::selection::{AnchorRule, Selection};
use crate::contract::{EntryId, EntryKind, ListEntry, ListSource, Point, Rect, UniformRows};

const W: f32 = 600.0;
const H: f32 = 400.0;

struct Host {
    look: Look,
    menu: ContextMenuState,
    fields: FieldMenuState<u8>,
    edit: TextEdit,
    secret: TextEdit,
    heard: Vec<String>,
    dismissed: u32,
    changed: u32,
    closed: u32,
    undo: bool,
    /// Field 0 has gone (its entry was deleted mid-rename, say).
    gone: bool,
    /// The window's Ctrl+A ran: the view's Select All, as Strata binds it.
    selected_all: bool,
    /// The field menu is for read-only fields.
    read_only: bool,
    /// The field menu pastes through the host's own paste: what it got.
    own_paste: Option<Vec<String>>,
}

impl Host {
    fn new() -> Self {
        Self {
            look: Look::fallback(false),
            menu: ContextMenuState::new(),
            fields: FieldMenuState::new(),
            edit: TextEdit::new("hello world"),
            secret: TextEdit::new("hunter2").secure(),
            heard: Vec::new(),
            dismissed: 0,
            changed: 0,
            closed: 0,
            undo: false,
            gone: false,
            selected_all: false,
            read_only: false,
            own_paste: None,
        }
    }
}

fn menu() -> ContextMenu<Host> {
    ContextMenu::new("m", |s: &Host| &s.menu, |s: &mut Host| &mut s.menu)
        .name("Notes.md")
        .on_intent(|s: &mut Host, intent, _| match intent {
            MenuIntent::Activate { target, action } => s
                .heard
                .push(format!("{action} {:?}", target.entry().map(EntryId::get))),
        })
        .on_dismiss(|s: &mut Host, _| s.dismissed += 1)
}

fn edit_mut(s: &mut Host, f: u8) -> Option<&mut TextEdit> {
    match f {
        0 if s.gone => None,
        0 => Some(&mut s.edit),
        1 => Some(&mut s.secret),
        _ => None,
    }
}

fn edit(s: &Host, f: u8) -> Option<&TextEdit> {
    match f {
        0 if s.gone => None,
        0 => Some(&s.edit),
        1 => Some(&s.secret),
        _ => None,
    }
}

fn fields(s: &Host) -> FieldMenu<Host, u8> {
    let mut m = FieldMenu::new(
        "f",
        |s: &Host| &s.fields,
        |s: &mut Host| &mut s.fields,
        edit_mut,
        edit,
    )
    .undo(s.undo)
    .on_change(|s: &mut Host, _, _| s.changed += 1)
    .on_close(|s: &mut Host, _| s.closed += 1);
    if s.read_only {
        m = m.read_only();
    }
    if s.own_paste.is_some() {
        m = m.on_paste(|s: &mut Host, _, text: &str, _| {
            s.own_paste.get_or_insert_default().push(text.to_owned())
        });
    }
    m
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    let ty = s.look.kit.ty.ui_compact;
    let mut layer = Vec::new();
    let opener = row()
        .key("opener")
        .role("button")
        .label("Opener")
        .size(80.0, 30.0)
        .focusable(true)
        .on_press(|_: &mut Host| {});
    let field = text_input(&s.edit, ty)
        .key("field")
        .single_line()
        .w(200.0)
        .h(24.0)
        .on_edit(|s: &mut Host| &mut s.edit)
        .on_secondary_press(fields(s).hook(0));
    let secret = text_input(&s.secret, ty)
        .key("secret")
        .single_line()
        .w(200.0)
        .h(24.0)
        .on_edit(|s: &mut Host| &mut s.secret)
        .on_secondary_press(fields(s).hook(1));
    let a = menu().build(s, &s.look, &mut layer);
    let b = fields(s).build(s, &s.look, &mut layer);
    rune_kit::root(
        col()
            .gap(8.0)
            .child(opener)
            .child(field)
            .child(secret)
            .child(a)
            .child(b),
        layer,
    )
}

/// The host's poll: a field menu whose field went closes.
fn poll(s: &mut Host, _: f64, _: &mut rune::Tx) -> bool {
    fields(s).check(s)
}

/// The window's keys: Ctrl+A selects everything in the view, as Strata's binding does.
fn app_key(s: &mut Host, ev: &KeyEvent, _: &mut rune::Tx) -> bool {
    if ev.is_command('a') {
        s.selected_all = true;
        return true;
    }
    false
}

fn runtime() -> Runtime<Host> {
    let mut rt = Runtime::new(Host::new(), view, (W, H), Env::new(false, 1.0));
    rt.poll = Some(poll);
    rt.on_key = Some(app_key);
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn step(rt: &mut Runtime<Host>) {
    for _ in 0..4 {
        rt.frame(1.0 / 60.0);
    }
}

fn press(rt: &mut Runtime<Host>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    step(rt);
}

fn key(rt: &mut Runtime<Host>, k: Key) {
    press(rt, k, Mods::default());
}

fn live(rt: &Runtime<Host>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
}

fn focused(rt: &Runtime<Host>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

fn tap(rt: &mut Runtime<Host>, key: &str) {
    let [x, y, w, h] = live(rt, key).unwrap_or_else(|| panic!("no {key}")).rect;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    rt.pointer_down(x, y);
    step(rt);
    rt.pointer_up(x, y);
    step(rt);
}

fn secondary(rt: &mut Runtime<Host>, key: &str) {
    let [x, y, w, h] = live(rt, key).unwrap_or_else(|| panic!("no {key}")).rect;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    rt.secondary_down(x, y);
    step(rt);
    rt.secondary_up(x, y);
    step(rt);
}

/// Header (2 lines), Open, Open With… (insensitive), an empty section, Send to (Desktop,
/// Mail), Copy, then a labelled section: Move to Trash, Permanently delete.
///
/// | key | row |
/// | --- | --- |
/// | m-item3 | Open |
/// | m-item4 | Open With… |
/// | m-item6 | Send to (submenu m-item6-sub-item0 Desktop, -item1 Mail) |
/// | m-item7 | Copy |
/// | m-item10 | Move to Trash |
/// | m-item11 | Permanently delete |
fn model() -> MenuModel {
    MenuModel::new()
        .header_line("Notes.md")
        .header_line("~/Documents")
        .section(Section::new([
            Item::new("open", "Open").shortcut("Enter"),
            Item::new("with", "Open With…")
                .enabled(false)
                .description("Looking for compatible applications…"),
        ]))
        .section(Section::new([]))
        .section(Section::new([
            Item::submenu(
                "send",
                "Send to",
                [Section::new([
                    Item::new("desktop", "Desktop"),
                    Item::new("mail", "Mail"),
                ])],
            ),
            Item::new("copy", "Copy").shortcut("Ctrl+C"),
        ]))
        .section(
            Section::new([
                Item::new("trash", "Move to Trash"),
                Item::new("delete", "Permanently delete").danger(true),
            ])
            .labelled("Remove"),
        )
}

/// The opener has the keyboard, then the menu opens over entry 4.
fn opened() -> Runtime<Host> {
    let mut rt = runtime();
    key(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some("opener"));
    rt.update(|s, _| {
        s.menu.open(
            MenuTarget::Entry(EntryId::new(4)),
            Point::new(120.0, 60.0),
            model(),
        )
    });
    step(&mut rt);
    rt
}

// --- The model ---

#[test]
fn item_at_counts_visible_items_across_sections_and_into_submenus() {
    let m = model();
    let label = |p: &[usize]| m.item_at(p).map(|i| i.label.as_str());
    assert_eq!(label(&[0]), Some("Open"));
    assert_eq!(
        label(&[2]),
        Some("Send to"),
        "the empty section counts nothing"
    );
    assert_eq!(label(&[2, 1]), Some("Mail"));
    assert_eq!(label(&[5]), Some("Permanently delete"));
    assert_eq!(label(&[6]), None);
    assert_eq!(label(&[0, 0]), None, "Open has no submenu");
}

#[test]
fn an_empty_section_draws_no_separator_or_label() {
    // browser/context-menu: "Actions hidden for the current target leave no empty group or
    // doubled separator."
    let rt = opened();
    let separators: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting && e.path.contains("m-menu"))
        .filter_map(|e| e.key.filter(|k| k.starts_with("sep")))
        .collect();
    assert_eq!(
        separators,
        ["sep2", "sep5", "sep8"],
        "after the header, then between the three non-empty sections"
    );
}

// --- State ---

#[test]
fn a_model_update_keeps_a_submenu_that_is_still_there() {
    let mut st = ContextMenuState::new();
    st.open(MenuTarget::Background, Point::default(), model());
    assert!(st.open_submenu(&[2]));
    assert!(!st.open_submenu(&[0]), "Open has no submenu");
    st.update_model(model());
    assert_eq!(st.submenu(), [2]);
    st.update_model(MenuModel::new().section(Section::new([Item::new("a", "A")])));
    assert!(st.submenu().is_empty(), "the submenu's row went");
    assert_eq!(st.close(), Some(MenuTarget::Background));
    assert!(!st.is_open());
}

// --- Opening, focus and choosing ---

#[test]
fn the_menu_opens_with_the_keyboard_on_its_first_action_and_is_named() {
    // browser/context-menu: "A menu opens with keyboard focus on its first visible action".
    let rt = opened();
    assert_eq!(focused(&rt).as_deref(), Some("m-item3"));
    let surface = live(&rt, "m-menu").expect("the menu");
    assert_eq!(
        (surface.role, surface.label.as_deref()),
        ("menu", Some("Notes.md"))
    );
    assert!(
        !live(&rt, "m-item4").unwrap().focusable,
        "Open With… is insensitive"
    );
}

#[test]
fn up_and_down_skip_insensitive_rows_and_wrap_home_and_end_jump() {
    // browser/context-menu: "Up and Down move between enabled actions, skipping separators
    // and wrapping at either end; Home and End jump to the first and last action."
    let mut rt = opened();
    key(&mut rt, Key::Down);
    assert_eq!(focused(&rt).as_deref(), Some("m-item6"), "past Open With…");
    key(&mut rt, Key::End);
    assert_eq!(focused(&rt).as_deref(), Some("m-item11"));
    key(&mut rt, Key::Down);
    assert_eq!(focused(&rt).as_deref(), Some("m-item3"), "wraps to the top");
    key(&mut rt, Key::Up);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("m-item11"),
        "wraps to the bottom"
    );
    key(&mut rt, Key::Home);
    assert_eq!(focused(&rt).as_deref(), Some("m-item3"));
}

#[test]
fn enter_and_space_run_the_focused_action_close_the_menu_and_return_focus() {
    // browser/context-menu: "Enter or Space activates the focused action and closes the menu."
    for activate in [Key::Enter, Key::Text(" ".into())] {
        let mut rt = opened();
        key(&mut rt, Key::End);
        key(&mut rt, activate);
        assert_eq!(rt.state.heard, ["delete Some(4)"]);
        assert!(!rt.state.menu.is_open());
        assert_eq!(rt.state.dismissed, 0, "a choice isn't a dismissal");
        assert_eq!(focused(&rt).as_deref(), Some("opener"));
    }
}

#[test]
fn escape_closes_the_menu_and_returns_focus_to_what_opened_it() {
    // browser/context-menu: "Escape closes the menu, keeps the selection, and returns focus to
    // the entry or pane that opened it."
    let mut rt = opened();
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Escape);
    assert!(!rt.state.menu.is_open());
    assert_eq!((rt.state.dismissed, rt.state.heard.len()), (1, 0));
    assert_eq!(focused(&rt).as_deref(), Some("opener"));
}

#[test]
fn ctrl_a_leaves_the_menu_open() {
    // browser/context-menu: "Ctrl+A while a menu is open leaves the menu open."
    let mut rt = opened();
    press(
        &mut rt,
        Key::Command('a'),
        Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    );
    assert!(rt.state.menu.is_open());
    assert_eq!(focused(&rt).as_deref(), Some("m-item3"));
    assert!(
        !rt.state.selected_all,
        "the window's Select All didn't run under the menu"
    );
}

#[test]
fn a_press_outside_dismisses_the_menu() {
    let mut rt = opened();
    rt.pointer_down(W - 10.0, H - 10.0);
    step(&mut rt);
    rt.pointer_up(W - 10.0, H - 10.0);
    step(&mut rt);
    assert!(!rt.state.menu.is_open());
    assert_eq!(rt.state.dismissed, 1);
}

#[test]
fn a_click_on_a_row_runs_it() {
    let mut rt = opened();
    tap(&mut rt, "m-item7");
    assert_eq!(rt.state.heard, ["copy Some(4)"]);
}

#[test]
fn a_release_without_a_press_on_a_row_runs_nothing() {
    // browser/context-menu: "A button release that reaches an item or background menu, or
    // its submenus, before any press inside it is ignored." and "Pressing the secondary
    // button, moving onto a menu row, and releasing does not run the row".
    let mut rt = opened();
    let [x, y, w, h] = live(&rt, "m-item7").unwrap().rect;
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    step(&mut rt);
    rt.secondary_down(x - 50.0, y);
    step(&mut rt);
    rt.pointer_move(x + w / 2.0, y + h / 2.0);
    step(&mut rt);
    rt.secondary_up(x + w / 2.0, y + h / 2.0);
    step(&mut rt);
    assert!(rt.state.heard.is_empty());
    assert!(rt.state.menu.is_open());
    // "After that, a full click on a menu row runs it."
    tap(&mut rt, "m-item7");
    assert_eq!(rt.state.heard, ["copy Some(4)"]);
}

// --- Submenus ---

#[test]
fn right_opens_a_submenu_with_its_first_action_focused_and_left_returns_to_its_row() {
    // browser/context-menu: "On a submenu row such as Actions or Send to…, Right, Space, or
    // Enter opens the submenu with its first action focused." and "Left in a submenu closes
    // only the submenu and refocuses its row; Up and Down then move within the parent menu."
    for open in [Key::Right, Key::Enter, Key::Text(" ".into())] {
        let mut rt = opened();
        key(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("m-item6"));
        assert_eq!(live(&rt, "m-item6").unwrap().expanded, Some(false));
        key(&mut rt, open.clone());
        assert_eq!(rt.state.menu.submenu(), [2], "{open:?}");
        assert_eq!(live(&rt, "m-item6").unwrap().expanded, Some(true));
        assert_eq!(
            focused(&rt).as_deref(),
            Some("m-item6-sub-item0"),
            "{open:?}"
        );
        key(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("m-item6-sub-item1"));
        key(&mut rt, Key::Left);
        assert!(rt.state.menu.submenu().is_empty());
        assert!(rt.state.menu.is_open(), "only the submenu closed");
        assert_eq!(focused(&rt).as_deref(), Some("m-item6"));
        key(&mut rt, Key::Down);
        assert_eq!(
            focused(&rt).as_deref(),
            Some("m-item7"),
            "Down moves in the parent"
        );
    }
}

#[test]
fn enter_in_a_submenu_runs_its_action_and_escape_closes_the_whole_menu() {
    let mut rt = opened();
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.heard, ["mail Some(4)"]);
    assert!(!rt.state.menu.is_open());
    assert_eq!(focused(&rt).as_deref(), Some("opener"));

    let mut rt = opened();
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    key(&mut rt, Key::Escape);
    assert!(!rt.state.menu.is_open());
    assert_eq!(rt.state.dismissed, 1);
    assert_eq!(focused(&rt).as_deref(), Some("opener"));
}

#[test]
fn a_click_opens_a_submenu_and_leaves_the_parents_rows_clickable() {
    let mut rt = opened();
    tap(&mut rt, "m-item6");
    assert_eq!(rt.state.menu.submenu(), [2]);
    assert!(live(&rt, "m-item6-sub-item0").is_some());
    tap(&mut rt, "m-item7");
    assert_eq!(
        rt.state.heard,
        ["copy Some(4)"],
        "the parent's Copy still runs"
    );

    let mut rt = opened();
    tap(&mut rt, "m-item6");
    tap(&mut rt, "m-item6-sub-item0");
    assert_eq!(rt.state.heard, ["desktop Some(4)"]);
}

#[test]
fn layer_order_is_runekits() {
    // The surfaces are found by the order rune-kit pushes them (view.rs): the top menu gets
    // its name and the submenu Strata's corners. If a Rune bump moves them, this fails.
    let mut rt = opened();
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    let look = Look::fallback(false);
    let top = live(&rt, "m-menu").unwrap();
    let sub = live(&rt, "m-item6-sub-menu").unwrap();
    assert_eq!(top.label.as_deref(), Some("Notes.md"));
    assert_eq!(
        sub.label.as_deref(),
        Some("Send to"),
        "a submenu takes its row's name"
    );
    assert_eq!((top.radius, sub.radius), (look.round(9.0), look.round(9.0)));
}

// --- The keyboard anchor ---

fn listing(n: u64) -> ListSource {
    ListSource::new(
        (1..=n)
            .map(|i| ListEntry::new(EntryId::new(i), format!("e{i}"), EntryKind::File, ""))
            .collect(),
    )
}

#[test]
fn the_keyboard_anchor_is_the_selected_cursors_edge_else_the_panes_centre() {
    // browser/context-menu: "A keyboard-opened item menu anchors to the entry, not the
    // pointer: its right edge in Columns and Icons, its center in List." and "With nothing
    // selected, Menu or Shift+F10 opens the active pane's background menu."
    let src = listing(20);
    let rows = UniformRows::new(20, 30.0, 300.0);
    let visible = Rect::new(0.0, 0.0, 300.0, 200.0);
    let window = Point::new(50.0, 100.0);
    let mut sel = Selection::new();
    let anchor = |sel: &Selection, edge| keyboard_anchor(&src, sel, &rows, visible, window, edge);

    let a = anchor(&sel, AnchorEdge::Right);
    assert_eq!(
        (a.target, a.at, a.reveal),
        (None, Point::new(200.0, 200.0), None)
    );

    sel.select_only(&src, 2);
    let a = anchor(&sel, AnchorEdge::Right);
    assert_eq!(a.target, Some(EntryId::new(3)));
    assert_eq!(
        a.at,
        Point::new(350.0, 175.0),
        "right edge, centred on row 2"
    );
    let a = anchor(&sel, AnchorEdge::Center);
    assert_eq!(a.at, Point::new(200.0, 175.0));

    // The cursor moved on, off the selection: the background menu.
    sel.move_cursor(&src, 5, AnchorRule::Keep);
    assert_eq!(anchor(&sel, AnchorEdge::Right).target, None);
}

#[test]
fn a_cursor_out_of_view_is_revealed_and_the_menu_opens_where_it_lands() {
    let src = listing(100);
    let rows = UniformRows::new(100, 30.0, 300.0);
    let visible = Rect::new(0.0, 600.0, 300.0, 200.0);
    let window = Point::new(0.0, 0.0);
    let mut sel = Selection::new();
    sel.select_only(&src, 50);
    let a = keyboard_anchor(&src, &sel, &rows, visible, window, AnchorEdge::Center);
    assert_eq!(a.reveal, Some(50));
    assert_eq!(a.at.y, 200.0 - 15.0, "at the bottom edge it scrolls to");
    sel.select_only(&src, 2);
    let a = keyboard_anchor(&src, &sel, &rows, visible, window, AnchorEdge::Center);
    assert_eq!((a.reveal, a.at.y), (Some(2), 15.0), "at the top edge");
}

#[test]
fn menu_and_shift_f10_open_with_or_without_super_and_other_chords_open_nothing() {
    // browser/context-menu: "Ctrl+Shift+F10, Shift+Menu, and Ctrl+Menu open no menu." and
    // "Menu or Shift+F10 with Hyper or Meta held opens the menu as without them."
    let ev = |key, m: Mods| KeyEvent { key, mods: m };
    let none = Mods::default();
    let shift = Mods {
        shift: true,
        ..none
    };
    let logo = Mods { logo: true, ..none };
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..none
    };
    let alt = Mods { alt: true, ..none };
    let f10 = Key::Function(10);
    let menu = Key::ContextMenu;
    for (k, m, want) in [
        (menu.clone(), none, Some(true)),
        (menu.clone(), logo, Some(true)),
        (f10.clone(), shift, Some(true)),
        (
            f10.clone(),
            Mods {
                logo: true,
                ..shift
            },
            Some(true),
        ),
        (menu.clone(), shift, Some(false)),
        (menu.clone(), ctrl, Some(false)),
        (menu.clone(), alt, Some(false)),
        (
            f10.clone(),
            Mods {
                ctrl: true,
                cmd: true,
                ..shift
            },
            Some(false),
        ),
        (f10.clone(), none, None),
        (Key::Enter, none, None),
    ] {
        assert_eq!(menu_chord(&ev(k.clone(), m)), want, "{k:?} {m:?}");
    }
}

// --- The text-field menu ---

fn with_clipboard(text: Option<&str>) -> (Runtime<Host>, MemoryClipboard) {
    let mut rt = runtime();
    let m = rune::clipboard::memory();
    if let Some(t) = text {
        m.set_text(t);
    }
    rt.use_clipboard(&m);
    (rt, m)
}

fn enabled(rt: &Runtime<Host>) -> Vec<(String, bool)> {
    let mut rows: Vec<ElementInfo> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting && e.role == "menuitem" && e.path.contains("f-menu"))
        .collect();
    rows.sort_by(|a, b| a.rect[1].total_cmp(&b.rect[1]));
    rows.into_iter()
        .map(|e| (e.label.unwrap_or_default(), e.focusable))
        .collect()
}

fn names(v: &[(&str, bool)]) -> Vec<(String, bool)> {
    v.iter().map(|(n, b)| (n.to_string(), *b)).collect()
}

#[test]
fn a_right_click_on_a_field_opens_the_text_menu_with_nothing_selected() {
    // browser/context-menu: "Right-click menus in text inputs use the same surface, border,
    // corners, font, and separators as file menus".
    let (mut rt, _) = with_clipboard(Some("pasted"));
    secondary(&mut rt, "field");
    assert_eq!(rt.state.fields.field(), Some(0));
    assert_eq!(rt.state.fields.clipboard(), Clipboard::Text);
    let surface = live(&rt, "f-menu").expect("the field menu");
    assert_eq!(surface.label.as_deref(), Some("Text editing"));
    assert_eq!(surface.radius, rt.state.look.round(9.0));
    assert_eq!(
        enabled(&rt),
        names(&[
            ("Cut", false),
            ("Copy", false),
            ("Paste", true),
            ("Delete", false),
            ("Select All", true),
        ])
    );
}

#[test]
fn menu_and_shift_f10_in_a_focused_field_open_its_text_menu() {
    // browser/context-menu: "While a text input has focus, Menu and Shift+F10 open the
    // input's text-editing menu instead of a file menu."
    for (k, m) in [
        (Key::ContextMenu, Mods::default()),
        (
            Key::Function(10),
            Mods {
                shift: true,
                ..Mods::default()
            },
        ),
    ] {
        let (mut rt, _) = with_clipboard(Some("x"));
        rt.update(|s, _| s.edit.focused = true);
        step(&mut rt);
        assert_eq!(focused(&rt).as_deref(), Some("field"));
        press(&mut rt, k, m);
        assert_eq!(rt.state.fields.field(), Some(0));
        // Nothing is selected, so Cut and Copy are insensitive: Paste is the first action.
        assert_eq!(
            focused(&rt).as_deref(),
            Some("f-item2"),
            "the menu has the keyboard"
        );
    }
}

#[test]
fn select_all_then_copy_and_cut_write_the_clipboard_and_the_keyboard_returns() {
    let (mut rt, m) = with_clipboard(None);
    rt.update(|s, _| s.edit.focused = true);
    step(&mut rt);
    secondary(&mut rt, "field");
    tap(&mut rt, "f-item5"); // Select All, after the separator
    assert_eq!(rt.state.edit.range(), (0, 11));
    assert_eq!(focused(&rt).as_deref(), Some("field"), "back on the field");
    assert_eq!(rt.state.closed, 1);

    secondary(&mut rt, "field");
    assert_eq!(
        rt.state.edit.range(),
        (0, 11),
        "the menu keeps the selection"
    );
    tap(&mut rt, "f-item1"); // Copy
    assert_eq!(m.text().as_deref().map(String::as_str), Some("hello world"));
    assert_eq!(rt.state.changed, 0, "a copy changes nothing");

    secondary(&mut rt, "field");
    tap(&mut rt, "f-item0"); // Cut
    assert_eq!(rt.state.edit.text, "");
    assert_eq!(rt.state.changed, 1);
}

#[test]
fn paste_pastes_the_clipboards_text_on_one_line() {
    let (mut rt, _) = with_clipboard(Some("two\nlines\t!"));
    rt.update(|s, _| {
        s.edit.focused = true;
        s.edit.set_selection(0, 5);
    });
    step(&mut rt);
    secondary(&mut rt, "field");
    tap(&mut rt, "f-item2");
    step(&mut rt);
    assert_eq!(rt.state.edit.text, "twolines! world");
    assert_eq!(rt.state.changed, 1);
}

#[test]
fn paste_is_insensitive_once_the_clipboard_reads_empty() {
    let (mut rt, _) = with_clipboard(None);
    secondary(&mut rt, "field");
    assert_eq!(rt.state.fields.clipboard(), Clipboard::Empty);
    assert_eq!(enabled(&rt)[2], ("Paste".to_owned(), false));
}

#[test]
fn delete_removes_the_selection_and_a_secure_fields_text_is_never_copied() {
    let (mut rt, _) = with_clipboard(Some("x"));
    rt.update(|s, _| s.edit.set_selection(0, 6));
    step(&mut rt);
    secondary(&mut rt, "field");
    tap(&mut rt, "f-item3");
    assert_eq!(rt.state.edit.text, "world");

    rt.update(|s, _| s.secret.set_selection(0, 7));
    step(&mut rt);
    secondary(&mut rt, "secret");
    assert_eq!(
        enabled(&rt),
        names(&[
            ("Cut", false),
            ("Copy", false),
            ("Paste", true),
            ("Delete", true),
            ("Select All", true),
        ])
    );
}

#[test]
fn undo_and_redo_show_only_when_asked_and_follow_the_history() {
    let (mut rt, _) = with_clipboard(Some("x"));
    rt.update(|s, _| {
        s.undo = true;
        s.edit.set_selection(0, 6);
    });
    step(&mut rt);
    secondary(&mut rt, "field");
    assert_eq!(
        enabled(&rt)[..2],
        names(&[("Undo", false), ("Redo", false)])
    );
    tap(&mut rt, "f-item6"); // Delete: Undo, Redo, separator, Cut, Copy, Paste, Delete
    assert_eq!(rt.state.edit.text, "world");
    secondary(&mut rt, "field");
    assert_eq!(enabled(&rt)[..2], names(&[("Undo", true), ("Redo", false)]));
    tap(&mut rt, "f-item0");
    assert_eq!(rt.state.edit.text, "hello world");
    secondary(&mut rt, "field");
    assert_eq!(enabled(&rt)[..2], names(&[("Undo", false), ("Redo", true)]));
    tap(&mut rt, "f-item1");
    assert_eq!(rt.state.edit.text, "world");
}

#[test]
fn escape_closes_the_field_menu_and_gives_the_field_the_keyboard_back() {
    let (mut rt, _) = with_clipboard(Some("x"));
    rt.update(|s, _| s.edit.focused = true);
    step(&mut rt);
    secondary(&mut rt, "field");
    key(&mut rt, Key::Escape);
    assert!(!rt.state.fields.is_open());
    assert_eq!(rt.state.closed, 1);
    assert_eq!(focused(&rt).as_deref(), Some("field"));
}

#[test]
fn a_menu_whose_field_went_closes_on_the_hosts_poll_and_stays_closed() {
    let (mut rt, _) = with_clipboard(Some("x"));
    secondary(&mut rt, "field");
    assert!(rt.state.fields.is_open());
    rt.update(|s, _| s.gone = true);
    step(&mut rt);
    assert!(!rt.state.fields.is_open(), "the menu closed with its field");
    assert_eq!(rt.state.closed, 1, "on_close heard it: C5's menu_closed");
    // The same field again (the next rename with the same `F`): no stale menu comes back.
    rt.update(|s, _| s.gone = false);
    step(&mut rt);
    assert!(live(&rt, "f-menu").is_none());
}

#[test]
fn a_menu_for_a_field_that_isnt_there_doesnt_open() {
    let (mut rt, _) = with_clipboard(Some("x"));
    rt.update(|s, tx| fields(s).open(s, 7, Point::new(10.0, 10.0), tx));
    step(&mut rt);
    assert!(!rt.state.fields.is_open());
    assert_eq!(rt.state.closed, 0, "nothing opened, so nothing closed");
}

#[test]
fn with_a_keyboard_opened_submenu_a_parent_row_still_runs_on_a_click() {
    let mut rt = opened();
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    assert_eq!(focused(&rt).as_deref(), Some("m-item6-sub-item0"));
    tap(&mut rt, "m-item7");
    assert_eq!(rt.state.heard, ["copy Some(4)"], "the parent's Copy runs");
    assert!(!rt.state.menu.is_open());
}

#[test]
fn with_a_keyboard_opened_submenu_a_press_outside_both_closes_the_whole_menu() {
    let mut rt = opened();
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    rt.pointer_down(W - 10.0, H - 10.0);
    step(&mut rt);
    rt.pointer_up(W - 10.0, H - 10.0);
    step(&mut rt);
    assert!(!rt.state.menu.is_open(), "the whole menu closed");
    assert_eq!(rt.state.dismissed, 1);
}

/// A parent about 200 pt wide whose submenu is wider, about 300 pt, with described rows.
fn wide_submenu() -> MenuModel {
    MenuModel::new().section(Section::new([
        Item::new("open", "Open the selected file"),
        Item::new("with", "Open With…").description("Looking for apps…"),
        Item::submenu(
            "send",
            "Send to",
            [Section::new([
                Item::new("a", "A compressed archive in the same folder as it"),
                Item::new("b", "Bluetooth device").description("Pair a device first, in Settings"),
            ])],
        ),
    ]))
}

fn open_wide(rt: &mut Runtime<Host>) -> (ElementInfo, ElementInfo) {
    rt.update(|s, _| {
        s.menu.open(
            MenuTarget::Background,
            Point::new(20.0, 20.0),
            wide_submenu(),
        );
        s.menu.open_submenu(&[2]);
    });
    step(rt);
    let top = live(rt, "m-menu").expect("the menu");
    let sub = live(rt, "m-item2-sub-menu").expect("the submenu");
    (top, sub)
}

#[test]
fn the_chooser_minimum_leaves_room_for_a_wider_submenu_on_either_side() {
    let mut rt = runtime();
    let (top, sub) = open_wide(&mut rt);
    let (tw, sw) = (top.rect[2], sub.rect[2]);
    assert!(sw > tw + 50.0, "the submenu is the wider: {tw} {sw}");
    let m = chooser_minimum(&[wide_submenu()], &rt.state.look);
    // The menu slides to stay in; the submenu needs its width free on one side of it.
    assert_eq!(m.width, tw + 2.0 * sw + 16.0);
}

#[test]
fn the_chooser_minimum_counts_described_rows_at_their_height() {
    let mut rt = runtime();
    let (top, sub) = open_wide(&mut rt);
    let m = chooser_minimum(&[wide_submenu()], &rt.state.look);
    assert_eq!(m.unscrolled_height, top.rect[3] + 16.0);
    assert_eq!(m.height, sub.rect[3] + 16.0);
}

#[test]
fn an_empty_fields_menu_has_select_all_insensitive() {
    let (mut rt, _) = with_clipboard(None);
    rt.update(|s, _| s.edit = TextEdit::new(""));
    step(&mut rt);
    secondary(&mut rt, "field");
    assert_eq!(enabled(&rt)[4], ("Select All".to_owned(), false));
}

#[test]
fn an_anchor_past_the_viewports_side_is_held_inside_it() {
    // Rows wider than the viewport (a scrolled List): the right edge is off to the side, so
    // the menu opens at the viewport's edge instead.
    let src = listing(20);
    let rows = UniformRows::new(20, 30.0, 500.0);
    let visible = Rect::new(0.0, 0.0, 300.0, 200.0);
    let mut sel = Selection::new();
    sel.select_only(&src, 1);
    let a = keyboard_anchor(
        &src,
        &sel,
        &rows,
        visible,
        Point::new(10.0, 0.0),
        AnchorEdge::Right,
    );
    assert_eq!(a.at, Point::new(310.0, 45.0));
}

#[test]
fn paste_into_a_field_rune_never_edited_keeps_one_line() {
    // The field was never focused or typed in. Rune's focus sync gives every mounted field its
    // line mode when the menu takes the keyboard, so `TextEdit::paste` drops the line break.
    let (mut rt, _) = with_clipboard(Some("two\nlines"));
    secondary(&mut rt, "field");
    tap(&mut rt, "f-item2");
    step(&mut rt);
    assert_eq!(rt.state.edit.text, "hello worldtwolines");
}

#[test]
fn a_held_menu_key_opens_one_menu() {
    // The menu takes the keyboard on the first press, so repeats reach it, not the collection;
    // but a host whose policy opens no menu for a target leaves the keyboard on the collection,
    // where a held key would ask again on every repeat.
    use crate::collection::element::{Ctx, Effect, KeyController, KeyInput};
    let src = listing(5);
    let rows = UniformRows::new(5, 30.0, 300.0);
    let mut sel = Selection::new();
    sel.select_only(&src, 1);
    let mut keys = MenuKeys::alone(AnchorEdge::Center);
    let ev = KeyEvent {
        key: Key::ContextMenu,
        mods: Mods::default(),
    };
    let asks = |repeat: bool, keys: &mut MenuKeys, sel: &mut Selection| {
        let mut input = KeyInput::new(&ev);
        input.repeat = repeat;
        let mut cx = Ctx::new(&src, &rows, Rect::new(0.0, 0.0, 300.0, 150.0), 0.0, sel);
        keys.key(&input, &mut cx);
        cx.effects()
            .iter()
            .filter(|e| matches!(e, Effect::Intent(_)))
            .count()
    };
    assert_eq!(asks(false, &mut keys, &mut sel), 1);
    assert_eq!(
        asks(true, &mut keys, &mut sel),
        0,
        "a repeat asks for nothing"
    );
}

#[test]
fn a_paste_lands_only_in_the_edit_it_was_asked_for() {
    // The field changed (a new rename in the same slot, say) while the clipboard read was out.
    let (mut rt, _) = with_clipboard(Some("pasted"));
    secondary(&mut rt, "field");
    let [x, y, w, h] = live(&rt, "f-item2").unwrap().rect;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.edit = TextEdit::new("another name"));
    step(&mut rt);
    assert_eq!(rt.state.edit.text, "another name");
}

#[test]
fn chords_in_a_field_follow_runes_rule_shift_menu_opens_super_menu_doesnt() {
    // A field's Menu key is Rune's (`context_menu_key`), which hands it to the field's own
    // `on_secondary_press`: plain or with Shift, never with Ctrl, Alt or Super.
    for (mods, opens) in [
        (
            Mods {
                shift: true,
                ..Mods::default()
            },
            true,
        ),
        (
            Mods {
                logo: true,
                ..Mods::default()
            },
            false,
        ),
        (
            Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            },
            false,
        ),
    ] {
        let (mut rt, _) = with_clipboard(Some("x"));
        rt.update(|s, _| s.edit.focused = true);
        step(&mut rt);
        press(&mut rt, Key::ContextMenu, mods);
        assert_eq!(rt.state.fields.is_open(), opens, "{mods:?}");
    }
}

#[test]
fn the_collections_menu_chords_that_open_nothing_stop_at_menu_keys() {
    // Passed on, Rune would hand them to the collection's own secondary press at its
    // bottom-left, which opens nothing only because that point is on the viewport's bottom
    // edge, outside it. Taken here, nothing behind them depends on that.
    use crate::collection::element::{Ctx, Flow, KeyController, KeyInput};
    let src = listing(5);
    let rows = UniformRows::new(5, 30.0, 300.0);
    let mut sel = Selection::new();
    sel.select_only(&src, 1);
    let mut keys = MenuKeys::alone(AnchorEdge::Center);
    for (key, mods) in [
        (
            Key::ContextMenu,
            Mods {
                shift: true,
                ..Mods::default()
            },
        ),
        (
            Key::ContextMenu,
            Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            },
        ),
        (
            Key::Function(10),
            Mods {
                shift: true,
                ctrl: true,
                cmd: true,
                ..Mods::default()
            },
        ),
    ] {
        let ev = KeyEvent {
            key: key.clone(),
            mods,
        };
        let input = KeyInput::new(&ev);
        let mut cx = Ctx::new(
            &src,
            &rows,
            Rect::new(0.0, 0.0, 300.0, 150.0),
            0.0,
            &mut sel,
        );
        assert_eq!(keys.key(&input, &mut cx), Flow::Handled, "{key:?} {mods:?}");
        assert!(cx.effects().is_empty(), "and opens nothing");
    }
}

#[test]
fn a_model_update_that_replaces_a_row_in_place_keeps_the_keyboard_on_its_action() {
    // browser/context-menu: "focus stays there while Open With lookup finishes". Rows are
    // keyed by position (U-C21-5), so this holds when the lookup replaces Open With… where it
    // was; a row added above the focused one gives the focus to another (see update_model).
    let mut rt = opened();
    key(&mut rt, Key::Down);
    assert_eq!(focused(&rt).as_deref(), Some("m-item6"));
    rt.update(|s, _| {
        let mut m = model();
        m.sections[0].items[1] = Item::new("with", "Open With…");
        s.menu.update_model(m);
    });
    step(&mut rt);
    let row = live(&rt, "m-item6").unwrap();
    assert_eq!((row.focused, row.label.as_deref()), (true, Some("Send to")));
    assert!(
        live(&rt, "m-item4").unwrap().focusable,
        "Open With… is sensitive now"
    );
}

// --- Review round 2 ---

/// Open, Send to ▸ (Desktop, More ▸ (X, Y), Mail), Copy: a submenu two levels in, its rows
/// sensitive when `enabled`.
///
/// | key | row |
/// | --- | --- |
/// | m-item0 | Open |
/// | m-item1 | Send to (m-item1-sub-item0 Desktop, -item1 More, -item2 Mail) |
/// | m-item1-sub-item1-sub-item0, -item1 | X, Y |
/// | m-item2 | Copy |
fn nested(enabled: bool) -> MenuModel {
    MenuModel::new().section(Section::new([
        Item::new("open", "Open"),
        Item::submenu(
            "send",
            "Send to",
            [Section::new([
                Item::new("desktop", "Desktop"),
                Item::submenu(
                    "more",
                    "More",
                    [Section::new([
                        Item::new("x", "X").enabled(enabled),
                        Item::new("y", "Y").enabled(enabled),
                    ])],
                ),
                Item::new("mail", "Mail"),
            ])],
        ),
        Item::new("copy", "Copy"),
    ]))
}

fn opened_with(m: MenuModel) -> Runtime<Host> {
    let mut rt = runtime();
    key(&mut rt, Key::Tab);
    rt.update(|s, _| {
        s.menu.open(
            MenuTarget::Entry(EntryId::new(4)),
            Point::new(60.0, 40.0),
            m,
        )
    });
    step(&mut rt);
    rt
}

#[test]
fn right_on_a_submenu_with_no_sensitive_row_opens_it_and_keeps_the_keyboard_on_its_row() {
    // SF-B: the Tab that moves the keyboard into a submenu would find no row there, and Rune's
    // Tab would leave the menu's chain for the top level's first row.
    let mut rt = opened_with(nested(false));
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    key(&mut rt, Key::Down);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("m-item1-sub-item1"),
        "on More"
    );
    key(&mut rt, Key::Right);
    assert_eq!(
        rt.state.menu.submenu(),
        [1, 1],
        "More's rows show, insensitive"
    );
    assert_eq!(
        focused(&rt).as_deref(),
        Some("m-item1-sub-item1"),
        "the keyboard stays on More"
    );
    key(&mut rt, Key::Down);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("m-item1-sub-item2"),
        "and moves on to Mail"
    );
}

#[test]
fn right_on_a_nested_submenu_with_sensitive_rows_moves_into_it() {
    let mut rt = opened_with(nested(true));
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    assert_eq!(focused(&rt).as_deref(), Some("m-item1-sub-item1-sub-item0"));
    key(&mut rt, Key::Left);
    assert_eq!(focused(&rt).as_deref(), Some("m-item1-sub-item1"));
    assert_eq!(rt.state.menu.submenu(), [1]);
}

#[test]
fn tab_inside_a_keyboard_opened_submenu_moves_as_down() {
    // The Tab the keys sent to enter the submenu is Rune's; the next is the menu's Down again.
    let mut rt = opened_with(nested(true));
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Right);
    assert_eq!(focused(&rt).as_deref(), Some("m-item1-sub-item0"));
    key(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some("m-item1-sub-item1"));
    key(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some("m-item1-sub-item2"));
}

#[test]
fn ctrl_tab_and_modified_arrows_leave_the_menu_alone() {
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    let mut rt = opened_with(nested(true));
    press(&mut rt, Key::Tab, ctrl);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("m-item0"),
        "Ctrl+Tab isn't a Down"
    );
    key(&mut rt, Key::Down);
    press(&mut rt, Key::Right, shift);
    press(&mut rt, Key::Enter, ctrl);
    assert!(
        rt.state.menu.submenu().is_empty(),
        "Shift+Right and Ctrl+Enter open nothing"
    );
    assert_eq!(focused(&rt).as_deref(), Some("m-item1"));
}

#[test]
fn the_chooser_minimum_counts_a_submenu_two_levels_in() {
    // More's rows are far wider than Send to's, so the window must fit Send to beside it.
    let model = MenuModel::new().section(Section::new([
        Item::new("open", "Open the selected file and more"),
        Item::submenu(
            "send",
            "Send to",
            [Section::new([
                Item::new("d", "Desktop"),
                Item::submenu(
                    "more",
                    "More",
                    [Section::new([
                        Item::new("x", "A compressed archive in the same folder as the file"),
                        Item::new("y", "Y").description("Pair a device first"),
                        Item::new("z", "Z"),
                        Item::new("w", "W"),
                    ])],
                ),
            ])],
        ),
    ]));
    let mut rt = runtime();
    rt.update(|s, _| {
        s.menu.open(
            MenuTarget::Background,
            Point::new(10.0, 10.0),
            model.clone(),
        );
        s.menu.open_submenu(&[1, 1]);
    });
    step(&mut rt);
    let w = |k: &str| live(&rt, k).unwrap_or_else(|| panic!("no {k}")).rect;
    let (top, sub, sub2) = (
        w("m-menu"),
        w("m-item1-sub-menu"),
        w("m-item1-sub-item1-sub-menu"),
    );
    assert!(
        top[2] > sub[2] + 20.0 && sub2[2] > top[2],
        "{top:?} {sub:?} {sub2:?}"
    );
    let m = chooser_minimum(&[model], &rt.state.look);
    assert_eq!(m.width, sub[2] + 2.0 * sub2[2] + 16.0);
    assert_eq!(m.height, sub2[3] + 16.0, "the deeper submenu is the taller");
}

#[test]
fn a_hosts_own_paste_gets_the_text_instead_of_the_field() {
    let (mut rt, _) = with_clipboard(Some("pasted"));
    rt.update(|s, _| s.own_paste = Some(Vec::new()));
    step(&mut rt);
    secondary(&mut rt, "field");
    tap(&mut rt, "f-item2");
    step(&mut rt);
    assert_eq!(
        rt.state.own_paste.as_deref(),
        Some(&["pasted".to_owned()][..])
    );
    assert_eq!(
        rt.state.edit.text, "hello world",
        "the field itself is untouched"
    );
    assert_eq!(rt.state.changed, 0, "on_change is the host's to call");
}

#[test]
fn a_read_only_fields_menu_only_copies_and_selects() {
    let (mut rt, _) = with_clipboard(Some("x"));
    rt.update(|s, _| {
        s.read_only = true;
        s.edit.set_selection(0, 5);
    });
    step(&mut rt);
    secondary(&mut rt, "field");
    assert_eq!(
        enabled(&rt),
        names(&[
            ("Cut", false),
            ("Copy", true),
            ("Paste", false),
            ("Delete", false),
            ("Select All", true),
        ])
    );
}

#[test]
fn a_clipboard_read_for_an_earlier_menu_is_dropped() {
    // The first menu's read is out when a second menu opens (the host opened it without a
    // read, as a scene does): the empty answer is the first menu's, not this one's.
    let (mut rt, _) = with_clipboard(None);
    rt.update(|s, tx| {
        fields(s).open(s, 0, Point::new(10.0, 10.0), tx);
        s.fields.open(1, Point::new(10.0, 40.0));
    });
    step(&mut rt);
    assert_eq!(rt.state.fields.field(), Some(1));
    assert_eq!(rt.state.fields.clipboard(), Clipboard::Unknown);
}

#[test]
fn a_right_click_outside_an_open_menu_leaves_it_open_on_its_target() {
    // Bullet 184 is not met: the scrim takes no secondary press (U-C21-6), so the menu
    // neither retargets nor closes.
    let mut rt = opened();
    rt.secondary_down(W - 10.0, H - 10.0);
    step(&mut rt);
    rt.secondary_up(W - 10.0, H - 10.0);
    step(&mut rt);
    assert!(rt.state.menu.is_open());
    assert_eq!(
        rt.state.menu.target(),
        Some(&MenuTarget::Entry(EntryId::new(4)))
    );
    assert_eq!(rt.state.dismissed, 0);
}

#[test]
fn a_checked_row_takes_the_menus_first_focus() {
    let m = MenuModel::new().section(Section::new([
        Item::new("a", "Name").checked(false),
        Item::new("b", "Size").checked(true),
        Item::new("c", "Date").checked(false),
    ]));
    let rt = opened_with(m);
    assert_eq!(focused(&rt).as_deref(), Some("m-item1"));
}
