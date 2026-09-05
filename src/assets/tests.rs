// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::TextureExt;

use super::{
    contrasting_foreground, folder_decoration_texture, icons, primary_icon_texture,
    recolor_icon_source, svg_body,
};

#[test]
fn themed_icons_replace_every_legacy_fallback_color() {
    for fallback in ["#8bc9eb", "#22d3ee", "#2e3436"] {
        let source = format!(r##"<svg stroke="{fallback}"/>"##);
        assert_eq!(
            recolor_icon_source(&source, "#ab6a57"),
            r##"<svg stroke="#ab6a57"/>"##
        );
    }
}

#[test]
fn on_primary_icons_keep_their_contrast_color() {
    assert_eq!(
        recolor_icon_source(r##"<svg stroke="#ffffff"/>"##, "#ab6a57"),
        r##"<svg stroke="#ffffff"/>"##
    );
}

#[test]
fn customization_choices_are_unique_and_whitelisted() {
    let mut names: Vec<_> = icons::CUSTOMIZATION_CHOICES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    names.sort_unstable();
    names.dedup();

    assert_eq!(names.len(), icons::CUSTOMIZATION_CHOICES.len());
    assert!(
        icons::CUSTOMIZATION_CHOICES
            .iter()
            .all(|(name, label)| icons::is_customization_choice(name) && !label.is_empty())
    );
    assert!(!icons::is_customization_choice("folder-from-system-theme"));
}

#[test]
fn custom_emoji_preferences_are_bounded_and_safe_to_render() {
    assert_eq!(icons::custom_emoji("emoji:🚀"), Some("🚀"));
    assert_eq!(icons::custom_emoji("emoji:👨‍👩‍👧‍👦"), Some("👨‍👩‍👧‍👦"));
    assert_eq!(icons::custom_emoji("emoji:"), None);
    assert_eq!(icons::custom_emoji("emoji:\n"), None);
    assert_eq!(
        icons::custom_emoji(&format!("emoji:{}", "x".repeat(65))),
        None
    );
    assert_eq!(contrasting_foreground("#e5a50a"), "#172033");
    assert_eq!(contrasting_foreground("#8e4ec6"), "#f8fafc");
}

#[test]
fn svg_body_preserves_bundled_icon_geometry() {
    assert_eq!(
        svg_body(r#"<svg viewBox="0 0 24 24"><path d="M1 2" /></svg>"#),
        Some(r#"<path d="M1 2" />"#)
    );
}

#[test]
fn folder_emoji_renders_at_high_resolution() {
    gio::resources_register_include!("strata.gresource").expect("resources register");
    let texture = folder_decoration_texture("emoji:🚀", "#e5484d").expect("emoji renders");
    assert_eq!(texture.width(), 96);
    assert_eq!(texture.height(), 96);
}

#[test]
fn primary_icons_rasterize_at_high_resolution() {
    gio::resources_register_include!("strata.gresource").expect("resources register");
    let texture = primary_icon_texture(icons::DOCUMENTS, "#8bc9eb").expect("icon renders");
    assert_eq!(texture.width(), 96);
    assert_eq!(texture.height(), 96);
}
