// SPDX-License-Identifier: MIT

use super::{
    await_cancellable, cancelled_event,
    mount_profile::folder_name_limit,
    names::numbered_name,
    operation_handle,
    release::{self, ActiveOperation},
    run_blocking, typed_names, validated_child, was_cancelled,
};
use crate::{
    adapters::{gio_file_for_location, location_for_file, volume::NameLimit},
    model::Location,
    services::{LoadHandle, OperationEvent, OperationRequestId},
};
use gtk::{gio, glib, prelude::*};
use std::{collections::HashSet, ffi::OsStr, rc::Rc};

pub(super) fn start(
    id: OperationRequestId,
    parent: Location,
    name: String,
    unique_name: bool,
    is_directory: bool,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let operation_cancellable = cancellable.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let _operation = ActiveOperation::begin(release::natives([&parent]));
        let directory = gio_file_for_location(&parent);
        let mut suffix = 0u64;
        let mut limit = NameLimit::default();
        loop {
            let candidate = if suffix == 0 {
                name.clone()
            } else {
                let numbered = numbered_name(
                    OsStr::new(&name),
                    suffix,
                    OsStr::new(""),
                    limit,
                    is_directory,
                );
                numbered.to_string_lossy().into_owned()
            };
            let file = match validated_child(&directory, &candidate) {
                Ok(file) => file,
                Err(message) => {
                    emit(OperationEvent::Failed {
                        request_id: id,
                        message: crate::i18n::tr(message),
                        password_failure: None,
                    });
                    return;
                }
            };
            // Only typed names are refused; default ones are Strata's own.
            if !unique_name
                && let Some(message) =
                    typed_names::refused_in(&directory, &name, is_directory).await
            {
                emit(OperationEvent::Failed {
                    request_id: id,
                    message,
                    password_failure: None,
                });
                return;
            }
            let Some(location) = location_for_file(&file) else {
                emit(OperationEvent::Failed {
                    request_id: id,
                    message: crate::i18n::tr("The new item has an invalid URI"),
                    password_failure: None,
                });
                return;
            };
            let affected = HashSet::from([parent.clone()]);
            if operation_cancellable.is_cancelled() {
                emit(cancelled_event(
                    id,
                    Vec::new(),
                    Vec::new(),
                    vec![location],
                    affected,
                ));
                return;
            }
            let result = if is_directory {
                await_cancellable(
                    &file,
                    &operation_cancellable,
                    |file, cancellable, result| {
                        file.make_directory_async(
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    },
                )
                .await
            } else {
                create_empty_file(&file, &operation_cancellable).await
            };
            match result {
                Ok(()) => {
                    if unique_name {
                        emit(OperationEvent::EntryCreated {
                            request_id: id,
                            location,
                        });
                    } else {
                        emit(OperationEvent::Created { request_id: id });
                    }
                }
                // Retry only a collision from atomic creation, never a preflight exists() check.
                Err(error)
                    if unique_name && name_taken(&error, is_directory) && suffix < u64::MAX =>
                {
                    if suffix == 0 {
                        limit = folder_name_limit(&directory).await;
                    }
                    suffix += 1;
                    continue;
                }
                Err(error) if name_taken(&error, is_directory) => {
                    emit(OperationEvent::Failed {
                        request_id: id,
                        message: rust_i18n::t!("“%{name}” already exists", name = candidate)
                            .into_owned(),
                        password_failure: None,
                    });
                }
                Err(error) if was_cancelled(&error) => {
                    emit(cancelled_event(
                        id,
                        Vec::new(),
                        vec![location],
                        Vec::new(),
                        affected,
                    ));
                }
                Err(error) => emit(OperationEvent::Failed {
                    request_id: id,
                    message: creation_failed(&candidate, &error),
                    password_failure: None,
                }),
            }
            break;
        }
    });
    operation_handle(cancellable)
}

/// Finish closing the empty file before publishing it for rename.
pub(super) async fn create_empty_file(
    file: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    if file.is_native() {
        let file = file.clone();
        let cancellable = cancellable.clone();
        return run_blocking(move || {
            cancellable.set_error_if_cancelled()?;
            file.create(gio::FileCreateFlags::NONE, gio::Cancellable::NONE)?
                .close(gio::Cancellable::NONE)
        })
        .await;
    }
    let stream = await_cancellable(file, cancellable, |file, cancellable, result| {
        file.create_async(
            gio::FileCreateFlags::NONE,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?;
    stream.close_future(glib::Priority::DEFAULT).await
}

/// SMB with POSIX extensions fails an exclusive file create over a folder
/// with `EISDIR`, which GIO reports as `IsDirectory`.
pub(super) fn name_taken(error: &glib::Error, is_directory: bool) -> bool {
    error.matches(gio::IOErrorEnum::Exists)
        || (!is_directory && error.matches(gio::IOErrorEnum::IsDirectory))
}

fn creation_failed(name: &str, error: &glib::Error) -> String {
    rust_i18n::t!(
        "Could not create “%{name}”: %{error}",
        name = name,
        error = crate::services::gio_error_detail(error)
    )
    .into_owned()
}
