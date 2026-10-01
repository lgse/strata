// SPDX-License-Identifier: MIT

use super::acceptance::{request, wait_until};
use super::*;
use crate::ui::browser_modes::BrowserMode;
use gtk::gdk::{Key, ModifierType};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

type Response = ashpd::backend::Result<SelectedFiles>;

struct Chooser {
    state: Rc<ChooserState>,
    response: Rc<RefCell<Option<Response>>>,
    responses: Rc<Cell<usize>>,
    keys: gtk::EventControllerKey,
    root: tempfile::TempDir,
}

impl Chooser {
    fn open(kind: ChooserKind, mode: BrowserMode, tree: &[&str]) -> Self {
        Self::open_with(kind, mode, tree, true)
    }

    fn open_with(kind: ChooserKind, mode: BrowserMode, tree: &[&str], tenxer: bool) -> Self {
        crate::ui::prepare_portal_ui();
        let preferences = PreferenceManager::shared();
        preferences.set_browser_mode(mode);
        preferences.set_tenxer_mode(tenxer);
        let root = tempfile::tempdir().expect("fixture");
        for path in tree {
            let target = root.path().join(path.trim_end_matches('/'));
            if path.ends_with('/') {
                std::fs::create_dir_all(&target).expect("fixture folder");
            } else {
                std::fs::write(&target, path.as_bytes()).expect("fixture file");
            }
        }
        let mut chooser_request = request(root.path().to_path_buf());
        chooser_request.kind = kind;
        let response = Rc::new(RefCell::new(None));
        let responses = Rc::new(Cell::new(0));
        let (received, counted) = (response.clone(), responses.clone());
        let state = build_chooser(
            chooser_request,
            Arc::new(AtomicBool::new(false)),
            move |value| {
                counted.set(counted.get() + 1);
                received.replace(Some(value));
            },
        )
        .expect("chooser");
        let browser = state.view.browser();
        wait_until(|| {
            browser
                .column_snapshot(0)
                .is_some_and(|column| !column.loading)
        });
        let controllers = state.window.observe_controllers();
        let keys = (0..controllers.n_items())
            .find_map(|index| {
                controllers
                    .item(index)
                    .and_downcast::<gtk::EventControllerKey>()
            })
            .expect("chooser key controller");
        Self {
            state,
            response,
            responses,
            keys,
            root,
        }
    }

    fn press(&self, key: Key) -> bool {
        self.press_with(key, ModifierType::empty())
    }

    fn press_with(&self, key: Key, modifiers: ModifierType) -> bool {
        self.keys
            .emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
    }

    fn focus_files(&self) {
        self.state.view.browser().focus_active();
        wait_until(|| self.state.view.item_view_has_focus());
    }

    fn cursor_name(&self) -> Option<String> {
        self.state
            .view
            .focused_target()
            .map(|entry| entry.display_name)
    }

    fn move_to(&self, name: &str) {
        let next = if self.state.view.view_mode() == BrowserMode::Icons {
            Key::l
        } else {
            Key::j
        };
        self.press(Key::Home);
        for _ in 0..12 {
            if self.cursor_name().as_deref() == Some(name) {
                return;
            }
            self.press(next);
        }
        panic!("cursor never reached {name}: {:?}", self.cursor_name());
    }

    fn feedback(&self) -> String {
        widget_with_class(self.state.window.upcast_ref(), "shortcut-footer-feedback")
            .and_downcast::<gtk::Label>()
            .map(|label| label.text().to_string())
            .unwrap_or_default()
    }

    fn prompt(&self) -> gtk::Entry {
        widget_with_class(self.state.window.upcast_ref(), "shortcut-footer-prompt")
            .and_downcast::<gtk::Entry>()
            .expect("footer prompt")
    }

    fn footer_visible(&self) -> bool {
        widget_with_class(self.state.window.upcast_ref(), "chooser-footer")
            .is_some_and(|footer| footer.is_visible())
    }

    fn prompt_focused(&self) -> bool {
        let prompt = self.prompt();
        gtk::prelude::RootExt::focus(&self.state.window)
            .is_some_and(|focus| focus == prompt || focus.is_ancestor(&prompt))
    }

    fn uri(&self, name: &str) -> String {
        gio::File::for_path(self.root.path().join(name))
            .uri()
            .to_string()
    }

    fn chosen(&self) -> Vec<String> {
        wait_until(|| self.response.borrow().is_some());
        let selected = self
            .response
            .borrow_mut()
            .take()
            .expect("response")
            .expect("accepted");
        selected.uris().iter().map(|uri| uri.to_string()).collect()
    }

    fn cancelled(&self) -> bool {
        matches!(
            self.response.borrow().as_ref(),
            Some(Err(PortalError::Cancelled(_)))
        )
    }

    fn open_request(&self) -> bool {
        self.responses.get() == 0 && self.state.window.is_visible()
    }
}

impl Drop for Chooser {
    fn drop(&mut self) {
        self.state.window.close();
    }
}

fn widget_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = widget_with_class(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn single_file() -> ChooserKind {
    ChooserKind::Open {
        directory: false,
        multiple: false,
    }
}

#[test]
fn tenxer_keys_choose_the_cursor_file_in_every_view() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::tenxer_keys_choose_the_cursor_file_in_every_view",
        || {
            for (mode, confirm) in [
                (BrowserMode::Columns, Key::Return),
                (BrowserMode::List, Key::o),
                (BrowserMode::Icons, Key::Return),
            ] {
                let chooser =
                    Chooser::open(single_file(), mode, &["alpha.txt", "beta.txt", "folder/"]);
                wait_until(|| chooser.state.view.item_view_has_focus());
                assert!(chooser.footer_visible(), "{mode:?}");
                chooser.move_to("beta.txt");
                assert!(chooser.open_request(), "{mode:?} motion chose a file");

                assert!(chooser.press(confirm));
                assert_eq!(chooser.chosen(), [chooser.uri("beta.txt")], "{mode:?}");
                chooser.press(confirm);
                chooser.press(Key::Escape);
                assert_eq!(chooser.responses.get(), 1, "{mode:?} responded twice");
            }
        },
    );
}

#[test]
fn folder_keys_navigate_and_ctrl_enter_chooses_the_cursor_folder() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::folder_keys_navigate_and_ctrl_enter_chooses_the_cursor_folder",
        || {
            let chooser = Chooser::open(
                ChooserKind::Open {
                    directory: true,
                    multiple: false,
                },
                BrowserMode::List,
                &["alpha/", "beta/", "beta/inner/", "notes.txt"],
            );
            chooser.focus_files();
            chooser.move_to("beta");
            chooser.press(Key::Return);
            let browser = chooser.state.view.browser();
            let beta = Location::local(chooser.root.path().join("beta"));
            wait_until(|| browser.active_location().as_ref() == Some(&beta));
            assert!(chooser.open_request(), "Enter on a folder chose it");

            chooser.press(Key::BackSpace);
            wait_until(|| {
                browser.active_location() == Some(Location::local(chooser.root.path()))
                    && browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
            });
            chooser.focus_files();
            chooser.move_to("alpha");
            assert!(chooser.press_with(Key::Return, ModifierType::CONTROL_MASK));
            assert_eq!(chooser.chosen(), [chooser.uri("alpha")]);
        },
    );
}

#[test]
fn multiple_requests_fill_with_space_and_choose_the_fill() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::multiple_requests_fill_with_space_and_choose_the_fill",
        || {
            let multiple = || ChooserKind::Open {
                directory: false,
                multiple: true,
            };
            let tree = ["a.txt", "b.txt", "c.txt"];

            let chooser = Chooser::open(multiple(), BrowserMode::List, &tree);
            chooser.focus_files();
            chooser.move_to("a.txt");
            chooser.press(Key::space);
            chooser.press(Key::space);
            assert_eq!(chooser.cursor_name().as_deref(), Some("c.txt"));
            assert!(chooser.press(Key::Return));
            assert_eq!(
                chooser.chosen(),
                [chooser.uri("a.txt"), chooser.uri("b.txt")]
            );
            drop(chooser);

            let chooser = Chooser::open(multiple(), BrowserMode::List, &tree);
            chooser.focus_files();
            chooser.move_to("c.txt");
            assert!(chooser.press(Key::Return));
            assert_eq!(chooser.chosen(), [chooser.uri("c.txt")]);
        },
    );
}

#[test]
fn chooser_refuses_commands_its_request_does_not_allow() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::chooser_refuses_commands_its_request_does_not_allow",
        || {
            let chooser = Chooser::open(
                single_file(),
                BrowserMode::List,
                &["a.txt", "b.txt", "sub/"],
            );
            chooser.focus_files();
            chooser.move_to("a.txt");
            let browser = chooser.state.view.browser();
            let origin = browser.active_location();
            for (key, modifiers, feedback) in [
                (
                    Key::y,
                    ModifierType::empty(),
                    "Not available in the file chooser",
                ),
                (
                    Key::p,
                    ModifierType::empty(),
                    "Not available in the file chooser",
                ),
                (
                    Key::O,
                    ModifierType::SHIFT_MASK,
                    "Not available in the file chooser",
                ),
                (
                    Key::M,
                    ModifierType::SHIFT_MASK,
                    "Not available in the file chooser",
                ),
                (
                    Key::C,
                    ModifierType::SHIFT_MASK,
                    "Not available in the file chooser",
                ),
                (
                    Key::R,
                    ModifierType::SHIFT_MASK,
                    "Not available in the file chooser",
                ),
                (
                    Key::semicolon,
                    ModifierType::empty(),
                    "Not available in the file chooser",
                ),
                (
                    Key::i,
                    ModifierType::empty(),
                    "Not available in the file chooser",
                ),
                (
                    Key::Q,
                    ModifierType::SHIFT_MASK,
                    "Not available in the file chooser",
                ),
                (
                    Key::v,
                    ModifierType::CONTROL_MASK,
                    "Not available in the file chooser",
                ),
                (
                    Key::space,
                    ModifierType::empty(),
                    "Only one item can be chosen",
                ),
                (Key::v, ModifierType::empty(), "Only one item can be chosen"),
                (
                    Key::a,
                    ModifierType::CONTROL_MASK,
                    "Only one item can be chosen",
                ),
            ] {
                assert!(chooser.press_with(key, modifiers), "{key:?}");
                assert_eq!(chooser.feedback(), feedback, "{key:?}");
                assert!(chooser.open_request(), "{key:?} ended the request");
                assert!(browser.selected_entries().is_empty(), "{key:?} filled");
            }
            assert_eq!(chooser.cursor_name().as_deref(), Some("a.txt"));

            chooser.press(Key::g);
            chooser.press(Key::t);
            assert_eq!(chooser.feedback(), "Only local folders can be opened here");
            chooser.press(Key::g);
            chooser.press(Key::n);
            assert_eq!(browser.active_location(), origin);
            for (key, modifiers) in [
                (Key::plus, ModifierType::SHIFT_MASK),
                (Key::minus, ModifierType::empty()),
            ] {
                chooser.press(Key::g);
                assert!(chooser.press_with(key, modifiers), "g {key:?}");
                assert_eq!(
                    chooser.feedback(),
                    "Not available in the file chooser",
                    "g {key:?}"
                );
            }
            assert!(chooser.open_request());
        },
    );
}

#[test]
fn escape_dismisses_one_interaction_before_cancelling_and_the_toggle_keeps_the_request() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::escape_dismisses_one_interaction_before_cancelling_and_the_toggle_keeps_the_request",
        || {
            let chooser = Chooser::open(single_file(), BrowserMode::List, &["a.txt", "b.txt"]);
            chooser.focus_files();
            chooser.move_to("b.txt");
            assert!(chooser.press(Key::slash));
            wait_until(|| chooser.prompt_focused());
            assert!(chooser.press(Key::Escape));
            assert!(chooser.open_request(), "Esc in the prompt cancelled");
            assert!(chooser.state.view.item_view_has_focus());

            chooser.press(Key::l);
            wait_until(|| {
                gtk::prelude::RootExt::focus(&chooser.state.window).is_some()
                    && !chooser.state.view.item_view_has_focus()
            });
            assert!(chooser.press(Key::Escape));
            assert!(chooser.open_request(), "Esc in the preview cancelled");
            assert!(chooser.press(Key::Escape));
            assert!(chooser.cancelled());
            drop(chooser);

            let chooser = Chooser::open(single_file(), BrowserMode::List, &["a.txt"]);
            chooser.focus_files();
            assert!(chooser.press(Key::q));
            assert!(PreferenceManager::shared().tenxer_mode(), "q left the mode");
            assert!(chooser.press_with(
                Key::m,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            assert!(!PreferenceManager::shared().tenxer_mode());
            assert!(
                chooser.open_request(),
                "leaving the mode cancelled the request"
            );
            wait_until(|| !chooser.footer_visible());
            chooser.press(Key::Escape);
            assert!(chooser.cancelled());
            assert_eq!(chooser.responses.get(), 1);
        },
    );
}

#[test]
fn live_mode_changes_switch_the_chooser_between_maps() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::live_mode_changes_switch_the_chooser_between_maps",
        || {
            let chooser = Chooser::open_with(single_file(), BrowserMode::List, &["a.txt"], false);
            chooser.focus_files();
            assert!(!chooser.footer_visible());
            chooser.press(Key::slash);
            assert!(
                !chooser.prompt().is_mapped(),
                "the default map opened a prompt"
            );

            assert!(chooser.press_with(
                Key::M,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            assert!(PreferenceManager::shared().tenxer_mode());
            wait_until(|| chooser.footer_visible());
            chooser.focus_files();
            assert!(chooser.press(Key::slash));
            wait_until(|| chooser.prompt_focused());

            PreferenceManager::shared().set_tenxer_mode(false);
            wait_until(|| !chooser.footer_visible());
            assert!(chooser.prompt().text().is_empty());
            assert!(chooser.open_request());
        },
    );
}

#[test]
fn folder_only_search_hits_exclude_files_and_choose_the_focused_folder() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::folder_only_search_hits_exclude_files_and_choose_the_focused_folder",
        || {
            let chooser = Chooser::open(
                ChooserKind::Open {
                    directory: true,
                    multiple: false,
                },
                BrowserMode::List,
                &[
                    "docs/",
                    "docs/nested/",
                    "docs/nested.txt",
                    "nested-note.txt",
                ],
            );
            chooser.focus_files();
            assert!(chooser.press(Key::s));
            wait_until(|| chooser.prompt_focused());
            chooser.prompt().set_text("nest");
            assert!(chooser.press(Key::Return));
            wait_until(|| {
                chooser
                    .state
                    .view
                    .focused_target()
                    .is_some_and(|entry| entry.display_name == "nested")
            });
            let hits = chooser.state.view.selected_search_results();
            assert!(
                hits.iter().flatten().all(|entry| entry.is_directory()),
                "{hits:?}"
            );
            assert!(chooser.press(Key::y));
            assert!(chooser.open_request());
            assert!(chooser.press_with(Key::Return, ModifierType::CONTROL_MASK));
            assert_eq!(chooser.chosen(), [chooser.uri("docs/nested")]);
        },
    );
}

#[test]
fn save_starts_in_the_files_r_edits_the_name_and_enter_saves_here() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::save_starts_in_the_files_r_edits_the_name_and_enter_saves_here",
        || {
            let chooser = Chooser::open(
                ChooserKind::SaveFile {
                    current_name: Some("output.txt".into()),
                },
                BrowserMode::List,
                &["existing.txt", "folder/", "other.txt"],
            );
            let name = chooser.state.filename.clone().expect("name field");
            let name_focused = || {
                gtk::prelude::RootExt::focus(&chooser.state.window)
                    .is_some_and(|focus| focus.is_ancestor(&name))
            };
            wait_until(|| chooser.state.view.item_view_has_focus());
            let hints = widget_with_class(chooser.state.window.upcast_ref(), "chooser-save-hints")
                .expect("save hints");
            assert!(hints.is_visible());
            PreferenceManager::shared().set_tenxer_mode(false);
            assert!(!hints.is_visible());
            PreferenceManager::shared().set_tenxer_mode(true);
            assert!(hints.is_visible());
            chooser.move_to("other.txt");
            chooser.move_to("folder");
            assert_eq!(name.text(), "output.txt", "the cursor renamed the file");

            assert!(chooser.press(Key::r));
            assert!(name_focused());
            assert_eq!(name.selection_bounds(), Some((0, 6)));
            for (key, modifiers) in [
                (Key::q, ModifierType::empty()),
                (Key::Q, ModifierType::SHIFT_MASK),
                (Key::j, ModifierType::empty()),
                (Key::r, ModifierType::empty()),
                (Key::a, ModifierType::CONTROL_MASK),
            ] {
                assert!(!chooser.press_with(key, modifiers), "{key:?} was not typed");
            }
            assert!(PreferenceManager::shared().tenxer_mode());
            name.set_text("report.txt");
            assert!(chooser.press(Key::Escape));
            assert!(chooser.state.view.item_view_has_focus());
            assert_eq!(chooser.cursor_name().as_deref(), Some("folder"));
            assert_eq!(name.text(), "report.txt");
            assert!(chooser.open_request());

            assert!(chooser.press(Key::Return));
            assert_eq!(chooser.chosen(), [chooser.uri("report.txt")]);
        },
    );
}

#[test]
fn save_name_and_location_do_not_dispatch_listing_shortcuts() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::save_name_and_location_do_not_dispatch_listing_shortcuts",
        || {
            let chooser = Chooser::open(
                ChooserKind::SaveFile {
                    current_name: Some("output.txt".into()),
                },
                BrowserMode::List,
                &["existing.txt"],
            );
            let name = chooser.state.filename.clone().expect("name field");
            let location = widget_with_class(chooser.state.window.upcast_ref(), "location-entry")
                .and_downcast::<gtk::Entry>()
                .expect("location field");
            let browser = chooser.state.view.browser();
            let original_location = browser.active_location();
            for entry in [&name, &location] {
                if entry == &location {
                    chooser.state.view.begin_location_edit();
                } else {
                    entry.grab_focus();
                }
                let has_focus = || {
                    gtk::prelude::RootExt::focus(&chooser.state.window)
                        .is_some_and(|focus| focus == *entry || focus.is_ancestor(entry))
                };
                wait_until(has_focus);
                for (key, modifiers) in [
                    (Key::f, ModifierType::CONTROL_MASK),
                    (Key::l, ModifierType::CONTROL_MASK),
                    (Key::h, ModifierType::CONTROL_MASK),
                    (
                        Key::b,
                        ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
                    ),
                    (Key::n, ModifierType::CONTROL_MASK),
                    (
                        Key::n,
                        ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
                    ),
                    (Key::_1, ModifierType::CONTROL_MASK),
                    (Key::F5, ModifierType::empty()),
                    (Key::space, ModifierType::empty()),
                ] {
                    assert!(!chooser.press_with(key, modifiers), "{key:?} intercepted");
                    assert!(has_focus(), "{key:?} moved focus");
                    assert_eq!(browser.active_location(), original_location);
                    assert_eq!(chooser.state.view.view_mode(), BrowserMode::List);
                    assert!(!chooser.state.view.filter_has_focus());
                    assert!(chooser.open_request());
                }
                assert!(chooser.press(Key::Escape));
                assert!(chooser.state.view.item_view_has_focus());
            }
        },
    );
}

#[test]
fn saving_over_an_existing_file_confirms_with_cancel_focused() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::saving_over_an_existing_file_confirms_with_cancel_focused",
        || {
            let chooser = Chooser::open(
                ChooserKind::SaveFile {
                    current_name: Some("output.txt".into()),
                },
                BrowserMode::List,
                &["existing.txt"],
            );
            chooser.focus_files();
            let focused_button = || {
                gtk::prelude::RootExt::focus(&chooser.state.window).and_then(|focus| {
                    focus
                        .clone()
                        .downcast::<gtk::Button>()
                        .ok()
                        .or_else(|| focus.ancestor(gtk::Button::static_type()).and_downcast())
                })
            };
            let confirm = || {
                chooser.move_to("existing.txt");
                assert!(chooser.press(Key::o));
                wait_until(|| {
                    visible_modal_layer(&chooser.state.window).is_some()
                        && focused_button().is_some()
                });
                focused_button().expect("focused confirmation button")
            };

            let cancel = confirm();
            assert!(cancel.has_css_class("action-dialog-cancel"));
            cancel.emit_clicked();
            wait_until(|| visible_modal_layer(&chooser.state.window).is_none());
            assert!(chooser.open_request(), "Cancel ended the request");

            chooser.focus_files();
            confirm();
            let layer = visible_modal_layer(&chooser.state.window).expect("confirmation");
            let replace = widget_with_class(&layer, "action-dialog-confirm")
                .and_downcast::<gtk::Button>()
                .expect("Replace");
            replace.emit_clicked();
            assert_eq!(chooser.chosen(), [chooser.uri("existing.txt")]);
            assert_eq!(chooser.responses.get(), 1);
            assert_eq!(
                std::fs::read(chooser.root.path().join("existing.txt")).expect("unchanged"),
                b"existing.txt"
            );
        },
    );
}

#[test]
fn footer_create_and_rename_keep_the_request_open() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::keyboard::footer_create_and_rename_keep_the_request_open",
        || {
            let chooser = Chooser::open(single_file(), BrowserMode::List, &["draft.txt"]);
            chooser.focus_files();
            chooser.move_to("draft.txt");

            assert!(chooser.press(Key::r));
            wait_until(|| chooser.prompt_focused());
            assert_eq!(chooser.prompt().text(), "draft.txt");
            assert!(chooser.press(Key::Escape));
            assert!(chooser.state.view.item_view_has_focus());
            assert!(chooser.root.path().join("draft.txt").exists());

            assert!(chooser.press(Key::r));
            wait_until(|| chooser.prompt_focused());
            chooser.prompt().set_text("final.txt");
            assert!(chooser.press(Key::Return));
            wait_until(|| chooser.root.path().join("final.txt").exists());

            chooser.focus_files();
            assert!(chooser.press(Key::a));
            wait_until(|| chooser.prompt_focused());
            chooser.prompt().set_text("made/");
            assert!(chooser.press(Key::Return));
            wait_until(|| chooser.root.path().join("made").is_dir());
            assert!(chooser.open_request());
            assert!(!chooser.root.path().join("draft.txt").exists());
        },
    );
}
