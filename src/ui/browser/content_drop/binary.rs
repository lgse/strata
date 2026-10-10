// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{gdk, gio, glib, prelude::*};

use crate::model::Location;

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "StrataBrowserFile")]
pub(crate) struct BrowserFile {
    pub(super) name: Option<String>,
    pub(super) bytes: Vec<u8>,
    pub(super) destination: Location,
}

pub(crate) fn file_format(formats: &gdk::ContentFormats) -> Option<String> {
    formats
        .mime_types()
        .iter()
        .find(|mime| {
            mime.as_str() == "application/octet-stream"
                || (mime.len() <= 1024 && mime.starts_with("application/octet-stream;name=\""))
        })
        .map(ToString::to_string)
}

pub(crate) async fn read_file(
    stream: &gio::InputStream,
    mime: &str,
    destination: Location,
) -> Result<BrowserFile, glib::Error> {
    let name = mime
        .strip_prefix("application/octet-stream;name=\"")
        .and_then(|name| name.strip_suffix('"'))
        .map(|name| name.replace("\\\"", "\"").replace("\\\\", "\\"));
    let mut bytes = Vec::new();
    loop {
        let chunk = stream
            .read_bytes_future(64 * 1024, glib::Priority::DEFAULT)
            .await?;
        if chunk.is_empty() {
            return Ok(BrowserFile {
                name,
                bytes,
                destination,
            });
        }
        if bytes.len() + chunk.len() > crate::services::web_image::MAX_IMAGE_BYTES {
            return Err(glib::Error::new(
                gio::IOErrorEnum::InvalidData,
                &crate::i18n::tr("Unable to save dropped content"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
}

pub(crate) fn install(
    widget: &impl IsA<gtk::Widget>,
    target: &gtk::DropTarget,
    state: &Rc<super::super::clipboard::FileDropState>,
) {
    let controller = gtk::DropTargetAsync::new(None, gdk::DragAction::COPY);
    controller.set_propagation_phase(target.propagation_phase());
    let weak_controller = controller.downgrade();
    target.connect_propagation_phase_notify(move |target| {
        if let Some(controller) = weak_controller.upgrade() {
            controller.set_propagation_phase(target.propagation_phase());
        }
    });
    let state_for_accept = state.clone();
    controller.connect_accept(move |_, offered| {
        file_format(&offered.formats()).is_some()
            && state_for_accept
                .destination()
                .is_some_and(|destination| super::accepts_content_at(&destination))
    });
    controller.connect_drag_enter(|_, _, _, _| gdk::DragAction::COPY);
    controller.connect_drag_motion(|_, _, _, _| gdk::DragAction::COPY);
    let weak = target.downgrade();
    let state_for_drop = state.clone();
    controller.connect_drop(move |_, offered, x, y| {
        let Some(mime) = file_format(&offered.formats()) else {
            return false;
        };
        let Some(destination) = state_for_drop
            .destination()
            .filter(super::accepts_content_at)
        else {
            return false;
        };
        let offered = offered.clone();
        let weak = weak.clone();
        let state = state_for_drop.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                let (stream, mime) = offered
                    .read_future(&[mime.as_str()], glib::Priority::DEFAULT)
                    .await?;
                read_file(&stream, &mime, destination).await
            }
            .await;
            let accepted = match (weak.upgrade(), result) {
                (Some(target), Ok(file)) => {
                    state.forwarding_content.set(true);
                    let accepted = target.emit_by_name::<bool>(
                        "drop",
                        &[&glib::BoxedValue(file.to_value()), &x, &y],
                    );
                    state.forwarding_content.set(false);
                    accepted
                }
                _ => false,
            };
            offered.finish(if accepted {
                gdk::DragAction::COPY
            } else {
                gdk::DragAction::empty()
            });
        });
        true
    });
    widget.add_controller(controller);
}
