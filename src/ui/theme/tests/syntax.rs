// SPDX-License-Identifier: MIT

use super::*;

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
