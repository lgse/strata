// SPDX-License-Identifier: MIT

use std::{io, io::Write, path::Path, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    model::Location,
    services::{DropCommit, WebLink},
};

use super::{
    ViewState,
    clipboard::{FileDropState, file_drop_commit, locations_from_file_list_value},
    paths::is_trash_location,
};

mod binary;
pub(crate) use binary::{
    BrowserFile, file_format as binary_drop_format, install as install_binary_drop_target,
};

#[cfg(test)]
mod tests;

pub(crate) enum DropRequest {
    Files(Vec<Location>, DropCommit),
    Content(Vec<ContentFile>),
    Binary(Location, Vec<ContentFile>),
}

pub(crate) struct ContentFile {
    stem: String,
    extension: &'static str,
    data: ContentData,
}

enum ContentData {
    Bytes(Vec<u8>),
    Url(WebLink),
    BrowserFile(BrowserFile),
}

pub(crate) fn supports_drop_formats(formats: &gdk::ContentFormats) -> bool {
    let formats = formats.clone().union_deserialize_types();
    [
        gdk::Texture::static_type(),
        gdk::FileList::static_type(),
        String::static_type(),
    ]
    .into_iter()
    .any(|kind| formats.contains_type(kind))
}

pub(crate) fn is_content_value(value: &glib::Value) -> bool {
    if let Ok(files) = value.get::<gdk::FileList>() {
        return files.files().iter().any(|file| {
            glib::Uri::parse_scheme(&file.uri()).is_some_and(|scheme| {
                matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
            })
        });
    }
    value.type_() == BrowserFile::static_type()
        || value.type_().is_a(gdk::Texture::static_type())
        || value.type_() == String::static_type()
}

pub(crate) fn accepts_content_at(destination: &Location) -> bool {
    destination.native_path().is_some()
        && !is_trash_location(destination)
        && !destination.is_recent_location()
}

impl DropRequest {
    pub(crate) fn from_value(
        target: &gtk::DropTarget,
        value: &glib::Value,
        destination: &Location,
        state: &Rc<FileDropState>,
    ) -> Option<Self> {
        if let Ok(file) = value.get::<BrowserFile>() {
            if !accepts_content_at(&file.destination) {
                return None;
            }
            return Some(Self::Binary(
                file.destination.clone(),
                content_files(value)?,
            ));
        }
        if is_content_value(value) {
            if !accepts_content_at(destination) {
                return None;
            }
            Some(Self::Content(content_files(value)?))
        } else {
            let sources = locations_from_file_list_value(value)?;
            let commit = file_drop_commit(target, destination, &sources, state);
            (commit != DropCommit::Forbidden).then_some(Self::Files(sources, commit))
        }
    }
}

fn content_files(value: &glib::Value) -> Option<Vec<ContentFile>> {
    if let Ok(file) = value.get::<BrowserFile>() {
        let stem = if let Some(name) = &file.name {
            crate::services::validate_basename(name).ok()?;
            Path::new(name).file_stem()?.to_str()?.to_owned()
        } else {
            crate::i18n::tr("pasted_image.name")
        };
        return Some(vec![ContentFile {
            stem,
            extension: "png",
            data: ContentData::BrowserFile(file),
        }]);
    }
    if let Ok(texture) = value.get::<gdk::Texture>() {
        return Some(vec![ContentFile {
            stem: crate::i18n::tr("pasted_image.name"),
            extension: "png",
            data: ContentData::Bytes(texture.save_to_png_bytes().as_ref().to_vec()),
        }]);
    }
    if let Ok(files) = value.get::<gdk::FileList>() {
        return files
            .files()
            .iter()
            .map(|file| WebLink::parse(&file.uri()).map(link_file))
            .collect();
    }
    let text = value.get::<String>().ok()?;
    if text.is_empty() {
        return None;
    }
    if let Some(link) = WebLink::parse(&text) {
        return Some(vec![link_file(link)]);
    }
    Some(vec![ContentFile {
        stem: crate::i18n::tr("dropped_text.name"),
        extension: "txt",
        data: ContentData::Bytes(text.into_bytes()),
    }])
}

fn link_file(link: WebLink) -> ContentFile {
    ContentFile {
        stem: link.name.clone(),
        extension: "desktop",
        data: ContentData::Url(link),
    }
}

fn save_file(directory: &Path, content: &ContentFile) -> io::Result<std::path::PathBuf> {
    crate::services::validate_basename(&format!("{}.{}", content.stem, content.extension))
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".strata-drop-")
        .tempfile_in(directory)?;
    let (stem, extension, bytes) = match &content.data {
        ContentData::Bytes(bytes) => (
            content.stem.clone(),
            content.extension,
            std::borrow::Cow::Borrowed(bytes.as_slice()),
        ),
        ContentData::BrowserFile(file) => {
            let image = crate::services::web_image::image_bytes(file.bytes.clone())?;
            (
                content.stem.clone(),
                image.extension,
                std::borrow::Cow::Owned(image.bytes),
            )
        }
        ContentData::Url(link) => match crate::services::web_image::download(link)? {
            Some(image) => {
                let name = crate::services::remote_file_name(&link.address)
                    .unwrap_or_else(|| crate::i18n::tr("pasted_image.name"));
                let stem = Path::new(&name)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(&name)
                    .to_owned();
                (stem, image.extension, std::borrow::Cow::Owned(image.bytes))
            }
            None => (
                content.stem.clone(),
                content.extension,
                std::borrow::Cow::Owned(link.desktop_entry()),
            ),
        },
    };
    crate::services::validate_basename(&format!("{stem}.{extension}"))
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    temporary.write_all(&bytes)?;
    for suffix in 0u64.. {
        let name = if suffix == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem} ({suffix}).{extension}")
        };
        let destination = directory.join(name);
        match temporary.persist_noclobber(&destination) {
            Ok(_) => return Ok(destination),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                temporary = error.file;
            }
            Err(error) => return Err(error.error),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

impl ViewState {
    pub(super) fn commit_drop(self: &Rc<Self>, mut destination: Location, request: DropRequest) {
        let request = match request {
            DropRequest::Binary(captured, contents) => {
                destination = captured;
                DropRequest::Content(contents)
            }
            request => request,
        };
        match request {
            DropRequest::Binary(..) => unreachable!(),
            DropRequest::Files(sources, commit) => {
                self.commit_file_drop(destination, sources, commit)
            }
            DropRequest::Content(contents) => {
                self.stop_drag_autoscroll();
                self.drop_active_depths.set(None);
                self.horizontal_scroll_generation
                    .set(self.horizontal_scroll_generation.get().saturating_add(1));
                self.suppress_scroll_after_drop.set(false);
                glib::MainContext::default()
                    .spawn_local(self.save_dropped_content(destination, contents));
            }
        }
    }

    pub(super) fn save_dropped_content(
        self: &Rc<Self>,
        destination: Location,
        contents: Vec<ContentFile>,
    ) -> impl std::future::Future<Output = ()> + use<> {
        let directory = destination.native_path().map(Path::to_path_buf);
        let reveal = crate::ui::preferences::PreferenceManager::shared().open_folder_after_drop();
        let navigation_generation = self.browser.navigation_generation();
        let weak = Rc::downgrade(self);
        async move {
            let Some(directory) = directory else {
                return;
            };
            let result = gio::spawn_blocking(move || {
                contents
                    .iter()
                    .map(|content| save_file(&directory, content))
                    .collect::<io::Result<Vec<_>>>()
            })
            .await
            .unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::Other)));
            let Some(state) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(paths)
                    if reveal && state.browser.navigation_generation() == navigation_generation =>
                {
                    state.reveal_locations(
                        destination,
                        paths.into_iter().map(Location::local).collect(),
                        false,
                    )
                }
                Ok(_) => {}
                Err(error) => super::show_error_dialog(
                    &state.overlay,
                    &crate::i18n::tr("Unable to save dropped content"),
                    &crate::services::io_error_message(&error),
                ),
            }
        }
    }
}
