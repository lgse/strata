// SPDX-License-Identifier: MIT

use super::super::*;
use crate::test_support::gtk_test;

fn page_text(root: &gtk::Widget) -> String {
    let mut text = Vec::new();
    if let Ok(label) = root.clone().downcast::<gtk::Label>() {
        text.push(label.text().to_string());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        text.push(page_text(&widget));
    }
    text.join("\n")
}

fn named_widget(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = named_widget(&widget, name) {
            return Some(found);
        }
    }
    None
}

#[test]
fn language_choices_and_restart_notice_synchronize_without_changing_running_ui() {
    gtk_test(
        "ui::settings::tests::general::language_choices_and_restart_notice_synchronize_without_changing_running_ui",
        || {
            let manager = PreferenceManager::shared();
            let startup = manager.language();
            let running = rust_i18n::locale().to_string();
            let (first, _, _) = general_page(manager.clone());
            let (second, _, _) = general_page(manager.clone());
            let alternate = if running == "ja" {
                crate::i18n::Language::French
            } else {
                crate::i18n::Language::Japanese
            };
            manager.set_language(alternate);
            for page in [&first, &second] {
                let choice = named_widget(page, "settings-language")
                    .expect("language selector")
                    .downcast::<gtk::MenuButton>()
                    .expect("language menu button");
                assert_eq!(
                    choice.label().expect("selected language label"),
                    if alternate == crate::i18n::Language::Japanese {
                        "日本語"
                    } else {
                        "Français"
                    }
                );
                assert!(
                    named_widget(page, "settings-language-restart")
                        .expect("restart button")
                        .is_visible()
                );
            }
            let (rebuilt, _, _) = general_page(manager.clone());
            assert!(
                named_widget(&rebuilt, "settings-language-restart")
                    .expect("rebuilt restart button")
                    .is_visible()
            );
            assert_eq!(&*rust_i18n::locale(), running);
            manager.set_language(startup);
            for page in [&first, &second, &rebuilt] {
                assert!(
                    !named_widget(page, "settings-language-restart")
                        .expect("restart button")
                        .is_visible()
                );
            }
        },
    );
}

#[test]
fn restart_honors_window_close_guards() {
    gtk_test(
        "ui::settings::tests::general::restart_honors_window_close_guards",
        || {
            let application = gtk::Application::builder()
                .application_id("org.strata.LanguageRestartTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gio::Cancellable>)
                .expect("register isolated application");
            let windows: Vec<_> = (0..2)
                .map(|_| {
                    gtk::ApplicationWindow::builder()
                        .application(&application)
                        .build()
                })
                .collect();
            let busy = Rc::new(std::cell::Cell::new(true));
            let guard = busy.clone();
            // Jobs only refuse for the window whose close would exit the application.
            crate::ui::close_guard::install(&windows[1], move |closing_application| {
                (closing_application && guard.get()).then(|| crate::ui::close_guard::CloseBlocker {
                    title: "Busy".to_owned(),
                    detail: String::new(),
                })
            });
            for window in &windows {
                window.present();
            }
            assert!(!close_windows_for_restart(&application));
            assert_eq!(application.windows().len(), 2);
            busy.set(false);
            assert!(close_windows_for_restart(&application));
        },
    );
}

#[test]
fn tenxer_experimental_label_follows_the_mode_across_windows() {
    gtk_test(
        "ui::settings::tests::general::tenxer_experimental_label_follows_the_mode_across_windows",
        || {
            let manager = PreferenceManager::shared();
            manager.set_tenxer_mode(false);
            let (first, _, _) = general_page(manager.clone());
            let (second, _, _) = general_page(manager.clone());
            let phrase = crate::ui::shortcut_reference::EXPERIMENTAL_LABEL;
            for page in [&first, &second] {
                assert!(!page_text(page).contains(phrase));
            }

            manager.set_tenxer_mode(true);
            for page in [&first, &second] {
                assert!(page_text(page).contains(phrase));
            }

            manager.set_tenxer_mode(false);
            for page in [&first, &second] {
                assert!(!page_text(page).contains(phrase));
            }
        },
    );
}

fn auto_refresh_choice(page: &gtk::Widget) -> gtk::MenuButton {
    let mut pending = vec![page.clone()];
    while let Some(widget) = pending.pop() {
        if let Ok(button) = widget.clone().downcast::<gtk::MenuButton>()
            && let Some(popover) = button.popover()
            && page_text(popover.upcast_ref()).contains("10 min")
        {
            return button;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            pending.push(next);
        }
    }
    panic!("the General page has no auto-refresh choice");
}

fn auto_refresh_label_mismatch(
    manager: &PreferenceManager,
    choice: &gtk::MenuButton,
    route: &str,
) -> Option<String> {
    let interval = manager.auto_refresh_interval();
    let label = choice.label().unwrap_or_default();
    let expected = crate::ui::preferences::AUTO_REFRESH_CHOICES
        .iter()
        .find(|(_, choice)| *choice == interval)
        .map(|(label, _)| *label);
    (expected != Some(label.as_str())).then(|| {
        format!("{route}: Settings shows {label:?} while the browser refreshes every {interval} s")
    })
}

#[test]
fn auto_refresh_choice_never_says_off_while_a_refresh_interval_runs() {
    gtk_test(
        "ui::settings::tests::general::auto_refresh_choice_never_says_off_while_a_refresh_interval_runs",
        || {
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let path = crate::ui::preferences::config_directory().join("settings.toml");
            let saved = std::fs::read_to_string(&path).expect("seeded preferences");
            assert!(saved.contains("auto_refresh_interval = 600"));
            std::fs::write(
                &path,
                saved.replace("auto_refresh_interval = 600", "auto_refresh_interval = 1"),
            )
            .expect("hand-edited interval");

            let manager = PreferenceManager::shared();
            let (page, _, _) = general_page(manager.clone());
            let choice = auto_refresh_choice(&page);
            let mut mismatches = Vec::new();
            mismatches.extend(auto_refresh_label_mismatch(&manager, &choice, "loaded 1"));
            for secs in [45, 120, 3600] {
                manager.set_auto_refresh_interval(secs);
                mismatches.extend(auto_refresh_label_mismatch(
                    &manager,
                    &choice,
                    &format!("set to {secs}"),
                ));
            }
            assert!(mismatches.is_empty(), "{mismatches:#?}");
        },
    );
}
