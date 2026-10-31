// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;
use gtk::glib;

#[test]
fn validation_rejects_ambiguous_paths_and_equivalent_duplicates() {
    let current = vec!["cache".to_owned(), "~/Secret".to_owned()];
    for invalid in [
        "",
        " ",
        "/",
        "//",
        "~",
        "~/",
        "~/.",
        "/./",
        ".",
        "..",
        "project/build",
        "~user/dir",
        "/var/../log",
        "bad\0name",
    ] {
        assert!(
            validate_exclusion_input(invalid, &current).is_err(),
            "{invalid:?}"
        );
    }
    for duplicate in [
        " CACHE/ ".to_owned(),
        "~/Secret/./".to_owned(),
        glib::home_dir()
            .join("Secret")
            .to_string_lossy()
            .into_owned(),
    ] {
        assert_eq!(
            validate_exclusion_input(&duplicate, &current),
            Err("This exclusion has already been added.")
        );
    }
    for valid in ["[build]", r"foo\bar", "/var/log"] {
        assert_eq!(
            validate_exclusion_input(valid, &current),
            Ok(valid.to_owned())
        );
    }
    assert_eq!(
        validate_exclusion_input("~/secret", &current),
        Ok(glib::home_dir()
            .join("secret")
            .to_string_lossy()
            .into_owned())
    );
}

fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![root.clone()];
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(descendants(&widget));
    }
    result
}

fn button(root: &gtk::Widget, label: &str) -> gtk::Button {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.label().as_deref() == Some(label))
        .expect("button")
}

fn label(root: &gtk::Widget, text: &str) -> Option<gtk::Label> {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .find(|label| label.text() == text)
}

#[test]
fn inline_editors_synchronize_add_remove_preserve_drafts_and_release() {
    gtk_test(
        "ui::settings::exclusions::tests::inline_editors_synchronize_add_remove_preserve_drafts_and_release",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let themes = crate::ui::theme::ThemeManager::shared();
            let windows: Vec<_> = (0..2)
                .map(|_| {
                    let editor = search_exclusions_control(&manager);
                    let window = gtk::Window::builder().child(&editor).build();
                    window.present();
                    (window, editor)
                })
                .collect();
            let first = windows[0].1.upcast_ref::<gtk::Widget>();
            let second = windows[1].1.upcast_ref::<gtk::Widget>();
            assert!(label(first, ".venv").is_some());
            assert!(label(second, ".venv").is_some());
            let entries: Vec<_> = windows
                .iter()
                .map(|(_, editor)| {
                    descendants(editor.upcast_ref())
                        .into_iter()
                        .find_map(|widget| widget.downcast::<gtk::Entry>().ok())
                        .expect("exclusion entry")
                })
                .collect();
            entries[1].set_text("unfinished draft");
            entries[0].set_text("private-build");
            entries[0].emit_activate();
            assert!(label(first, "private-build").is_some());
            assert!(label(second, "private-build").is_some());
            assert_eq!(entries[1].text(), "unfinished draft");
            assert!(entries[0].text().is_empty());
            entries[0].set_text("PRIVATE-BUILD");
            button(first, "Add").emit_clicked();
            assert!(
                label(first, "This exclusion has already been added.")
                    .is_some_and(|label| label.is_visible())
            );
            let row = label(second, "private-build")
                .expect("added exclusion")
                .parent()
                .expect("exclusion row");
            let remove = descendants(&row)
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::Button>().ok())
                .expect("remove button");
            remove.emit_clicked();
            assert!(label(first, "private-build").is_none());
            assert!(label(second, "private-build").is_none());
            assert!(
                !manager
                    .search_exclusions()
                    .contains(&"private-build".to_owned())
            );
            drop(remove);
            drop(row);
            let weak_entries: Vec<_> = entries.iter().map(|entry| entry.downgrade()).collect();
            drop(entries);
            for (window, _) in windows {
                window.set_child(gtk::Widget::NONE);
                window.destroy();
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while weak_entries.iter().any(|entry| entry.upgrade().is_some()) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "inline editor retained its entries after destruction"
                );
                glib::MainContext::default().iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let rebuilt = search_exclusions_control(&manager);
            assert!(label(rebuilt.upcast_ref(), ".venv").is_some());
            assert!(label(rebuilt.upcast_ref(), "private-build").is_none());
            drop(themes);
        },
    );
}
