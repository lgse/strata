// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{folder_decoration_texture, icons, recolor_icon_source};

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
            for resource in names {
                let name = resource.strip_suffix(".svg").expect("bundled SVG icon");
                let image = super::primary_icon(name, 18);
                let first = image
                    .paintable()
                    .expect("cold icon renders without a decoder process");
                assert!(
                    first.is::<gtk::gdk::MemoryTexture>(),
                    "raw pixels, not a loader-backed icon"
                );
                super::set_custom_colored_icon(&image, name, "#d46b31");
                let recolored = image.paintable().expect("live color update renders");
                assert!(recolored.is::<gtk::gdk::MemoryTexture>());
                assert_ne!(
                    first, recolored,
                    "recoloring must request the new color variant"
                );
                let texture = recolored.downcast::<gtk::gdk::Texture>().expect("texture");
                let stride = texture.width() as usize * 4;
                let mut pixels = vec![0; stride * texture.height() as usize];
                texture.download(&mut pixels, stride);
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| u32::from_ne_bytes(*pixel) >> 24 != 0),
                    "bundled icon must render visible geometry: {name}"
                );
            }
            assert!(folder_decoration_texture(icons::PICTURES, "#d46b31").is_some());
            assert!(super::emoji_icon_paintable("🚀").is_some());
            assert!(folder_decoration_texture("emoji:🚀", "#d46b31").is_some());
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
            assert_eq!(cache.entries.len(), 0);
            assert_eq!(cache.recent.len(), 0);
        },
    );
}
