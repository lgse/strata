// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn rename(request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let operation_cancellable = cancellable.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        if let Err(message) = validate_basename(&request.new_name) {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: crate::i18n::tr(message),
                password_failure: None,
            });
            return;
        }
        let file = request
            .entry
            .location
            .native_path()
            .map(gio::File::for_path)
            .unwrap_or_else(|| {
                gio::File::for_uri(request.entry.location.uri_value().unwrap_or_default())
            });
        if let Some(folder) = file.parent()
            && let Some(message) = typed_names::refused_in(&folder, &request.new_name, false).await
        {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message,
                password_failure: None,
            });
            return;
        }
        let item = request.entry.location.clone();
        let new_name = request.new_name.clone();
        let affected_locations = item.parent().into_iter().collect();
        let roots = release::natives([&item]);
        let operation = ActiveOperation::begin(release::parents(&roots));
        operation.release(roots, &operation_cancellable).await;
        if operation_cancellable.is_cancelled() {
            emit(cancelled_event(
                request.id,
                Vec::new(),
                Vec::new(),
                vec![item],
                affected_locations,
            ));
            return;
        }
        let result = match request.entry.location.native_path() {
            Some(path) => match glib::filename_from_utf8(&request.new_name) {
                Ok((name, _)) => {
                    rename_native(
                        path.to_path_buf(),
                        path.with_file_name(name),
                        &operation_cancellable,
                    )
                    .await
                }
                Err(error) => Err(glib::Error::new(
                    gio::IOErrorEnum::InvalidFilename,
                    &error.to_string(),
                )),
            },
            None => set_display_name(&file, &request.new_name, &operation_cancellable)
                .await
                .map(drop),
        };
        match result {
            Ok(()) => emit(OperationEvent::Renamed {
                request_id: request.id,
            }),
            Err(error) if was_cancelled(&error) => {
                emit(cancelled_event(
                    request.id,
                    Vec::new(),
                    vec![item],
                    Vec::new(),
                    affected_locations,
                ));
            }
            Err(error) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: if error.matches(gio::IOErrorEnum::Exists) {
                    rust_i18n::t!("“%{name}” already exists", name = new_name).into_owned()
                } else {
                    crate::services::gio_error_message(&error)
                },
                password_failure: None,
            }),
        }
    });
    operation_handle(cancellable)
}
