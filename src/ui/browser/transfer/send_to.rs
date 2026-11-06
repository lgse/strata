// SPDX-License-Identifier: MIT

use super::{ConflictFocus, TransferRequest};
use crate::model::Location;
use crate::ui::browser::ViewState;
use crate::ui::modal::show_error_dialog;
use gtk::gio;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

const SEND_TO_SUCCESS_DURATION: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(in crate::ui::browser) struct SendToTransferContext {
    pub(in crate::ui::browser) device_name: String,
}

pub(super) struct SendToRoot {
    pub(super) device_id: String,
    pub(super) root: PathBuf,
    pub(super) relative: Option<PathBuf>,
}

fn unavailable_text(has_relative: bool) -> String {
    crate::i18n::tr(if has_relative {
        "The removable device or folder is no longer available."
    } else {
        "The removable device is no longer available."
    })
}

impl SendToRoot {
    fn unavailable(&self) -> String {
        unavailable_text(self.relative.is_some())
    }

    pub(super) async fn destination(self) -> Result<Location, String> {
        let unavailable = self.unavailable();
        let Ok(Some(current_root)) = crate::adapters::probes::writable_folder(&self.root).await
        else {
            return Err(unavailable);
        };
        let Some(relative) = self.relative else {
            return Ok(Location::local(current_root));
        };
        let Some((destination, inside, relative)) =
            resolve_within_root(current_root, relative).await
        else {
            return Err(unavailable);
        };
        crate::ui::preferences::PreferenceManager::shared().remember_send_to_destination(
            &self.device_id,
            &inside,
            Some(&relative),
        );
        Ok(Location::local(destination))
    }
}

/// The canonical destination and its path inside the root, when it is still a valid one.
async fn resolve_within_root(
    current_root: PathBuf,
    relative: PathBuf,
) -> Option<(PathBuf, PathBuf, PathBuf)> {
    gio::spawn_blocking(move || {
        let destination = crate::ui::browser::destination::canonical_directory_within(
            &current_root,
            &current_root.join(&relative),
        )?;
        let inside = destination.strip_prefix(&current_root).ok()?.to_owned();
        crate::ui::preferences::is_valid_send_to_relative_path(&inside).then_some((
            destination,
            inside,
            relative,
        ))
    })
    .await
    .ok()
    .flatten()
}

#[derive(Clone)]
pub(in crate::ui::browser) struct PendingSendToCompletion {
    pub(in crate::ui::browser) device_name: String,
    pub(in crate::ui::browser) item_count: usize,
}

#[derive(Clone)]
pub(in crate::ui::browser) struct FinishedSendToCompletion {
    pub(in crate::ui::browser) completion: PendingSendToCompletion,
    pub(in crate::ui::browser) progress_shown: bool,
}

pub(super) fn send_to_display_name(id: &str, root: &Path) -> String {
    crate::ui::removable_destinations()
        .into_iter()
        .find(|destination| destination.id == id)
        .map(|destination| destination.name)
        .or_else(|| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
}

fn send_to_success_text(device_name: &str, item_count: usize) -> String {
    if item_count == 1 {
        rust_i18n::t!("Copied to %{device_name}", device_name = device_name).into_owned()
    } else {
        rust_i18n::t!(
            "%{items} copied to %{device_name}",
            items = crate::i18n::count("items", item_count),
            device_name = device_name
        )
        .into_owned()
    }
}

impl ViewState {
    pub(in crate::ui::browser) fn send_to_removable_device(
        self: &Rc<Self>,
        id: String,
        sources: Vec<Location>,
    ) {
        self.send_to_removable_device_with_resolver(
            &id,
            sources,
            crate::ui::removable_destination_root,
        );
    }

    pub(in crate::ui::browser) fn send_to_removable_device_with_resolver(
        self: &Rc<Self>,
        id: &str,
        sources: Vec<Location>,
        resolve: impl FnOnce(&str) -> Option<PathBuf>,
    ) {
        let device = resolve(id).map(|root| SendToRoot {
            device_id: id.to_owned(),
            root,
            relative: None,
        });
        self.send_to_resolved_device(device, false, sources);
    }

    pub(in crate::ui::browser) fn send_to_recent_destination(
        self: &Rc<Self>,
        id: String,
        relative: PathBuf,
        sources: Vec<Location>,
    ) {
        self.send_to_recent_destination_with_resolver(
            &id,
            &relative,
            sources,
            crate::ui::removable_destination_root,
        );
    }

    pub(in crate::ui::browser) fn send_to_recent_destination_with_resolver(
        self: &Rc<Self>,
        id: &str,
        relative: &Path,
        sources: Vec<Location>,
        resolve: impl FnOnce(&str) -> Option<PathBuf>,
    ) {
        let device = resolve(id)
            .filter(|_| crate::ui::preferences::is_valid_send_to_relative_path(relative))
            .map(|root| SendToRoot {
                device_id: id.to_owned(),
                root,
                relative: Some(relative.to_owned()),
            });
        self.send_to_resolved_device(device, true, sources);
    }

    fn send_to_resolved_device(
        self: &Rc<Self>,
        device: Option<SendToRoot>,
        has_relative: bool,
        sources: Vec<Location>,
    ) {
        let Some(device) = device else {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Destination unavailable"),
                &unavailable_text(has_relative),
            );
            return;
        };
        self.send_to_device(device, sources);
    }

    fn send_to_device(self: &Rc<Self>, device: SendToRoot, sources: Vec<Location>) {
        let destination = Location::local(match &device.relative {
            Some(relative) => device.root.join(relative),
            None => device.root.clone(),
        });
        let device_name = send_to_display_name(&device.device_id, &device.root);
        self.transfer_replaces_cursor.set(false);
        self.begin_transfer(
            TransferRequest {
                destination,
                sources,
                move_sources: false,
                reveal: false,
                send_to: Some(SendToTransferContext { device_name }),
                focus: ConflictFocus::Replace,
            },
            Some(device),
        );
    }

    pub(in crate::ui::browser) fn send_to(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        send_to: SendToTransferContext,
    ) {
        self.start_transfer_with_reveal(TransferRequest {
            destination,
            sources,
            move_sources: false,
            reveal: false,
            send_to: Some(send_to),
            focus: ConflictFocus::Replace,
        });
    }

    /// Shows transient success feedback for a fast Send-to whose transfer
    /// finished before the normal progress UI became visible.
    pub(in crate::ui::browser) fn show_send_to_success(
        &self,
        device_name: &str,
        item_count: usize,
    ) {
        crate::ui::notice::show_notice(
            &self.overlay,
            None,
            &send_to_success_text(device_name, item_count),
            SEND_TO_SUCCESS_DURATION,
        );
    }
}
