// SPDX-License-Identifier: MIT

use crate::{
    sandbox::Cancellation,
    services::{
        DocumentMedia,
        document_media::{self, DOCUMENT_MEDIA_LIMIT},
    },
};
use gtk::{gdk, gio, glib, prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::PathBuf,
    rc::Rc,
};

type MediaKey = (usize, Option<usize>);
const MEDIA_RENDER_SLOTS: usize = 4;

struct Entry {
    source: DocumentMedia,
    alt: String,
    result: Option<Result<gdk::Texture, String>>,
    rows: Vec<glib::WeakRef<gtk::Box>>,
}

pub(super) struct MediaCache {
    path: Option<PathBuf>,
    cancellation: Cancellation,
    entries: RefCell<HashMap<MediaKey, Entry>>,
    pending: RefCell<VecDeque<MediaKey>>,
    running: Cell<usize>,
}

impl Drop for MediaCache {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl MediaCache {
    pub(super) fn new(path: Option<PathBuf>) -> Rc<Self> {
        Rc::new(Self {
            path,
            cancellation: Cancellation::default(),
            entries: RefCell::new(HashMap::new()),
            pending: RefCell::new(VecDeque::new()),
            running: Cell::new(0),
        })
    }

    pub(super) fn bind(
        self: &Rc<Self>,
        index: MediaKey,
        source: &DocumentMedia,
        alt: &str,
        row: &gtk::Box,
    ) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.add_css_class("preview-document-media");
        row.append(&content);
        let mut entries = self.entries.borrow_mut();
        if !entries.contains_key(&index) && entries.len() >= DOCUMENT_MEDIA_LIMIT {
            show_error(
                &content,
                source,
                alt,
                "Preview limited to 16 images, diagrams, and equations",
            );
            return;
        }
        let entry = entries.entry(index).or_insert_with(|| {
            self.pending.borrow_mut().push_back(index);
            Entry {
                source: source.clone(),
                alt: image_description(alt),
                result: None,
                rows: Vec::new(),
            }
        });
        entry.rows.retain(|row| row.upgrade().is_some());
        entry.rows.push(content.downgrade());
        show_entry(&content, entry);
        drop(entries);
        self.start_next();
    }

    fn start_next(self: &Rc<Self>) {
        if self.running.get() >= MEDIA_RENDER_SLOTS {
            return;
        }
        let Some(index) = self.pending.borrow_mut().pop_front() else {
            return;
        };
        self.running.set(self.running.get() + 1);
        let source = self.entries.borrow()[&index].source.clone();
        let path = self.path.clone();
        let cancellation = self.cancellation.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                document_media::render(&source, path.as_deref(), &cancellation)
            })
            .await;
            let Some(cache) = weak.upgrade() else {
                return;
            };
            let result = result
                .unwrap_or_else(|_| Err("Image renderer stopped unexpectedly".into()))
                .and_then(|bytes| {
                    gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
                        .map_err(|_| "Cannot display decoded image".into())
                });
            {
                let mut entries = cache.entries.borrow_mut();
                if let Some(entry) = entries.get_mut(&index) {
                    entry.result = Some(result);
                    for row in &entry.rows {
                        if let Some(row) = row.upgrade() {
                            show_entry(&row, entry);
                            if let Some(view) = row
                                .ancestor(super::document_view::DocumentTextView::static_type())
                                .and_downcast::<super::document_view::DocumentTextView>()
                            {
                                super::virtual_preview::schedule_document_view_size(
                                    &view,
                                    view.width(),
                                );
                            }
                        }
                    }
                }
            }
            cache.running.set(cache.running.get().saturating_sub(1));
            cache.start_next();
        });
    }
}

fn show_entry(row: &gtk::Box, entry: &Entry) {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    match &entry.result {
        None => {
            let label = gtk::Label::new(Some(
                if matches!(entry.source, DocumentMedia::Math { display: false, .. }) {
                    "…"
                } else {
                    "Loading preview…"
                },
            ));
            label.add_css_class("preview-note");
            row.append(&label);
        }
        Some(Err(message)) => show_error(row, &entry.source, &entry.alt, message),
        Some(Ok(texture)) => {
            let picture = gtk::Picture::for_paintable(texture);
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            let frame: MediaPicture = glib::Object::new();
            frame.imp().inline.set(matches!(
                entry.source,
                DocumentMedia::Math { display: false, .. }
            ));
            frame
                .imp()
                .natural_size
                .set((texture.width(), texture.height()));
            picture.set_parent(&frame);
            row.append(&frame);
            picture.set_alternative_text(Some(match &entry.source {
                DocumentMedia::Image(_) => &entry.alt,
                DocumentMedia::Mermaid(_) => "Mermaid diagram",
                DocumentMedia::Math { .. } => "LaTeX equation",
            }));
            if matches!(&entry.source, DocumentMedia::Math { display: false, .. }) {
                register_diagram(&frame);
            }
            let source_to_copy = match &entry.source {
                DocumentMedia::Mermaid(source) => Some((source, "Copy diagram source")),
                DocumentMedia::Math {
                    source,
                    display: true,
                } => Some((source, "Copy equation source")),
                _ => None,
            };
            if let Some((source, label)) = source_to_copy {
                register_diagram(&frame);
                let button = gtk::Button::with_label(label);
                button.add_css_class("preview-header-action");
                button.set_halign(gtk::Align::End);
                let source = source.clone();
                button.connect_clicked(move |button| button.clipboard().set_text(&source));
                row.append(&button);
            }
        }
    }
}

fn image_description(alt: &str) -> String {
    let mut chars = alt.chars();
    let mut description: String = chars.by_ref().take(512).collect();
    if chars.next().is_some() {
        description.push('…');
    }
    description
}

fn show_error(row: &gtk::Box, source: &DocumentMedia, alt: &str, message: &str) {
    let alt = image_description(alt);
    if let DocumentMedia::Math {
        source,
        display: false,
    } = source
    {
        let label = gtk::Label::new(Some(&format!("${source}$")));
        label.set_tooltip_text(Some(message));
        label.add_css_class("preview-document");
        row.append(&label);
        return;
    }
    let description = match source {
        DocumentMedia::Math { .. } => format!("Equation unavailable: {message}"),
        DocumentMedia::Image(_) => format!("Image: {alt}\n{message}"),
        DocumentMedia::Mermaid(_) => format!("Mermaid diagram unavailable: {message}"),
    };
    let label = gtk::Label::new(Some(&description));
    label.add_css_class("preview-note");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    row.append(&label);
    if let DocumentMedia::Mermaid(source) | DocumentMedia::Math { source, .. } = source {
        let label = gtk::Label::new(Some(source));
        label.add_css_class("monospace");
        label.set_selectable(true);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_xalign(0.0);
        row.append(&label);
    }
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct MediaPicture {
        pub colors: Cell<Option<(gdk::RGBA, gdk::RGBA)>>,
        pub natural_size: Cell<(i32, i32)>,
        pub inline: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for MediaPicture {
        const NAME: &'static str = "StrataDocumentMediaPicture";
        type Type = super::MediaPicture;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for MediaPicture {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }
    impl WidgetImpl for MediaPicture {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                child.allocate(width, height, baseline, None);
            }
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let (width, height) = self.natural_size.get();
            if orientation == gtk::Orientation::Horizontal {
                (if self.inline.get() { width } else { 0 }, width, -1, -1)
            } else {
                let available = if for_size < 0 { width } else { for_size };
                let scale = (f64::from(available) / f64::from(width.max(1))).min(1.0);
                let height = (f64::from(height) * scale).ceil().max(1.0) as i32;
                (height, height, -1, -1)
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some((text, surface)) = self.colors.get() else {
                if let Some(child) = self.obj().first_child() {
                    self.obj().snapshot_child(&child, snapshot);
                }
                return;
            };
            // Map the raster's luminance onto semantic colors without re-running untrusted input.
            let delta = [
                surface.red() - text.red(),
                surface.green() - text.green(),
                surface.blue() - text.blue(),
            ];
            let matrix = gtk::graphene::Matrix::from_float([
                0.2126 * delta[0],
                0.2126 * delta[1],
                0.2126 * delta[2],
                0.0,
                0.7152 * delta[0],
                0.7152 * delta[1],
                0.7152 * delta[2],
                0.0,
                0.0722 * delta[0],
                0.0722 * delta[1],
                0.0722 * delta[2],
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ]);
            snapshot.push_color_matrix(
                &matrix,
                &gtk::graphene::Vec4::new(text.red(), text.green(), text.blue(), 0.0),
            );
            if let Some(child) = self.obj().first_child() {
                self.obj().snapshot_child(&child, snapshot);
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct MediaPicture(ObjectSubclass<imp::MediaPicture>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

thread_local! {
    static DIAGRAMS: RefCell<Vec<glib::WeakRef<MediaPicture>>> = const { RefCell::new(Vec::new()) };
    static COLORS: Cell<Option<(gdk::RGBA, gdk::RGBA)>> = const { Cell::new(None) };
}

fn register_diagram(diagram: &MediaPicture) {
    let _manager = super::theme::ThemeManager::shared();
    diagram.imp().colors.set(COLORS.get());
    DIAGRAMS.with_borrow_mut(|diagrams| {
        diagrams.retain(|diagram| diagram.upgrade().is_some());
        diagrams.push(diagram.downgrade());
    });
}

pub(super) fn apply_theme(tokens: &super::theme::ThemeTokens) {
    let (Ok(text), Ok(surface)) = (
        gdk::RGBA::parse(&tokens.text),
        gdk::RGBA::parse(&tokens.surface),
    ) else {
        return;
    };
    COLORS.set(Some((text, surface)));
    DIAGRAMS.with_borrow_mut(|diagrams| {
        diagrams.retain(|diagram| {
            let Some(diagram) = diagram.upgrade() else {
                return false;
            };
            diagram.imp().colors.set(Some((text, surface)));
            diagram.queue_draw();
            true
        })
    });
}

#[cfg(test)]
mod tests;
