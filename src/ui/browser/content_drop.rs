// SPDX-License-Identifier: MIT

//! Drops of content rather than existing files: images, selected text, and web
//! addresses from another application, saved as new files in local folders.

use std::path::Path;
use std::rc::Rc;

use gtk::{gdk, gio, glib, prelude::*};

use crate::model::Location;
use crate::services::{
    DropCommit, DroppedFile, MAX_DROPPED_FILE_BYTES, WebLink, io_error_message, save_dropped_files,
};
use crate::ui::browser::ViewState;
use crate::ui::browser::clipboard::{
    FileDropState, file_drop_commit, locations_from_file_list_value,
};
use crate::ui::browser::paths::is_trash_location;

#[cfg(test)]
mod tests;

/// What a drop target hands its view once it understands the dropped value.
pub(crate) enum DropRequest {
    Files(Vec<Location>, DropCommit),
    Content(Vec<DroppedFile>),
}

pub(crate) type DropHandler = Rc<dyn Fn(Location, DropRequest)>;

/// Chromium offers a dragged image's encoded bytes this way, optionally followed
/// by `;name="…"`; GTK has no deserializer for it.
const FILE_CONTENTS_MIME: &str = "application/octet-stream";

/// Texture comes first because browsers offer a dragged image together with its address.
pub(super) fn drop_types() -> [glib::Type; 3] {
    [
        gdk::Texture::static_type(),
        gdk::FileList::static_type(),
        String::static_type(),
    ]
}

/// New content is only written to local folders; Trash, Recent, and remote
/// locations are URIs.
pub(crate) fn accepts_content_at(destination: &Location) -> bool {
    destination.native_path().is_some()
}

/// Strata's own non-file drags, such as tab and sidebar reordering, carry
/// private strings that must never be saved as text.
pub(super) fn accepts_offer(
    target: &gtk::DropTarget,
    offered: &gdk::Drop,
    destination: Option<&Location>,
) -> bool {
    let Some(destination) = destination else {
        return false;
    };
    if !offered.actions().intersects(target.actions())
        || is_trash_location(destination)
        || destination.is_recent_location()
    {
        return false;
    }
    let formats = offered.formats();
    let foreign = offered.drag().is_none();
    if foreign && file_contents_mime(&formats).is_some() {
        return false;
    }
    formats.contains_type(gdk::FileList::static_type())
        || (foreign
            && accepts_content_at(destination)
            && (formats.contains_type(gdk::Texture::static_type())
                || formats.contains_type(String::static_type())))
}

/// File lists are read while hovering to choose copy or move; images are only
/// decoded once dropped.
pub(super) fn preloads(offered: &gdk::Drop) -> bool {
    !offered.formats().contains_type(gdk::Texture::static_type())
}

/// Content always copies. Returns `None` when the drop holds existing files.
pub(super) fn content_drop_action(
    target: &gtk::DropTarget,
    destination: Option<&Location>,
) -> Option<gdk::DragAction> {
    let content = match target.value() {
        Some(value) => is_content_value(&value),
        None => target.current_drop().is_some_and(|offered| {
            let formats = offered.formats();
            formats.contains_type(gdk::Texture::static_type())
                || !formats.contains_type(gdk::FileList::static_type())
        }),
    };
    content.then(|| {
        if destination.is_some_and(accepts_content_at) {
            gdk::DragAction::COPY
        } else {
            gdk::DragAction::empty()
        }
    })
}

pub(super) fn is_content_value(value: &glib::Value) -> bool {
    value
        .get::<gdk::FileList>()
        .map_or(true, |files| web_links(&files).is_some())
}

/// Web addresses arrive as URI lists too; they become links, never transfers,
/// so nothing is fetched.
fn web_links(files: &gdk::FileList) -> Option<Vec<WebLink>> {
    let files = files.files();
    if files.is_empty() {
        return None;
    }
    files
        .iter()
        .map(|file| WebLink::parse(&file.uri()))
        .collect()
}

/// The files a dropped value saves, or `None` when it holds existing files.
pub(super) fn dropped_content(value: &glib::Value) -> Option<Vec<DroppedFile>> {
    if let Ok(texture) = value.get::<gdk::Texture>() {
        return Some(vec![DroppedFile::png(
            &crate::i18n::tr("pasted_image.name"),
            texture.save_to_png_bytes().to_vec(),
        )]);
    }
    if let Ok(files) = value.get::<gdk::FileList>() {
        return web_links(&files).map(|links| links.iter().map(DroppedFile::link).collect());
    }
    let text = value.get::<String>().ok().filter(|text| !text.is_empty())?;
    Some(vec![match WebLink::parse(&text) {
        Some(link) => DroppedFile::link(&link),
        None => DroppedFile::text(&crate::i18n::tr("dropped_text.name"), text),
    }])
}

/// Hands a dropped value to `handler`, classifying existing files as copy or
/// move first. Returns whether the drop was taken.
pub(crate) fn commit_dropped_value(
    target: &gtk::DropTarget,
    value: &glib::Value,
    destination: Location,
    state: &Rc<FileDropState>,
    handler: impl FnOnce(Location, DropRequest) + 'static,
) -> bool {
    if let Some(content) = dropped_content(value) {
        if !accepts_content_at(&destination) {
            return false;
        }
        handler(destination, DropRequest::Content(content));
        return true;
    }
    let Some(sources) = locations_from_file_list_value(value) else {
        return false;
    };
    file_drop_commit(
        target,
        destination,
        sources,
        state,
        move |destination, sources, commit| {
            handler(destination, DropRequest::Files(sources, commit));
        },
    );
    true
}

/// Adds the drop target with a companion that reads browser-supplied image
/// bytes, which a `gtk::DropTarget` cannot deserialize.
pub(crate) fn attach_drop_target(
    widget: &impl IsA<gtk::Widget>,
    target: &gtk::DropTarget,
    state: &Rc<FileDropState>,
    handler: DropHandler,
) {
    let contents = gtk::DropTargetAsync::new(None, gdk::DragAction::COPY);
    contents.set_propagation_phase(target.propagation_phase());
    let state_for_accept = state.clone();
    contents.connect_accept(move |_, offered| {
        offered.drag().is_none()
            && file_contents_mime(&offered.formats()).is_some()
            && state_for_accept
                .destination()
                .is_some_and(|destination| accepts_content_at(&destination))
    });
    contents.connect_drag_enter(|_, _, _, _| gdk::DragAction::COPY);
    contents.connect_drag_motion(|_, _, _, _| gdk::DragAction::COPY);
    let state = state.clone();
    contents.connect_drop(move |_, offered, _, _| {
        let Some(mime) = file_contents_mime(&offered.formats()) else {
            return false;
        };
        let Some(destination) = state.destination().filter(accepts_content_at) else {
            return false;
        };
        let offered = offered.clone();
        let handler = handler.clone();
        glib::MainContext::default().spawn_local(async move {
            let file = match read_file_contents(&offered, &mime).await {
                Ok(bytes) => DroppedFile::image(
                    supplied_name(&mime).as_deref(),
                    &crate::i18n::tr("pasted_image.name"),
                    bytes,
                ),
                Err(error) => {
                    tracing::warn!(%error, "unable to read dropped file contents");
                    None
                }
            };
            let Some(file) = file else {
                offered.finish(gdk::DragAction::empty());
                return;
            };
            handler(destination, DropRequest::Content(vec![file]));
            offered.finish(gdk::DragAction::COPY);
        });
        true
    });
    widget.add_controller(target.clone());
    widget.add_controller(contents);
}

fn file_contents_mime(formats: &gdk::ContentFormats) -> Option<glib::GString> {
    formats.mime_types().into_iter().find(|mime| {
        mime.strip_prefix(FILE_CONTENTS_MIME)
            .is_some_and(|parameters| parameters.is_empty() || parameters.starts_with(";name=\""))
    })
}

fn supplied_name(mime: &str) -> Option<String> {
    let quoted = mime
        .strip_prefix(FILE_CONTENTS_MIME)?
        .strip_prefix(";name=\"")?
        .strip_suffix('"')?;
    let mut name = String::with_capacity(quoted.len());
    let mut escaped = false;
    for c in quoted.chars() {
        if escaped || c != '\\' {
            name.push(c);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    Some(name)
}

async fn read_file_contents(offered: &gdk::Drop, mime: &str) -> Result<Vec<u8>, glib::Error> {
    let (stream, _) = offered
        .read_future(&[mime], glib::Priority::DEFAULT)
        .await?;
    let mut bytes = Vec::new();
    loop {
        let chunk = stream
            .read_bytes_future(64 * 1024, glib::Priority::DEFAULT)
            .await?;
        if chunk.is_empty() {
            return Ok(bytes);
        }
        if bytes.len() + chunk.len() > MAX_DROPPED_FILE_BYTES {
            return Err(glib::Error::new(
                gio::IOErrorEnum::MessageTooLarge,
                "dropped file contents are too large",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
}

impl ViewState {
    pub(super) fn commit_drop(self: &Rc<Self>, destination: Location, request: DropRequest) {
        match request {
            DropRequest::Files(sources, commit) => {
                self.commit_file_drop(destination, sources, commit);
            }
            DropRequest::Content(files) => self.save_dropped_content(destination, files),
        }
    }

    fn save_dropped_content(self: &Rc<Self>, destination: Location, files: Vec<DroppedFile>) {
        self.stop_drag_autoscroll();
        self.drop_active_depths.set(None);
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().saturating_add(1));
        self.suppress_scroll_after_drop.set(false);
        let Some(directory) = destination.native_path().map(Path::to_path_buf) else {
            return;
        };
        let reveal = crate::ui::preferences::PreferenceManager::shared().open_folder_after_drop();
        let navigation = self.browser.navigation_generation();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let saved = gio::spawn_blocking(move || save_dropped_files(&directory, &files))
                .await
                .unwrap_or_else(|_| Err(std::io::Error::other("the save was interrupted")));
            let Some(state) = weak.upgrade() else {
                return;
            };
            match saved {
                // Navigating away while saving outranks the preference.
                Ok(paths) if reveal && state.browser.navigation_generation() == navigation => {
                    state.reveal_locations(
                        destination,
                        paths.into_iter().map(Location::local).collect(),
                        false,
                    );
                }
                Ok(_) => {}
                Err(error) => super::show_error_dialog(
                    &state.overlay,
                    &crate::i18n::tr("Unable to save dropped content"),
                    &io_error_message(&error),
                ),
            }
        });
    }
}
