// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{ArchiveDirectory, ArchiveNode, ArchivePreviewTree};

const LONG_LINES: usize = 600;

/// Serves a long document, an archive tree, and a password-protected archive.
struct OwnershipPreview;

impl PreviewProvider for OwnershipPreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        glib::idle_add_local_once(move || {
            let name = request.entry.native_name.to_string_lossy().into_owned();
            if name == "locked.zip" && request.archive_password.is_none() {
                emit(PreviewEvent::NeedsPassword {
                    request_id: request.id,
                    entry: request.entry,
                });
                return;
            }
            let content = if name.ends_with(".zip") {
                PreviewContent::Archive {
                    tree: ArchivePreviewTree {
                        root: ArchiveDirectory {
                            name: String::new(),
                            children: vec![
                                ArchiveNode::Directory(ArchiveDirectory {
                                    name: "docs".into(),
                                    children: vec![ArchiveNode::File {
                                        name: "inside.txt".into(),
                                        size: 3,
                                    }],
                                }),
                                ArchiveNode::File {
                                    name: "top.txt".into(),
                                    size: 3,
                                },
                            ],
                        },
                        file_count: 2,
                    },
                }
            } else if name == "short.txt" {
                PreviewContent::Text {
                    content: "short one\nshort two\n".into(),
                    truncated: false,
                }
            } else {
                PreviewContent::Text {
                    content: (0..LONG_LINES)
                        .map(|line| format!("line {line}\n"))
                        .collect(),
                    truncated: false,
                }
            };
            emit(PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry,
                content_type: "text/plain".into(),
                content,
            }))
        });
        LoadHandle::new(|| {})
    }
}

fn ownership_fixture() -> KeyboardFixture {
    let fixture = KeyboardFixture::with_provider(Rc::new(OwnershipPreview));
    let root = fixture._directory.path();
    std::fs::write(root.join("long.txt"), b"long").expect("long document");
    std::fs::write(root.join("short.txt"), b"short").expect("short document");
    std::fs::write(root.join("bundle.zip"), b"zip").expect("archive");
    std::fs::write(root.join("locked.zip"), b"zip").expect("locked archive");
    std::fs::write(root.join("package.deb"), b"!<arch>").expect("unsupported file");
    std::fs::create_dir(root.join("empty")).expect("empty folder");
    let preferences = PreferenceManager::shared();
    preferences.set_tenxer_mode(true);
    preferences.set_group_by_type(false);
    let browser = fixture.view.browser();
    // The window wires cursor-follow; the shared fixture does not.
    fixture.preview.observe_browser(&browser);
    fixture.view.refresh();
    wait_loaded(&browser, 0);
    wait_until(|| entry_count(&browser) == 9);
    browser.set_folders_first(0, false);
    browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
    wait_until(|| {
        let names = source_names(&browser);
        names.len() == 9 && names.is_sorted()
    });
    fixture.view.set_view_mode(BrowserMode::List);
    wait_until(|| list_display_names(&fixture.view.widget()) == source_names(&browser));
    fixture
}

fn record_opens(browser: &crate::app::Browser) -> Rc<RefCell<Vec<crate::model::Location>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let record = opened.clone();
    browser.observe(move |event| {
        if let BrowserEvent::OpenRequested { location } = event {
            record.borrow_mut().push(location.clone());
        }
    });
    opened
}

fn document_scroll(fixture: &KeyboardFixture) -> f64 {
    fixture.preview.document_scroll_value().unwrap_or(-1.0)
}

fn wait_document(fixture: &KeyboardFixture) {
    wait_until(|| {
        fixture.preview.scroll_document(DocumentScroll::End);
        document_scroll(fixture) > 0.0
    });
    fixture.preview.scroll_document(DocumentScroll::Start);
}

fn owner_bar(fixture: &KeyboardFixture) -> bool {
    widget_with_class(&fixture.preview.widget(), "preview-keyboard-owner").is_some()
}

fn focused_has_class(fixture: &KeyboardFixture, class: &str) -> bool {
    gtk::prelude::RootExt::focus(&fixture.window)
        .is_some_and(|focused| focused.has_css_class(class))
}

fn emit_on_focused(fixture: &KeyboardFixture, key: Key, modifiers: ModifierType) {
    let focused = gtk::prelude::RootExt::focus(&fixture.window).expect("focused widget");
    let controllers = focused.observe_controllers();
    for keys in (0..controllers.n_items()).filter_map(|index| {
        controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
    }) {
        if keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]) {
            return;
        }
    }
}

fn clipboard_text(fixture: &KeyboardFixture) -> String {
    let text = Rc::new(RefCell::new(None));
    let slot = text.clone();
    fixture
        .window
        .clipboard()
        .read_text_async(None::<&gtk::gio::Cancellable>, move |result| {
            slot.replace(Some(
                result
                    .ok()
                    .flatten()
                    .map(|text| text.to_string())
                    .unwrap_or_default(),
            ));
        });
    wait_until(|| text.borrow().is_some());
    text.take().unwrap_or_default()
}

fn archive_has_focus(fixture: &KeyboardFixture) -> bool {
    fixture
        .preview
        .archive_list_has_focus(gtk::prelude::RootExt::focus(&fixture.window).as_ref())
}

fn password_has_focus(fixture: &KeyboardFixture) -> bool {
    fixture
        .preview
        .password_has_focus(gtk::prelude::RootExt::focus(&fixture.window).as_ref())
}

#[test]
fn tenxer_preview_owns_document_keys_until_returned() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::preview_ownership::tenxer_preview_owns_document_keys_until_returned",
        || {
            let fixture = ownership_fixture();
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let opened = record_opens(&browser);
            for mode in [BrowserMode::List, BrowserMode::Columns] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                move_to_named(&fixture, &browser, "long.txt");
                let selection = fixture.selected();

                fixture.press(Key::l, ModifierType::empty());
                wait_until(|| fixture.preview.is_open() && preview_has_focus(&fixture));
                wait_document(&fixture);
                if mode == BrowserMode::Columns {
                    assert!(
                        widget_with_class(&fixture.view.widget(), "active-column").is_none(),
                        "the column yields its destination to the preview"
                    );
                }
                for key in [Key::l, Key::Right, Key::Return, Key::o, Key::space] {
                    assert!(
                        fixture.press(key, ModifierType::empty()),
                        "{mode:?} {key:?}"
                    );
                    assert!(
                        preview_has_focus(&fixture),
                        "{mode:?} {key:?} kept the keys"
                    );
                }
                fixture.press(Key::Delete, ModifierType::empty());
                pump(50);
                assert!(
                    !modal_visible(&fixture.overlay),
                    "{mode:?} Delete stays with the preview"
                );
                assert!(
                    opened.borrow().is_empty(),
                    "{mode:?} preview keys never launch"
                );
                assert_eq!(browser.active_location(), origin, "{mode:?}");

                let top = document_scroll(&fixture);
                for (key, modifiers) in [
                    (Key::j, ModifierType::empty()),
                    (Key::Down, ModifierType::empty()),
                    (Key::d, ModifierType::CONTROL_MASK),
                    (Key::f, ModifierType::CONTROL_MASK),
                    (Key::Page_Down, ModifierType::empty()),
                    (Key::J, ModifierType::SHIFT_MASK),
                ] {
                    let before = document_scroll(&fixture);
                    fixture.press(key, modifiers);
                    assert!(
                        document_scroll(&fixture) > before,
                        "{mode:?} {key:?} scrolls down"
                    );
                }
                for (key, modifiers) in [
                    (Key::k, ModifierType::empty()),
                    (Key::Up, ModifierType::empty()),
                    (Key::u, ModifierType::CONTROL_MASK),
                    (Key::b, ModifierType::CONTROL_MASK),
                    (Key::Page_Up, ModifierType::empty()),
                ] {
                    let before = document_scroll(&fixture);
                    fixture.press(key, modifiers);
                    assert!(
                        document_scroll(&fixture) < before,
                        "{mode:?} {key:?} scrolls up"
                    );
                }
                fixture.press(Key::G, ModifierType::SHIFT_MASK);
                let bottom = document_scroll(&fixture);
                fixture.press(Key::End, ModifierType::empty());
                assert_eq!(document_scroll(&fixture), bottom, "{mode:?}");
                assert!(bottom > top);
                fixture.press(Key::Home, ModifierType::empty());
                assert_eq!(document_scroll(&fixture), top, "{mode:?}");
                fixture.press(Key::G, ModifierType::SHIFT_MASK);
                fixture.press(Key::g, ModifierType::empty());
                assert_eq!(fixture.shortcuts.chord_hint().as_deref(), Some("g top"));
                fixture.press(Key::g, ModifierType::empty());
                assert_eq!(document_scroll(&fixture), top, "{mode:?} g g");
                fixture.press(Key::g, ModifierType::empty());
                fixture.press(Key::h, ModifierType::empty());
                assert_eq!(fixture.shortcuts.feedback_text(), "Unknown chord");
                assert!(preview_has_focus(&fixture), "{mode:?} g h stays");
                assert_eq!(browser.active_location(), origin, "{mode:?} g h");
                fixture.shortcuts.dismiss_feedback();
                assert_eq!(focused_name(&browser), "long.txt", "{mode:?}");
                assert_eq!(fixture.selected(), selection, "{mode:?}");

                fixture.press(Key::h, ModifierType::empty());
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(fixture.preview.is_open(), "{mode:?} h keeps the drawer");
                if mode == BrowserMode::Columns {
                    assert!(
                        widget_with_class(&fixture.view.widget(), "active-column").is_some(),
                        "returning restores the column destination"
                    );
                }
                assert_eq!(
                    browser.active_location(),
                    origin,
                    "{mode:?} h did not go up"
                );
                assert_eq!(focused_name(&browser), "long.txt", "{mode:?}");
                fixture.press(Key::J, ModifierType::SHIFT_MASK);
                assert!(
                    document_scroll(&fixture) > top,
                    "{mode:?} J scrolls from the list"
                );
                fixture.press(Key::K, ModifierType::SHIFT_MASK);
                assert_eq!(document_scroll(&fixture), top, "{mode:?}");
                assert!(
                    fixture.view.item_view_has_focus(),
                    "{mode:?} J / K keep focus"
                );

                fixture.press(Key::Right, ModifierType::empty());
                wait_until(|| preview_has_focus(&fixture));
                fixture.press(Key::Tab, ModifierType::SHIFT_MASK);
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(
                    fixture.preview.is_open(),
                    "{mode:?} Shift+Tab keeps the drawer"
                );

                fixture.press(Key::l, ModifierType::empty());
                wait_until(|| preview_has_focus(&fixture));
                fixture.press(Key::Escape, ModifierType::empty());
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(
                    !fixture.preview.is_enabled(),
                    "{mode:?} Esc closes the drawer"
                );

                fixture.press(Key::l, ModifierType::empty());
                wait_until(|| preview_has_focus(&fixture));
                fixture.press(Key::i, ModifierType::empty());
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(
                    !fixture.preview.is_enabled(),
                    "{mode:?} i closes the drawer"
                );
                assert_eq!(focused_name(&browser), "long.txt", "{mode:?}");
                if mode == BrowserMode::Columns {
                    assert!(
                        widget_with_class(&fixture.view.widget(), "active-column").is_some(),
                        "i returns focus to the column"
                    );
                }

                fixture.press(Key::l, ModifierType::empty());
                wait_until(|| preview_has_focus(&fixture));
                PreferenceManager::shared().set_tenxer_mode(false);
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(
                    fixture.preview.is_open(),
                    "{mode:?} mode exit keeps the drawer"
                );
                PreferenceManager::shared().set_tenxer_mode(true);
                fixture.preview.close();

                move_to_named(&fixture, &browser, "package.deb");
                fixture.shortcuts.dismiss_feedback();
                fixture.press(Key::l, ModifierType::empty());
                assert_eq!(fixture.shortcuts.feedback_text(), "Nothing to preview");
                assert!(!fixture.preview.is_enabled(), "{mode:?}");
                assert!(fixture.view.item_view_has_focus(), "{mode:?}");
            }
            assert!(opened.borrow().is_empty());

            fixture.view.set_view_mode(BrowserMode::List);
            wait_loaded(&browser, 0);
            move_to_named(&fixture, &browser, "long.txt");
            fixture.press(Key::l, ModifierType::empty());
            wait_until(|| preview_has_focus(&fixture));
            fixture.press(Key::h, ModifierType::empty());
            wait_until(|| fixture.view.item_view_has_focus());
            fixture.press(Key::h, ModifierType::empty());
            wait_loaded(&browser, 0);
            assert_ne!(
                browser.active_location(),
                origin,
                "a second h goes to the parent"
            );
            fixture.press(Key::H, ModifierType::SHIFT_MASK);
            wait_loaded(&browser, 0);
            assert_eq!(browser.active_location(), origin);

            fixture.preview.close();
            move_to_named(&fixture, &browser, "empty");
            fixture.press(Key::l, ModifierType::empty());
            wait_loaded(&browser, 0);
            assert_eq!(entry_count(&browser), 0);
            focus_files(&fixture);
            fixture.shortcuts.dismiss_feedback();
            fixture.press(Key::l, ModifierType::empty());
            assert_eq!(fixture.shortcuts.feedback_text(), "Nothing to preview");
            assert!(!fixture.preview.is_enabled());
            fixture.press(Key::BackSpace, ModifierType::empty());
            wait_loaded(&browser, 0);

            move_to_named(&fixture, &browser, "long.txt");
            fixture.press(Key::l, ModifierType::empty());
            wait_until(|| focused_has_class(&fixture, "preview-virtual-list"));
            assert!(owner_bar(&fixture));
            assert!(
                !fixture.press(Key::a, ModifierType::CONTROL_MASK),
                "Ctrl+A goes to the document"
            );
            emit_on_focused(&fixture, Key::a, ModifierType::CONTROL_MASK);
            assert!(!fixture.press(Key::c, ModifierType::CONTROL_MASK));
            emit_on_focused(&fixture, Key::c, ModifierType::CONTROL_MASK);
            let copied = clipboard_text(&fixture);
            assert!(
                copied.contains("line 0") && copied.contains(&format!("line {}", LONG_LINES - 1)),
                "the whole document is copied, not listing paths: {copied:?}"
            );
            assert!(fixture.press(Key::i, ModifierType::empty()));
            wait_until(|| fixture.view.item_view_has_focus());

            move_to_named(&fixture, &browser, "short.txt");
            fixture.press(Key::l, ModifierType::empty());
            wait_until(|| focused_has_class(&fixture, "preview-text"));
            assert!(
                fixture.press(Key::j, ModifierType::empty()),
                "source text is a document"
            );
            assert!(focused_has_class(&fixture, "preview-text"));
            assert!(!fixture.press(Key::a, ModifierType::CONTROL_MASK));
            fixture.press(Key::h, ModifierType::empty());
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(fixture.preview.is_open());
            assert_eq!(focused_name(&browser), "short.txt");
        },
    );
}

#[test]
fn tenxer_interactive_previews_keep_a_defined_key_owner() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::preview_ownership::tenxer_interactive_previews_keep_a_defined_key_owner",
        || {
            let fixture = ownership_fixture();
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let before = directory_names(fixture._directory.path());
            let opened = record_opens(&browser);
            fixture.view.set_view_mode(BrowserMode::List);
            wait_loaded(&browser, 0);

            move_to_named(&fixture, &browser, "bundle.zip");
            let selection = fixture.selected();
            fixture.press(Key::l, ModifierType::empty());
            wait_until(|| archive_has_focus(&fixture));
            assert!(fixture.preview.archive_at_root());
            fixture.press(Key::l, ModifierType::empty());
            assert!(
                !fixture.preview.archive_at_root(),
                "l opens the archive folder"
            );
            assert!(archive_has_focus(&fixture));
            assert!(
                owner_bar(&fixture),
                "rebuilt archive rows keep the owner bar"
            );
            for key in [Key::l, Key::Return, Key::o, Key::space, Key::j, Key::G] {
                let modifiers = if key == Key::G {
                    ModifierType::SHIFT_MASK
                } else {
                    ModifierType::empty()
                };
                assert!(fixture.press(key, modifiers), "{key:?}");
                assert!(archive_has_focus(&fixture), "{key:?} stays in the archive");
            }
            fixture.press(Key::h, ModifierType::empty());
            assert!(
                fixture.preview.archive_at_root(),
                "h goes up inside the archive"
            );
            assert!(archive_has_focus(&fixture));
            fixture.press(Key::h, ModifierType::empty());
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(
                fixture.preview.is_open(),
                "h at the archive root keeps the drawer"
            );
            assert_eq!(focused_name(&browser), "bundle.zip");
            assert_eq!(fixture.selected(), selection);
            fixture.press(Key::l, ModifierType::empty());
            wait_until(|| archive_has_focus(&fixture));
            fixture.press(Key::Escape, ModifierType::empty());
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(
                !fixture.preview.is_enabled(),
                "Esc closes the archive drawer"
            );
            fixture.press(Key::l, ModifierType::empty());
            wait_until(|| archive_has_focus(&fixture));
            fixture.press(Key::i, ModifierType::empty());
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(!fixture.preview.is_enabled(), "i closes the archive drawer");

            move_to_named(&fixture, &browser, "long.txt");
            fixture.press(Key::i, ModifierType::empty());
            wait_until(|| fixture.preview.is_open());
            assert!(
                fixture.view.item_view_has_focus(),
                "i leaves focus in the list"
            );
            fixture.press(Key::k, ModifierType::empty());
            assert_eq!(focused_name(&browser), "locked.zip");
            wait_until(|| password_has_focus(&fixture));
            assert!(
                owner_bar(&fixture),
                "a password prompt that takes focus owns the keys"
            );
            assert!(
                !fixture.press(Key::i, ModifierType::empty()),
                "i is typed into the password"
            );
            assert!(fixture.preview.is_enabled());
            assert!(password_has_focus(&fixture));
            fixture.press(Key::Escape, ModifierType::empty());
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(!owner_bar(&fixture));

            move_to_named(&fixture, &browser, "locked.zip");
            fixture.press(Key::i, ModifierType::empty());
            wait_until(|| password_has_focus(&fixture));
            assert!(owner_bar(&fixture));
            fixture.press(Key::Escape, ModifierType::empty());
            wait_until(|| fixture.view.item_view_has_focus());

            move_to_named(&fixture, &browser, "locked.zip");
            fixture.press(Key::l, ModifierType::empty());
            wait_until(|| password_has_focus(&fixture));
            let preferences = PreferenceManager::shared();
            for (key, modifiers) in [
                (Key::h, ModifierType::empty()),
                (Key::j, ModifierType::empty()),
                (Key::l, ModifierType::empty()),
                (Key::q, ModifierType::empty()),
                (Key::space, ModifierType::empty()),
                (Key::asciitilde, ModifierType::SHIFT_MASK),
                (Key::Delete, ModifierType::empty()),
                (Key::a, ModifierType::CONTROL_MASK),
                (Key::v, ModifierType::CONTROL_MASK),
                (
                    Key::N,
                    ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
                ),
            ] {
                assert!(
                    !fixture.press(key, modifiers),
                    "{key:?} belongs to the password field"
                );
                assert!(password_has_focus(&fixture), "{key:?}");
            }
            assert!(preferences.tenxer_mode(), "q is typed, not a mode exit");
            assert!(!modal_visible(&fixture.overlay));
            fixture.press(Key::Tab, ModifierType::SHIFT_MASK);
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(
                fixture.preview.is_open(),
                "Shift+Tab keeps the password prompt"
            );
            fixture.press(Key::l, ModifierType::empty());
            wait_until(|| password_has_focus(&fixture));
            let password = gtk::prelude::RootExt::focus(&fixture.window)
                .and_then(|focused| focused.ancestor(gtk::PasswordEntry::static_type()))
                .and_downcast::<gtk::PasswordEntry>()
                .expect("password entry");
            password.set_text("secret");
            password.emit_by_name::<()>("activate", &[]);
            wait_until(|| archive_has_focus(&fixture));
            assert!(
                fixture.preview.archive_at_root(),
                "unlocking hands keys to the tree"
            );
            fixture.press(Key::Escape, ModifierType::empty());
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(!fixture.preview.is_enabled());

            assert!(
                opened.borrow().is_empty(),
                "browsing members never opens a file"
            );
            assert_eq!(browser.active_location(), origin);
            assert_eq!(directory_names(fixture._directory.path()), before);

            preferences.set_tenxer_mode(false);
            select_named(&fixture, "bundle.zip");
            fixture.press(Key::space, ModifierType::empty());
            wait_until(|| archive_has_focus(&fixture));
            fixture.press(Key::Right, ModifierType::empty());
            assert!(
                !fixture.preview.archive_at_root(),
                "default Right opens a folder"
            );
            fixture.press(Key::Left, ModifierType::empty());
            fixture.press(Key::Left, ModifierType::empty());
            assert!(
                archive_has_focus(&fixture),
                "default Left at the root does nothing"
            );
            fixture.press(Key::space, ModifierType::empty());
            wait_until(|| !fixture.preview.is_enabled());
        },
    );
}
