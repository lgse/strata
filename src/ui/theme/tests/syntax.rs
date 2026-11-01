// SPDX-License-Identifier: MIT

use super::*;
use sourceview5::prelude::*;

#[test]
fn bundled_syntax_palettes_load_in_gtksourceview() {
    gtk_test(
        "ui::theme::tests::syntax::bundled_syntax_palettes_load_in_gtksourceview",
        || {
            let themes = builtins();
            assert!(!themes.is_empty());
            for theme in themes {
                let tokens = theme.tokens;
                let directory = tempfile::tempdir().expect("scheme directory");
                std::fs::write(
                    directory.path().join("strata-current.xml"),
                    source_style_scheme_xml(&tokens, None),
                )
                .expect("write scheme");
                let manager = sourceview5::StyleSchemeManager::new();
                manager.set_search_path(&[directory.path().to_str().expect("UTF-8 scheme path")]);
                let scheme = manager.scheme("strata-current").expect(&theme.id);
                for (style, color) in [
                    ("def:statement", tokens.syntax_keyword),
                    ("def:string", tokens.syntax_string),
                    ("def:constant", tokens.syntax_constant),
                    ("def:type", tokens.syntax_type),
                    ("def:preprocessor", tokens.syntax_preprocessor),
                ] {
                    let color = color.expect("bundled theme must provide every syntax role");
                    assert_eq!(
                        scheme
                            .style(style)
                            .expect("syntax style")
                            .foreground()
                            .as_deref(),
                        Some(color_to_hex(&color).as_str()),
                        "{} {style}",
                        theme.id
                    );
                }
            }
        },
    );
}

#[test]
fn source_buffers_ignore_the_users_sourceview_folders() {
    gtk_test(
        "ui::theme::tests::syntax::source_buffers_ignore_the_users_sourceview_folders",
        || {
            let cache = gtk::glib::user_cache_dir().join("strata");
            std::fs::create_dir_all(&cache).expect("cache folder");
            std::fs::write(cache.join("source-styles"), b"").expect("unwritable style folder");
            let styles = gtk::glib::user_data_dir().join("gtksourceview-5/styles");
            std::fs::create_dir_all(&styles).expect("user style folder");
            std::fs::write(
                styles.join("strata-current.xml"),
                r##"<style-scheme id="strata-current" name="Stray" version="1.0"><style name="def:string" foreground="#123456"/></style-scheme>"##,
            )
            .expect("stray scheme");
            let languages = gtk::glib::user_data_dir().join("gtksourceview-5/language-specs");
            std::fs::create_dir_all(&languages).expect("user language folder");
            std::fs::write(
                languages.join("strata-stray.lang"),
                r#"<?xml version="1.0" encoding="UTF-8"?><language id="strata-stray" name="Stray" version="2.0" _section="Other"><metadata><property name="globs">*.stray</property></metadata><definitions><context id="strata-stray"/></definitions></language>"#,
            )
            .expect("stray language");
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(false);
            let buffer = crate::ui::theme::new_source_buffer();
            let mut seen = HashSet::new();
            let themes: Vec<_> = builtins()
                .into_iter()
                .filter(|theme| seen.insert(theme.tokens.syntax_string.clone()))
                .take(3)
                .collect();
            assert_eq!(themes.len(), 3, "themes with distinct string colors");
            for theme in &themes {
                manager.select_theme(&theme.id);
                crate::storage::writer::settle();
                let expected = color_to_hex(
                    theme
                        .tokens
                        .syntax_string
                        .as_deref()
                        .expect("bundled theme must provide every syntax role"),
                );
                assert_eq!(
                    buffer
                        .style_scheme()
                        .expect("installed source scheme")
                        .style("def:string")
                        .expect("syntax style")
                        .foreground()
                        .as_deref(),
                    Some(expected.as_str()),
                    "{}",
                    theme.id
                );
            }
            let languages = crate::ui::theme::source_languages();
            assert!(languages.language("sh").is_some(), "bundled languages load");
            assert!(languages.language("strata-stray").is_none());
            assert!(
                languages
                    .guess_language(Some("notes.stray"), None)
                    .is_none()
            );
        },
    );
}

#[test]
fn legacy_custom_themes_keep_independent_fallbacks_and_preserve_overrides() {
    let source = r##"
name = "Legacy"
background = "#000000"
surface = "#111111"
text = "#ffffff"
accent = "#336699"
muted = "#222222"
highlight = "#333333"
border = "#444444"
dim_text = "#888888"
"##;
    let mut tokens: ThemeTokens = toml::from_str(source).expect("legacy theme");
    let derived = super::super::resolved_source_palette(&tokens, None);
    tokens.syntax_string = Some("rgb(0,255,0)".into());
    let resolved = super::super::resolved_source_palette(&tokens, None);
    assert_eq!(resolved.statement, derived.statement);
    assert_eq!(resolved.type_color, derived.type_color);
    assert_eq!(resolved.constant, derived.constant);
    assert_eq!(resolved.preprocessor, derived.preprocessor);
    assert_eq!(color_to_hex(&resolved.string), "#00ff00");
    tokens.initialize_syntax_colors();
    let restored: ThemeTokens =
        toml::from_str(&toml::to_string(&tokens).expect("serialize theme")).expect("restore theme");
    assert_eq!(restored, tokens);
    assert!(validate_tokens(&restored).is_ok());
    for slot in ["keyword", "string", "constant", "type", "preprocessor"] {
        let invalid: ThemeTokens =
            toml::from_str(&format!("{source}\nsyntax_{slot} = \"not-a-color\"\n"))
                .expect("parse invalid-color fixture");
        assert!(validate_tokens(&invalid).is_err(), "{slot}");
    }
}
