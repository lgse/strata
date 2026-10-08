// SPDX-License-Identifier: MIT

use super::{
    await_cancellable, cancellation_handle, cancelled_event, validated_child, was_cancelled,
};
use crate::{
    adapters::{gio_file_for_location, location_for_file},
    model::Location,
    services::{LoadHandle, OperationEvent, OperationRequestId},
};
use gtk::{gio, glib, prelude::*};
use std::{collections::HashSet, rc::Rc};

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
        let directory = gio_file_for_location(&parent);
        let mut suffix = 0u64;
        loop {
            let candidate = if suffix == 0 {
                name.clone()
            } else {
                format!("{name} ({suffix})")
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
                .map(|()| None)
            } else {
                await_cancellable(
                    &file,
                    &operation_cancellable,
                    |file, cancellable, result| {
                        file.create_async(
                            gio::FileCreateFlags::NONE,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    },
                )
                .await
                .map(Some)
            };
            match result {
                Ok(stream) => {
                    // Finish closing the empty file before publishing it for rename.
                    if let Some(stream) = stream
                        && let Err(error) = stream.close_future(glib::Priority::DEFAULT).await
                    {
                        emit(OperationEvent::Failed {
                            request_id: id,
                            message: creation_failed(&candidate, &error),
                            password_failure: None,
                        });
                        return;
                    }
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
                    if unique_name
                        && error.matches(gio::IOErrorEnum::Exists)
                        && suffix < u64::MAX =>
                {
                    suffix += 1;
                    continue;
                }
                Err(error) if error.matches(gio::IOErrorEnum::Exists) => {
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
    cancellation_handle(cancellable)
}

fn creation_failed(name: &str, error: &glib::Error) -> String {
    rust_i18n::t!(
        "Could not create “%{name}”: %{error}",
        name = name,
        error = crate::services::gio_error_detail(error)
    )
    .into_owned()
}
