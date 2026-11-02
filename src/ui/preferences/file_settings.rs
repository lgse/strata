// SPDX-License-Identifier: MIT

//! Folders and files: startup folder and tabs, folder colors and icons, devices, drops, dates, and updates.

use super::*;

impl PreferenceManager {
    pub fn folder_peeking(&self) -> bool {
        self.preferences.borrow().folder_peeking
    }

    pub fn set_folder_peeking(&self, enabled: bool) {
        self.preferences.borrow_mut().folder_peeking = enabled;
        self.save_preferences();
    }

    pub fn folder_color(&self, path: &Path) -> Option<FolderColorValue> {
        let preferences = self.preferences.borrow();
        if preferences.folder_colors.is_empty() {
            return None;
        }
        let key = path.to_string_lossy();
        let color_name = preferences.folder_colors.get(key.as_ref())?;
        FolderColorValue::parse(color_name)
    }

    pub fn set_folder_color(&self, path: &Path, color: Option<FolderColorValue>) {
        self.set_folder_colors(&[path.to_path_buf()], color);
    }

    pub fn set_folder_colors(&self, paths: &[PathBuf], color: Option<FolderColorValue>) {
        if paths.is_empty() {
            return;
        }
        {
            let mut preferences = self.preferences.borrow_mut();
            for path in paths {
                let key = path.to_string_lossy().into_owned();
                if let Some(color) = &color {
                    preferences
                        .folder_colors
                        .insert(key, color.to_preference_string());
                } else {
                    preferences.folder_colors.remove(&key);
                }
            }
        }
        self.save_preferences();
        super::super::thumbnail::refresh_customized_icons(paths);
    }

    pub fn custom_icon(&self, path: &Path) -> Option<String> {
        let preferences = self.preferences.borrow();
        if preferences.custom_icons.is_empty() {
            return None;
        }
        let key = path.to_string_lossy();
        preferences
            .custom_icons
            .get(key.as_ref())
            .filter(|name| crate::assets::icons::is_customization_choice(name))
            .cloned()
    }

    pub fn set_custom_icon(&self, path: &Path, icon_name: Option<&str>) {
        {
            let mut preferences = self.preferences.borrow_mut();
            let key = path.to_string_lossy().into_owned();
            if let Some(name) =
                icon_name.filter(|name| crate::assets::icons::is_customization_choice(name))
            {
                preferences.custom_icons.insert(key, name.to_owned());
            } else {
                preferences.custom_icons.remove(&key);
            }
        }
        self.save_preferences();
        super::super::thumbnail::refresh_customized_icons(&[path.to_path_buf()]);
    }

    pub fn clear_item_customization(&self, path: &Path) {
        {
            let mut preferences = self.preferences.borrow_mut();
            let key = path.to_string_lossy();
            preferences.folder_colors.remove(key.as_ref());
            preferences.custom_icons.remove(key.as_ref());
        }
        self.save_preferences();
        super::super::thumbnail::refresh_customized_icons(&[path.to_path_buf()]);
    }

    pub fn watch_network_folders(&self) -> bool {
        self.preferences.borrow().watch_network_folders
    }

    pub fn set_watch_network_folders(&self, enabled: bool) {
        self.preferences.borrow_mut().watch_network_folders = enabled;
        self.save_preferences();
    }

    pub fn default_directory(&self) -> Option<PathBuf> {
        self.preferences.borrow().default_directory.clone()
    }

    pub fn set_default_directory(&self, path: Option<PathBuf>) {
        self.preferences.borrow_mut().default_directory = path;
        self.save_preferences();
    }

    pub fn restore_tabs(&self) -> bool {
        self.preferences.borrow().restore_tabs
    }

    pub fn set_restore_tabs(&self, enabled: bool) {
        self.preferences.borrow_mut().restore_tabs = enabled;
        if !enabled {
            super::super::tabs_session::remove();
        }
        self.save_preferences();
    }

    pub(in crate::ui) fn device_label(&self, device_id: &str) -> Option<String> {
        self.preferences
            .borrow()
            .device_labels
            .get(device_id)
            .filter(|label| !label.trim().is_empty() && is_storable_label(label))
            .cloned()
    }

    pub(in crate::ui) fn set_device_label(&self, device_id: &str, label: &str) {
        let label = label.trim();
        if device_id.is_empty() || !is_storable_label(label) {
            return;
        }
        {
            let mut preferences = self.preferences.borrow_mut();
            if label.is_empty() {
                preferences.device_labels.remove(device_id);
            } else {
                preferences
                    .device_labels
                    .insert(device_id.to_owned(), label.to_owned());
            }
        }
        self.save_preferences();
    }

    pub(in crate::ui) fn send_to_recent_destinations(&self, device_id: &str) -> Vec<PathBuf> {
        self.preferences
            .borrow()
            .send_to_recent_destinations
            .get(device_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(in crate::ui) fn remember_send_to_destination(
        &self,
        device_id: &str,
        relative_path: &Path,
        remove_relative_path: Option<&Path>,
    ) {
        if !is_valid_send_to_relative_path(relative_path) {
            return;
        }
        {
            let mut preferences = self.preferences.borrow_mut();
            let recent = preferences
                .send_to_recent_destinations
                .entry(device_id.to_owned())
                .or_default();
            recent.retain(|path| {
                path != relative_path && Some(path.as_path()) != remove_relative_path
            });
            recent.insert(0, relative_path.to_path_buf());
            recent.truncate(SEND_TO_RECENT_DESTINATIONS_LIMIT);
        }
        self.save_preferences();
    }

    pub fn open_folder_after_drop(&self) -> bool {
        self.preferences.borrow().open_folder_after_drop
    }

    pub fn set_open_folder_after_drop(&self, enabled: bool) {
        self.preferences.borrow_mut().open_folder_after_drop = enabled;
        self.save_preferences();
    }

    pub fn date_format(&self) -> crate::util::DateFormat {
        crate::util::DateFormat::parse(&self.preferences.borrow().date_format)
    }

    pub fn set_date_format(&self, format: crate::util::DateFormat) {
        if self.date_format() == format {
            return;
        }
        self.preferences.borrow_mut().date_format = format.as_str().to_owned();
        crate::util::set_date_format(format);
        self.save_preferences();
    }

    pub fn cross_volume_drop_strategy(&self) -> CrossVolumeDropStrategy {
        CrossVolumeDropStrategy::parse(&self.preferences.borrow().cross_volume_drop_strategy)
    }

    pub fn set_cross_volume_drop_strategy(&self, strategy: CrossVolumeDropStrategy) {
        if self.cross_volume_drop_strategy() == strategy {
            return;
        }
        self.preferences.borrow_mut().cross_volume_drop_strategy = strategy.as_str().to_owned();
        self.save_preferences();
    }

    pub fn release_channel(&self) -> Channel {
        crate::services::InstallSource::detect()
            .managed()
            .and_then(crate::services::ManagedInstall::tracked_channel)
            .unwrap_or_else(|| Channel::parse(&self.preferences.borrow().release_channel))
    }

    pub fn set_release_channel(&self, channel: Channel) {
        if crate::services::InstallSource::detect()
            .managed()
            .and_then(crate::services::ManagedInstall::tracked_channel)
            .is_some()
        {
            return;
        }
        self.preferences.borrow_mut().release_channel = channel.as_str().to_owned();
        self.save_preferences();
    }

    pub fn on_release_channel_changed(
        &self,
        anchor: &impl IsA<gtk::Widget>,
        refresh: Rc<dyn Fn()>,
    ) {
        let initial = Cell::new(true);
        self.bind_preference(anchor, Self::release_channel, move |_, _| {
            if !initial.replace(false) {
                refresh();
            }
        });
    }

    pub fn checks_for_updates(&self) -> bool {
        self.preferences.borrow().check_for_updates
    }

    pub fn set_checks_for_updates(&self, enabled: bool) {
        self.preferences.borrow_mut().check_for_updates = enabled;
        self.save_preferences();
    }
}

/// Device labels are short single-line names.
fn is_storable_label(label: &str) -> bool {
    label.chars().count() <= 255 && !label.chars().any(char::is_control)
}
