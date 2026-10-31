// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType as Modifiers};

use super::{Binding, Digits, Event, KeyPress, normalize};

const NONE: Modifiers = Modifiers::empty();
const SHIFT: Modifiers = Modifiers::SHIFT_MASK;
const CTRL: Modifiers = Modifiers::CONTROL_MASK;
const CTRL_SHIFT: Modifiers = CTRL.union(SHIFT);
const CAPS: Modifiers = Modifiers::LOCK_MASK;

/// A layout's level 0 and 1 characters by GDK keycode, as `Display::map_keycode`
/// reports them (rows recorded from GDK 4.22 on Xvfb with xkeyboard-config).
type Layout = &'static [(u32, [Key; 2])];

const US: Layout = &[
    (10, [Key::_1, Key::exclam]),
    (11, [Key::_2, Key::at]),
    (38, [Key::a, Key::A]),
    (44, [Key::j, Key::J]),
    (54, [Key::c, Key::C]),
    (60, [Key::period, Key::greater]),
    (61, [Key::slash, Key::question]),
];
const RU: Layout = &[
    (11, [Key::_2, Key::quotedbl]),
    (29, [Key::Cyrillic_en, Key::Cyrillic_EN]),
    (38, [Key::Cyrillic_ef, Key::Cyrillic_EF]),
    (44, [Key::Cyrillic_o, Key::Cyrillic_O]),
    (45, [Key::Cyrillic_el, Key::Cyrillic_EL]),
    (54, [Key::Cyrillic_es, Key::Cyrillic_ES]),
    (60, [Key::Cyrillic_yu, Key::Cyrillic_YU]),
    (61, [Key::period, Key::comma]),
];
const UK: Layout = &[
    (39, [Key::Ukrainian_i, Key::Ukrainian_I]),
    (54, [Key::Cyrillic_es, Key::Cyrillic_ES]),
];
const EL: Layout = &[
    (24, [Key::semicolon, Key::colon]),
    (25, [Key::Greek_finalsmallsigma, Key::Greek_SIGMA]),
    (54, [Key::Greek_psi, Key::Greek_PSI]),
];
const HE: Layout = &[(24, [Key::slash, Key::Q]), (54, [Key::hebrew_bet, Key::C])];
const FR: Layout = &[
    (10, [Key::ampersand, Key::_1]),
    (11, [Key::eacute, Key::_2]),
    (15, [Key::minus, Key::_6]),
    (19, [Key::agrave, Key::_0]),
    (24, [Key::a, Key::A]),
    (38, [Key::q, Key::Q]),
    (59, [Key::semicolon, Key::period]),
];
const DE: Layout = &[
    (11, [Key::_2, Key::quotedbl]),
    (20, [Key::ssharp, Key::question]),
    (29, [Key::z, Key::Z]),
    (52, [Key::y, Key::Y]),
];
const DVORAK: Layout = &[(54, [Key::j, Key::J])];
const LT: Layout = &[(10, [Key::aogonek, Key::Aogonek])];
const CZ: Layout = &[(10, [Key::plus, Key::_1])];
const HU: Layout = &[
    (19, [Key::odiaeresis, Key::Odiaeresis]),
    (49, [Key::_0, Key::section]),
];

struct Press {
    layouts: &'static [Layout],
    layout: u32,
    keycode: u32,
    level: u32,
    typed: Key,
    matched: (Key, Modifiers),
    state: Modifiers,
}

impl Press {
    fn read(&self) -> KeyPress {
        let event = Event {
            matched: self.matched,
            typed: self.typed,
            state: self.state,
            keycode: self.keycode,
            layout: self.layout,
            level: self.level,
            modifier: false,
        };
        normalize(&event, |keycode| {
            self.layouts
                .iter()
                .enumerate()
                .flat_map(|(layout, rows)| {
                    rows.iter()
                        .filter(move |(code, _)| *code == keycode)
                        .flat_map(move |(_, levels)| {
                            levels
                                .iter()
                                .enumerate()
                                .map(move |(level, key)| (layout as u32, level as u32, *key))
                        })
                })
                .collect::<Vec<_>>()
        })
    }
}

fn press(
    layouts: &'static [Layout],
    layout: u32,
    keycode: u32,
    level: u32,
    typed: Key,
    matched: (Key, Modifiers),
    state: Modifiers,
) -> KeyPress {
    Press {
        layouts,
        layout,
        keycode,
        level,
        typed,
        matched,
        state,
    }
    .read()
}

#[test]
fn letters_follow_the_label_then_the_physical_key() {
    let cases = [
        (
            "us Ctrl+C",
            press(&[US], 0, 54, 0, Key::c, (Key::c, CTRL), CTRL),
            Binding::ctrl(Key::c),
        ),
        (
            "us,ru Ctrl+С",
            press(
                &[US, RU],
                1,
                54,
                0,
                Key::Cyrillic_es,
                (Key::Cyrillic_es, CTRL),
                CTRL,
            ),
            Binding::ctrl(Key::c),
        ),
        (
            "ru-only Ctrl+С uses US-QWERTY",
            press(
                &[RU],
                0,
                54,
                0,
                Key::Cyrillic_es,
                (Key::Cyrillic_es, CTRL),
                CTRL,
            ),
            Binding::ctrl(Key::c),
        ),
        (
            "ru-only Ctrl+Л is Ctrl+K",
            press(
                &[RU],
                0,
                45,
                0,
                Key::Cyrillic_el,
                (Key::Cyrillic_el, CTRL),
                CTRL,
            ),
            Binding::ctrl(Key::k),
        ),
        (
            "us,ru о is j",
            press(
                &[US, RU],
                1,
                44,
                0,
                Key::Cyrillic_o,
                (Key::Cyrillic_o, NONE),
                NONE,
            ),
            Binding::plain(Key::j),
        ),
        (
            "us,ru Shift+О is Shift+j",
            press(
                &[US, RU],
                1,
                44,
                1,
                Key::Cyrillic_O,
                (Key::Cyrillic_o, SHIFT),
                SHIFT,
            ),
            Binding::shift(Key::j),
        ),
        (
            "uk,us і is s",
            press(
                &[UK, US],
                0,
                39,
                0,
                Key::Ukrainian_i,
                (Key::Ukrainian_i, CTRL),
                CTRL,
            ),
            Binding::ctrl(Key::s),
        ),
        (
            "el,us ψ is c",
            press(
                &[EL, US],
                0,
                54,
                0,
                Key::Greek_psi,
                (Key::Greek_psi, CTRL),
                CTRL,
            ),
            Binding::ctrl(Key::c),
        ),
        (
            "el,us ς is w",
            press(
                &[EL, US],
                0,
                25,
                0,
                Key::Greek_finalsmallsigma,
                (Key::Greek_finalsmallsigma, NONE),
                NONE,
            ),
            Binding::plain(Key::w),
        ),
        (
            "he,us ב is c",
            press(
                &[HE, US],
                0,
                54,
                0,
                Key::hebrew_bet,
                (Key::hebrew_bet, CTRL),
                CTRL,
            ),
            Binding::ctrl(Key::c),
        ),
        (
            "he Shift types Latin C",
            press(
                &[HE, US],
                0,
                54,
                1,
                Key::C,
                (Key::c, CTRL_SHIFT),
                CTRL_SHIFT,
            ),
            Binding::ctrl_shift(Key::c),
        ),
        (
            "fr AZERTY A key",
            press(&[FR], 0, 24, 0, Key::a, (Key::a, CTRL), CTRL),
            Binding::ctrl(Key::a),
        ),
        (
            "de QWERTZ Z key",
            press(&[DE], 0, 29, 0, Key::z, (Key::z, CTRL), CTRL),
            Binding::ctrl(Key::z),
        ),
        (
            "Dvorak C position types j",
            press(&[DVORAK], 0, 54, 0, Key::j, (Key::j, NONE), NONE),
            Binding::plain(Key::j),
        ),
    ];
    for (name, press, binding) in cases {
        assert!(press.is(binding), "{name}: {press:?}");
    }
    let de_ru = press(
        &[DE, RU],
        1,
        29,
        0,
        Key::Cyrillic_en,
        (Key::Cyrillic_en, CTRL),
        CTRL,
    );
    assert!(
        de_ru.is(Binding::ctrl(Key::z)) && !de_ru.is(Binding::ctrl(Key::y)),
        "a configured Latin layout wins over the US table: {de_ru:?}"
    );
}

#[test]
fn caps_lock_never_changes_a_shortcut_or_the_fallback_level() {
    let cases = [
        (
            "us Caps Lock",
            press(&[US], 0, 54, 1, Key::C, (Key::c, CTRL), CTRL.union(CAPS)),
            Binding::ctrl(Key::c),
        ),
        (
            "ru Caps Lock raises the level, not Shift",
            press(
                &[US, RU],
                1,
                54,
                1,
                Key::Cyrillic_ES,
                (Key::Cyrillic_es, CTRL),
                CTRL.union(CAPS),
            ),
            Binding::ctrl(Key::c),
        ),
        (
            "ru Caps Lock with Shift",
            press(
                &[US, RU],
                1,
                54,
                0,
                Key::Cyrillic_es,
                (Key::Cyrillic_es, CTRL_SHIFT),
                CTRL_SHIFT.union(CAPS),
            ),
            Binding::ctrl_shift(Key::c),
        ),
    ];
    for (name, press, binding) in cases {
        assert!(press.is(binding), "{name}: {press:?}");
    }
}

#[test]
fn the_active_layouts_ascii_character_wins_over_the_fallback() {
    // On Russian the key at the US slash position types `.`, and `ю` sits on the US `.` key.
    let period = press(&[US, RU], 1, 61, 0, Key::period, (Key::period, CTRL), CTRL);
    let yu = press(
        &[US, RU],
        1,
        60,
        0,
        Key::Cyrillic_yu,
        (Key::Cyrillic_yu, CTRL),
        CTRL,
    );
    let greek_q = press(
        &[EL, US],
        0,
        24,
        0,
        Key::semicolon,
        (Key::semicolon, CTRL),
        CTRL,
    );
    assert!(period.is(Binding::ctrl(Key::period)) && !period.is(Binding::ctrl(Key::slash)));
    assert!(yu.is(Binding::ctrl(Key::period)));
    assert!(greek_q.is(Binding::ctrl(Key::semicolon)) && !greek_q.is(Binding::ctrl(Key::q)));
}

#[test]
fn symbols_ignore_the_shift_that_types_them() {
    let ru_shift_yu = press(
        &[US, RU],
        1,
        60,
        1,
        Key::Cyrillic_YU,
        (Key::Cyrillic_yu, CTRL_SHIFT),
        CTRL_SHIFT,
    );
    let us_greater = press(
        &[US],
        0,
        60,
        1,
        Key::greater,
        (Key::greater, CTRL),
        CTRL_SHIFT,
    );
    let azerty_period = press(
        &[FR],
        0,
        59,
        1,
        Key::period,
        (Key::period, CTRL),
        CTRL_SHIFT,
    );
    assert!(ru_shift_yu.is(Binding::ctrl(Key::greater)));
    assert!(us_greater.is(Binding::ctrl(Key::greater)));
    assert!(azerty_period.is(Binding::ctrl(Key::period)));
}

#[test]
fn altgr_characters_never_fall_back() {
    let superscript = press(
        &[FR],
        0,
        10,
        2,
        Key::onesuperior,
        (Key::onesuperior, NONE),
        NONE,
    );
    let macron = press(
        &[US],
        0,
        10,
        2,
        Key::dead_macron,
        (Key::dead_macron, NONE),
        NONE,
    );
    assert!(!superscript.used_fallback());
    assert_eq!(superscript.text(), Some('¹'));
    assert!(!macron.used_fallback());
    assert_eq!(macron.text(), None, "a dead key types nothing by itself");
}

#[test]
fn digit_row_keys_are_digits_by_position() {
    let azerty_ctrl_1 = press(
        &[FR],
        0,
        10,
        0,
        Key::ampersand,
        (Key::ampersand, CTRL),
        CTRL,
    );
    let azerty_ctrl_shift_1 = press(&[FR], 0, 10, 1, Key::_1, (Key::_1, CTRL), CTRL_SHIFT);
    let qwertz_ctrl_shift_2 = press(
        &[DE],
        0,
        11,
        1,
        Key::quotedbl,
        (Key::quotedbl, CTRL),
        CTRL_SHIFT,
    );
    let us_ctrl_shift_1 = press(
        &[US],
        0,
        10,
        1,
        Key::exclam,
        (Key::exclam, CTRL),
        CTRL_SHIFT,
    );
    assert!(azerty_ctrl_1.is(Binding::ctrl(Key::_1)));
    assert!(!azerty_ctrl_1.is(Binding::ctrl_shift(Key::_1)));
    assert!(azerty_ctrl_shift_1.is(Binding::ctrl_shift(Key::_1)));
    assert!(!azerty_ctrl_shift_1.is(Binding::ctrl(Key::_1)));
    assert!(qwertz_ctrl_shift_2.is(Binding::ctrl_shift(Key::_2)));
    assert!(us_ctrl_shift_1.is(Binding::ctrl_shift(Key::_1)));
    assert!(!us_ctrl_shift_1.is(Binding::ctrl(Key::_1)));

    let tab_numbers = Digits::new(CTRL_SHIFT, 9, true);
    assert_eq!(azerty_ctrl_shift_1.digit_in(tab_numbers), Some(1));
    assert_eq!(azerty_ctrl_1.digit_in(tab_numbers), None);
    // A chord's digit step accepts the Shift AZERTY types digits with.
    let chord_step = Digits::after(Binding::plain(Key::g).shown(), 9, false);
    let azerty_shift_6 = press(&[FR], 0, 15, 1, Key::_6, (Key::_6, NONE), SHIFT);
    assert_eq!(azerty_shift_6.digit_in(chord_step), Some(6));
    // A top-row + or − is both: g and Ctrl read the symbol first, while t and ;
    // have no symbol step and take the digit.
    let azerty_minus = press(&[FR], 0, 15, 0, Key::minus, (Key::minus, NONE), NONE);
    let cz_plus = press(&[CZ], 0, 10, 0, Key::plus, (Key::plus, NONE), NONE);
    assert!(azerty_minus.is(Binding::plain(Key::minus)));
    assert_eq!(azerty_minus.digit_in(chord_step), Some(6));
    assert!(cz_plus.is(Binding::plain(Key::plus)));
    assert_eq!(cz_plus.digit_in(chord_step), Some(1));

    // Hungarian prints 0 left of 1 and Ö on the tenth key; both read as 0.
    let hu_zero = press(&[HU], 0, 49, 0, Key::_0, (Key::_0, CTRL), CTRL);
    let hu_tenth = press(
        &[HU],
        0,
        19,
        0,
        Key::odiaeresis,
        (Key::odiaeresis, CTRL),
        CTRL,
    );
    assert!(hu_zero.is(Binding::ctrl(Key::_0)), "{hu_zero:?}");
    assert!(hu_tenth.is(Binding::ctrl(Key::_0)), "{hu_tenth:?}");
    let hu_section = press(
        &[HU],
        0,
        49,
        1,
        Key::section,
        (Key::section, CTRL),
        CTRL_SHIFT,
    );
    assert!(!hu_section.is(Binding::ctrl_shift(Key::_0)));

    // GDK reports no AltGr modifier: above level 1 the row counts only a digit
    // it types, as Lithuanian does, never an AltGr symbol.
    let lt_ctrl_1 = press(&[LT], 0, 10, 2, Key::_1, (Key::_1, CTRL), CTRL);
    assert!(lt_ctrl_1.is(Binding::ctrl(Key::_1)));
    let azerty_ctrl_superscript = press(
        &[FR],
        0,
        10,
        2,
        Key::onesuperior,
        (Key::onesuperior, CTRL),
        CTRL,
    );
    assert!(!azerty_ctrl_superscript.is(Binding::ctrl(Key::_1)));
    let azerty_tilde = press(
        &[FR],
        0,
        11,
        2,
        Key::asciitilde,
        (Key::asciitilde, NONE),
        NONE,
    );
    assert_eq!(azerty_tilde.digit_in(chord_step), None);
    // German Mac types \ with Shift+AltGr+7: Ctrl+\, not tab 7.
    let de_mac_backslash = press(
        &[DE],
        0,
        16,
        3,
        Key::backslash,
        (Key::backslash, CTRL),
        CTRL_SHIFT,
    );
    assert_eq!(de_mac_backslash.digit_in(tab_numbers), None);
    assert!(de_mac_backslash.is(Binding::ctrl(Key::backslash)));
}

#[test]
fn a_captured_press_records_the_key_dispatch_reads() {
    let us = |keycode, level, typed, state| {
        press(
            &[US],
            0,
            keycode,
            level,
            typed,
            (typed, state - SHIFT),
            state,
        )
    };
    let cases = [
        (
            us(12, 1, Key::numbersign, CTRL_SHIFT),
            Binding::ctrl_shift(Key::_3),
        ),
        (
            press(&[FR], 0, 10, 1, Key::_1, (Key::_1, CTRL), CTRL_SHIFT),
            Binding::ctrl_shift(Key::_1),
        ),
        (
            press(
                &[DE],
                0,
                11,
                1,
                Key::quotedbl,
                (Key::quotedbl, CTRL),
                CTRL_SHIFT,
            ),
            Binding::ctrl_shift(Key::_2),
        ),
        (
            us(12, 1, Key::numbersign, SHIFT),
            Binding::plain(Key::numbersign),
        ),
        (us(12, 0, Key::_3, CTRL), Binding::ctrl(Key::_3)),
        (
            press(
                &[DE],
                0,
                16,
                3,
                Key::backslash,
                (Key::backslash, CTRL),
                CTRL_SHIFT,
            ),
            Binding::ctrl(Key::backslash),
        ),
    ];
    for (pressed, recorded) in cases {
        assert_eq!(pressed.binding(), Some(recorded), "{pressed:?}");
        assert!(pressed.is(recorded), "{pressed:?}");
    }
    assert_eq!(
        us(12, 1, Key::numbersign, CTRL_SHIFT).digit_in(Digits::new(CTRL_SHIFT, 9, true)),
        Some(3),
        "tab numbers read the key the capture records"
    );

    // AZERTY's Ctrl+& is also Ctrl+1, which a capture must check as well.
    let azerty_ctrl_1 = press(
        &[FR],
        0,
        10,
        0,
        Key::ampersand,
        (Key::ampersand, CTRL),
        CTRL,
    );
    assert_eq!(azerty_ctrl_1.binding(), Some(Binding::ctrl(Key::ampersand)));
    assert_eq!(azerty_ctrl_1.digit_position(), Some(Binding::ctrl(Key::_1)));
    assert!(azerty_ctrl_1.is(Binding::ctrl(Key::_1)));
    assert_eq!(us(12, 0, Key::_3, NONE).digit_position(), None);

    // AltGr and a layout switch start a press rather than make one, and a
    // key the keymap cannot load is never recorded.
    for typed in [Key::ISO_Level3_Shift, Key::ISO_Next_Group] {
        let held = press(&[DE], 0, 108, 0, typed, (typed, CTRL), CTRL);
        assert!(held.is_modifier(), "{typed:?}");
    }
    assert_eq!(us(0, 0, Key::VoidSymbol, CTRL).binding(), None);
}

/// Real key events, which `forward` needs; an input method's `filter_key`
/// builds them from a keycode.
mod key_event {
    use gtk::{gdk, glib, prelude::*, subclass::prelude::*};
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Imp(RefCell<Option<gdk::Event>>);

    #[glib::object_subclass]
    impl ObjectSubclass for Imp {
        const NAME: &'static str = "StrataTestKeyEventBuilder";
        type Type = Builder;
        type ParentType = gtk::IMContext;
    }

    impl ObjectImpl for Imp {}

    impl IMContextImpl for Imp {
        fn filter_keypress(&self, event: &gdk::Event) -> bool {
            self.0.replace(Some(event.clone()));
            true
        }
    }

    glib::wrapper! {
        pub struct Builder(ObjectSubclass<Imp>) @extends gtk::IMContext;
    }

    pub fn press(window: &gtk::Window, key: gdk::Key) {
        let display = WidgetExt::display(window);
        let keycode = display
            .map_keyval(key)
            .and_then(|keys| {
                keys.into_iter()
                    .find(|key| key.group() == 0 && key.level() == 0)
            })
            .expect("the keymap types the key")
            .keycode();
        let keyboard = display
            .default_seat()
            .and_then(|seat| seat.keyboard())
            .expect("a keyboard");
        let builder = glib::Object::new::<Builder>();
        builder.filter_key(
            true,
            &window.surface().expect("a presented window"),
            &keyboard,
            0,
            keycode,
            super::NONE,
            0,
        );
        let event = builder.imp().0.take().expect("a key event");
        #[expect(deprecated, reason = "GTK has no other way to deliver a key event")]
        display.put_event(&event);
        while glib::MainContext::default().iteration(false) {}
    }
}

#[test]
fn key_handlers_yield_to_a_composition_in_the_focused_field() {
    crate::test_support::gtk_test(
        "ui::keys::tests::key_handlers_yield_to_a_composition_in_the_focused_field",
        || {
            use gtk::prelude::*;
            use std::{cell::Cell, rc::Rc};

            let fields: [fn() -> gtk::Widget; 2] = [
                || gtk::Text::new().upcast(),
                || gtk::TextView::new().upcast(),
            ];
            let typed = |field: &gtk::Widget| match field.downcast_ref::<gtk::TextView>() {
                Some(view) => {
                    let buffer = view.buffer();
                    buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)
                }
                None => field.downcast_ref::<gtk::Text>().expect("a field").text(),
            };
            for field in fields {
                let window = gtk::Window::new();
                let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let composing = field();
                let other = field();
                column.append(&composing);
                column.append(&other);
                window.set_child(Some(&column));
                let keys = gtk::EventControllerKey::new();
                keys.set_propagation_phase(gtk::PropagationPhase::Capture);
                let handled = Rc::new(Cell::new(0));
                let count = handled.clone();
                super::on_press(&keys, move |_, _| {
                    count.set(count.get() + 1);
                    gtk::glib::Propagation::Stop
                });
                window.add_controller(keys.clone());
                window.present();
                let press = |key| key_event::press(&window, key);
                let preedit =
                    |text: &str| composing.emit_by_name::<()>("preedit-changed", &[&text]);

                assert!(composing.grab_focus());
                preedit("ˇ");
                press(Key::a);
                assert_eq!(handled.get(), 0, "the input method takes a");
                assert_eq!(typed(&composing), "a");
                press(Key::Escape);
                assert_eq!(
                    handled.get(),
                    1,
                    "a key the input method passes back reaches the handler"
                );
                assert!(other.grab_focus());
                press(Key::a);
                assert_eq!(
                    handled.get(),
                    2,
                    "only the focused field's composition holds keys"
                );
                assert!(composing.grab_focus());
                preedit("");
                press(Key::a);
                assert_eq!(handled.get(), 3, "a finished composition releases the keys");
                window.destroy();
            }
        },
    );
}

#[test]
fn keypad_digits_count_only_with_num_lock() {
    let digit = press(&[US], 0, 87, 0, Key::KP_1, (Key::KP_1, CTRL), CTRL);
    let end = press(&[US], 0, 87, 0, Key::KP_End, (Key::KP_End, CTRL), CTRL);
    assert!(digit.is(Binding::ctrl(Key::_1)));
    assert!(!end.is(Binding::ctrl(Key::_1)));
}

#[test]
fn modifiers_match_exactly_over_ctrl_alt_shift_and_super() {
    let ctrl_alt_c = press(
        &[US],
        0,
        54,
        0,
        Key::c,
        (Key::c, CTRL.union(Modifiers::ALT_MASK)),
        CTRL.union(Modifiers::ALT_MASK),
    );
    let ctrl_super_c = press(
        &[US],
        0,
        54,
        0,
        Key::c,
        (Key::c, CTRL.union(Modifiers::SUPER_MASK)),
        CTRL.union(Modifiers::SUPER_MASK),
    );
    let ctrl_hyper_c = press(
        &[US],
        0,
        54,
        0,
        Key::c,
        (Key::c, CTRL.union(Modifiers::HYPER_MASK)),
        CTRL.union(Modifiers::HYPER_MASK),
    );
    assert!(!ctrl_alt_c.is(Binding::ctrl(Key::c)));
    assert!(!ctrl_super_c.is(Binding::ctrl(Key::c)));
    assert!(ctrl_hyper_c.is(Binding::ctrl(Key::c)));
    assert!(!press(&[US], 0, 54, 0, Key::c, (Key::c, NONE), NONE).is(Binding::ctrl(Key::c)));
}

#[test]
fn typing_receives_the_character_the_layout_produced() {
    let es = press(
        &[US, RU],
        1,
        54,
        0,
        Key::Cyrillic_es,
        (Key::Cyrillic_es, NONE),
        NONE,
    );
    let caps_es = press(
        &[US, RU],
        1,
        54,
        1,
        Key::Cyrillic_ES,
        (Key::Cyrillic_es, NONE),
        CAPS,
    );
    let ctrl_es = press(
        &[US, RU],
        1,
        54,
        0,
        Key::Cyrillic_es,
        (Key::Cyrillic_es, CTRL),
        CTRL,
    );
    assert_eq!(es.text(), Some('с'));
    assert!(es.is(Binding::plain(Key::c)));
    assert_eq!(caps_es.text(), Some('С'));
    assert_eq!(ctrl_es.text(), None);
}

#[test]
fn keypad_keys_act_as_the_main_keys() {
    let enter = press(&[US], 0, 104, 0, Key::KP_Enter, (Key::KP_Enter, NONE), NONE);
    let up = press(&[US], 0, 80, 0, Key::KP_Up, (Key::KP_Up, SHIFT), SHIFT);
    let add = press(&[US], 0, 86, 0, Key::KP_Add, (Key::KP_Add, CTRL), CTRL);
    assert!(enter.is(Binding::plain(Key::Return)));
    assert!(up.is(Binding::shift(Key::Up)));
    assert!(add.is(Binding::ctrl(Key::plus)));
    assert_eq!(add.text(), None);
}

#[test]
fn ignored_modifiers_are_the_only_ones_a_binding_tolerates() {
    let escape = Binding::plain(Key::Escape).ignoring(super::SHORTCUT_MODIFIERS);
    let arrow = Binding::plain(Key::Up).ignoring(CTRL_SHIFT);
    let shift_escape = press(&[US], 0, 9, 0, Key::Escape, (Key::Escape, SHIFT), SHIFT);
    let ctrl_shift_up = press(&[US], 0, 111, 0, Key::Up, (Key::Up, CTRL_SHIFT), CTRL_SHIFT);
    let alt_up = press(
        &[US],
        0,
        111,
        0,
        Key::Up,
        (Key::Up, Modifiers::ALT_MASK),
        Modifiers::ALT_MASK,
    );
    assert!(shift_escape.is(escape));
    assert!(!shift_escape.is(Binding::plain(Key::Escape)));
    assert!(ctrl_shift_up.is(arrow));
    assert!(!alt_up.is(arrow));
}

#[test]
fn signals_without_an_event_read_as_us_presses() {
    let exclam = KeyPress::from_signal(Key::exclam, 0, CTRL_SHIFT);
    assert!(exclam.is(Binding::ctrl_shift(Key::_1)));
    assert!(exclam.is(Binding::ctrl(Key::exclam)));
    for letter in [Key::K, Key::k] {
        assert!(KeyPress::from_signal(letter, 0, CTRL_SHIFT).is(Binding::ctrl_shift(Key::k)));
    }
    assert!(
        KeyPress::from_signal(Key::G, 0, CAPS).is(Binding::plain(Key::g)),
        "Caps Lock alone types G"
    );
    assert!(KeyPress::from_signal(Key::ISO_Left_Tab, 0, NONE).is(Binding::shift(Key::Tab)));
    assert!(KeyPress::from_signal(Key::question, 0, SHIFT).is(Binding::plain(Key::question)));
    assert!(KeyPress::from_signal(Key::Shift_L, 0, NONE).is_modifier());
    assert_eq!(
        KeyPress::from_signal(Key::Control_R, 0, SHIFT).modifier_key(),
        CTRL
    );
    let typed = KeyPress::from_signal(Key::Cyrillic_es, 0, NONE);
    assert_eq!(typed.text(), Some('с'));
    assert!(!typed.used_fallback());
}

mod labels {
    use gtk::gdk::{Key, ModifierType as Modifiers};

    use super::super::{Binding, Digits, Keys, Shown, digit_label, label_in, shown};

    const CTRL: Modifiers = Modifiers::CONTROL_MASK;

    fn english(keys: Keys) -> String {
        label_in("en", &keys)
    }

    #[test]
    fn notation_orders_modifiers_and_spells_out_named_keys() {
        let cases = [
            (Keys::One(Binding::ctrl(Key::c).shown()), "Ctrl+C"),
            (
                Keys::One(Binding::ctrl_shift(Key::n).shown()),
                "Ctrl+Shift+N",
            ),
            (
                Keys::One(
                    Binding::new(
                        Key::t,
                        Modifiers::SUPER_MASK | Modifiers::SHIFT_MASK | CTRL | Modifiers::ALT_MASK,
                    )
                    .shown(),
                ),
                "Ctrl+Alt+Shift+Super+T",
            ),
            (Keys::One(Binding::plain(Key::j).shown()), "j"),
            (Keys::One(Binding::shift(Key::j).shown()), "J"),
            (Keys::One(Binding::plain(Key::Page_Up).shown()), "Page Up"),
            (
                Keys::One(Binding::shift(Key::Delete).shown()),
                "Shift+Delete",
            ),
            (Keys::One(Binding::shift(Key::Tab).shown()), "Shift+Tab"),
            (Keys::One(Binding::alt(Key::Return).shown()), "Alt+Enter"),
            (Keys::One(Binding::plain(Key::Escape).shown()), "Escape"),
            (Keys::One(Binding::ctrl(Key::plus).shown()), "Ctrl++"),
            (
                Keys::One(Binding::ctrl(Key::minus).shown()),
                "Ctrl+\u{2212}",
            ),
            (Keys::One(Binding::ctrl(Key::backslash).shown()), "Ctrl+\\"),
            (Keys::One(Binding::plain(Key::F5).shown()), "F5"),
            (Keys::One(Binding::plain(Key::question).shown()), "?"),
            (Keys::One(Binding::plain(Key::EuroSign).shown()), "€"),
            (Keys::One(Binding::ctrl(Key::ae).shown()), "Ctrl+Æ"),
            (Keys::One(Binding::plain(Key::ae).shown()), "æ"),
            (
                Keys::One(Binding::ctrl(Key::dead_circumflex).shown()),
                "Ctrl+^",
            ),
            (Keys::One(Binding::plain(Key::dead_diaeresis).shown()), "¨"),
            (Keys::One(Binding::plain(Key::Multi_key).shown()), "Compose"),
        ];
        for (keys, expected) in cases {
            assert_eq!(english(keys), expected);
        }
    }

    #[test]
    fn alternatives_chords_and_digit_ranges() {
        const RENAME: &[Shown] = shown![Binding::plain(Key::F2), Binding::ctrl(Key::r)];
        const ARROWS: &[Shown] =
            shown![Binding::ctrl_alt(Key::Left), Binding::ctrl_alt(Key::Right)];
        const SCROLL: &[Shown] = shown![Binding::ctrl(Key::u), Binding::ctrl(Key::d)];
        const GO_HOME: &[Shown] = shown![Binding::plain(Key::g), Binding::plain(Key::h)];
        const TOP: [Keys; 2] = [
            Keys::Chord(shown![Binding::plain(Key::g), Binding::plain(Key::g)]),
            Keys::One(Binding::plain(Key::Home).shown()),
        ];
        const TAB_NUMBERS: Digits = Digits::new(CTRL.union(Modifiers::SHIFT_MASK), 9, true);
        const TAB_CHORD_NUMBERS: Digits = Digits::after(Binding::plain(Key::t).shown(), 9, true);
        let cases = [
            (Keys::Any(RENAME), "F2 / Ctrl+R"),
            (Keys::Any(ARROWS), "Ctrl+Alt+← / →"),
            (Keys::Any(SCROLL), "Ctrl+U / Ctrl+D"),
            (Keys::Chord(GO_HOME), "g h"),
            (Keys::Mixed(&TOP), "g g / Home"),
            (Keys::Digits(TAB_NUMBERS), "Ctrl+Shift+1\u{2013}9 / 0"),
            (Keys::Digits(TAB_CHORD_NUMBERS), "t 1\u{2013}9 / t 0"),
            (Keys::Digits(TAB_CHORD_NUMBERS.step()), "1\u{2013}9 / 0"),
            (Keys::Digits(TAB_NUMBERS.step()), "1\u{2013}9 / 0"),
            (
                Keys::Digits(Digits::after(Binding::plain(Key::g).shown(), 9, false)),
                "g 1\u{2013}9",
            ),
        ];
        for (keys, expected) in cases {
            assert_eq!(english(keys), expected);
        }
        // Neither label is translated, so the locale other tests set cannot change them.
        assert_eq!(digit_label(TAB_CHORD_NUMBERS, 0), "t 0");
        assert_eq!(digit_label(TAB_NUMBERS.step(), 3), "3");
    }

    #[test]
    fn key_names_follow_the_interface_language() {
        let cases = [
            ("de", Keys::One(Binding::ctrl(Key::c).shown()), "Strg+C"),
            ("de", Keys::One(Binding::plain(Key::Delete).shown()), "Entf"),
            (
                "de",
                Keys::One(Binding::ctrl_shift(Key::Page_Up).shown()),
                "Strg+Umschalt+Bild auf",
            ),
            (
                "fr",
                Keys::One(Binding::plain(Key::Delete).shown()),
                "Suppr",
            ),
            (
                "fr",
                Keys::One(Binding::shift(Key::Return).shown()),
                "Maj+Entrée",
            ),
            ("es", Keys::One(Binding::plain(Key::Delete).shown()), "Supr"),
            (
                "es",
                Keys::One(Binding::ctrl(Key::Multi_key).shown()),
                "Ctrl+Componer",
            ),
            ("ru", Keys::One(Binding::ctrl(Key::c).shown()), "Ctrl+C"),
            (
                "ja",
                Keys::One(Binding::plain(Key::Escape).shown()),
                "Escape",
            ),
        ];
        for (locale, keys, expected) in cases {
            assert_eq!(label_in(locale, &keys), expected, "{locale}");
        }
    }
}

#[test]
fn catalog_bindings_use_the_canonical_key() {
    use super::{Map, bindings::Entry};
    use gtk::glib::translate::IntoGlib;
    let mut checked = 0;
    for entry in super::bindings::entries() {
        let (name, bindings) = match entry {
            Entry::Binding(binding) => (format!("{binding:?}"), vec![binding]),
            Entry::Command(command) => (
                command.id().to_owned(),
                [Map::Default, Map::Tenxer]
                    .into_iter()
                    .flat_map(|map| command.defaults(map).iter().copied())
                    .collect(),
            ),
            Entry::Digits(_) => continue,
        };
        for binding in bindings {
            checked += 1;
            let key = binding.key();
            assert_eq!(key, key.to_lower(), "{name} names an uppercase letter");
            assert_eq!(key, super::main_key(key), "{name} names a keypad key");
            assert_ne!(
                key,
                Key::ISO_Left_Tab,
                "{name} spells Shift+Tab as ISO_Left_Tab"
            );
            let symbol = (0x21..=0x7e).contains(&key.into_glib())
                && !key.to_unicode().is_some_and(|c| c.is_ascii_alphanumeric());
            assert!(
                !(symbol && binding.modifiers().contains(SHIFT)),
                "{name} adds the Shift that types its symbol"
            );
        }
    }
    assert!(checked > 0);
}

/// Constants with equal keys each need a row of their own, since a row shows
/// bindings rather than constants. A command needs its row in each map its
/// catalog block lists it in.
#[test]
fn every_listed_binding_has_its_own_row_in_its_maps() {
    use super::{
        CUSTOM_ACTIONS, Keymap, Keys, Map,
        bindings::{Entry, Listed},
        command::{Kind, registry},
        use_keymap,
    };
    /// What a row shows for `constant` in `map`: its bindings, the chord key
    /// they follow, or its digit range.
    #[derive(PartialEq)]
    enum Wanted {
        Keys(Vec<Binding>, Option<Binding>),
        Digits(Digits),
    }
    fn wanted(constant: &Listed, map: Map) -> Wanted {
        match constant.entry {
            Entry::Binding(binding) => Wanted::Keys(vec![binding], None),
            Entry::Digits(digits) => Wanted::Digits(digits),
            Entry::Command(command) => {
                let after = match command.kind() {
                    Kind::Step(group) => {
                        registry().find(|trigger| trigger.kind() == Kind::Trigger(group))
                    }
                    Kind::Single | Kind::Trigger(_) | Kind::Structural => constant.after,
                };
                Wanted::Keys(
                    command.bindings_in(map),
                    after.and_then(|after| after.bindings_in(map).first().copied()),
                )
            }
        }
    }
    fn shows(keys: &Keys, wanted: &Wanted, map: Map) -> bool {
        let primary = |shown: &super::Shown| shown.bindings(map).first().copied();
        match (keys, wanted) {
            (Keys::Mixed(alternatives), _) => {
                alternatives.iter().any(|keys| shows(keys, wanted, map))
            }
            (Keys::Digits(digits), Wanted::Digits(wanted)) => digits == wanted,
            (Keys::Chord(steps), Wanted::Keys(bindings, Some(first))) => {
                let steps: Vec<Option<Binding>> = steps.iter().map(primary).collect();
                bindings.iter().any(|binding| {
                    steps
                        .windows(2)
                        .any(|pair| pair == [Some(*first), Some(*binding)])
                })
            }
            (Keys::Chord(steps), Wanted::Keys(bindings, None)) => steps
                .first()
                .and_then(primary)
                .is_some_and(|step| bindings.contains(&step)),
            (Keys::One(shown), Wanted::Keys(bindings, None)) => shown
                .bindings(map)
                .iter()
                .any(|binding| bindings.contains(binding)),
            (Keys::Any(shown), Wanted::Keys(bindings, None)) => shown
                .iter()
                .flat_map(|shown| shown.bindings(map))
                .any(|binding| bindings.contains(&binding)),
            _ => false,
        }
    }
    // Custom action slots ship no key, so bind each to find its row.
    let slot_keys = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
    ];
    let keymap =
        CUSTOM_ACTIONS
            .iter()
            .zip(slot_keys)
            .fold(Keymap::defaults(), |keymap, (slot, key)| {
                let binding = Binding::new(key, Modifiers::ALT_MASK | Modifiers::SUPER_MASK);
                keymap.with(Map::Default, *slot, vec![binding])
            });
    let _keymap = use_keymap(keymap);
    let listed: Vec<Listed> = super::bindings::listed().collect();
    assert!(!listed.is_empty());
    for map in [Map::Default, Map::Tenxer] {
        let tenxer = map == Map::Tenxer;
        let rows = crate::ui::shortcut_reference::reference_rows(tenxer);
        let wanted: Vec<(&Listed, Wanted)> = listed
            .iter()
            .filter(|constant| constant.maps.include(tenxer))
            .map(|constant| (constant, wanted(constant, map)))
            .collect();
        for (constant, keys) in &wanted {
            assert!(
                !matches!(keys, Wanted::Keys(bindings, _) if bindings.is_empty()),
                "{} is listed in the {map:?} map but has no binding there",
                constant.name
            );
            let sharing: Vec<&str> = wanted
                .iter()
                .filter(|(_, other)| other == keys)
                .map(|(other, _)| other.name)
                .collect();
            let shown = rows
                .iter()
                .filter(|row| row.iter().any(|row_keys| shows(row_keys, keys, map)))
                .count();
            assert!(
                shown >= sharing.len(),
                "{sharing:?} need a row each in the {map:?} reference, which has {shown}; add \
                 a row, or move a constant to the catalog block of the maps that list it"
            );
        }
    }
}
