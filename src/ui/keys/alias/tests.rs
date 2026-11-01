// SPDX-License-Identifier: MIT

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    prelude::*,
};

use super::{
    super::{
        Binding, Command, Event, KeyPress, Keymap, Map,
        bindings::{
            DOWN, ENTER, ENTER_ANY, ENTER_ANY_SHIFT, ESCAPE, ESCAPE_ANY, ESCAPE_ANY_SHIFT,
            FOCUS_NEXT, LEFT, RIGHT, UP,
        },
        install_native_aliases, normalize, use_keymap,
    },
    Context, rewrite,
};

const NONE: Modifiers = Modifiers::empty();

fn press(key: Key, modifiers: Modifiers) -> KeyPress {
    KeyPress::from_signal(key, 0, modifiers)
}

fn listing() -> Context {
    Context::of(None, false)
}

#[test]
fn alias_rewrites_typed_keys_only_outside_text_and_popovers() {
    crate::test_support::gtk_test(
        "ui::keys::alias::tests::alias_rewrites_typed_keys_only_outside_text_and_popovers",
        || {
            let _keymap = use_keymap(Keymap::defaults().with(
                Map::Default,
                ENTER,
                vec![
                    Binding::plain(Key::Return),
                    Binding::ctrl(Key::m),
                    Binding::plain(Key::e),
                ],
            ));
            let ctrl_m = press(Key::m, Modifiers::CONTROL_MASK);
            let e = press(Key::e, NONE);
            for alternate in [ctrl_m, e] {
                let read = rewrite(alternate, listing());
                assert!(read.is(ENTER) && read.aliased(), "{alternate:?}");
                assert_eq!(read.text(), alternate.text());
            }

            let text = gtk::Text::new();
            let popover = gtk::Popover::new();
            let option = gtk::Button::with_label("Option");
            popover.set_child(Some(&option));
            for owner in [text.upcast_ref::<gtk::Widget>(), option.upcast_ref()] {
                let typing = Context::of(Some(owner), false);
                assert!(rewrite(ctrl_m, typing).is(ENTER), "{owner:?}");
                let typed = rewrite(e, typing);
                assert!(!typed.is(ENTER) && !typed.aliased(), "{owner:?}");
            }
            let field = gtk::TextView::new();
            assert!(!rewrite(e, Context::of(Some(field.upcast_ref()), false)).aliased());
            let document = gtk::TextView::builder().editable(false).build();
            assert!(!rewrite(e, Context::of(Some(document.upcast_ref()), false)).aliased());

            let type_to_search = Context::of(None, true);
            assert!(rewrite(ctrl_m, type_to_search).is(ENTER));
            assert!(!rewrite(e, type_to_search).is(ENTER));

            // The shipped h j k l follow the same rule in both maps.
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            for tenxer in [false, true] {
                preferences.set_tenxer_mode(tenxer);
                for (letter, arrow) in [
                    (Key::h, LEFT),
                    (Key::j, DOWN),
                    (Key::k, UP),
                    (Key::l, RIGHT),
                ] {
                    let plain = press(letter, NONE);
                    assert!(rewrite(plain, listing()).is(arrow), "{letter:?} {tenxer}");
                    assert!(!rewrite(plain, type_to_search).is(arrow));
                    assert!(!rewrite(plain, Context::of(Some(text.upcast_ref()), false)).is(arrow));
                    // Only 10xer reads a read-only text view as a document.
                    assert_eq!(
                        rewrite(plain, Context::of(Some(document.upcast_ref()), false)).is(arrow),
                        tenxer,
                        "{letter:?}"
                    );
                    for modifier in [
                        Modifiers::SHIFT_MASK,
                        Modifiers::CONTROL_MASK,
                        Modifiers::ALT_MASK,
                        Modifiers::SUPER_MASK,
                    ] {
                        assert!(!rewrite(press(letter, modifier), listing()).aliased());
                    }
                }
            }
            preferences.set_tenxer_mode(false);
        },
    );
}

#[test]
fn canonical_keys_never_rewrite() {
    let escape_elsewhere = [
        Binding::plain(Key::Up),
        Binding::plain(Key::Escape),
        Binding::shift(Key::Escape),
        Binding::ctrl(Key::Escape),
    ];
    let _keymap = use_keymap(
        Keymap::defaults()
            .with(Map::Default, UP, escape_elsewhere.to_vec())
            .with(Map::Default, ESCAPE, vec![Binding::ctrl(Key::bracketleft)])
            .with(Map::Default, ENTER, vec![Binding::ctrl(Key::m)]),
    );
    for modifiers in [
        NONE,
        Modifiers::SHIFT_MASK,
        Modifiers::CONTROL_MASK,
        Modifiers::ALT_MASK | Modifiers::SUPER_MASK,
    ] {
        let escape = rewrite(press(Key::Escape, modifiers), listing());
        assert!(!escape.aliased() && !escape.is(UP), "{modifiers:?}");
        assert!(escape.is(ESCAPE_ANY), "{modifiers:?}");
    }
    // Escape is a hidden fallback; Enter can be reassigned away entirely.
    assert!(rewrite(press(Key::Escape, NONE), listing()).is(ESCAPE));
    assert!(rewrite(press(Key::bracketleft, Modifiers::CONTROL_MASK), listing()).is(ESCAPE));
    assert!(!rewrite(press(Key::Return, NONE), listing()).is(ENTER));

    // On Russian the key at the US j position types о and reads as j.
    let event = Event {
        matched: (Key::Cyrillic_o, NONE),
        typed: Key::Cyrillic_o,
        state: NONE,
        keycode: 44,
        layout: 1,
        level: 0,
        modifier: false,
    };
    let russian = normalize(&event, |_| [(0, 0, Key::j)]);
    assert!(russian.used_fallback());
    let read = rewrite(russian, listing());
    assert!(read.is(DOWN) && read.aliased());
    assert!(read.used_fallback());
    assert_eq!(read.text(), Some('о'));
}

/// Dialogs accept Escape and Enter with any modifiers; an alternate keeps its
/// own, so its bare key still types in a dialog's field or a rename.
#[test]
fn modifier_tolerant_matchers_keep_an_alternates_own_modifiers() {
    crate::test_support::gtk_test(
        "ui::keys::alias::tests::modifier_tolerant_matchers_keep_an_alternates_own_modifiers",
        modifier_tolerant_matchers,
    );
}

fn modifier_tolerant_matchers() {
    let _keymap = use_keymap(
        Keymap::defaults()
            .with(
                Map::Default,
                ESCAPE,
                vec![Binding::plain(Key::Escape), Binding::ctrl(Key::bracketleft)],
            )
            .with(
                Map::Default,
                ENTER,
                vec![Binding::plain(Key::Return), Binding::ctrl(Key::m)],
            ),
    );
    let text = gtk::Text::new();
    for context in [listing(), Context::of(Some(text.upcast_ref()), false)] {
        for (key, matcher) in [
            (Key::bracketleft, ESCAPE_ANY),
            (Key::bracketleft, ESCAPE_ANY_SHIFT),
            (Key::m, ENTER_ANY),
            (Key::m, ENTER_ANY_SHIFT),
        ] {
            for modifiers in [NONE, Modifiers::SHIFT_MASK, Modifiers::ALT_MASK] {
                let read = rewrite(press(key, modifiers), context);
                assert!(!read.is(matcher), "{key:?} {modifiers:?} {context:?}");
            }
        }
        let held = Modifiers::CONTROL_MASK | Modifiers::ALT_MASK;
        assert!(rewrite(press(Key::bracketleft, held), context).is(ESCAPE_ANY));
        assert!(rewrite(press(Key::m, held), context).is(ENTER_ANY));
        assert!(!rewrite(press(Key::m, held), context).is(ENTER_ANY_SHIFT));
    }
}

fn settle(until: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !until() {
        assert!(std::time::Instant::now() < deadline, "GTK did not settle");
        gtk::glib::MainContext::default().iteration(false);
    }
}

/// What GTK does for the canonical key happens for an unclaimed alias once
/// propagation reaches the window, in widgets Strata leaves to GTK.
#[test]
fn unclaimed_aliases_get_the_canonical_keys_gtk_effect() {
    crate::test_support::gtk_test(
        "ui::keys::alias::tests::unclaimed_aliases_get_the_canonical_keys_gtk_effect",
        || {
            let extra = |keymap: Keymap, command: Command, added: &[Binding]| {
                let mut bindings = command.defaults(Map::Default).to_vec();
                bindings.extend_from_slice(added);
                keymap.with(Map::Default, command, bindings)
            };
            let keymap = extra(Keymap::defaults(), DOWN, &[Binding::ctrl(Key::j)]);
            let keymap = extra(keymap, LEFT, &[Binding::ctrl(Key::b)]);
            let keymap = extra(keymap, ESCAPE, &[Binding::ctrl(Key::bracketleft)]);
            let keymap = extra(keymap, FOCUS_NEXT, &[Binding::plain(Key::F7)]);
            let _keymap = use_keymap(extra(
                keymap,
                ENTER,
                &[Binding::ctrl(Key::m), Binding::plain(Key::e)],
            ));

            let list = gtk::ListBox::new();
            for name in ["a", "b"] {
                list.append(&gtk::Label::new(Some(name)));
            }
            let text = gtk::Text::new();
            text.set_text("abc");
            let button = gtk::Button::with_label("Button");
            let clicks = std::rc::Rc::new(std::cell::Cell::new(0));
            let counted = clicks.clone();
            button.connect_clicked(move |_| counted.set(counted.get() + 1));
            let menu = gtk::MenuButton::new();
            let option = gtk::Button::with_label("Option");
            let popover = gtk::Popover::builder().child(&option).build();
            menu.set_popover(Some(&popover));
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for widget in [
                list.upcast_ref::<gtk::Widget>(),
                text.upcast_ref(),
                button.upcast_ref(),
                menu.upcast_ref(),
            ] {
                content.append(widget);
            }
            let window = gtk::Window::builder().child(&content).build();
            let native = install_native_aliases(&window);
            window.present();
            let focus = || gtk::prelude::RootExt::focus(&window);
            let press = |key: Key, modifiers: Modifiers| {
                native.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
            };
            let first = list.row_at_index(0).expect("first row");
            let second = list.row_at_index(1).expect("second row");

            first.grab_focus();
            settle(|| focus().as_ref() == Some(first.upcast_ref()));
            assert!(press(Key::j, Modifiers::CONTROL_MASK));
            assert_eq!(
                focus().as_ref(),
                Some(second.upcast_ref()),
                "Down in a list box"
            );

            text.grab_focus();
            text.set_position(3);
            assert!(press(Key::b, Modifiers::CONTROL_MASK));
            assert_eq!(text.position(), 2, "Left in a text field");
            assert!(!press(Key::e, NONE), "a plain alternate is typing");

            assert!(press(Key::F7, NONE));
            assert_eq!(focus().as_ref(), Some(button.upcast_ref()), "Tab moves on");
            assert!(press(Key::m, Modifiers::CONTROL_MASK));
            settle(|| clicks.get() == 1);
            assert!(!press(Key::e, NONE));

            menu.popup();
            option.grab_focus();
            settle(|| popover.is_visible() && focus().as_ref() == Some(option.upcast_ref()));
            assert!(press(Key::bracketleft, Modifiers::CONTROL_MASK));
            assert!(!popover.is_visible(), "Escape closes the popover");

            assert!(
                !press(Key::bracketleft, NONE),
                "only aliases get GTK's effect"
            );
            window.destroy();
        },
    );
}
