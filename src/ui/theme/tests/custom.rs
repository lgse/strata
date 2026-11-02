// SPDX-License-Identifier: MIT

use std::{path::Path, rc::Rc};

use super::{save_custom_theme, settle_live_theme, wait_for_live_theme};
use crate::test_support::gtk_test;
use crate::ui::theme::{ThemeManager, ThemeTokens, azure_tokens, themes_directory};
use gtk::prelude::*;

#[test]
fn saving_a_custom_theme_never_overwrites_an_existing_theme_file() {
    gtk_test(
        "ui::theme::tests::custom::saving_a_custom_theme_never_overwrites_an_existing_theme_file",
        || {
            let directory = themes_directory();
            let dotfiles = directory
                .parent()
                .expect("config directory")
                .join("dotfiles");
            std::fs::create_dir_all(&directory).expect("themes directory");
            std::fs::create_dir_all(&dotfiles).expect("dotfiles directory");
            let mut tokens = azure_tokens();
            tokens.name = "Ocean Blue".to_owned();
            let valid = toml::to_string_pretty(&tokens).expect("theme file");
            std::fs::write(directory.join("ocean-blue.toml"), "not a theme").expect("broken theme");
            std::fs::write(dotfiles.join("broken.toml"), "name = [").expect("broken dotfile");
            std::os::unix::fs::symlink(
                dotfiles.join("broken.toml"),
                directory.join("ocean-blue-2.toml"),
            )
            .expect("link to a broken dotfile");
            std::os::unix::fs::symlink(
                dotfiles.join("missing.toml"),
                directory.join("ocean-blue-3.toml"),
            )
            .expect("dangling link");
            let manager = ThemeManager::shared();
            std::fs::write(dotfiles.join("valid.toml"), &valid).expect("valid dotfile");
            std::os::unix::fs::symlink(
                dotfiles.join("valid.toml"),
                directory.join("ocean-blue-4.toml"),
            )
            .expect("link added after startup");

            let id = save_custom_theme(&manager, tokens).expect("saved theme");

            assert_eq!(id, "ocean-blue-5");
            assert!(directory.join("ocean-blue-5.toml").is_file());
            assert_eq!(
                std::fs::read_to_string(directory.join("ocean-blue.toml")).expect("broken theme"),
                "not a theme"
            );
            for (link, target) in [
                ("ocean-blue-2.toml", "broken.toml"),
                ("ocean-blue-3.toml", "missing.toml"),
                ("ocean-blue-4.toml", "valid.toml"),
            ] {
                assert_eq!(
                    std::fs::read_link(directory.join(link)).expect("theme link"),
                    dotfiles.join(target)
                );
            }
            assert_eq!(
                std::fs::read_to_string(dotfiles.join("broken.toml")).expect("broken dotfile"),
                "name = ["
            );
            assert!(!dotfiles.join("missing.toml").exists());
            assert_eq!(
                std::fs::read_to_string(dotfiles.join("valid.toml")).expect("valid dotfile"),
                valid
            );
        },
    );
}

#[test]
fn rewriting_the_selected_custom_theme_applies_without_a_restart() {
    gtk_test(
        "ui::theme::tests::custom::rewriting_the_selected_custom_theme_applies_without_a_restart",
        || {
            let directory = themes_directory();
            std::fs::create_dir_all(&directory).expect("themes directory");
            let mut tokens = azure_tokens();
            tokens.name = "Live Reload".to_owned();
            tokens.accent = "#112233".to_owned();
            let path = directory.join("live-reload.toml");
            let write_theme = |tokens: &ThemeTokens| {
                std::fs::write(&path, toml::to_string_pretty(tokens).expect("theme file"))
                    .expect("written theme");
            };
            write_theme(&tokens);

            let manager = ThemeManager::shared();
            manager.select_theme("live-reload");
            assert_eq!(manager.active_model_palette().accent, 0x11_22_33);

            tokens.accent = "#445566".to_owned();
            write_theme(&tokens);
            wait_for_live_theme(|| manager.active_model_palette().accent == 0x44_55_66);
            assert_eq!(
                manager.current_tokens().expect("selected theme").accent,
                "#445566"
            );

            std::fs::write(&path, "not a theme").expect("broken theme");
            settle_live_theme();
            assert_eq!(manager.active_model_palette().accent, 0x44_55_66);
            assert!(
                manager
                    .themes()
                    .iter()
                    .any(|theme| theme.id == "live-reload"),
                "a broken file keeps the selected theme listed"
            );

            tokens.accent = "#778899".to_owned();
            write_theme(&tokens);
            wait_for_live_theme(|| manager.active_model_palette().accent == 0x77_88_99);
        },
    );
}

#[test]
fn rewriting_a_symlinked_theme_target_applies_without_a_restart() {
    gtk_test(
        "ui::theme::tests::custom::rewriting_a_symlinked_theme_target_applies_without_a_restart",
        || {
            let directory = themes_directory();
            let config = directory.parent().expect("config directory");
            let (dotfiles, generated) = (config.join("dotfiles"), config.join("generated"));
            for path in [&directory, &dotfiles, &generated] {
                std::fs::create_dir_all(path).expect("theme directories");
            }
            let write_theme = |folder: &Path, name: &str, accent: &str| {
                let mut tokens = azure_tokens();
                tokens.name = name.to_owned();
                tokens.accent = accent.to_owned();
                std::fs::write(
                    folder.join(format!("{name}.toml")),
                    toml::to_string_pretty(&tokens).expect("theme file"),
                )
                .expect("written dotfile");
            };
            let link = |folder: &Path, name: &str| {
                std::os::unix::fs::symlink(
                    folder.join(format!("{name}.toml")),
                    directory.join(format!("{name}.toml")),
                )
                .expect("dotfiles link");
            };
            write_theme(&dotfiles, "linked", "#112233");
            link(&dotfiles, "linked");

            let manager = ThemeManager::shared();
            manager.select_theme("linked");
            assert_eq!(manager.active_model_palette().accent, 0x11_22_33);

            // Dangling until the generator first writes it, in a directory not yet watched.
            link(&generated, "later");
            write_theme(&dotfiles, "linked", "#445566");
            wait_for_live_theme(|| manager.active_model_palette().accent == 0x44_55_66);

            write_theme(&generated, "later", "#778899");
            wait_for_live_theme(|| manager.themes().iter().any(|theme| theme.id == "later"));
            manager.select_theme("later");
            assert_eq!(manager.active_model_palette().accent, 0x77_88_99);

            write_theme(&generated, "later", "#aabbcc");
            wait_for_live_theme(|| manager.active_model_palette().accent == 0xaa_bb_cc);
        },
    );
}

#[test]
fn custom_theme_colors_load_as_css_hex_without_rewriting_the_file() {
    gtk_test(
        "ui::theme::tests::custom::custom_theme_colors_load_as_css_hex_without_rewriting_the_file",
        || {
            let directory = themes_directory();
            std::fs::create_dir_all(&directory).expect("themes directory");
            let mut written = azure_tokens();
            written.name = "Deep Hex".to_owned();
            written.accent = "#000aaafff".to_owned();
            written.highlight = "#0000aaaaffff8888".to_owned();
            written.syntax_string = Some("#333333333".to_owned());
            let source = toml::to_string_pretty(&written).expect("theme file");
            let path = directory.join("deep-hex.toml");
            std::fs::write(&path, &source).expect("custom theme");

            let manager = ThemeManager::shared();
            let loaded = manager
                .themes()
                .into_iter()
                .find(|theme| theme.id == "deep-hex")
                .expect("custom theme with GTK-only hex colors loads")
                .tokens;
            let mut expected = written.clone();
            expected.accent = "#00aaff".to_owned();
            expected.highlight = "#00aaff88".to_owned();
            expected.syntax_string = Some("#333333".to_owned());
            assert_eq!(loaded, expected, "GTK-only hex forms load as CSS hex");

            let window = gtk::Window::new();
            manager.select_theme("deep-hex");
            for (name, expected) in [
                ("strata_accent", "#00aaff"),
                ("strata_highlight", "#00aaff88"),
            ] {
                #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
                let applied = window.style_context().lookup_color(name);
                assert_eq!(
                    applied,
                    Some(gtk::gdk::RGBA::parse(expected).expect("canonical color")),
                    "{name}"
                );
            }
            assert_eq!(crate::assets::primary_icon_color(), "#00aaff");
            assert_eq!(
                std::fs::read_to_string(&path).expect("custom theme"),
                source,
                "loading never rewrites the user's theme file"
            );
            window.close();
        },
    );
}

#[test]
fn non_ascii_theme_names_save_under_a_fallback_id() {
    gtk_test(
        "ui::theme::tests::custom::non_ascii_theme_names_save_under_a_fallback_id",
        || {
            let directory = themes_directory();
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("azure-glow");

            let mut tokens = azure_tokens();
            tokens.name = "日本語".to_owned();
            let held = crate::storage::writer::hold();
            let saved = Rc::new(std::cell::RefCell::new(None));
            let done = saved.clone();
            manager.save_custom_theme(tokens, move |result| {
                done.replace(Some(result));
            });
            for _ in 0..20 {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert!(
                saved.borrow().is_none(),
                "the save waits for the settings writer"
            );
            assert!(!directory.join("custom-theme.toml").exists());
            assert_eq!(manager.selected_id(), "azure-glow");
            drop(held);
            crate::storage::writer::settle();
            let id = saved
                .take()
                .expect("theme save finished")
                .expect("a non-ASCII name saves");
            assert_eq!(id, "custom-theme");
            let saved = std::fs::read_to_string(directory.join("custom-theme.toml"))
                .expect("saved theme file");
            assert_eq!(
                toml::from_str::<ThemeTokens>(&saved)
                    .expect("saved theme parses")
                    .name,
                "日本語"
            );
            assert!(
                manager
                    .themes()
                    .iter()
                    .any(|theme| theme.id == "custom-theme"
                        && theme.custom
                        && theme.tokens.name == "日本語")
            );
            assert_eq!(manager.selected_id(), "custom-theme");

            let files_before = std::fs::read_dir(&directory).expect("themes").count();
            let mut blank = azure_tokens();
            blank.name = "   ".to_owned();
            let error = save_custom_theme(&manager, blank).expect_err("a blank name is rejected");
            assert_eq!(error.to_string(), "Enter a theme name");
            assert_eq!(
                std::fs::read_dir(&directory).expect("themes").count(),
                files_before
            );
        },
    );
}
