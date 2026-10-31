// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use crate::{
    model::Location,
    ui::{
        keys::{
            KeyPress, Map,
            bindings::{THUMBNAILS_LARGER, THUMBNAILS_RESET, THUMBNAILS_SMALLER},
        },
        preferences::PreferenceManager,
    },
};

use super::DEFAULT_ICONS_THUMBNAIL_SIZE;

#[cfg(test)]
mod tests;

/// The sizes the thumbnail shortcuts and the wheel move through, finer where
/// one step is a larger share of the size.
const STEPS: [i32; 17] = [
    32, 36, 40, 44, 48, 56, 64, 72, 80, 96, 112, 128, 144, 160, 192, 224, 256,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum ThumbnailResize {
    /// Positive steps enlarge.
    Step(i32),
    Reset,
}

impl ThumbnailResize {
    /// The keys that resize from anywhere; the 10xer map's plain keys resize
    /// only from the icons, through [`Self::for_tenxer_key`].
    pub(in crate::ui) fn for_shortcut(press: &KeyPress) -> Option<Self> {
        if press.is(THUMBNAILS_LARGER.in_map(Map::Default)) {
            Some(Self::Step(1))
        } else if press.is(THUMBNAILS_SMALLER.in_map(Map::Default)) {
            Some(Self::Step(-1))
        } else if press.is(THUMBNAILS_RESET) {
            Some(Self::Reset)
        } else {
            None
        }
    }

    pub(in crate::ui) fn for_tenxer_key(press: &KeyPress) -> Option<Self> {
        if press.is(THUMBNAILS_LARGER) {
            Some(Self::Step(1))
        } else if press.is(THUMBNAILS_SMALLER) {
            Some(Self::Step(-1))
        } else {
            None
        }
    }

    /// Moves the slider, which saves the size the way dragging it does, and
    /// returns the size it lands on.
    pub(in crate::ui) fn apply(self, scale: &gtk::Scale, location: Option<&Location>) -> i32 {
        let size = match self {
            Self::Step(steps) => stepped(scale.value().round() as i32, steps),
            Self::Reset => {
                let preferences = PreferenceManager::shared();
                if let Some(location) = location.filter(|location| preferences.remembers(location))
                {
                    // Preferences move the slider to the size the folder now follows.
                    preferences.reset_folder_icons_size(location);
                    return preferences.icons_size_for(Some(location));
                }
                if preferences.remember_folder_views() {
                    preferences.icons_thumbnail_size()
                } else {
                    DEFAULT_ICONS_THUMBNAIL_SIZE
                }
            }
        };
        scale.set_value(f64::from(size));
        size
    }
}

/// `size` moved `steps` along [`STEPS`]. From a size between two steps, the
/// first step lands on the nearer one in the direction moved.
pub(super) fn stepped(size: i32, steps: i32) -> i32 {
    let index = match steps.signum() {
        0 => return size,
        1 => STEPS.partition_point(|&step| step <= size) as i32 - 1 + steps,
        _ => STEPS.partition_point(|&step| step < size) as i32 + steps,
    };
    STEPS[index.clamp(0, STEPS.len() as i32 - 1) as usize]
}
