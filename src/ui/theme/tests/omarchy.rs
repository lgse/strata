// SPDX-License-Identifier: MIT

use std::{
    path::Path,
    time::{Duration, Instant},
};

use super::{save_custom_theme, scheme_color_values, wait_for_live_theme};
use crate::adapters::{FsOp, FsQuirks, Quirks};
use crate::test_support::{Gate, gtk_test};
use crate::ui::theme::{
    OmarchyRefresh, OmarchyVariant, SourcePalette, ThemeManager, azure_tokens,
    is_omarchy_theme_event, omarchy_state_dir, resolved_source_palette,
    source_palette_from_quattro, source_style_scheme_xml, themes_directory, tokens_from_quattro,
};

#[test]
fn switching_omarchy_symlinks_keeps_direct_and_atomic_palette_edits_live() {
    gtk_test(
        "ui::theme::tests::omarchy::switching_omarchy_symlinks_keeps_direct_and_atomic_palette_edits_live",
        || {
            let state = omarchy_state_dir();
            let themes = gtk::glib::home_dir().join("palettes");
            let write = |folder: &Path, accent: &str, atomic: bool| {
                std::fs::create_dir_all(folder).expect("palette directory");
                let destination = folder.join("colors.toml");
                let path = if atomic {
                    folder.join("colors.new")
                } else {
                    destination.clone()
                };
                std::fs::write(
                    &path,
                    format!(
                        "background = '#112233'\nforeground = '#ddeeff'\naccent = '{accent}'\n"
                    ),
                )
                .expect("palette written");
                if atomic {
                    std::fs::rename(path, destination).expect("atomic palette replacement");
                }
            };
            let switch = |name: &str| {
                let pending = state.join("next-theme");
                std::os::unix::fs::symlink(themes.join(name), &pending).expect("new theme link");
                std::fs::rename(pending, state.join("theme")).expect("atomic theme switch");
            };
            std::fs::create_dir_all(&state).expect("Omarchy state");
            std::fs::write(state.join("theme.name"), "fixture").expect("theme name");
            write(&themes.join("a"), "#112244", false);
            write(&themes.join("b"), "#445566", false);
            switch("a");
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(true);
            assert_eq!(manager.active_model_palette().accent, 0x11_22_44);

            for (name, initial, edited, color) in [
                ("b", 0x44_55_66, "#778899", 0x77_88_99),
                ("a", 0x11_22_44, "#aabbcc", 0xaa_bb_cc),
            ] {
                switch(name);
                wait_for_live_theme(|| manager.active_model_palette().accent == initial);
                write(&themes.join(name), edited, false);
                wait_for_live_theme(|| manager.active_model_palette().accent == color);
                write(&themes.join(name), "#abcdef", true);
                wait_for_live_theme(|| manager.active_model_palette().accent == 0xab_cd_ef);
            }
        },
    );
}

#[test]
fn quattro_colors_map_to_strata_tokens() {
    let theme = tokens_from_quattro(
        "azure-glow",
        r##"
background = "#0a0f1a"
dark_background = "#070b13"
lighter_background = "#142033"
foreground = "#a8dfff"
accent = "#00aaff"
selection = "#1d3350"
muted = "#3a5068"
color8 = "#99aabb"
"##,
        OmarchyVariant::Lighter,
    )
    .expect("valid Quattro colors should map");

    for (token, actual, expected) in [
        ("name", theme.name.as_str(), "Azure Glow"),
        ("background", theme.background.as_str(), "#0a0f1a"),
        ("surface", theme.surface.as_str(), "#070b13"),
        ("muted", theme.muted.as_str(), "#142033"),
        ("highlight", theme.highlight.as_str(), "#1d3350"),
        ("border", theme.border.as_str(), "#3a5068"),
        ("accent", theme.accent.as_str(), "#00aaff"),
    ] {
        assert_eq!(actual, expected, "{token}");
    }

    let minimal = tokens_from_quattro(
        "azure-glow",
        "background = \"#0a0f1a\"\nlighter_background = \"#0a0f1a\"\nforeground = \"#a8dfff\"\naccent = \"#00aaff\"\ncolor8 = \"#99aabb\"\n",
        OmarchyVariant::Lighter,
    )
    .expect("required Quattro colors should map");
    assert_eq!(minimal.background, "#0a0f1a");
    assert_eq!(minimal.surface, "#0a0f1a");
    assert_ne!(minimal.muted, minimal.background);
    let derived = resolved_source_palette(&theme, None);
    for (source, expected) in [
        ("color2 = \"#00ff00\"", "#00ff00"),
        ("color2 = \"#00ff00\"\ngreen = \"#009900\"", "#009900"),
        ("green = \"invalid\"", derived.string.as_str()),
    ] {
        let source = format!(
            "background = \"#0a0f1a\"\nforeground = \"#a8dfff\"\naccent = \"#00aaff\"\nselection = \"#a8dfff\"\ncolor8 = \"#123247\"\n{source}"
        );
        let tokens = tokens_from_quattro("azure-glow", &source, OmarchyVariant::Lighter)
            .expect("partial Quattro palette");
        let palette = resolved_source_palette(&tokens, None);
        assert_eq!(palette.string, expected);
        assert_eq!(palette.statement, derived.statement);
    }
}

#[test]
fn quattro_syntax_colors_remain_theme_native() {
    let palette = source_palette_from_quattro(
        r##"
blue = "#111111"
cyan = "#222222"
green = "#333333"
yellow = "#444444"
orange = "#555555"
magenta = "#666666"
"##,
    )
    .expect("complete Quattro syntax palette");

    for (role, actual, expected) in [
        ("statement", palette.statement.as_str(), "#666666"),
        ("string", palette.string.as_str(), "#333333"),
        ("constant", palette.constant.as_str(), "#555555"),
        ("type", palette.type_color.as_str(), "#222222"),
        ("preprocessor", palette.preprocessor.as_str(), "#444444"),
    ] {
        assert_eq!(actual, expected, "{role}");
    }
}

#[test]
fn quattro_syntax_palette_treats_unparsable_colors_as_missing() {
    let base = [
        ("blue", "#111111"),
        ("cyan", "#222222"),
        ("green", "#333333"),
        ("yellow", "#444444"),
        ("orange", "#555555"),
        ("magenta", "#666666"),
    ];
    let source_with = |key: &str, value: &str| {
        base.iter()
            .map(|(name, color)| {
                let color = if *name == key { value } else { color };
                format!("{name} = \"{color}\"\n")
            })
            .collect::<String>()
    };
    type Field = fn(&SourcePalette) -> &str;
    let statement: Field = |palette| &palette.statement;
    let string: Field = |palette| &palette.string;
    let constant: Field = |palette| &palette.constant;
    let type_color: Field = |palette| &palette.type_color;
    for (key, value, expected) in [
        ("green", "invalid", None),
        ("yellow", "invalid", None),
        ("magenta", "0x666666", Some((statement, "#111111"))),
        ("orange", "notacolor", Some((constant, "#444444"))),
        ("cyan", "", Some((type_color, "#111111"))),
        ("green", "#333333333", Some((string, "#333333"))),
        (
            "magenta",
            "rgba(102,102,102,0.5)",
            Some((statement, "#66666680")),
        ),
    ] {
        let palette = source_palette_from_quattro(&source_with(key, value));
        match expected {
            None => assert!(palette.is_none(), "{key} = {value:?}"),
            Some((field, color)) => assert_eq!(
                palette.as_ref().map(field),
                Some(color),
                "{key} = {value:?}"
            ),
        }
    }

    let source = format!(
        "background = \"#0a0f1a\"\nforeground = \"#a8dfff\"\naccent = \"#00aaff\"\n{}",
        source_with("orange", "0x555555").replace("#666666", "notacolor")
    );
    let tokens = tokens_from_quattro("azure-glow", &source, OmarchyVariant::Lighter)
        .expect("required Quattro colors are valid");
    let xml = source_style_scheme_xml(&tokens, source_palette_from_quattro(&source).as_ref());
    let values = scheme_color_values(&xml);
    assert_eq!(values.len(), 12);
    for value in values {
        assert!(
            value.starts_with('#') && value.len() == 7,
            "scheme colors must be canonical #rrggbb, got {value}"
        );
    }
}

#[test]
fn legacy_palette_without_quattro_semantics_is_not_detected() {
    assert!(
        tokens_from_quattro("legacy", "color4 = \"#00aaff\"", OmarchyVariant::Lighter).is_none()
    );
}

#[test]
fn omarchy_monitor_ignores_unrelated_state_changes() {
    let state = omarchy_state_dir();
    for path in [
        state.clone(),
        state.join("theme"),
        state.join("theme.name"),
        state.join("theme/colors.toml"),
        state.parent().expect("Omarchy state parent").to_path_buf(),
    ] {
        assert!(is_omarchy_theme_event(&gtk::gio::File::for_path(path)));
    }
    for path in [
        state.join("next-theme"),
        state.join("background"),
        gtk::glib::home_dir().join("theme"),
    ] {
        assert!(!is_omarchy_theme_event(&gtk::gio::File::for_path(path)));
    }
}

#[test]
fn theme_and_omarchy_folders_are_watched_and_released_on_the_monitors_worker() {
    gtk_test(
        "ui::theme::tests::omarchy::theme_and_omarchy_folders_are_watched_and_released_on_the_monitors_worker",
        || {
            crate::ui::preferences::fixtures::seed_omarchy_for_test();
            std::fs::create_dir_all(themes_directory()).expect("custom themes folder");
            let creating = Gate::default();
            let watches = WatchCalls {
                themes: FsQuirks::install(&themes_directory(), Quirks::default()),
                omarchy: FsQuirks::install(
                    &gtk::glib::home_dir(),
                    Quirks {
                        before: vec![(FsOp::Watch, "*", 0, creating.racer())],
                        ..Quirks::default()
                    },
                ),
            };
            let watched = OmarchyRefresh::read().directories.len();
            assert!(watched > 1, "the Omarchy theme folder and its ancestors");

            let manager = ThemeManager::shared();
            pump_until(&|| "the first watch starts".to_owned(), &|| {
                creating.reached()
            });
            write_changes_outside_the_app();
            creating.release();
            watches.wait_for("watched at start", &|calls| calls == [1, 0, watched, 0]);
            pump_until(
                &|| "changes made while the watches started are read".to_owned(),
                &|| outside_changes_read(&manager),
            );

            let mut tokens = azure_tokens();
            tokens.name = "Watched".to_owned();
            save_custom_theme(&manager, tokens).expect("custom theme saves");
            manager.refresh_omarchy_state();
            watches.still(
                [1, 0, watched, 0],
                "a save and a refresh keep the watches they have",
            );

            let theme_folder = omarchy_state_dir().join("theme");
            std::fs::remove_dir_all(&theme_folder).expect("Omarchy theme folder removed");
            manager.refresh_omarchy_state();
            pump_until(&|| "the removed folder is read".to_owned(), &|| {
                crate::storage::writer::settle();
                manager.omarchy_state.borrow().is_none()
            });
            crate::ui::preferences::fixtures::seed_omarchy_for_test();
            manager.refresh_omarchy_state();
            watches.wait_for(
                "only the folder that came back is watched again",
                &|calls| rewatched_once(calls, watched),
            );
        },
    );
}

/// An Omarchy palette and a custom theme written while the watches are still starting.
fn write_changes_outside_the_app() {
    crate::ui::preferences::fixtures::seed_omarchy_colors_for_test(
        "background = '#010203'\nforeground = '#ddeeff'\naccent = '#445566'\n",
    );
    let mut outside = azure_tokens();
    outside.name = "Outside".to_owned();
    std::fs::write(
        themes_directory().join("outside.toml"),
        toml::to_string(&outside).expect("theme file"),
    )
    .expect("a theme saved elsewhere");
}

fn outside_changes_read(manager: &ThemeManager) -> bool {
    crate::storage::writer::settle();
    manager
        .omarchy_state
        .borrow()
        .as_ref()
        .is_some_and(|state| state.colors.contains("#010203"))
        && manager
            .themes()
            .iter()
            .any(|theme| theme.tokens.name == "Outside")
}

/// The themes folder keeps its watch; the Omarchy folder that came back is
/// watched again after at most one release.
fn rewatched_once(
    [themes_watched, themes_released, watching, released]: [usize; 4],
    watched: usize,
) -> bool {
    [themes_watched, themes_released, watching] == [1, 0, watched + 1] && released <= 1
}

fn pump_until(what: &dyn Fn() -> String, done: &dyn Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "{}", what());
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Watch and unwatch calls on the custom themes folder and on the home folder
/// that holds Omarchy's state.
struct WatchCalls {
    themes: FsQuirks,
    omarchy: FsQuirks,
}

impl WatchCalls {
    fn calls(&self) -> [usize; 4] {
        [
            self.themes.count(FsOp::Watch),
            self.themes.count(FsOp::Unwatch),
            self.omarchy.count(FsOp::Watch),
            self.omarchy.count(FsOp::Unwatch),
        ]
    }

    fn wait_for(&self, what: &str, expected: &dyn Fn([usize; 4]) -> bool) {
        pump_until(&|| format!("{what}: {:?}", self.calls()), &|| {
            expected(self.calls())
        });
    }

    fn still(&self, expected: [usize; 4], what: &str) {
        crate::storage::writer::settle();
        for _ in 0..50 {
            gtk::glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(self.calls(), expected, "{what}");
    }
}
