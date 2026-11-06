// SPDX-License-Identifier: MIT

use super::*;

/// Renders list pages on bind, starting from the page the preview opened with.
pub(super) struct PageLoader {
    pub(super) viewer: Rc<PdfViewer>,
    pub(super) provider: Rc<dyn PreviewProvider>,
    pub(super) loads: Rc<RefCell<HashMap<i32, LoadHandle>>>,
    pub(super) entry: FileEntry,
    pub(super) render_size: MediaPreviewSize,
    pub(super) initial: RefCell<Option<(i32, Vec<u8>)>>,
    pub(super) next_request: Cell<u64>,
}

struct PageWidgets {
    overlay: gtk::Overlay,
    picture: gtk::Picture,
    spinner: gtk::Spinner,
    text_area: gtk::DrawingArea,
}

impl PageWidgets {
    fn of(item: &gtk::ListItem) -> Option<Self> {
        let overlay = item.child().and_downcast::<gtk::Overlay>()?;
        let picture = overlay.child().and_downcast::<gtk::Picture>()?;
        let spinner = overlay.last_child().and_downcast::<gtk::Spinner>()?;
        let text_area = picture.next_sibling().and_downcast::<gtk::DrawingArea>()?;
        Some(Self {
            overlay,
            picture,
            spinner,
            text_area,
        })
    }

    fn stop_spinner(&self) {
        self.spinner.stop();
        self.spinner.set_visible(false);
    }
}

pub(super) fn page_factory(loader: &Rc<PageLoader>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&page_widget()));
        }
    });
    let loader_for_bind = loader.clone();
    factory.connect_bind(move |_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            loader_for_bind.bind(item);
        }
    });
    let loader = loader.clone();
    factory.connect_unbind(move |_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            loader.unbind(item.position() as i32);
        }
    });
    factory
}

fn page_widget() -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    let text_area = gtk::DrawingArea::new();
    text_area.set_hexpand(true);
    text_area.set_vexpand(true);
    text_area.set_accessible_role(gtk::AccessibleRole::Img);
    super::super::super::accessibility::set_label(&text_area, &crate::i18n::tr("PDF page text"));
    let spinner = gtk::Spinner::new();
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    overlay.set_child(Some(&picture));
    overlay.add_overlay(&text_area);
    overlay.add_overlay(&spinner);
    overlay.set_hexpand(true);
    overlay.set_halign(gtk::Align::Center);
    overlay.set_size_request(-1, 560);
    overlay
}

impl PageLoader {
    fn bind(&self, item: &gtk::ListItem) {
        let page_index = item.position() as i32;
        let Some(page) = PageWidgets::of(item) else {
            return;
        };
        let binding_name = format!("pdf-page-{page_index}");
        page.overlay.set_widget_name(&binding_name);
        crate::ui::accessibility::set_description(&page.overlay, None);
        let target_width = self.viewer.page_width.get();
        page.overlay
            .set_size_request(if target_width > 0 { target_width } else { -1 }, 560);
        page.picture.set_paintable(gtk::gdk::Paintable::NONE);
        page.spinner.start();
        page.spinner.set_visible(true);
        draw_selection(&page.text_area, page_index, &self.viewer);
        self.viewer.pages.borrow_mut().insert(
            page_index,
            (
                page.overlay.clone(),
                page.picture.clone(),
                page.text_area.clone(),
            ),
        );
        if let Some(png) = self.take_initial(page_index) {
            set_pdf_page_texture(
                &page.overlay,
                &page.picture,
                png,
                self.viewer.page_width.get(),
            );
            page.stop_spinner();
            return;
        }
        self.request_page(page_index, &page, binding_name);
    }

    fn take_initial(&self, page_index: i32) -> Option<Vec<u8>> {
        let is_initial_page = self
            .initial
            .borrow()
            .as_ref()
            .is_some_and(|(page, _)| *page == page_index);
        let cached = if is_initial_page {
            self.initial.borrow_mut().take()
        } else {
            None
        };
        cached.map(|(_, png)| png)
    }

    fn request_page(&self, page_index: i32, page: &PageWidgets, binding_name: String) {
        let request_id = PreviewRequestId(self.next_request.get());
        self.next_request
            .set(self.next_request.get().saturating_add(1));
        let target = PageTarget {
            request_id,
            page_index,
            binding_name,
            overlay: page.overlay.downgrade(),
            picture: page.picture.downgrade(),
            spinner: page.spinner.downgrade(),
            text_area: page.text_area.downgrade(),
            layers: self.viewer.layers.clone(),
            page_width: self.viewer.page_width.clone(),
        };
        let loads_for_event = self.loads.clone();
        let emit = Rc::new(move |event| {
            loads_for_event.borrow_mut().remove(&page_index);
            target.receive(event);
        });
        let load = self.provider.load(
            PreviewRequest {
                id: request_id,
                entry: self.entry.clone(),
                text_byte_limit: TEXT_BYTE_LIMIT,
                render_document: false,
                pdf_page: page_index,
                media_size: self.render_size,
                model_palette: super::super::super::theme::ThemeManager::shared()
                    .active_model_palette(),
                model_view: Default::default(),
                archive_password: None,
            },
            emit,
        );
        self.loads.borrow_mut().insert(page_index, load);
    }

    fn unbind(&self, page: i32) {
        self.loads.borrow_mut().remove(&page);
        self.viewer.pages.borrow_mut().remove(&page);
        pdf_drop_unselected_layer(&self.viewer.layers, &self.viewer.ranges, page);
    }
}

/// Paints the selection runs of `page_index` over its rendered image.
fn draw_selection(text_area: &gtk::DrawingArea, page_index: i32, viewer: &PdfViewer) {
    let layers = viewer.layers.clone();
    let ranges = viewer.ranges.clone();
    text_area.set_draw_func(move |_, cr, width, height| {
        let Some(layer) = layers.borrow().get(&page_index).cloned() else {
            return;
        };
        let Some(&(start, end)) = ranges.borrow().get(&page_index) else {
            return;
        };
        if start == end {
            return;
        }
        let Some(color) = pdf_selection_color() else {
            return;
        };
        let (ox, oy, s) = pdf_text::image_bounds(&layer, f64::from(width), f64::from(height));
        cr.set_source_rgba(
            f64::from(color.red()),
            f64::from(color.green()),
            f64::from(color.blue()),
            0.42,
        );
        for [x1, y1, x2, y2] in pdf_text::selection_runs(&layer, start, end) {
            let (rx, ry) = (ox + f64::from(x1) * s, oy + f64::from(y1) * s);
            let (rw, rh) = (f64::from(x2 - x1) * s, f64::from(y2 - y1) * s);
            let pad = rh * 0.06;
            rounded_rect(
                cr,
                (rx - pad, ry - pad, rw + pad * 2.0, rh + pad * 2.0),
                (rh * 0.16).min(3.0),
            );
        }
        let _ = cr.fill();
    });
}

fn pdf_selection_color() -> Option<gtk::gdk::RGBA> {
    crate::ui::theme::ThemeManager::shared()
        .current_tokens()
        .and_then(|tokens| gtk::gdk::RGBA::parse(&tokens.accent).ok())
}

fn rounded_rect(cr: &cairo::Context, (x, y, w, h): (f64, f64, f64, f64), r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// The page binding a render request answers; a rebound row ignores stale pages.
struct PageTarget {
    request_id: PreviewRequestId,
    page_index: i32,
    binding_name: String,
    overlay: glib::WeakRef<gtk::Overlay>,
    picture: glib::WeakRef<gtk::Picture>,
    spinner: glib::WeakRef<gtk::Spinner>,
    text_area: glib::WeakRef<gtk::DrawingArea>,
    layers: Rc<RefCell<PdfLayers>>,
    page_width: Rc<Cell<i32>>,
}

impl PageTarget {
    fn receive(&self, event: PreviewEvent) {
        let Some(overlay) = self
            .overlay
            .upgrade()
            .filter(|overlay| overlay.widget_name() == self.binding_name)
        else {
            return;
        };
        match event {
            PreviewEvent::Ready(Preview {
                request_id: response_id,
                content:
                    PreviewContent::Pdf {
                        png,
                        page,
                        text_layer,
                        ..
                    },
                ..
            }) if response_id == self.request_id && page == self.page_index => {
                self.show_page(&overlay, png, text_layer);
            }
            PreviewEvent::Failed {
                request_id: response_id,
                ..
            } if response_id == self.request_id => {
                crate::ui::accessibility::set_description(
                    &overlay,
                    Some(&crate::i18n::tr("Unable to render this PDF page")),
                );
            }
            PreviewEvent::Progress { .. }
            | PreviewEvent::Ready(_)
            | PreviewEvent::Failed { .. }
            | PreviewEvent::NeedsPassword { .. } => return,
        }
        if let Some(spinner) = self.spinner.upgrade() {
            spinner.stop();
            spinner.set_visible(false);
        }
    }

    fn show_page(
        &self,
        overlay: &gtk::Overlay,
        png: Vec<u8>,
        text_layer: Option<Arc<PdfTextLayer>>,
    ) {
        if let Some(layer) = text_layer {
            self.layers.borrow_mut().insert(self.page_index, layer);
            if let Some(area) = self.text_area.upgrade() {
                area.queue_draw();
            }
        }
        if let Some(picture) = self.picture.upgrade() {
            set_pdf_page_texture(overlay, &picture, png, self.page_width.get());
        }
    }
}
