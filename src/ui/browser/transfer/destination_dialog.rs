// SPDX-License-Identifier: MIT

use super::RemovableRootResolver;
use super::send_to::{SendToTransferContext, send_to_display_name};
use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::can_remove_location;
use crate::ui::modal::show_error_dialog;
use futures_lite::future::BoxedLocal;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct TransferDialogOptions {
    base: PathBuf,
    root_limit: Option<PathBuf>,
    allow_create: bool,
    completion: TransferDialogCompletion,
}

#[derive(Clone)]
enum TransferDialogCompletion {
    CopyMove {
        move_sources: bool,
    },
    SendTo {
        device_id: String,
        opened_root: PathBuf,
        resolve: RemovableRootResolver,
    },
}

impl TransferDialogCompletion {
    fn moves_sources(&self) -> bool {
        matches!(self, Self::CopyMove { move_sources: true })
    }

    fn title(&self) -> String {
        match self {
            Self::SendTo { .. } => crate::i18n::tr("Send to folder"),
            _ if self.moves_sources() => crate::i18n::tr("Move to"),
            _ => crate::i18n::tr("Copy to"),
        }
    }
}

type ValidatedRoot = Rc<RefCell<Option<PathBuf>>>;

fn unavailable() -> String {
    crate::i18n::tr("The removable device is no longer available.")
}

/// Accepts only an existing folder inside the device's current root, and records that root.
async fn validate_send_to_folder(
    root: Option<PathBuf>,
    relative: Result<PathBuf, std::path::StripPrefixError>,
    validated: ValidatedRoot,
) -> Result<PathBuf, String> {
    let root = root.ok_or_else(unavailable)?;
    let current_root = crate::adapters::probes::writable_folder(&root)
        .await
        .ok()
        .flatten()
        .ok_or_else(unavailable)?;
    let outside = || crate::i18n::tr("Choose an existing folder inside this removable device.");
    let relative = relative.map_err(|_| outside())?;
    let destination = crate::adapters::probes::destination_folder(&current_root.join(relative))
        .await
        .ok()
        .filter(|destination| destination.starts_with(&current_root))
        .ok_or_else(outside)?;
    validated.replace(Some(current_root));
    Ok(destination)
}

type DestinationValidator = Rc<dyn Fn(&Path) -> BoxedLocal<Result<PathBuf, String>>>;

fn destination_validator(
    completion: TransferDialogCompletion,
    validated_root: &ValidatedRoot,
) -> DestinationValidator {
    let validated = validated_root.clone();
    Rc::new(move |path: &Path| -> BoxedLocal<Result<PathBuf, String>> {
        let TransferDialogCompletion::SendTo {
            device_id,
            opened_root,
            resolve,
        } = &completion
        else {
            return crate::ui::browser::destination::existing_folder(path);
        };
        let root = resolve(device_id);
        let relative = path.strip_prefix(opened_root).map(Path::to_path_buf);
        Box::pin(validate_send_to_folder(root, relative, validated.clone()))
    })
}

impl ViewState {
    pub(in crate::ui::browser) fn show_transfer_dialog(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        move_sources: bool,
    ) {
        if entries.is_empty() {
            return;
        }
        if move_sources
            && entries
                .iter()
                .any(|entry| !can_remove_location(&entry.location))
        {
            return;
        }
        let base = self
            .browser
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf))
            .unwrap_or_else(glib::home_dir);
        self.show_destination_dialog(
            entries.into_iter().map(|entry| entry.location).collect(),
            TransferDialogOptions {
                base,
                root_limit: None,
                allow_create: true,
                completion: TransferDialogCompletion::CopyMove { move_sources },
            },
        );
    }

    pub(in crate::ui::browser) fn show_send_to_folder_dialog(
        self: &Rc<Self>,
        id: String,
        sources: Vec<Location>,
    ) {
        self.show_send_to_folder_dialog_with_resolver(
            id,
            sources,
            Rc::new(crate::ui::removable_destination_root),
        );
    }

    pub(in crate::ui::browser) fn show_send_to_folder_dialog_with_resolver(
        self: &Rc<Self>,
        id: String,
        sources: Vec<Location>,
        resolve: RemovableRootResolver,
    ) {
        if sources.is_empty() {
            return;
        }
        let root = resolve(&id);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let root = match root {
                Some(root) => crate::adapters::probes::writable_folder(&root)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            let Some(state) = weak.upgrade() else {
                return;
            };
            let Some(root) = root else {
                show_error_dialog(
                    &state.overlay,
                    &crate::i18n::tr("Destination unavailable"),
                    &unavailable(),
                );
                return;
            };
            state.show_destination_dialog(
                sources,
                TransferDialogOptions {
                    base: root.clone(),
                    root_limit: Some(root.clone()),
                    allow_create: false,
                    completion: TransferDialogCompletion::SendTo {
                        device_id: id,
                        opened_root: root,
                        resolve,
                    },
                },
            );
        });
    }

    fn show_destination_dialog(
        self: &Rc<Self>,
        sources: Vec<Location>,
        options: TransferDialogOptions,
    ) {
        let Some(parent) = self.overlay.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let TransferDialogOptions {
            base,
            root_limit,
            allow_create,
            completion,
        } = options;
        let move_sources = completion.moves_sources();
        let title = completion.title();
        let validated_root = Rc::new(RefCell::new(None));
        let validate = destination_validator(completion.clone(), &validated_root);
        let state = self.clone();
        crate::ui::chooser::present_destination_chooser(
            crate::ui::chooser::DestinationRequest {
                parent,
                title,
                accept_label: crate::i18n::tr(if move_sources {
                    "Move here"
                } else {
                    "Copy here"
                }),
                initial_directory: base,
                root_limit,
                allow_create,
                validate,
            },
            move |path| {
                if let TransferDialogCompletion::SendTo {
                    device_id,
                    opened_root,
                    ..
                } = &completion
                {
                    state.finish_send_to_folder(
                        path,
                        sources,
                        (device_id, opened_root),
                        &validated_root,
                    );
                } else {
                    state.finish_copy_move_to(path, sources, move_sources);
                }
            },
        );
    }

    fn finish_send_to_folder(
        self: &Rc<Self>,
        path: PathBuf,
        sources: Vec<Location>,
        (device_id, opened_root): (&str, &Path),
        validated_root: &ValidatedRoot,
    ) {
        let Some(current_root) = validated_root.take() else {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Destination unavailable"),
                &unavailable(),
            );
            return;
        };
        if let Ok(relative) = path.strip_prefix(&current_root) {
            crate::ui::preferences::PreferenceManager::shared()
                .remember_send_to_destination(device_id, relative, None);
        }
        self.send_to(
            Location::local(path),
            sources,
            SendToTransferContext {
                device_name: send_to_display_name(device_id, opened_root),
            },
        );
    }

    fn finish_copy_move_to(
        self: &Rc<Self>,
        path: PathBuf,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        self.pending_navigate.replace(Some(Location::local(&path)));
        let names = sources.iter().filter_map(|source| {
            source
                .native_path()?
                .file_name()?
                .to_str()
                .map(String::from)
        });
        self.pending_select.borrow_mut().extend(names);
        self.start_transfer(Location::local(path), sources, move_sources);
    }
}
