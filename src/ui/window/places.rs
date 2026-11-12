// SPDX-License-Identifier: MIT

//! The catalogue of standard sidebar places: identifiers, icons, chord keys
//! and the persisted order.

use gtk::glib;

use crate::{model::Location, ui::preferences::PreferenceManager};

use super::home_directory;

pub(super) const STANDARD_PLACE_IDS: &[&str] = &[
    "home",
    "computer",
    "trash",
    "network",
    "recent",
    "desktop",
    "documents",
    "downloads",
    "music",
    "pictures",
    "videos",
];

pub(super) fn is_standard_place_location(location: &Location) -> bool {
    let Some(path) = location.native_path() else {
        return false;
    };
    if path == home_directory() {
        return true;
    }
    [
        glib::UserDirectory::Desktop,
        glib::UserDirectory::Documents,
        glib::UserDirectory::Downloads,
        glib::UserDirectory::Music,
        glib::UserDirectory::Pictures,
        glib::UserDirectory::Videos,
    ]
    .into_iter()
    .filter_map(glib::user_special_dir)
    .any(|standard| standard == path)
}

pub(super) fn should_show_standard_place(
    id: &str,
    path: &std::path::Path,
    home: &std::path::Path,
) -> bool {
    id != "desktop" || path != home
}

pub(super) fn sidebar_standard_place_visible(manager: &PreferenceManager, id: &str) -> bool {
    match id {
        "desktop" => manager.sidebar_show_desktop(),
        "documents" => manager.sidebar_show_documents(),
        "downloads" => manager.sidebar_show_downloads(),
        "music" => manager.sidebar_show_music(),
        "pictures" => manager.sidebar_show_pictures(),
        "videos" => manager.sidebar_show_videos(),
        _ => true,
    }
}

pub(super) fn resolve_place_order(persisted: &[String]) -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    for id in persisted {
        if let Some(canonical) = canonical_place_id(id)
            && !order.contains(&canonical)
        {
            order.push(canonical);
        }
    }
    for (index, &id) in STANDARD_PLACE_IDS.iter().enumerate() {
        insert_missing_place(&mut order, index, id);
    }
    order
}

fn insert_missing_place(order: &mut Vec<&'static str>, index: usize, id: &'static str) {
    if !order.contains(&id) {
        order.insert(upgraded_place_position(order, index), id);
    }
}

fn canonical_place_id(id: &str) -> Option<&'static str> {
    STANDARD_PLACE_IDS
        .iter()
        .copied()
        .find(|known| *known == id)
}

/// Preserve the old sidebar layout when upgrading orders without special places:
/// a missing place lands after the closest earlier standard place already present.
fn upgraded_place_position(order: &[&'static str], index: usize) -> usize {
    STANDARD_PLACE_IDS[..index]
        .iter()
        .rev()
        .find_map(|earlier| {
            order
                .iter()
                .position(|place| place == earlier)
                .map(|position| position + 1)
        })
        .unwrap_or(0)
}

/// Only the 10xer Go chord shows these keycaps, so they always show that map.
pub(super) fn place_keycap(step: crate::ui::keys::Command) -> String {
    crate::ui::keys::label_in_map(
        &rust_i18n::locale(),
        &crate::ui::keys::Keys::One(step.shown()),
        crate::ui::keys::Map::Tenxer,
    )
}

pub(super) fn pin_keycap(ordinal: usize) -> Option<String> {
    use crate::ui::keys::bindings::PIN_NUMBERS;
    let pin = u8::try_from(ordinal + 1)
        .ok()
        .filter(|pin| PIN_NUMBERS.contains(*pin))?;
    Some(crate::ui::keys::digit_label(PIN_NUMBERS.step(), pin))
}

pub(super) fn standard_place_chord_key(id: &str) -> Option<crate::ui::keys::Command> {
    use crate::ui::keys::bindings::{GO_DOCUMENTS, GO_DOWNLOADS, GO_MUSIC, GO_PICTURES, GO_VIDEOS};
    match id {
        "documents" => Some(GO_DOCUMENTS),
        "downloads" => Some(GO_DOWNLOADS),
        "music" => Some(GO_MUSIC),
        "pictures" => Some(GO_PICTURES),
        "videos" => Some(GO_VIDEOS),
        _ => None,
    }
}

pub(super) fn standard_place(
    id: &str,
) -> Option<(&'static str, &'static str, glib::UserDirectory)> {
    match id {
        "desktop" => Some((
            crate::assets::icons::FOLDER,
            "Desktop",
            glib::UserDirectory::Desktop,
        )),
        "documents" => Some((
            crate::assets::icons::DOCUMENTS,
            "Documents",
            glib::UserDirectory::Documents,
        )),
        "downloads" => Some((
            crate::assets::icons::DOWNLOADS,
            "Downloads",
            glib::UserDirectory::Downloads,
        )),
        "music" => Some((
            crate::assets::icons::MUSIC,
            "Music",
            glib::UserDirectory::Music,
        )),
        "pictures" => Some((
            crate::assets::icons::PICTURES,
            "Pictures",
            glib::UserDirectory::Pictures,
        )),
        "videos" => Some((
            crate::assets::icons::VIDEOS,
            "Videos",
            glib::UserDirectory::Videos,
        )),
        _ => None,
    }
}
