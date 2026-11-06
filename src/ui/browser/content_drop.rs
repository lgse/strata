// SPDX-License-Identifier: MIT

//! Drops of content rather than existing files: images, selected text, and web
//! addresses from another application, saved as new files in local folders.

use std::path::Path;
use std::rc::Rc;

use gtk::{gdk, gio, glib, prelude::*};

use crate::model::Location;
use crate::services::{
    DropCommit, DroppedFile, MAX_DROPPED_FILE_BYTES, WebLink, gio_error_message, has_web_scheme,
    io_error_message, save_dropped_files,
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
    /// Content that was taken but could not be read, with the reason to show.
    Unreadable(String),
}

pub(crate) type DropHandler = Rc<dyn Fn(Location, DropRequest)>;

/// Chromium offers a dragged image's encoded bytes this way, optionally followed
/// by `;name="…"`; GTK has no deserializer for it.
const FILE_CONTENTS_MIME: &str = "application/octet-stream";

/// File lists come first so that real files are copied or moved even when image
/// data comes with them. Images offered with web addresses only are read by the
/// companion target from [`attach_drop_target`].
pub(super) fn drop_types() -> [glib::Type; 3] {
    [
        gdk::FileList::static_type(),
        gdk::Texture::static_type(),
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
    formats.contains_type(gdk::FileList::static_type())
        || (offered.drag().is_none()
            && accepts_content_at(destination)
            && (formats.contains_type(gdk::Texture::static_type())
                || formats.contains_type(String::static_type())
                || file_contents_mime(&formats).is_some()))
}

/// File lists are read while hovering to choose copy or move; images and text
/// are only read once dropped.
pub(super) fn preloads(offered: &gdk::Drop) -> bool {
    offered
        .formats()
        .contains_type(gdk::FileList::static_type())
}

/// Content always copies, and a refused URI list never reaches file
/// classification. Returns `None` when the drop holds existing files.
pub(super) fn content_drop_action(
    target: &gtk::DropTarget,
    destination: Option<&Location>,
) -> Option<gdk::DragAction> {
    let kind = match target.value() {
        Some(value) => value_kind(&value),
        None if target.current_drop().is_some_and(|offered| {
            offered
                .formats()
                .contains_type(gdk::FileList::static_type())
        }) =>
        {
            ValueKind::Files
        }
        None => ValueKind::Content,
    };
    match kind {
        ValueKind::Files => None,
        ValueKind::Refused => Some(gdk::DragAction::empty()),
        ValueKind::Content if destination.is_some_and(accepts_content_at) => {
            Some(gdk::DragAction::COPY)
        }
        ValueKind::Content => Some(gdk::DragAction::empty()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ValueKind {
    Files,
    Content,
    Refused,
}

pub(super) fn value_kind(value: &glib::Value) -> ValueKind {
    match value.get::<gdk::FileList>() {
        Ok(files) => match uri_list(&files) {
            UriList::Files => ValueKind::Files,
            UriList::Links(_) => ValueKind::Content,
            UriList::Refused => ValueKind::Refused,
        },
        Err(_) => ValueKind::Content,
    }
}

enum UriList {
    Files,
    Links(Vec<WebLink>),
    Refused,
}

/// Web addresses arrive as URI lists too. They become links, never transfers,
/// so nothing is fetched; a list mixing them with files, or holding one that
/// cannot be a link, is refused rather than partly handed to GIO.
fn uri_list(files: &gdk::FileList) -> UriList {
    let uris = files
        .files()
        .iter()
        .map(|file| file.uri())
        .collect::<Vec<_>>();
    if !uris.iter().any(|uri| has_web_scheme(uri)) {
        return UriList::Files;
    }
    uris.iter()
        .map(|uri| WebLink::parse(uri))
        .collect::<Option<Vec<_>>>()
        .map_or(UriList::Refused, UriList::Links)
}

/// The files a dropped value saves, or `None` when it holds existing files or
/// nothing to save.
pub(super) fn dropped_content(value: &glib::Value) -> Option<Vec<DroppedFile>> {
    if let Ok(texture) = value.get::<gdk::Texture>() {
        return Some(vec![texture_file(&texture)]);
    }
    if let Ok(files) = value.get::<gdk::FileList>() {
        return match uri_list(&files) {
            UriList::Links(links) => Some(link_files(&links)),
            UriList::Files | UriList::Refused => None,
        };
    }
    let text = value.get::<String>().ok().filter(|text| !text.is_empty())?;
    Some(vec![match WebLink::parse(&text) {
        Some(link) => DroppedFile::link(&link),
        None => DroppedFile::text(&crate::i18n::tr("dropped_text.name"), text),
    }])
}

fn texture_file(texture: &gdk::Texture) -> DroppedFile {
    DroppedFile::png(
        &crate::i18n::tr("pasted_image.name"),
        texture.save_to_png_bytes().to_vec(),
    )
}

fn link_files(links: &[WebLink]) -> Vec<DroppedFile> {
    links.iter().map(DroppedFile::link).collect()
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
    match value_kind(value) {
        ValueKind::Refused => false,
        ValueKind::Content => {
            let Some(content) = dropped_content(value) else {
                return false;
            };
            if !accepts_content_at(&destination) {
                return false;
            }
            handler(destination, DropRequest::Content(content));
            true
        }
        ValueKind::Files => {
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
    }
}

/// Adds the drop target with a companion that takes drops of images, which must
/// be read before the drop finishes: a `gtk::DropTarget` finishes as soon as its
/// handler returns, cannot read browser-supplied file contents, and reads only
/// one type, which is the file list when one comes with the image.
pub(crate) fn attach_drop_target(
    widget: &impl IsA<gtk::Widget>,
    target: &gtk::DropTarget,
    state: &Rc<FileDropState>,
    handler: DropHandler,
) {
    let contents = gtk::DropTargetAsync::new(None, gdk::DragAction::COPY);
    contents.set_propagation_phase(target.propagation_phase());
    let weak_target = target.downgrade();
    let state_for_accept = state.clone();
    contents.connect_accept(move |_, offered| {
        weak_target.upgrade().is_some_and(|target| {
            state_for_accept.accepts(&target, offered)
                && state_for_accept
                    .destination()
                    .is_some_and(|destination| reads_image_at_drop(offered, &destination))
        })
    });
    // The main target accepts every offer this one does and gives all hover
    // feedback; no action here leaves its status in place.
    contents.connect_drag_enter(|_, _, _, _| gdk::DragAction::empty());
    contents.connect_drag_motion(|_, _, _, _| gdk::DragAction::empty());
    let weak_target = target.downgrade();
    let state = state.clone();
    contents.connect_drop(move |_, offered, _, _| {
        let Some(target) = weak_target.upgrade() else {
            return false;
        };
        let Some(destination) = state.destination().filter(accepts_content_at) else {
            return false;
        };
        let Some(content) = DeferredContent::for_drop(&target, offered) else {
            return false;
        };
        // The main target never sees this drop; release its hover state as
        // its own drop would.
        target.emit_by_name::<()>("leave", &[]);
        target.reject();
        let offered = offered.clone();
        let handler = handler.clone();
        glib::MainContext::default().spawn_local(async move {
            match content.read(&offered).await {
                Ok(files) => {
                    handler(destination, DropRequest::Content(files));
                    offered.finish(gdk::DragAction::COPY);
                }
                Err(error) => {
                    tracing::warn!(error = %error.log_text(), "unable to read dropped content");
                    offered.finish(gdk::DragAction::empty());
                    handler(destination, DropRequest::Unreadable(error.message()));
                }
            }
        });
        true
    });
    widget.add_controller(target.clone());
    // Added last so it sees the drop first and can leave it to the main target.
    widget.add_controller(contents);
}

fn reads_image_at_drop(offered: &gdk::Drop, destination: &Location) -> bool {
    let formats = offered.formats();
    offered.drag().is_none()
        && accepts_content_at(destination)
        && (file_contents_mime(&formats).is_some()
            || formats.contains_type(gdk::Texture::static_type()))
}

/// An image read once dropped, with the drop's other content to fall back on.
struct DeferredContent {
    file_contents: Option<glib::GString>,
    texture: bool,
    links: Vec<WebLink>,
    text: bool,
}

impl DeferredContent {
    /// Leaves the drop to the main target when it offers real files, a refused
    /// list, or a list not read yet, which the main target reads on drop.
    fn for_drop(target: &gtk::DropTarget, offered: &gdk::Drop) -> Option<Self> {
        if offered.drag().is_some() {
            return None;
        }
        let formats = offered.formats();
        let file_contents = file_contents_mime(&formats);
        let texture = formats.contains_type(gdk::Texture::static_type());
        if file_contents.is_none() && !texture {
            return None;
        }
        let links = if formats.contains_type(gdk::FileList::static_type()) {
            let files = target.value()?.get::<gdk::FileList>().ok()?;
            match uri_list(&files) {
                UriList::Links(links) => links,
                UriList::Files | UriList::Refused => return None,
            }
        } else {
            Vec::new()
        };
        Some(Self {
            file_contents,
            texture,
            links,
            text: formats.contains_type(String::static_type()),
        })
    }

    /// Tries the image's own bytes, then its decoded data, then the address or
    /// text offered with it, reporting the image's failure if nothing remains.
    async fn read(self, offered: &gdk::Drop) -> Result<Vec<DroppedFile>, ContentError> {
        let mut failure = None;
        if let Some(mime) = &self.file_contents {
            let image = read_file_contents(offered, mime).await.and_then(|bytes| {
                DroppedFile::image(
                    supplied_name(mime).as_deref(),
                    &crate::i18n::tr("pasted_image.name"),
                    bytes,
                )
                .ok_or(ContentError::NotAnImage)
            });
            match image {
                Ok(file) => return Ok(vec![file]),
                Err(error) => failure = Some(error),
            }
        }
        if self.texture {
            match read_value::<gdk::Texture>(offered).await {
                Ok(texture) => return Ok(vec![texture_file(&texture)]),
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if !self.links.is_empty() {
            return Ok(link_files(&self.links));
        }
        if self.text
            && let Ok(text) = read_value::<String>(offered).await
            && let Some(files) = dropped_content(&text.to_value())
        {
            return Ok(files);
        }
        Err(failure.unwrap_or(ContentError::NotAnImage))
    }
}

async fn read_value<T: StaticType + for<'a> glib::value::FromValue<'a> + 'static>(
    offered: &gdk::Drop,
) -> Result<T, ContentError> {
    let value = offered
        .read_value_future(T::static_type(), glib::Priority::DEFAULT)
        .await
        .map_err(ContentError::Read)?;
    value.get::<T>().map_err(|_| ContentError::NotAnImage)
}

enum ContentError {
    Read(glib::Error),
    TooLarge,
    NotAnImage,
}

impl ContentError {
    fn message(&self) -> String {
        match self {
            Self::Read(error) => gio_error_message(error),
            Self::TooLarge => crate::i18n::tr("The dropped content is too large"),
            Self::NotAnImage => crate::i18n::tr("Unsupported image format"),
        }
    }

    fn log_text(&self) -> String {
        match self {
            Self::Read(error) => error.to_string(),
            Self::TooLarge => "dropped file contents are too large".to_owned(),
            Self::NotAnImage => "dropped file contents are not a recognized image".to_owned(),
        }
    }
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

async fn read_file_contents(offered: &gdk::Drop, mime: &str) -> Result<Vec<u8>, ContentError> {
    let (stream, _) = offered
        .read_future(&[mime], glib::Priority::DEFAULT)
        .await
        .map_err(ContentError::Read)?;
    let mut bytes = Vec::new();
    loop {
        let chunk = stream
            .read_bytes_future(64 * 1024, glib::Priority::DEFAULT)
            .await
            .map_err(ContentError::Read)?;
        if chunk.is_empty() {
            return Ok(bytes);
        }
        if bytes.len() + chunk.len() > MAX_DROPPED_FILE_BYTES {
            return Err(ContentError::TooLarge);
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
            DropRequest::Unreadable(reason) => {
                self.end_content_drop();
                show_save_error(&self.overlay, &reason);
            }
        }
    }

    fn end_content_drop(&self) {
        self.stop_drag_autoscroll();
        self.drop_active_depths.set(None);
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().saturating_add(1));
        self.suppress_scroll_after_drop.set(false);
    }

    fn save_dropped_content(self: &Rc<Self>, destination: Location, files: Vec<DroppedFile>) {
        self.end_content_drop();
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
                Err(error) => show_save_error(&state.overlay, &io_error_message(&error)),
            }
        });
    }
}

fn show_save_error(parent: &impl IsA<gtk::Widget>, reason: &str) {
    super::show_error_dialog(
        parent,
        &crate::i18n::tr("Unable to save dropped content"),
        reason,
    );
}
