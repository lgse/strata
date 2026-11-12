// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::preferences::PreferenceManager;

const NONE: gdk::ModifierType = gdk::ModifierType::empty();
const CTRL: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK;
const STOP: Option<glib::Propagation> = Some(glib::Propagation::Stop);

struct Presented {
    manager: Rc<PreferenceManager>,
    footer: ShortcutFooter,
    files: gtk::Button,
    overlay: gtk::Overlay,
    window: gtk::Window,
}

fn present_footer_beside_files() -> Presented {
    let manager = PreferenceManager::shared();
    manager.set_tenxer_mode(false);
    manager.set_show_keybinding_hints(true);
    let footer = ShortcutFooter::new(BrowserMode::Columns);
    footer.bind_preferences(&manager);
    let files = gtk::Button::new();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&files);
    root.append(footer.widget());
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&root));
    let window = gtk::Window::builder()
        .child(&overlay)
        .default_width(640)
        .default_height(480)
        .build();
    window.present();
    files.grab_focus();
    settle();
    Presented {
        manager,
        footer,
        files,
        overlay,
        window,
    }
}

fn backdrop_shown(overlay: &gtk::Overlay) -> bool {
    overlay
        .observe_children()
        .into_iter()
        .flatten()
        .any(|child| {
            child
                .downcast_ref::<gtk::Widget>()
                .is_some_and(|widget| widget.has_css_class("search-backdrop"))
        })
}

fn shows_only_the_pill(footer: &ShortcutFooter) -> bool {
    let phrase = crate::ui::shortcut_reference::EXPERIMENTAL_LABEL;
    footer
        .status
        .observe_children()
        .into_iter()
        .flatten()
        .all(|child| {
            child
                .downcast_ref::<gtk::Label>()
                .is_none_or(|label| !label.is_visible() || label.text() != phrase)
        })
}

fn the_default_map_hides_tenxer_rows(presented: &Presented) {
    let footer = &presented.footer;
    assert!(!footer.tag.is_visible());
    assert!(has_label(
        &reference_labels(footer),
        "Preview a file, or open a folder"
    ));
    expect_key(footer, gdk::Key::asciitilde, NONE, None);
    assert!(!footer.session.is_open());
}

fn entering_tenxer_shows_the_pill_and_column_rows(presented: &Presented) {
    let footer = &presented.footer;
    presented.manager.set_tenxer_mode(true);
    settle();
    assert!(footer.tag.is_visible());
    assert_eq!(footer.tag.text(), crate::ui::tenxer_mode::TAG_TEXT);
    assert!(
        shows_only_the_pill(footer),
        "the footer shows only the pill"
    );
    let tooltip = footer.tag.tooltip_text();
    assert!(tooltip.is_none());
    assert_rows(
        &reference_labels(footer),
        &[
            "Toggle 10xer mode",
            "Open the focused directory",
            "Open the next column for the focused directory",
            "Move half a page",
        ],
        &[
            "Preview a file, or open a folder",
            "Move spatially between tiles",
        ],
    );
}

fn the_mode_category_lists_tenxer_commands(footer: &ShortcutFooter) {
    category_named(footer, "10xer mode").emit_clicked();
    let mode_shortcuts = reference_labels(footer);
    for (keys, action) in [
        ("v / V", "Visual select / visual unset"),
        ("g g", "First item"),
        ("g Space", "Go to a folder, typed path, or URI"),
        ("s", "Search this folder and its subfolders"),
        ("f", "Filter this listing"),
        ("c c / c n", "Copy path / name"),
        ("M / C", "Move / copy to a typed folder"),
        ("Ctrl+R", "Invert the selection"),
        ("Space", "Toggle the focused item and move down"),
        ("i", "Open the next column for the focused directory"),
    ] {
        assert!(reference_pairs(&mode_shortcuts, keys, action), "{keys}");
    }
    assert!(!has_label(&mode_shortcuts, "Ctrl+C / Ctrl+X"));
}

fn icons_rows_replace_column_rows(footer: &ShortcutFooter) {
    footer.category_buttons()[0].emit_clicked();
    footer.set_mode(BrowserMode::Icons);
    assert_rows(
        &reference_labels(footer),
        &[
            "Move spatially between tiles",
            "Toggle folder peek for the focused directory",
        ],
        &[
            "Open the focused directory",
            "Open the next column for the focused directory",
        ],
    );
}

fn opening_the_reference_focuses_search_and_tab_cycles(presented: &Presented) {
    let footer = &presented.footer;
    footer.handle_key(&press(gdk::Key::F1, NONE));
    settle();
    assert!(footer.session.is_open());
    assert!(backdrop_shown(&presented.overlay));
    assert!(
        focus_within(&presented.window, &footer.search),
        "the open reference takes keyboard focus"
    );
    for _ in 0..4 {
        press_reference(footer, gdk::Key::Tab);
        settle();
        assert!(footer.category_buttons()[0].has_focus());
        press_reference(footer, gdk::Key::Tab);
        settle();
        assert!(footer.scroll.has_focus());
        press_reference(footer, gdk::Key::Tab);
        settle();
        assert!(focus_within(&presented.window, &footer.search));
    }
}

fn search_and_categories_filter_rows(footer: &ShortcutFooter) {
    footer.search.set_text("half a page");
    settle();
    assert_rows(
        &reference_labels(footer),
        &["Move half a page"],
        &["Toggle 10xer mode"],
    );
    footer.search.set_text("");
    settle();
    let places = footer.category_buttons()[2].clone();
    places.emit_clicked();
    assert_rows(
        &reference_labels(footer),
        &["Home / ~/.config"],
        &["Move half a page"],
    );
    footer.search.grab_focus();
    for _ in 0..2 {
        press_reference(footer, gdk::Key::Tab);
        settle();
        assert!(places.has_focus());
        press_reference(footer, gdk::Key::Tab);
        settle();
        assert!(footer.scroll.has_focus());
        press_reference(footer, gdk::Key::Tab);
        settle();
    }
    footer.category_buttons()[0].emit_clicked();
}

fn vim_keys_move_between_categories(footer: &ShortcutFooter) {
    expect_key(footer, gdk::Key::b, CTRL, STOP);
    assert!(footer.category_buttons()[0].has_focus());
    expect_key(footer, gdk::Key::j, NONE, STOP);
    assert_rows(
        &reference_labels(footer),
        &["Move half a page"],
        &["Toggle 10xer mode"],
    );
    expect_key(footer, gdk::Key::k, NONE, STOP);
    assert!(has_label(&reference_labels(footer), "Toggle 10xer mode"));
    expect_key(footer, gdk::Key::Right, NONE, STOP);
    assert!(footer.category_buttons()[1].has_focus());
    expect_key(footer, gdk::Key::h, NONE, STOP);
    assert!(footer.category_buttons()[0].has_focus());
}

fn list_keys_scroll_and_ctrl_f_l_move_focus(presented: &Presented) {
    let footer = &presented.footer;
    expect_key(footer, gdk::Key::Tab, NONE, STOP);
    assert!(footer.scroll.has_focus());
    let before = footer.scroll.vadjustment().value();
    expect_key(footer, gdk::Key::j, NONE, STOP);
    assert!(footer.scroll.vadjustment().value() > before);
    let scrolled = footer.scroll.vadjustment().value();
    press_reference(footer, gdk::Key::h);
    assert_eq!(
        footer.scroll.vadjustment().value(),
        scrolled,
        "h does not scroll the list"
    );
    expect_key(footer, gdk::Key::f, CTRL, STOP);
    assert!(focus_within(&presented.window, &footer.search));
    expect_key(footer, gdk::Key::l, CTRL, STOP);
    assert!(footer.scroll.has_focus());
    expect_key(footer, gdk::Key::f, CTRL, STOP);
    expect_key(footer, gdk::Key::Tab, gdk::ModifierType::SHIFT_MASK, STOP);
    assert!(footer.scroll.has_focus());
    presented.files.grab_focus();
    assert!(focus_within(&presented.window, &footer.search));
}

fn f1_closes_the_reference_and_tilde_reopens_it(presented: &Presented) {
    let footer = &presented.footer;
    press_reference(footer, gdk::Key::F1);
    settle();
    assert!(
        !footer.session.is_open(),
        "F1 from the open reference closes it"
    );
    assert!(!backdrop_shown(&presented.overlay));
    assert!(focus_within(&presented.window, &presented.files));
    footer.handle_key(&press(gdk::Key::asciitilde, NONE));
    settle();
    assert!(footer.session.is_open());
    footer.search.grab_focus();
    settle();
    expect_key(
        footer,
        gdk::Key::Delete,
        NONE,
        Some(glib::Propagation::Proceed),
    );
    footer.handle_key(&press(gdk::Key::Escape, NONE));
    settle();
    assert!(!footer.session.is_open());
}

fn a_list_footer_lists_its_own_rows(manager: &PreferenceManager) -> ShortcutFooter {
    let other = ShortcutFooter::new(BrowserMode::List);
    other.bind_preferences(manager);
    let list = reference_labels(&other);
    assert_rows(
        &list,
        &["Toggle 10xer mode", "Open the focused directory"],
        &["Open the next column for the focused directory"],
    );
    for (keys, action, why) in [
        (
            "Space",
            "Toggle the focused item and move down",
            "the List reference lists Space for the toggle action",
        ),
        (
            "Ctrl+R",
            "Invert the selection",
            "the List reference lists Ctrl+R for invert",
        ),
        (
            "v / V",
            "Visual select / visual unset",
            "the List reference lists v and V for visual ranges",
        ),
        (
            "i",
            "Toggle folder peek for the focused directory",
            "the List reference lists i for folder peek",
        ),
    ] {
        assert!(reference_pairs(&list, keys, action), "{why}");
    }
    other
}

fn a_chooser_footer_refuses_unavailable_rows(manager: &PreferenceManager) {
    let chooser = ShortcutFooter::new(BrowserMode::List);
    chooser.bind_preferences(manager);
    chooser.set_chooser(crate::ui::shortcut_reference::ChooserScope {
        request: crate::ui::shortcut_reference::ChooserRequest::Files,
        multiple: false,
    });
    let refused = reference_labels(&chooser);
    assert!(reference_pairs(
        &refused,
        "Enter / o on a file",
        "Choose the file"
    ));
    assert!(has_label(&refused, "Toggle 10xer mode"));
    for unavailable in [
        "Yank / cut the selection, or the focused item",
        "Close the current window",
        "Toggle folder peek for the focused directory",
        "Toggle the focused item and move down",
        "Open global search",
    ] {
        assert!(
            !has_label(&refused, unavailable),
            "the chooser refuses {unavailable:?}"
        );
    }
    category_named(&chooser, "10xer mode").emit_clicked();
    let chooser_mode = reference_labels(&chooser);
    assert!(reference_pairs(
        &chooser_mode,
        "s",
        "Search this folder and its subfolders"
    ));
    assert_rows(&chooser_mode, &[], &["v / V", "M / C"]);
}

fn leaving_tenxer_restores_default_rows(presented: &Presented, other: &ShortcutFooter) {
    let footer = &presented.footer;
    presented.manager.set_tenxer_mode(false);
    settle();
    assert!(!footer.tag.is_visible());
    assert!(footer.tag.tooltip_text().is_none());
    for shown in [footer, other] {
        let labels = reference_labels(shown);
        assert!(reference_pairs(
            &labels,
            "Space",
            "Preview a file, or open a folder"
        ));
        assert_rows(
            &labels,
            &[],
            &["Toggle the focused item and move down", "Toggle 10xer mode"],
        );
    }
}

#[test]
fn tenxer_reference_follows_the_active_map() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::reference::tenxer_reference_follows_the_active_map",
        || {
            let presented = present_footer_beside_files();
            the_default_map_hides_tenxer_rows(&presented);
            entering_tenxer_shows_the_pill_and_column_rows(&presented);
            the_mode_category_lists_tenxer_commands(&presented.footer);
            icons_rows_replace_column_rows(&presented.footer);
            opening_the_reference_focuses_search_and_tab_cycles(&presented);
            search_and_categories_filter_rows(&presented.footer);
            vim_keys_move_between_categories(&presented.footer);
            list_keys_scroll_and_ctrl_f_l_move_focus(&presented);
            f1_closes_the_reference_and_tilde_reopens_it(&presented);
            let other = a_list_footer_lists_its_own_rows(&presented.manager);
            a_chooser_footer_refuses_unavailable_rows(&presented.manager);
            leaving_tenxer_restores_default_rows(&presented, &other);
            presented.window.destroy();
        },
    );
}

#[test]
fn reference_follows_keymap_overrides_without_reopening() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::reference::reference_follows_keymap_overrides_without_reopening",
        || {
            use crate::ui::keys::{
                Binding, Map,
                bindings::{SEARCH, TAB_NEXT},
            };
            let manager = PreferenceManager::shared();
            manager.set_tenxer_mode(false);
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            footer.bind_preferences(&manager);
            let labels = reference_labels(&footer);
            assert!(
                reference_pairs(&labels, "Ctrl+K", "Open global search"),
                "{labels:?}"
            );
            manager.set_bindings(Map::Default, SEARCH, Vec::new());
            manager.set_bindings(
                Map::Default,
                TAB_NEXT,
                vec![
                    Binding::ctrl(crate::ui::keys::Key::Tab),
                    Binding::ctrl(crate::ui::keys::Key::Page_Down),
                    Binding::alt(crate::ui::keys::Key::n),
                ],
            );
            let labels = reference_labels(&footer);
            assert!(
                !has_label(&labels, "Open global search"),
                "an unbound command has no row: {labels:?}"
            );
            assert!(
                reference_pairs(
                    &labels,
                    "Ctrl+Tab / Ctrl+Page Down / Alt+N / Ctrl+Shift+Tab",
                    "Next / previous tab"
                ),
                "{labels:?}"
            );
            assert!(
                reference_pairs(&labels, "Ctrl+Page Up", "Previous / next tab"),
                "{labels:?}"
            );

            manager.set_type_to_search(true);
            manager.set_bindings(
                Map::Default,
                SEARCH,
                vec![
                    Binding::ctrl(crate::ui::keys::Key::k),
                    Binding::plain(crate::ui::keys::Key::e),
                ],
            );
            let labels = reference_labels(&footer);
            assert!(
                reference_pairs(&labels, "Ctrl+K", "Open global search"),
                "type-to-search types the added key: {labels:?}"
            );
            manager.set_type_to_search(false);
            let labels = reference_labels(&footer);
            assert!(
                reference_pairs(&labels, "Ctrl+K / e", "Open global search"),
                "{labels:?}"
            );
        },
    );
}
