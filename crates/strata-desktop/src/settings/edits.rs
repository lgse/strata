// SPDX-License-Identifier: MIT

//! Setter and start-up repair rules on [`Preferences`] shared between the GTK and Rune
//! interfaces.

use std::time::Duration;

use crate::{
    sandbox::MediaPreviewBackend,
    services::{Channel, InstallSource, ManagedInstall, SearchLimits},
};

use super::{MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, OmarchyVariant, Preferences};

/// The keys another Strata process can change that this process adopts as defaults.
pub const ADOPTED_DEFAULT_KEYS: [&str; 5] = [
    "sort_key",
    "sort_direction",
    "natural_sort",
    "case_sensitive_sort",
    "icons_thumbnail_size",
];

/// Session state needed to repair loaded theme and mode choices at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeAvailability {
    pub system: bool,
    pub omarchy: bool,
    pub settings_file_exists: bool,
}

impl Preferences {
    /// Repairs loaded theme and mode choices against the themes and modes available in
    /// this session.
    pub fn normalize_theme_selection(
        &mut self,
        theme_is_known: impl Fn(&str) -> bool,
        available: ThemeAvailability,
    ) {
        if !theme_is_known(&self.theme) {
            self.theme = "azure-glow".to_owned();
        }
        let mode_available = match self.mode.as_str() {
            "theme" => true,
            "system" => available.system,
            "omarchy" => available.omarchy,
            _ => false,
        };
        if !mode_available {
            self.mode = "theme".to_owned();
        } else if !available.settings_file_exists && available.omarchy {
            self.mode = "omarchy".to_owned();
        }
    }

    /// The variant remembered for `active_theme`, or the shipped default.
    pub fn omarchy_variant(&self, active_theme: Option<&str>) -> OmarchyVariant {
        active_theme
            .and_then(|theme| self.omarchy_variants.get(theme).copied())
            .unwrap_or_default()
    }

    /// Records `variant` for `active_theme`, removing the entry when it equals the
    /// default. Returns whether the stored choice changed.
    pub fn set_omarchy_variant(&mut self, active_theme: &str, variant: OmarchyVariant) -> bool {
        if self.omarchy_variant(Some(active_theme)) == variant {
            return false;
        }
        if variant == OmarchyVariant::default() {
            self.omarchy_variants.remove(active_theme);
        } else {
            self.omarchy_variants
                .insert(active_theme.to_owned(), variant);
        }
        true
    }

    /// Trims `theme`, migrates any legacy global contrast choice into `omarchy_variants`
    /// under it, and returns the trimmed theme name.
    pub fn migrate_active_omarchy_theme(&mut self, theme: &str) -> String {
        let theme = theme.trim().to_owned();
        // Older versions always saved "original", so only a contrast choice was deliberate.
        if let Some(legacy) = self.legacy_omarchy_variant.take()
            && legacy == OmarchyVariant::Contrast
        {
            self.omarchy_variants.entry(theme.clone()).or_insert(legacy);
        }
        theme
    }

    pub fn set_hardware_accelerated_video_previews(&mut self, enabled: bool) {
        self.hardware_accelerated_video_previews = Some(enabled);
    }

    /// Stores `backend` when it is one of the hardware choices. `Software` is controlled
    /// by `hardware_accelerated_video_previews` and returns `false` without mutating.
    pub fn set_video_preview_backend(&mut self, backend: MediaPreviewBackend) -> bool {
        let backend = match backend {
            MediaPreviewBackend::Automatic => "automatic",
            MediaPreviewBackend::VaApi => "vaapi",
            MediaPreviewBackend::Vulkan => "vulkan",
            MediaPreviewBackend::Software => return false,
        };
        self.video_preview_backend = backend.to_owned();
        true
    }

    /// The effective release channel, honouring a package-managed install's tracked channel.
    pub fn release_channel(&self) -> Channel {
        InstallSource::detect()
            .managed()
            .and_then(ManagedInstall::tracked_channel)
            .unwrap_or_else(|| Channel::parse(&self.release_channel))
    }

    /// Stores `channel` unless a package manager owns the channel.
    pub fn set_release_channel(&mut self, channel: Channel) -> bool {
        if InstallSource::detect()
            .managed()
            .and_then(ManagedInstall::tracked_channel)
            .is_some()
        {
            return false;
        }
        self.release_channel = channel.as_str().to_owned();
        true
    }

    /// Search walk time and entry limits; zero means no limit.
    pub fn search_limits(&self) -> SearchLimits {
        SearchLimits {
            time_budget: (self.search_time_limit != 0)
                .then(|| Duration::from_secs(u64::from(self.search_time_limit))),
            max_entries: (self.search_entry_limit != 0).then_some(self.search_entry_limit as usize),
        }
    }

    /// Adopts the default sort and icon size saved by another Strata process. Returns
    /// whether any of the five fields changed.
    pub fn adopt_defaults(&mut self, saved: Self) -> bool {
        let size = saved
            .icons_thumbnail_size
            .clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        let current = (
            self.sort_key.as_str(),
            self.sort_direction.as_str(),
            self.natural_sort,
            self.case_sensitive_sort,
            self.icons_thumbnail_size,
        );
        let incoming = (
            saved.sort_key.as_str(),
            saved.sort_direction.as_str(),
            saved.natural_sort,
            saved.case_sensitive_sort,
            size,
        );
        if current == incoming {
            return false;
        }
        self.sort_key = saved.sort_key;
        self.sort_direction = saved.sort_direction;
        self.natural_sort = saved.natural_sort;
        self.case_sensitive_sort = saved.case_sensitive_sort;
        self.icons_thumbnail_size = size;
        true
    }
}

#[cfg(test)]
mod tests;
