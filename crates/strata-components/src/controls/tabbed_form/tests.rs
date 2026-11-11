use rune::{
    Cx, El, ElementInfo, Env, Key, KeyEvent, Mods, Runtime, ScrollSpec, TextEdit, Tx, col, el, row,
    text, text_input,
};
use strata_kit::look::Look;
use strata_kit::shape::LookParams;

use super::{FormPage, Mounting, TabbedForm, TabbedFormState};

const FRAME: f64 = 1.0 / 60.0;

struct Host {
    tabs: TabbedFormState,
    mounting: Mounting,
    name: TextEdit,
    look: Look,
    heard: Vec<usize>,
    /// Ask for the error from the next poll, before the form's tick runs.
    error_from_poll: bool,
}

fn form(h: &Host) -> TabbedForm<Host> {
    TabbedForm::new("f", |h: &Host| &h.tabs, |h: &mut Host| &mut h.tabs)
        .mounting(h.mounting)
        .on_select(|h: &mut Host, i, _| h.heard.push(i))
}

fn pages(h: &Host) -> Vec<FormPage> {
    vec![
        FormPage::new("general", "General").errors(h.name.text.is_empty()),
        FormPage::new("script", "Script"),
        FormPage::new("behavior", "Behavior"),
    ]
}

/// Page 0 holds the name field; page 1 a tall scrolled list; page 2 a button.
fn body(h: &Host, i: usize) -> El {
    match i {
        0 => col().child(
            text_input(&h.name, h.look.kit.ty.ui)
                .single_line()
                .key("name")
                .label("Name")
                .w(200.0)
                .h(24.0)
                .on_edit(|h: &mut Host| &mut h.name),
        ),
        1 => {
            col()
                .key("script-scroll")
                .h(100.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(col().key("script-lines").children(
                    (0..40).map(|r| el().h(20.0).w_full().child(text(format!("line {r}")))),
                ))
        }
        _ => col().child(
            el().key("behavior-button")
                .size(40.0, 20.0)
                .focusable(true)
                .on_press(|_: &mut Host| {}),
        ),
    }
}

fn view(h: &Host, _: &mut Cx<Host>) -> El {
    let f = form(h);
    let save = el()
        .key("save")
        .size(60.0, 20.0)
        .focusable(true)
        .on_press_tx(|h: &mut Host, tx: &mut Tx| {
            if h.name.text.is_empty() {
                h.tabs.show_error(0, "name", tx);
            }
        });
    col()
        .key("dialog")
        .child(f.build(h, &h.look, &pages(h), body))
        .child(row().key("footer").child(save))
}

fn poll(h: &mut Host, _: f64, tx: &mut Tx) -> bool {
    let asked = std::mem::take(&mut h.error_from_poll);
    if asked {
        h.tabs.show_error(0, "name", tx);
    }
    h.tabs.tick(tx) || asked
}

fn on_key(h: &mut Host, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let n = pages(h).len();
    form(h).window_key(h, n, ev, tx)
}

fn mount(mounting: Mounting, name: &str) -> Runtime<Host> {
    mount_look(mounting, name, Look::fallback(false))
}

fn mount_look(mounting: Mounting, name: &str, look: Look) -> Runtime<Host> {
    let host = Host {
        tabs: TabbedFormState::new(),
        mounting,
        name: TextEdit::new(name),
        look,
        heard: Vec::new(),
        error_from_poll: false,
    };
    // Rune's own font: a text system built here would move the fonts the crate's other tests
    // measure with (docs/testing.md, R-K6-1).
    let mut rt = Runtime::new(host, view, (500.0, 400.0), Env::new(false, 1.0));
    rt.poll = Some(poll);
    rt.on_key = Some(on_key);
    rt.frame(0.0);
    run(&mut rt, 3);
    rt
}

fn run(rt: &mut Runtime<Host>, frames: usize) {
    for _ in 0..frames {
        rt.frame(FRAME);
    }
}

fn info(rt: &Runtime<Host>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
}

fn focused(rt: &Runtime<Host>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn tap(rt: &mut Runtime<Host>, key: &str) {
    let r = info(rt, key).unwrap_or_else(|| panic!("no {key}")).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    run(rt, 3);
}

fn press(rt: &mut Runtime<Host>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    run(rt, 3);
}

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

/// The script page's offset: what its lines are scrolled by.
fn script_offset(rt: &Runtime<Host>) -> f32 {
    info(rt, "script-lines").map_or(0.0, |e| e.scrolled[1])
}

fn scroll_script(rt: &mut Runtime<Host>) {
    let r = info(rt, "script-scroll").expect("the script page").rect;
    rt.pointer_move(r[0] + 10.0, r[1] + 10.0);
    run(rt, 2);
    rt.wheel(r[0] + 10.0, r[1] + 10.0, -200.0);
    run(rt, 60);
}

#[test]
fn ctrl_page_keys_choose_the_next_and_previous_page_wrapping() {
    let mut rt = mount(Mounting::Kept, "x");
    press(&mut rt, Key::PageUp, ctrl());
    assert_eq!(
        rt.state.tabs.selected(),
        2,
        "Ctrl+Page Up from the first wraps to the last"
    );
    press(&mut rt, Key::PageDown, ctrl());
    assert_eq!(
        rt.state.tabs.selected(),
        0,
        "Ctrl+Page Down from the last wraps to the first"
    );
    press(&mut rt, Key::PageDown, ctrl());
    assert_eq!(rt.state.tabs.selected(), 1);
    for mods in [
        Mods::default(),
        Mods {
            shift: true,
            ..ctrl()
        },
        Mods {
            alt: true,
            ..ctrl()
        },
        Mods {
            logo: true,
            ..ctrl()
        },
    ] {
        press(&mut rt, Key::PageDown, mods);
    }
    assert_eq!(
        rt.state.tabs.selected(),
        1,
        "only plain Ctrl switches pages"
    );
    assert_eq!(rt.state.heard, [2, 0, 1], "on_select heard each change");
}

#[test]
fn a_kept_page_keeps_its_scroll_offset_and_a_rebuilt_one_starts_at_the_top() {
    for (mounting, kept) in [(Mounting::Kept, true), (Mounting::Rebuilt, false)] {
        let mut rt = mount(mounting, "x");
        tap(&mut rt, "f-tab-1");
        scroll_script(&mut rt);
        let scrolled = script_offset(&rt);
        assert!(scrolled > 0.0, "{mounting:?}: the wheel scrolled the page");
        tap(&mut rt, "f-tab-2");
        run(&mut rt, 30);
        tap(&mut rt, "f-tab-1");
        let back = script_offset(&rt);
        if kept {
            assert_eq!(
                back, scrolled,
                "{mounting:?}: the offset survives the switch"
            );
        } else {
            assert_eq!(
                back, 0.0,
                "{mounting:?}: the page starts afresh (was {scrolled})"
            );
        }
    }
}

#[test]
fn a_hidden_kept_page_takes_neither_focus_nor_the_tree() {
    let mut rt = mount(Mounting::Kept, "x");
    assert!(
        info(&rt, "behavior-button").is_some(),
        "the hidden page is built"
    );
    let tree = rt.accessibility_json().to_string();
    assert!(
        !tree.contains("behavior-button") && !tree.contains("script-scroll"),
        "but out of the tree"
    );
    tap(&mut rt, "f-tab-0");
    let mut seen = Vec::new();
    for _ in 0..4 {
        press(&mut rt, Key::Tab, Mods::default());
        seen.push(focused(&rt));
    }
    assert!(
        !seen.iter().flatten().any(|k| k == "behavior-button"),
        "Tab never reaches a hidden page: {seen:?}"
    );
    assert!(
        seen.iter().flatten().any(|k| k == "name"),
        "it reaches the shown one: {seen:?}"
    );
}

#[test]
fn a_failed_save_shows_the_invalid_fields_page_and_moves_the_keyboard_to_it() {
    for mounting in [Mounting::Kept, Mounting::Rebuilt] {
        let mut rt = mount(mounting, "");
        tap(&mut rt, "f-tab-2");
        assert_ne!(focused(&rt).as_deref(), Some("name"));
        tap(&mut rt, "save");
        run(&mut rt, 3);
        assert_eq!(rt.state.tabs.selected(), 0, "{mounting:?}: General shows");
        assert_eq!(
            focused(&rt).as_deref(),
            Some("name"),
            "{mounting:?}: and the name field has the keyboard"
        );
    }
}

#[test]
fn arrows_choose_as_they_move_and_wrap() {
    let mut rt = mount(Mounting::Kept, "x");
    tap(&mut rt, "f-tab-0");
    let mut seen = Vec::new();
    for key in [
        Key::Right,
        Key::Right,
        Key::Right,
        Key::Left,
        Key::End,
        Key::Home,
    ] {
        press(&mut rt, key, Mods::default());
        seen.push((focused(&rt), rt.state.tabs.selected()));
    }
    let want = [1, 2, 0, 2, 2, 0].map(|i| (Some(format!("f-tab-{i}")), i));
    assert_eq!(seen, want);
}

/// The keyboard is on the page being hidden, and the host asks from its poll: the page
/// changes and the keyboard still moves.
#[test]
fn an_error_asked_for_from_inside_the_hidden_page_still_moves_the_keyboard() {
    let mut rt = mount(Mounting::Kept, "");
    tap(&mut rt, "f-tab-2");
    tap(&mut rt, "save");
    run(&mut rt, 3);
    tap(&mut rt, "f-tab-2");
    press(&mut rt, Key::Tab, Mods::default());
    assert_eq!(focused(&rt).as_deref(), Some("behavior-button"));
    rt.state.error_from_poll = true;
    run(&mut rt, 4);
    assert_eq!(focused(&rt).as_deref(), Some("name"));
}

/// GtkNotebook's `real_switch_page`: Ctrl+Page Up and Down with the keyboard on the page being
/// hidden carry it into the new page (its first focusable element), else to the new page's
/// tab; with the keyboard on a tab it stays on the tabs, on the new one; elsewhere it stays.
#[test]
fn ctrl_page_keys_carry_the_keyboard_out_of_the_hidden_page() {
    for mounting in [Mounting::Kept, Mounting::Rebuilt] {
        let mut rt = mount(mounting, "x");
        tap(&mut rt, "name");
        assert_eq!(focused(&rt).as_deref(), Some("name"));
        let mut seen = Vec::new();
        for key in [
            Key::PageUp,
            Key::PageDown,
            Key::PageDown,
            Key::PageDown,
            Key::PageUp,
        ] {
            press(&mut rt, key, ctrl());
            seen.push((rt.state.tabs.selected(), focused(&rt)));
        }
        let want = [
            (2, "behavior-button"),
            (0, "name"),
            (1, "f-tab-1"),
            (2, "f-tab-2"),
            (1, "f-tab-1"),
        ]
        .map(|(i, k)| (i, Some(k.to_owned())));
        assert_eq!(seen, want, "{mounting:?}");
        tap(&mut rt, "save");
        press(&mut rt, Key::PageDown, ctrl());
        assert_eq!(rt.state.tabs.selected(), 2, "{mounting:?}");
        assert_eq!(
            focused(&rt).as_deref(),
            Some("save"),
            "{mounting:?}: a keyboard outside the form stays"
        );
        assert_eq!(rt.state.heard, [2, 0, 1, 2, 1, 2], "{mounting:?}");
    }
}

/// Strata's tabs are at least 36 px tall (`min-height`): at a large text size a tab grows to
/// hold its label rather than clipping it.
#[test]
fn a_tab_grows_to_hold_a_large_label() {
    let big = Look::fallback(false).with_params(&LookParams {
        text_size: 48,
        ..LookParams::default()
    });
    let rt = mount_look(Mounting::Kept, "x", big);
    let tab = info(&rt, "f-tab-0").expect("the tab");
    let label = rt
        .elements()
        .into_iter()
        .find(|e| e.text.as_deref() == Some("General") && e.path.starts_with(&tab.path))
        .expect("the tab's label")
        .rect;
    let tab = tab.rect;
    let (top, bottom) = (tab[1], tab[1] + tab[3]);
    assert!(
        label[1] >= top && label[1] + label[3] <= bottom,
        "the label {label:?} fits in the tab {tab:?}"
    );
}
