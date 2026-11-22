// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_kit::{
    icons::{catppuccin, lucide},
    look::Look,
    theme::Library,
};

use super::*;

fn file_entry(name: &str, kind: EntryKind, listable: MetadataValue<bool>) -> FileEntry {
    FileEntry {
        location: Location::local(PathBuf::from("/tmp").join(name)),
        thumbnail_path: None,
        display_name: name.to_owned(),
        native_name: name.into(),
        kind,
        is_hidden: name.starts_with('.'),
        listable,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        size: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
    }
}

fn dark_look(brand_icons: bool) -> Look {
    let library = Library::new(Vec::new());
    let theme = library.resolve(Some("catppuccin-mocha"));
    let mut look = theme.look();
    look.icons = if brand_icons {
        catppuccin::for_background(theme.dark_background())
    } else {
        lucide::SET
    };
    look
}

fn rgba_at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn assert_transparent_corners(rgba: &[u8], w: u32, h: u32) {
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        assert_eq!(
            rgba_at(rgba, w, x, y),
            [0, 0, 0, 0],
            "corner ({x}, {y}) must be fully transparent"
        );
    }
}

#[test]
fn entry_icon_selection_resolves_file_types_and_folder_states() {
    let look = dark_look(true);

    let nix = file_entry("flake.nix", EntryKind::File, MetadataValue::Unknown);
    let spec = PreviewSpec::from_entries(&look, &[nix]);
    assert!(
        std::ptr::eq(
            spec.icon,
            look.icon_named(crate::entry_icon::catppuccin::NIX)
                .expect("nix icon")
        ),
        "drag preview for .nix file must resolve the NixOS Catppuccin icon"
    );

    let folder = file_entry("projects", EntryKind::Directory, MetadataValue::Known(true));
    let spec = PreviewSpec::from_entries(&look, &[folder]);
    assert!(std::ptr::eq(
        spec.icon,
        look.icon_named(crate::entry_icon::FOLDER)
            .expect("folder icon")
    ));

    let locked = file_entry("secret", EntryKind::Directory, MetadataValue::Known(false));
    let spec = PreviewSpec::from_entries(&look, &[locked]);
    assert!(std::ptr::eq(
        spec.icon,
        look.icon_named(crate::entry_icon::FOLDER_LOCK)
            .expect("locked folder icon")
    ));

    let broken = file_entry("dangling", EntryKind::SymbolicLink, MetadataValue::Unknown);
    let spec = PreviewSpec::from_entries(&look, &[broken]);
    assert!(std::ptr::eq(
        spec.icon,
        look.icon_named(crate::entry_icon::X).expect("broken icon")
    ));
}

#[test]
fn nix_drag_icon_renders_crisp_brand_colors_on_transparent_background() {
    let look = dark_look(true);
    let nix = file_entry("flake.nix", EntryKind::File, MetadataValue::Unknown);
    let image = drag_icon(&look, &[nix], 2.0)
        .render_for_test()
        .expect("nix drag icon rendered");
    let (w, h) = image.size();
    let rgba = image.rgba();

    assert_transparent_corners(rgba, w, h);

    // Catppuccin Mocha Nix icon uses bright sky/blue tones, never dim monochrome gray.
    let bright_blue_pixels = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] >= 200 && p[2] >= 180 && p[2] > p[0].saturating_add(40))
        .count();
    assert!(
        bright_blue_pixels > 40,
        "expected crisp Catppuccin blue/sky Nix icon pixels, found only {bright_blue_pixels}"
    );
}

#[test]
fn theme_colored_drag_icon_uses_active_theme_icon_accent() {
    let look = dark_look(false);
    let folder = file_entry(
        "documents",
        EntryKind::Directory,
        MetadataValue::Known(true),
    );
    let image = drag_icon(&look, &[folder], 1.0)
        .render_for_test()
        .expect("folder drag icon rendered");
    let (w, h) = image.size();
    let rgba = image.rgba();

    assert_transparent_corners(rgba, w, h);

    // Catppuccin Mocha icon_accent is bright blue/lavender, whereas the old
    // `Look::fallback(false)` rendered a dim dark-gray stroke.
    let bright_opaque_pixels = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] >= 200 && u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2]) > 350)
        .count();
    assert!(
        bright_opaque_pixels > 20,
        "theme-colored folder icon must be tinted with active theme icon_accent, not dim fallback"
    );
}

#[test]
fn multi_item_drag_icon_keeps_transparent_corners_and_draws_count_badge() {
    let look = dark_look(true);
    let entries = vec![
        file_entry("flake.nix", EntryKind::File, MetadataValue::Unknown),
        file_entry("shell.nix", EntryKind::File, MetadataValue::Unknown),
        file_entry("default.nix", EntryKind::File, MetadataValue::Unknown),
    ];
    let single = drag_icon(&look, &entries[..1], 1.0)
        .render_for_test()
        .expect("single icon rendered");
    let multi = drag_icon(&look, &entries, 1.0)
        .render_for_test()
        .expect("multi icon rendered");
    let (w, h) = multi.size();

    assert_transparent_corners(multi.rgba(), w, h);

    // Multi-item badge adds opaque pixels in the bottom-right quadrant where single-item is clear.
    let bottom_right_opaque = |img: &strata_platform::dnd::DragImage| {
        let (iw, ih) = img.size();
        let mut count = 0usize;
        for y in (ih * 2 / 3)..ih {
            for x in (iw * 2 / 3)..iw {
                if rgba_at(img.rgba(), iw, x, y)[3] >= 200 {
                    count += 1;
                }
            }
        }
        count
    };
    assert!(
        bottom_right_opaque(&multi) > bottom_right_opaque(&single) + 30,
        "multi-item drag icon must draw the count badge in the bottom-right"
    );
}
