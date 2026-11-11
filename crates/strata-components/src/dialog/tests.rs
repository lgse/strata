use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, TextEdit, col, text_input};
use rune_kit::{Button, Kit};

use super::keys::{Route, route};
use super::text::korean_word_ranges;
use super::{
    Measure, fit, keep_words_whole, limit_on_change, remaining, text_area, trim_insertion,
    trim_to_fit,
};

fn key(k: Key) -> KeyEvent {
    KeyEvent {
        key: k,
        mods: Mods::default(),
    }
}

fn footer() -> Vec<String> {
    ["d-skip", "d-secondary", "d-primary"]
        .map(String::from)
        .to_vec()
}

#[test]
fn enter_presses_the_focused_control_and_leaves_the_rest_to_the_default_button() {
    let f = footer();
    let enter = key(Key::Enter);
    assert_eq!(
        route(&enter, false, Some("d-secondary"), "d-dialog", &f),
        Route::Press,
        "Enter on Cancel presses Cancel"
    );
    assert_eq!(
        route(&enter, false, Some("remember"), "d-dialog", &f),
        Route::Press,
        "Enter on a body checkbox toggles it"
    );
    assert_eq!(route(&enter, false, None, "d-dialog", &f), Route::Pass);
    assert_eq!(
        route(&enter, false, Some("d-dialog"), "d-dialog", &f),
        Route::Pass
    );
    assert_eq!(
        route(&enter, true, Some("d-primary"), "d-dialog", &f),
        Route::Consume,
        "a held Return presses once"
    );
    let shifted = KeyEvent {
        key: Key::Enter,
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
    };
    assert_eq!(
        route(&shifted, false, Some("d-secondary"), "d-dialog", &f),
        Route::Pass
    );
}

#[test]
fn left_and_right_move_along_the_footer_and_stop_at_its_ends() {
    let f = footer();
    let at = |k: Key, focused: &str| route(&key(k), false, Some(focused), "d-dialog", &f);
    assert_eq!(at(Key::Left, "d-skip"), Route::Consume);
    assert_eq!(at(Key::Right, "d-skip"), Route::Focus(1));
    assert_eq!(at(Key::Left, "d-primary"), Route::Focus(1));
    assert_eq!(at(Key::Right, "d-primary"), Route::Consume);
    assert_eq!(
        at(Key::Left, "name-field"),
        Route::Pass,
        "arrows off the footer stay Rune's"
    );
}

/// A dialog's multi-line field, limited to `room` characters, beside a default button.
#[derive(Default)]
struct Notes {
    notes: TextEdit,
    room: usize,
    submits: u32,
}

fn notes_view(s: &Notes, _cx: &mut Cx<Notes>) -> El {
    let k = Kit::neutral(false);
    col()
        .child(
            text_area(&s.notes, |s: &mut Notes| &mut s.notes)
                .label("Notes")
                .on_change(limit_on_change(
                    |s: &mut Notes| &mut s.notes,
                    |s: &Notes| (s.room, Measure::Chars),
                ))
                .build(&k)
                .key("notes"),
        )
        .child(
            Button::new("OK")
                .on_press(|s: &mut Notes| s.submits += 1)
                .build(&k)
                .key("ok")
                .default_button(),
        )
}

fn type_notes(room: usize, keys: &[Key]) -> Notes {
    let mut notes = TextEdit::new("");
    notes.focused = true;
    let state = Notes {
        notes,
        room,
        submits: 0,
    };
    let mut rt = Runtime::new(state, notes_view, (400.0, 300.0), Env::new(false, 1.0));
    rt.frame(0.0);
    for k in keys {
        rt.key(key(k.clone()));
        // A sent key lands on the next frame.
        rt.frame(1.0 / 60.0);
        rt.frame(1.0 / 60.0);
    }
    let Runtime { state, .. } = rt;
    state
}

#[test]
fn return_in_a_dialog_text_area_types_a_newline_through_the_field() {
    let s = type_notes(
        10,
        &[Key::Text("a".into()), Key::Enter, Key::Text("b".into())],
    );
    assert_eq!(s.notes.text, "a\nb");
    assert_eq!(s.submits, 0, "Return doesn't press the default button");

    // The newline goes through the field's on_change, so its limit holds.
    let s = type_notes(3, &[Key::Text("abc".into()), Key::Enter, Key::Enter]);
    assert_eq!(s.notes.text, "abc");
    assert_eq!(s.submits, 0);
}

#[test]
fn a_limit_counts_in_its_unit_and_cuts_on_a_whole_character() {
    assert_eq!(remaining("ÉTÉ-2026", 11, Measure::Bytes), 1);
    assert_eq!(remaining("ÉTÉ-2026", 11, Measure::Chars), 3);
    assert_eq!(remaining("𝄞a", 4, Measure::Utf16), 1);
    assert_eq!(fit("aÉb", 2, Measure::Bytes), "a", "É doesn't split");
    assert_eq!(fit("aÉb", 3, Measure::Bytes), "aÉ");
    assert_eq!(fit("𝄞𝄞", 3, Measure::Utf16), "𝄞");
    assert_eq!(fit("abc", 9, Measure::Chars), "abc");
}

#[test]
fn an_insertion_keeps_its_longest_leading_part_that_fits() {
    // "ab|cd" with "ÉÉÉ" pasted at the caret, under a 7-byte limit: one É fits.
    let mut e = TextEdit::new("abÉÉÉcd");
    e.set_caret(8);
    trim_insertion(&mut e, 7, Measure::Bytes);
    assert_eq!(e.text, "abÉcd");

    let mut e = TextEdit::new("abcde");
    e.set_caret(5);
    trim_insertion(&mut e, 3, Measure::Chars);
    assert_eq!(
        e.text, "abc",
        "a limit lowered below the text cuts it to fit"
    );

    let mut e = TextEdit::new("ÉTÉ-2026ab");
    trim_to_fit(&mut e, 11, Measure::Bytes);
    assert_eq!(
        e.text, "ÉTÉ-2026a",
        "switching the unit to bytes cuts the tail"
    );

    let mut e = TextEdit::new("hi");
    e.set_caret(2);
    trim_insertion(&mut e, 5, Measure::Chars);
    assert_eq!(e.text, "hi");
}

#[test]
fn short_korean_words_are_kept_whole_and_long_runs_stay_breakable() {
    let text = "다음 중 하나를 설치하세요: readonly에 a b";
    let words: Vec<&str> = korean_word_ranges(text)
        .into_iter()
        .map(|r| &text[r])
        .collect();
    assert_eq!(words, ["다음", "하나를", "설치하세요:", "readonly에"]);
    assert!(korean_word_ranges(&"가나다".repeat(5)).is_empty());
    assert!(korean_word_ranges("Cannot create “readonly”").is_empty());

    let long = "переноса_строк.txt을(를) /home/user/fx/readonly에서: archive.tar.gz.";
    let words: Vec<&str> = korean_word_ranges(long)
        .into_iter()
        .map(|r| &long[r])
        .collect();
    assert_eq!(words, ["t을(를)", "y에서:"]);

    let joined = keep_words_whole("다음 중 하나를");
    assert_eq!(joined, "다\u{2060}음 중 하\u{2060}나\u{2060}를");
    assert_eq!(joined.replace('\u{2060}', ""), "다음 중 하나를");
    assert_eq!(keep_words_whole("Delete 3 items?"), "Delete 3 items?");
}

#[derive(Default)]
struct Closable {
    closable: bool,
    dismissed: u32,
}

fn closable_view(s: &Closable, _cx: &mut Cx<Closable>) -> El {
    let look = strata_kit::look::Look::fallback(false);
    let mut layer = Vec::new();
    super::StrataDialog::new("d", "Copying items", |s: &mut Closable| s.dismissed += 1)
        .open(true)
        .closable(s.closable)
        .secondary("Cancel", |_: &mut Closable| {})
        .build(&look, &mut layer);
    rune_kit::root(col(), layer)
}

#[test]
fn a_dialog_that_is_not_closable_has_no_close_button_and_escape_still_dismisses_it() {
    for closable in [true, false] {
        let state = Closable {
            closable,
            dismissed: 0,
        };
        let mut rt = Runtime::new(state, closable_view, (600.0, 400.0), Env::new(false, 1.0));
        rt.frame(0.0);
        rt.frame(0.3);
        let keys: Vec<String> = rt.elements().into_iter().filter_map(|e| e.key).collect();
        assert_eq!(
            keys.iter().any(|k| k == "d-close"),
            closable,
            "closable({closable})"
        );
        assert!(keys.iter().any(|k| k == "d-secondary"));
        rt.key(key(Key::Escape));
        rt.frame(1.0 / 60.0);
        assert_eq!(
            rt.state.dismissed, 1,
            "Escape dismisses, closable({closable})"
        );
    }
}

/// A form whose single-line field has focus, over a primary that is or isn't the default button.
struct Form {
    name: TextEdit,
    default_button: bool,
    pressed: u32,
}

fn form_view(s: &Form, _cx: &mut Cx<Form>) -> El {
    let look = strata_kit::look::Look::fallback(false);
    let mut layer = Vec::new();
    super::StrataDialog::new("d", "Trust this host?", |_: &mut Form| {})
        .open(true)
        .child(
            text_input(&s.name, look.kit.ty.ui)
                .single_line()
                .key("name")
                .label("Name")
                .on_edit(|s: &mut Form| &mut s.name),
        )
        .secondary("Cancel", |_: &mut Form| {})
        .primary("Log In Anyway", |s: &mut Form| s.pressed += 1)
        .default_button(s.default_button)
        .build(&look, &mut layer);
    rune_kit::root(col(), layer)
}

#[test]
fn enter_in_a_field_presses_the_primary_only_while_it_is_the_default_button() {
    for default_button in [true, false] {
        let mut name = TextEdit::new("");
        name.focused = true;
        let state = Form {
            name,
            default_button,
            pressed: 0,
        };
        let mut rt = Runtime::new(state, form_view, (600.0, 400.0), Env::new(false, 1.0));
        rt.frame(0.0);
        rt.frame(0.3);
        assert!(rt.state.name.focused, "the field has the keyboard");
        rt.key(key(Key::Enter));
        rt.frame(1.0 / 60.0);
        rt.frame(1.0 / 60.0);
        assert_eq!(
            rt.state.pressed,
            u32::from(default_button),
            "default_button({default_button})"
        );
    }
}

/// A question whose footer buttons both commit, opening on the close button unless `initial`
/// says otherwise.
#[derive(Default)]
struct Commit {
    dismissed: u32,
    pressed: Vec<&'static str>,
    initial: Option<super::Focus>,
}

fn commit_view(s: &Commit, _cx: &mut Cx<Commit>) -> El {
    let look = strata_kit::look::Look::fallback(false);
    let mut layer = Vec::new();
    super::StrataDialog::new("d", "Trust this host?", |s: &mut Commit| s.dismissed += 1)
        .open(true)
        .message("The identity sent is SHA256:abc.")
        .action("other", "Log In Once", |s: &mut Commit| {
            s.pressed.push("other")
        })
        .primary("Log In Anyway", |s: &mut Commit| s.pressed.push("primary"))
        .default_button(false)
        .initial_focus(s.initial.clone().unwrap_or(super::Focus::Close))
        .build(&look, &mut layer);
    rune_kit::root(col(), layer)
}

#[test]
fn a_dialog_opening_on_close_dismisses_on_enter_and_tabs_on_in_document_order() {
    let focused = |rt: &Runtime<Commit>| {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    };
    let open = || {
        let mut rt = Runtime::new(
            Commit::default(),
            commit_view,
            (600.0, 400.0),
            Env::new(false, 1.0),
        );
        rt.frame(0.0);
        rt.frame(0.3);
        rt
    };
    let mut rt = open();
    assert_eq!(focused(&rt).as_deref(), Some("d-close"));
    rt.key(key(Key::Enter));
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.dismissed, 1, "Enter on close dismisses");
    assert!(rt.state.pressed.is_empty(), "{:?}", rt.state.pressed);

    let mut rt = open();
    let mut cycle = Vec::new();
    for _ in 0..3 {
        rt.key(key(Key::Tab));
        rt.frame(1.0 / 60.0);
        cycle.extend(focused(&rt));
    }
    assert_eq!(cycle, ["d-other", "d-primary", "d-close"]);
}

/// The initial button takes focus as the dialog opens, and Tab cycles in document order from
/// there: the footer (Log In Once, Log In Anyway), then the close button.
#[test]
fn the_initial_button_opens_focused_and_tab_keeps_document_order() {
    let focused = |rt: &Runtime<Commit>| {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    };
    for (initial, want) in [
        (
            super::Focus::Primary,
            ["d-primary", "d-close", "d-other", "d-primary"],
        ),
        (
            super::Focus::Action("other".into()),
            ["d-other", "d-primary", "d-close", "d-other"],
        ),
    ] {
        let state = Commit {
            initial: Some(initial.clone()),
            ..Commit::default()
        };
        let mut rt = Runtime::new(state, commit_view, (600.0, 400.0), Env::new(false, 1.0));
        rt.frame(0.0);
        rt.frame(0.3);
        let mut seen = vec![focused(&rt).unwrap_or_default()];
        for _ in 0..3 {
            rt.key(key(Key::Tab));
            rt.frame(1.0 / 60.0);
            seen.push(focused(&rt).unwrap_or_default());
        }
        assert_eq!(seen, want, "{initial:?}");
    }
}
