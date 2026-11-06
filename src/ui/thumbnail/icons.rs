// SPDX-License-Identifier: MIT

//! Fallback icons and the per-path customizations (custom icons, emoji and
//! folder colors) drawn in place of a thumbnail, kept current as they change.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
};

use gtk::{gdk, glib, prelude::*};

use super::{
    super::preferences::PreferenceManager, ThumbnailSlot, cancel_thumbnail,
    clear_displayed_thumbnail, prepare_thumbnail_target, viewport,
};
use crate::{
    assets::{self, IconContext, icons::FOLDER},
    model::FolderColorValue,
};

thread_local! {
    pub(super) static TRACKED_CUSTOMIZED_ICONS: RefCell<HashMap<usize, TrackedCustomizedIcon>> =
        RefCell::new(HashMap::new());
    static TRACKED_CUSTOMIZED_IMAGES: RefCell<HashMap<usize, TrackedCustomizedImage>> =
        RefCell::new(HashMap::new());
    static REFRESHING_CUSTOMIZED_ICONS: Cell<bool> = const { Cell::new(false) };
}

pub(super) struct TrackedCustomizedIcon {
    image: glib::WeakRef<ThumbnailSlot>,
    path: PathBuf,
    icon: String,
    customized: bool,
}

struct TrackedCustomizedImage {
    image: glib::WeakRef<gtk::Image>,
    path: PathBuf,
    default_icon: String,
    kind: ImageIconKind,
}

#[derive(Clone, Copy)]
enum ImageIconKind {
    Default,
    Folder,
}

/// The size, scale and context a slot renders its icon at.
#[derive(Clone, Copy)]
struct IconSizing {
    size: i32,
    scale_factor: i32,
    context: IconContext,
}

impl IconSizing {
    fn of(image: &ThumbnailSlot) -> Self {
        Self {
            size: image.icon_pixel_size(),
            scale_factor: image.scale_factor(),
            context: image.icon_context(),
        }
    }

    fn paint(self, icon: &str, color: &str) -> Option<gdk::Texture> {
        assets::sized_icon_paintable(icon, color, self.size, self.scale_factor, self.context)
    }
}

/// What the user set for a path: a custom icon or emoji, a folder color, or both.
struct Customization {
    icon: Option<String>,
    color: Option<FolderColorValue>,
}

impl Customization {
    fn of(path: &Path) -> Self {
        let preference_manager = PreferenceManager::shared();
        Self {
            icon: preference_manager.custom_icon(path),
            color: preference_manager.folder_color(path),
        }
    }

    fn is_set(&self) -> bool {
        self.icon.is_some() || self.color.is_some()
    }

    fn color_or_primary(&self) -> String {
        self.color
            .as_ref()
            .map_or_else(assets::primary_icon_color, |color| color.hex().to_owned())
    }

    /// A custom icon that is an emoji rather than a named icon.
    fn emoji(&self) -> Option<&str> {
        self.icon.as_deref().and_then(assets::icons::custom_emoji)
    }
}

pub(in crate::ui) fn show_fallback_icon(image: &ThumbnailSlot, icon: &str, size: i32) {
    viewport::cancel_metadata(image.as_ptr() as usize);
    image.set_thumbnail_source(None);
    set_fallback_icon(image, None, icon, size);
}

pub(super) fn set_fallback_icon(
    image: &ThumbnailSlot,
    path: Option<&Path>,
    icon: &str,
    size: i32,
) -> (usize, u64) {
    let ids = prepare_thumbnail_target(image, size);
    clear_displayed_thumbnail(image);
    let (texture, customized) = path_icon_texture(path, icon, IconSizing::of(image));
    image.set_fallback(icon, texture.as_ref());
    if let Some(p) = path {
        register_tracked_icon(image, p, icon, customized);
    } else {
        TRACKED_CUSTOMIZED_ICONS.with_borrow_mut(|icons| {
            icons.remove(&(image.as_ptr() as usize));
        });
    }
    ids
}

/// A metadata fill can find that a bound folder needs administrator access; a
/// folder the user customized keeps its icon, as a fresh bind would.
pub(in crate::ui) fn show_locked_folder(image: &ThumbnailSlot) {
    use crate::assets::icons::FOLDER_LOCK;
    if image.fallback_icon().as_deref() != Some(FOLDER) {
        return;
    }
    let customized = TRACKED_CUSTOMIZED_ICONS.with_borrow_mut(|icons| {
        match icons.get_mut(&(image.as_ptr() as usize)) {
            Some(tracked) if tracked.customized => true,
            Some(tracked) => {
                tracked.icon = FOLDER_LOCK.to_owned();
                false
            }
            None => false,
        }
    });
    if customized {
        return;
    }
    let (texture, _) = path_icon_texture(None, FOLDER_LOCK, IconSizing::of(image));
    image.set_fallback(FOLDER_LOCK, texture.as_ref());
}

pub(in crate::ui) fn show_customized_icon(
    image: &ThumbnailSlot,
    path: &Path,
    fallback_icon: &str,
    size: i32,
) {
    viewport::cancel_metadata(image.as_ptr() as usize);
    image.set_thumbnail_source(None);
    set_fallback_icon(image, Some(path), fallback_icon, size);
}

pub(in crate::ui) fn show_customized_icon_image(
    image: &gtk::Image,
    path: &Path,
    fallback_icon: &str,
    size: i32,
) {
    size_image(image, size);
    show_path_icon_image(image, path, fallback_icon, ImageIconKind::Default);
}

/// Leaves sizing alone: the image follows the live interface scale, and a size
/// request taken here would outlast a later text-size change.
pub(in crate::ui) fn show_customized_folder_image(
    image: &gtk::Image,
    path: &Path,
    default_icon: &str,
) {
    show_path_icon_image(image, path, default_icon, ImageIconKind::Folder);
}

fn size_image(image: &gtk::Image, size: i32) {
    if image.pixel_size() != size {
        image.set_pixel_size(size);
    }
    if image.width_request() != size || image.height_request() != size {
        image.set_size_request(size, size);
    }
}

fn show_path_icon_image(image: &gtk::Image, path: &Path, default_icon: &str, kind: ImageIconKind) {
    apply_path_customization_image(image, path, default_icon, kind);
    TRACKED_CUSTOMIZED_IMAGES.with_borrow_mut(|images| {
        images.retain(|_, tracked| tracked.image.upgrade().is_some());
        images.insert(
            image.as_ptr() as usize,
            TrackedCustomizedImage {
                image: image.downgrade(),
                path: path.to_path_buf(),
                default_icon: default_icon.to_owned(),
                kind,
            },
        );
    });
}

fn path_icon_texture(
    path: Option<&Path>,
    fallback_icon: &str,
    sizing: IconSizing,
) -> (Option<gdk::Texture>, bool) {
    let Some(path) = path else {
        return (
            sizing.paint(fallback_icon, &assets::primary_icon_color()),
            false,
        );
    };
    let customization = Customization::of(path);
    let texture = customized_icon_texture(&customization, fallback_icon, sizing);
    (texture, customization.is_set())
}

fn customized_icon_texture(
    customization: &Customization,
    fallback_icon: &str,
    sizing: IconSizing,
) -> Option<gdk::Texture> {
    if fallback_icon == FOLDER
        && let Some(decoration) = customization.icon.as_deref()
    {
        return assets::sized_folder_decoration_paintable(
            decoration,
            &customization.color_or_primary(),
            sizing.size,
            sizing.scale_factor,
            sizing.context,
        );
    }
    if let Some(emoji) = customization.emoji() {
        return assets::emoji_icon_paintable(emoji);
    }
    let rendered_icon = customization.icon.as_deref().unwrap_or(fallback_icon);
    match &customization.color {
        Some(color) => sizing.paint(rendered_icon, color.hex()),
        None => sizing.paint(rendered_icon, &assets::primary_icon_color()),
    }
}

fn apply_path_customization(image: &ThumbnailSlot, path: &Path, fallback_icon: &str) -> bool {
    let (texture, customized) = path_icon_texture(Some(path), fallback_icon, IconSizing::of(image));
    image.set_fallback(fallback_icon, texture.as_ref());
    customized
}

fn apply_path_customization_image(
    image: &gtk::Image,
    path: &Path,
    default_icon: &str,
    kind: ImageIconKind,
) -> bool {
    let customization = Customization::of(path);
    let folder = matches!(kind, ImageIconKind::Folder) || default_icon == FOLDER;
    set_customized_image_icon(image, &customization, default_icon, folder);
    customization.is_set()
}

fn set_customized_image_icon(
    image: &gtk::Image,
    customization: &Customization,
    default_icon: &str,
    folder: bool,
) {
    if folder && let Some(decoration) = customization.icon.as_deref() {
        assets::set_folder_decoration_icon(image, decoration, &customization.color_or_primary());
    } else if let Some(emoji) = customization.emoji() {
        assets::set_emoji_icon(image, emoji);
    } else {
        let rendered_icon = customization.icon.as_deref().unwrap_or(default_icon);
        match &customization.color {
            Some(color) => assets::set_custom_colored_icon(image, rendered_icon, color.hex()),
            None => assets::set_primary_icon(image, rendered_icon),
        }
    }
}

pub(in crate::ui) fn refresh_slot_icon(image: &ThumbnailSlot) {
    if image.texture().is_some() {
        return;
    }
    let tracked = TRACKED_CUSTOMIZED_ICONS.with_borrow(|icons| {
        icons
            .get(&(image.as_ptr() as usize))
            .map(|tracked| (tracked.path.clone(), tracked.icon.clone()))
    });
    if let Some((path, icon)) = tracked {
        apply_path_customization(image, &path, &icon);
    } else if let Some(icon) = image.fallback_icon() {
        let (texture, _) = path_icon_texture(None, &icon, IconSizing::of(image));
        image.set_fallback(&icon, texture.as_ref());
    }
}

fn register_tracked_icon(image: &ThumbnailSlot, path: &Path, icon: &str, customized: bool) {
    TRACKED_CUSTOMIZED_ICONS.with_borrow_mut(|icons| {
        icons.insert(
            image.as_ptr() as usize,
            TrackedCustomizedIcon {
                image: image.downgrade(),
                path: path.to_path_buf(),
                icon: icon.to_owned(),
                customized,
            },
        );
    });
}

pub(in crate::ui) fn refresh_customized_icons(paths: &[PathBuf]) {
    refresh_tracked_icons(|tracked| paths.iter().any(|candidate| candidate == &tracked.path));
    refresh_tracked_images(|path| paths.iter().any(|candidate| candidate == path));
}

pub(in crate::ui) fn refresh_customized_icons_within(roots: &[PathBuf]) {
    refresh_tracked_icons(|tracked| roots.iter().any(|root| tracked.path.starts_with(root)));
    refresh_tracked_images(|path| roots.iter().any(|root| path.starts_with(root)));
}

pub(in crate::ui) fn refresh_all_customized_icons() {
    refresh_tracked_icons(|_| true);
    refresh_tracked_images(|_| true);
}

fn refresh_tracked_images(matches: impl Fn(&Path) -> bool) {
    let pending = TRACKED_CUSTOMIZED_IMAGES.with_borrow_mut(|images| {
        images.retain(|_, tracked| tracked.image.upgrade().is_some());
        images
            .values()
            .filter(|tracked| matches(&tracked.path))
            .filter_map(|tracked| {
                Some((
                    tracked.image.upgrade()?,
                    tracked.path.clone(),
                    tracked.default_icon.clone(),
                    tracked.kind,
                ))
            })
            .collect::<Vec<_>>()
    });
    for (image, path, default_icon, kind) in pending {
        apply_path_customization_image(&image, &path, &default_icon, kind);
    }
}

fn refresh_tracked_icons(matches: impl Fn(&TrackedCustomizedIcon) -> bool) {
    if REFRESHING_CUSTOMIZED_ICONS.with(|busy| busy.replace(true)) {
        return;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REFRESHING_CUSTOMIZED_ICONS.with(|busy| busy.set(false));
        }
    }
    let _reset = Reset;
    let pending = TRACKED_CUSTOMIZED_ICONS.with(|icons| {
        icons
            .borrow()
            .values()
            .filter(|tracked| matches(tracked))
            .filter_map(|tracked| {
                let image = tracked.image.upgrade()?;
                image
                    .texture()
                    .is_none()
                    .then(|| (image, tracked.path.clone(), tracked.icon.clone()))
            })
            .collect::<Vec<_>>()
    });
    for (image, path, icon) in pending {
        refresh_tracked_icon(&image, &path, &icon);
    }
}

fn refresh_tracked_icon(image: &ThumbnailSlot, path: &Path, icon: &str) {
    if PreferenceManager::shared().custom_icon(path).is_some() {
        cancel_thumbnail(image.as_ptr() as usize);
    }
    let customized = apply_path_customization(image, path, icon);
    TRACKED_CUSTOMIZED_ICONS.with(|icons| {
        let Ok(mut icons) = icons.try_borrow_mut() else {
            return;
        };
        if let Some(tracked) = icons.get_mut(&(image.as_ptr() as usize)) {
            tracked.customized = customized;
        }
    });
}
