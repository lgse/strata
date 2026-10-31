// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{icons, recolor_icon_source};

pub(crate) fn primary_icon_paintable(name: &str) -> Option<gtk::gdk::Texture> {
    custom_colored_icon_paintable(name, &super::primary_icon_color())
}

pub(crate) fn custom_colored_icon_paintable(name: &str, color: &str) -> Option<gtk::gdk::Texture> {
    super::primary_icon_texture_at(
        name,
        color,
        super::ICON_TEXTURE_PX,
        24,
        super::IconContext::Interface,
    )
}

pub(crate) fn folder_decoration_paintable(
    decoration: &str,
    color: &str,
) -> Option<gtk::gdk::Texture> {
    super::sized_folder_decoration_paintable(
        decoration,
        color,
        24,
        1,
        super::IconContext::Interface,
    )
}

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
fn icon_cache_distinguishes_logical_size_from_display_resolution() {
    crate::test_support::gtk_test(
        "assets::tests::icon_cache_distinguishes_logical_size_from_display_resolution",
        || {
            let render = |name, color, size, scale| {
                super::sized_icon_paintable(name, color, size, scale, super::IconContext::Grid)
                    .expect("render icon")
            };
            for name in [icons::FOLDER, icons::FILE_CODE, icons::FILE_SPREADSHEET] {
                let small = render(name, "#123456", 64, 2);
                let large = render(name, "#123456", 128, 1);
                let interface = super::primary_icon_texture_at(
                    name,
                    "#123456",
                    128,
                    64,
                    super::IconContext::Interface,
                )
                .expect("interface icon");
                assert_ne!(
                    small, interface,
                    "grid weight must not leak into interface icons"
                );
                assert_ne!(
                    small, large,
                    "equal raster resolution must not share stroke weight"
                );
                assert_eq!(small, render(name, "#123456", 64, 2));
                assert_eq!(large, render(name, "#123456", 128, 1));
                assert_ne!(large, render(name, "#abcdef", 128, 1));
            }
            let render_decoration = |decoration, size, scale| {
                super::sized_folder_decoration_paintable(
                    decoration,
                    "#123456",
                    size,
                    scale,
                    super::IconContext::Grid,
                )
                .expect("render decorated folder")
            };
            for decoration in [icons::PICTURES, "emoji:🚀"] {
                let small = render_decoration(decoration, 64, 2);
                let large = render_decoration(decoration, 128, 1);
                assert_ne!(small, large);
                assert_eq!(small, render_decoration(decoration, 64, 2));
            }
        },
    );
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
}

#[test]
fn cold_interface_icons_render_when_decoder_workers_cannot_start() {
    crate::test_support::gtk_test(
        "assets::tests::cold_interface_icons_render_when_decoder_workers_cannot_start",
        || {
            use gtk::prelude::*;
            use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
            struct RestoreLimit(Rlimit);
            impl Drop for RestoreLimit {
                fn drop(&mut self) {
                    setrlimit(Resource::Nproc, self.0).expect("restore process limit");
                }
            }
            let saved = getrlimit(Resource::Nproc);
            let _restore = RestoreLimit(saved);
            setrlimit(
                Resource::Nproc,
                Rlimit {
                    current: Some(0),
                    ..saved
                },
            )
            .expect("disable new decoder workers in this isolated process");
            super::ICON_TEXTURES.with(|cache| cache.borrow_mut().clear());
            let names = gio::resources_enumerate_children(
                "/io/github/lgse/Strata/icons/scalable/actions/",
                gio::ResourceLookupFlags::NONE,
            )
            .expect("bundled icon resources");
            assert!(!names.is_empty());
            let names = names
                .iter()
                .map(|resource| resource.strip_suffix(".svg").expect("bundled SVG icon"))
                .chain([icons::STRATA_LOGO]);
            for name in names {
                let image = super::primary_icon(name, 18);
                let first = image
                    .paintable()
                    .expect("cold icon renders without a decoder process");
                assert!(
                    first.is::<gtk::gdk::MemoryTexture>(),
                    "raw pixels, not a loader-backed icon"
                );
                assert_eq!(
                    first
                        .clone()
                        .downcast::<gtk::gdk::Texture>()
                        .expect("interface texture"),
                    primary_icon_paintable(name).expect("default interface weight"),
                );
                super::set_custom_colored_icon(&image, name, "#d46b31");
                let recolored = image.paintable().expect("live color update renders");
                assert!(recolored.is::<gtk::gdk::MemoryTexture>());
                assert_ne!(
                    first, recolored,
                    "recoloring must request the new color variant"
                );
                let texture = recolored.downcast::<gtk::gdk::Texture>().expect("texture");
                assert!(
                    crate::test_support::texture_has_visible_pixels(&texture),
                    "bundled icon must render visible geometry: {name}"
                );
            }
            assert!(folder_decoration_paintable(icons::PICTURES, "#d46b31").is_some());
            assert!(super::emoji_icon_paintable("🚀").is_some());
            assert!(folder_decoration_paintable("emoji:🚀", "#d46b31").is_some());
        },
    );
}

#[test]
fn nested_spin_button_steps_resolve_to_bundled_icons() {
    crate::test_support::gtk_test(
        "assets::tests::nested_spin_button_steps_resolve_to_bundled_icons",
        || {
            let spin = gtk::SpinButton::with_range(0.0, 10.0, 1.0);
            let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
            container.append(&spin);
            super::use_bundled_spin_icons(&container);

            let display = gtk::gdk::Display::default().expect("display");
            let theme = gtk::IconTheme::for_display(&display);
            let mut steps = Vec::new();
            let mut child = spin.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                let Some(button) = widget.downcast_ref::<gtk::Button>() else {
                    continue;
                };
                let name = button.icon_name().expect("step icon").to_string();
                let uri = theme
                    .lookup_icon(
                        &name,
                        &[],
                        16,
                        1,
                        gtk::TextDirection::Ltr,
                        gtk::IconLookupFlags::empty(),
                    )
                    .file()
                    .expect("resolved icon file")
                    .uri();
                assert!(
                    uri.starts_with("resource:///io/github/lgse/Strata/icons/"),
                    "{name} resolved outside the bundle: {uri}"
                );
                steps.push(name);
            }
            steps.sort();
            assert_eq!(steps, [icons::MINUS_SYMBOLIC, icons::PLUS_SYMBOLIC]);
        },
    );
}

#[test]
fn icon_texture_cache_bounds_entries_and_preserves_lru() {
    crate::test_support::gtk_test(
        "assets::tests::icon_texture_cache_bounds_entries_and_preserves_lru",
        || {
            let mut cache = super::IconTextureCache::default();
            let format = if cfg!(target_endian = "little") {
                gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied
            } else {
                gtk::gdk::MemoryFormat::A8r8g8b8Premultiplied
            };
            let dummy_texture = gtk::gdk::MemoryTexture::new(
                1,
                1,
                format,
                &gtk::glib::Bytes::from_static(&[0, 0, 0, 0]),
                4,
            )
            .upcast::<gtk::gdk::Texture>();

            for index in 0..super::ICON_TEXTURE_CACHE_LIMIT {
                let key = (format!("icon-{index}"), "#000000".to_owned(), 16);
                cache.insert(key, dummy_texture.clone());
            }
            assert_eq!(cache.entries.len(), super::ICON_TEXTURE_CACHE_LIMIT);

            let hot_key = ("icon-0".to_owned(), "#000000".to_owned(), 16);
            for _ in 0..super::ICON_TEXTURE_CACHE_LIMIT * 5 {
                assert!(cache.get(&hot_key).is_some());
            }
            assert!(cache.recent.len() <= super::ICON_TEXTURE_CACHE_LIMIT * 4);

            let new_key = ("icon-new".to_owned(), "#000000".to_owned(), 16);
            cache.insert(new_key.clone(), dummy_texture.clone());

            assert!(cache.get(&hot_key).is_some());
            let evicted_key = ("icon-1".to_owned(), "#000000".to_owned(), 16);
            assert!(cache.get(&evicted_key).is_none());
            assert!(cache.get(&new_key).is_some());
            assert_eq!(cache.entries.len(), super::ICON_TEXTURE_CACHE_LIMIT);

            cache.clear();
            assert!(cache.get(&hot_key).is_none());
            assert!(cache.get(&new_key).is_none());
            cache.insert(hot_key.clone(), dummy_texture);
            assert!(cache.get(&hot_key).is_some());
        },
    );
}

#[test]
fn catppuccin_file_icons_render_in_both_flavors() {
    crate::test_support::gtk_test(
        "assets::tests::catppuccin_file_icons_render_in_both_flavors",
        || {
            use super::{CatppuccinFlavor, IconContext};
            let names = gio::resources_enumerate_children(
                "/io/github/lgse/Strata/icons/catppuccin/",
                gio::ResourceLookupFlags::NONE,
            )
            .expect("bundled Catppuccin icons");
            assert!(names.len() > 300);
            for resource in &names {
                let name = resource.strip_suffix(".svg").expect("bundled SVG icon");
                assert!(name.starts_with(super::CATPPUCCIN_ICON_PREFIX), "{name}");
                super::set_catppuccin_flavor(CatppuccinFlavor::Mocha);
                let mocha = primary_icon_paintable(name).unwrap_or_else(|| panic!("{name}"));
                assert!(mocha.is::<gtk::gdk::MemoryTexture>(), "{name}");
                super::set_catppuccin_flavor(CatppuccinFlavor::Latte);
                let latte = primary_icon_paintable(name).unwrap_or_else(|| panic!("{name}"));
                assert_ne!(
                    mocha, latte,
                    "{name} must follow the flavor, not the accent"
                );
            }

            let name = "strata-ctp-rust";
            let grid = |size, scale| {
                super::sized_icon_paintable(name, "#123456", size, scale, IconContext::Grid)
                    .expect("render icon")
            };
            let interface =
                super::primary_icon_texture_at(name, "#123456", 128, 64, IconContext::Interface)
                    .expect("interface icon");
            assert_ne!(grid(64, 2), interface);
            assert_ne!(grid(64, 2), grid(128, 1));
            assert_eq!(grid(128, 1), grid(128, 1));
        },
    );
}

#[test]
fn catppuccin_file_icons_can_follow_the_theme_accent() {
    crate::test_support::gtk_test(
        "assets::tests::catppuccin_file_icons_can_follow_the_theme_accent",
        || {
            use super::{CatppuccinFlavor, IconContext};
            let render = |color| {
                super::sized_icon_paintable("strata-ctp-rust", color, 64, 1, IconContext::Grid)
                    .expect("render icon")
            };
            super::set_catppuccin_flavor(CatppuccinFlavor::Mocha);
            let brand = render("#123456");
            assert_eq!(brand, render("#654321"), "brand colors ignore the accent");

            super::set_catppuccin_theme_colors(true);
            let themed = render("#123456");
            assert_ne!(themed, brand);
            assert_ne!(themed, render("#654321"), "theme colors follow the accent");
            super::set_catppuccin_flavor(CatppuccinFlavor::Latte);
            assert_eq!(themed, render("#123456"), "theme colors ignore the flavor");

            super::set_catppuccin_theme_colors(false);
            super::set_catppuccin_flavor(CatppuccinFlavor::Mocha);
            assert_eq!(render("#123456"), brand);
        },
    );
}
