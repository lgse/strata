// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn legacy_omarchy_variant_migrates_to_the_active_theme() {
    gtk_test(
        "ui::preferences::tests::preferences::themes::legacy_omarchy_variant_migrates_to_the_active_theme",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let mut saved =
                toml::Table::try_from(non_default_preferences()).expect("saved preferences");
            saved.remove("omarchy_variants");
            saved.insert("mode".into(), "omarchy".into());
            saved.insert("omarchy_variant".into(), "high_contrast".into());
            fs::write(
                settings_path(),
                toml::to_string(&saved).expect("legacy settings"),
            )
            .expect("persist legacy variant");
            let themes = ThemeManager::shared();
            let preferences = PreferenceManager::shared();
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Contrast);
            let migrated = themes.appearance_tokens();
            preferences.set_folder_peeking(false);
            crate::storage::writer::settle();
            let saved = fs::read_to_string(settings_path()).expect("migrated settings");
            let saved: toml::Table = toml::from_str(&saved).expect("migrated settings table");
            assert!(!saved.contains_key("omarchy_variant"));
            assert_eq!(
                read_preferences()
                    .expect("migrated variants")
                    .omarchy_variants,
                HashMap::from([("fixture".into(), OmarchyVariant::Contrast)])
            );
            preferences.set_omarchy_variant(OmarchyVariant::Darker);
            assert_ne!(themes.appearance_tokens(), migrated);
        },
    );
}

#[test]
fn unavailable_or_unknown_theme_sources_normalize_to_manual() {
    gtk_test(
        "ui::preferences::tests::preferences::themes::unavailable_or_unknown_theme_sources_normalize_to_manual",
        || {
            seed_saved_preferences_for_test();
            let preferences = PreferenceManager::shared();
            for mode in ["system", "unknown"] {
                preferences.set_theme_selection(mode, None);
                preferences.normalize_loaded_theme_selection(|_| true, false, false);
                assert_eq!(preferences.theme_mode(), "theme", "{mode}");
            }
        },
    );
}

#[test]
fn saved_omarchy_mode_loads_and_changes_through_the_same_binding() {
    gtk_test(
        "ui::preferences::tests::preferences::themes::saved_omarchy_mode_loads_and_changes_through_the_same_binding",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let mut preferences = non_default_preferences();
            preferences.mode = "omarchy".into();
            fs::write(
                settings_path(),
                toml::to_string(&preferences).expect("Omarchy preference fixture"),
            )
            .expect("persist Omarchy mode");
            let manager = ThemeManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_theme_preference(
                &anchor,
                ThemeManager::follows_omarchy,
                move |_, value| observed.borrow_mut().push(value),
            );
            assert_eq!(*values.borrow(), [true]);
            let preferences = PreferenceManager::shared();
            let windows = [gtk::Window::new(), gtk::Window::new()];
            let buffer = gtk::TextBuffer::new(None);
            buffer.create_tag(Some("document-accent"), &[]);
            crate::ui::theme::register_document_buffer(&buffer);
            let source = crate::ui::theme::new_source_buffer();
            let startup = manager.appearance_tokens();
            for variant in [
                OmarchyVariant::Darker,
                OmarchyVariant::Lighter,
                OmarchyVariant::Contrast,
                OmarchyVariant::Darker,
            ] {
                preferences.set_omarchy_variant(variant);
                crate::storage::writer::settle();
                let tokens = manager.appearance_tokens();
                assert_eq!(tokens == startup, variant == OmarchyVariant::Lighter);
                for window in &windows {
                    assert_theme_colors(window, &tokens);
                    let rebuilt_view = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    window.set_child(Some(&rebuilt_view));
                    assert_theme_colors(&rebuilt_view, &tokens);
                }
                let rebuilt_buffer = gtk::TextBuffer::new(None);
                rebuilt_buffer.create_tag(Some("document-accent"), &[]);
                crate::ui::theme::register_document_buffer(&rebuilt_buffer);
                for buffer in [&buffer, &rebuilt_buffer] {
                    assert_eq!(
                        buffer
                            .tag_table()
                            .lookup("document-accent")
                            .expect("document accent tag")
                            .foreground_rgba(),
                        Some(gtk::gdk::RGBA::parse(&tokens.accent).expect("valid accent"))
                    );
                }
                assert_eq!(
                    source
                        .style_scheme()
                        .expect("active source scheme")
                        .style("text")
                        .expect("source text style")
                        .background()
                        .as_deref(),
                    Some(tokens.surface.as_str())
                );
                crate::storage::writer::settle();
                assert_eq!(
                    read_preferences()
                        .expect("saved variant")
                        .omarchy_variants
                        .get("fixture")
                        .copied()
                        .unwrap_or_default(),
                    variant
                );
            }
            let before_change = manager.appearance_tokens();
            fs::write(
                crate::ui::theme::omarchy_state_dir().join("theme/colors.toml"),
                "background = '#201a12'\nforeground = '#ffeedd'\naccent = '#eebb99'\n",
            )
            .expect("update Omarchy colors");
            wait_for_theme(|| {
                manager.appearance_tokens() != before_change && {
                    #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
                    let accent = windows[0]
                        .style_context()
                        .lookup_color("strata_accent")
                        .expect("applied accent");
                    accent
                        == gtk::gdk::RGBA::parse(&manager.appearance_tokens().accent)
                            .expect("valid updated accent")
                }
            });
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Darker);
            for window in &windows {
                assert_theme_colors(window, &manager.appearance_tokens());
            }
            manager.set_follow_omarchy(false);
            let builtin = manager.appearance_tokens();
            preferences.set_omarchy_variant(OmarchyVariant::Contrast);
            assert_eq!(manager.appearance_tokens(), builtin);
            for window in &windows {
                assert_theme_colors(window, &builtin);
            }
            manager.set_follow_omarchy(true);
            assert_ne!(manager.appearance_tokens(), builtin);
            assert_eq!(*values.borrow(), [true, false, true]);
            for window in windows {
                window.close();
            }
            crate::storage::writer::settle();
            assert_eq!(
                read_preferences().expect("saved theme mode").mode,
                "omarchy"
            );
        },
    );
}

fn assert_theme_colors(widget: &impl IsA<gtk::Widget>, tokens: &crate::ui::theme::ThemeTokens) {
    for (name, expected) in [
        ("strata_bg", &tokens.background),
        ("strata_surface", &tokens.surface),
        ("strata_accent", &tokens.accent),
        ("strata_text", &tokens.text),
    ] {
        #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
        let actual = widget
            .style_context()
            .lookup_color(name)
            .expect("applied theme color");
        assert_eq!(
            actual,
            gtk::gdk::RGBA::parse(expected).expect("valid token"),
            "{name}"
        );
    }
}

#[test]
fn omarchy_colors_in_gtk_only_hex_forms_apply_to_css_and_icons() {
    gtk_test(
        "ui::preferences::tests::preferences::themes::omarchy_colors_in_gtk_only_hex_forms_apply_to_css_and_icons",
        || {
            seed_omarchy_colors_for_test(
                "background = '#112233'\nforeground = '#ddeeff'\naccent = '#000aaafff'\nselection = '#1111222233338888'\n",
            );
            let themes = ThemeManager::shared();
            assert!(themes.follows_omarchy());
            let tokens = themes.appearance_tokens();
            assert_eq!(tokens.accent, "#00aaff");
            assert_eq!(tokens.highlight, "#11223388");
            let window = gtk::Window::new();
            for (name, expected) in [
                ("strata_accent", "#00aaff"),
                ("strata_highlight", "#11223388"),
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
            window.close();
        },
    );
}

#[test]
fn invalid_omarchy_palette_while_running_keeps_the_applied_palette() {
    gtk_test(
        "ui::preferences::tests::preferences::themes::invalid_omarchy_palette_while_running_keeps_the_applied_palette",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let mut preferences = non_default_preferences();
            preferences.mode = "omarchy".into();
            preferences.omarchy_variants.clear();
            fs::write(
                settings_path(),
                toml::to_string(&preferences).expect("Omarchy preference fixture"),
            )
            .expect("persist Omarchy mode");
            let themes = ThemeManager::shared();
            assert_eq!(crate::assets::primary_icon_color(), "#445566");
            assert_eq!(themes.appearance_tokens().accent, "#445566");

            seed_omarchy_colors_for_test(
                "background = '#112233'\nforeground = '#ddeeff'\naccent = '0x7aa2f7'\n",
            );
            themes.refresh_omarchy_state();
            crate::storage::writer::settle();
            assert!(themes.follows_omarchy());
            assert!(themes.is_omarchy_available());
            assert_eq!(crate::assets::primary_icon_color(), "#445566");
            assert_eq!(themes.appearance_tokens().accent, "#445566");
            let buffer = gtk::TextBuffer::new(None);
            buffer.create_tag(Some("document-accent"), &[]);
            crate::ui::theme::register_document_buffer(&buffer);
            assert_eq!(
                buffer
                    .tag_table()
                    .lookup("document-accent")
                    .expect("document accent tag")
                    .foreground_rgba(),
                Some(gtk::gdk::RGBA::parse("#445566").expect("accent"))
            );
            let mut preview = themes.starter_tokens();
            preview.accent = "#13579b".to_owned();
            themes.preview(&preview).expect("valid preview");
            assert_eq!(themes.appearance_tokens().accent, "#445566");
            themes.cancel_preview();
            themes.set_follow_omarchy(false);
            themes.set_follow_omarchy(true);
            assert!(themes.follows_omarchy());
            assert_eq!(
                themes.appearance_tokens(),
                themes.starter_tokens(),
                "a built-in theme applied since then replaces the remembered Omarchy palette"
            );

            fs::remove_file(crate::ui::theme::omarchy_state_dir().join("theme.name"))
                .expect("remove Omarchy theme name");
            let held = crate::storage::writer::hold();
            themes.refresh_omarchy_state();
            for _ in 0..20 {
                glib::MainContext::default().iteration(false);
            }
            assert!(
                themes.follows_omarchy(),
                "Omarchy files are read again on the settings writer"
            );
            drop(held);
            crate::storage::writer::settle();
            assert!(!themes.follows_omarchy());
            crate::storage::writer::settle();
            assert_eq!(read_preferences().expect("saved theme mode").mode, "theme");
        },
    );
}

fn wait_for_theme(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "live Omarchy refresh timed out"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
