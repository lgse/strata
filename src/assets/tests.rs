// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::TextureExt;

use super::{
    CHROME_ICON_PX, contrasting_foreground, folder_decoration_texture, icons, primary_icon_texture,
    primary_icon_texture_at, recolor_icon_source, svg_body, texture_px_for_pixel_size,
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

#[test]
fn chrome_icon_textures_are_twice_the_toolbar_size() {
    assert_eq!(texture_px_for_pixel_size(CHROME_ICON_PX), 32);
    assert_eq!(texture_px_for_pixel_size(48), 96);
    assert_eq!(texture_px_for_pixel_size(-1), 96);
    assert_eq!(texture_px_for_pixel_size(i32::MAX), 96);
    gio::resources_register_include!("strata.gresource").expect("resources register");
    let texture = primary_icon_texture_at(icons::SEARCH, "#8bc9eb", 32).expect("icon renders");
    assert_eq!(texture.width(), 32);
    assert_eq!(texture.height(), 32);
}

#[test]
fn chrome_icons_use_header_bar_pixel_size() {
    crate::test_support::gtk_test(
        "assets::tests::chrome_icons_use_header_bar_pixel_size",
        || {
            use gtk::prelude::*;
            let icon = crate::assets::chrome_icon(icons::SEARCH);
            assert_eq!(icon.pixel_size(), CHROME_ICON_PX);
            assert_eq!(icon.halign(), gtk::Align::Center);
            assert_eq!(icon.valign(), gtk::Align::Center);
            let texture_width =
                |image: &gtk::Image| image.paintable().expect("icon texture").intrinsic_width();
            assert_eq!(texture_width(&icon), 32);
            crate::assets::set_primary_icon(&icon, icons::X);
            assert_eq!(texture_width(&icon), 32);
            crate::assets::apply_primary_icon(&icon, icons::X, "#ffffff");
            assert_eq!(texture_width(&icon), 32);
            for size in [16, 20, 48] {
                let ordinary = crate::assets::primary_icon(icons::SEARCH, size);
                assert_eq!(texture_width(&ordinary), 96);
            }
        },
    );
}
