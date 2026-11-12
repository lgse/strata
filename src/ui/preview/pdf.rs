// SPDX-License-Identifier: MIT

use super::*;

mod pages;
mod selection;
mod zoom;

use selection::{LayerPoint, pdf_text_at, pdf_text_near};
pub(super) use selection::{
    pdf_apply_ranges, pdf_desired_ranges, pdf_drop_unselected_layer, pdf_selected_text,
};
use zoom::{
    pdf_page_width, preserve_pdf_view_center, resize_pdf_pages, set_adjustment_value,
    set_pdf_page_texture,
};

type PdfPages = HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>;
type PdfLayers = HashMap<i32, Arc<PdfTextLayer>>;
type PdfRanges = HashMap<i32, (usize, usize)>;

/// The page a PDF preview opens on, already rendered.
pub(super) struct PdfFirstPage {
    pub(super) png: Vec<u8>,
    pub(super) page: i32,
    pub(super) text_layer: Option<Arc<PdfTextLayer>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PdfDrag {
    Idle,
    Pan,
    Select,
}

/// The last selection press, which double and triple clicks extend.
#[derive(Clone, Copy)]
struct PdfPress {
    at: Instant,
    x: f64,
    y: f64,
    page: i32,
    streak: u8,
}

impl PdfPress {
    fn continues(&self, page: i32, (x, y): (f64, f64)) -> bool {
        let quick = self.at.elapsed() < Duration::from_millis(450);
        let same_spot = (x - self.x).abs() <= 6.0 && (y - self.y).abs() <= 6.0;
        self.page == page && quick && same_spot
    }
}

/// Zoom, shown pages and text selection of one PDF preview.
struct PdfViewer {
    zoom: Cell<f64>,
    page_width: Rc<Cell<i32>>,
    pages: RefCell<PdfPages>,
    layers: Rc<RefCell<PdfLayers>>,
    ranges: Rc<RefCell<PdfRanges>>,
    drag: Cell<PdfDrag>,
    drag_origin: Cell<(f64, f64)>,
    anchor: Cell<(i32, usize)>,
    granularity: Cell<u8>,
    press: Cell<PdfPress>,
}

impl PreviewState {
    pub(super) fn render_pdf_viewer(
        self: &Rc<Self>,
        entry: FileEntry,
        first: PdfFirstPage,
        pages: i32,
    ) {
        let page_count = pages.clamp(0, 10_000);
        let labels: Vec<_> = (1..=page_count).map(|page| page.to_string()).collect();
        let labels: Vec<_> = labels.iter().map(String::as_str).collect();
        let model = gtk::StringList::new(&labels);
        let selection = gtk::NoSelection::new(Some(model));
        let viewer = Rc::new(PdfViewer::new());
        if let Some(layer) = first.text_layer {
            viewer.layers.borrow_mut().insert(first.page, layer);
        }
        let loader = Rc::new(pages::PageLoader {
            viewer: viewer.clone(),
            provider: self.provider.clone(),
            loads: self.pdf_loads.clone(),
            entry,
            render_size: self.media_preview_size(),
            initial: RefCell::new(Some((first.page, first.png))),
            next_request: Cell::new(self.next_request.get().saturating_add(10_000)),
        });
        let factory = pages::page_factory(&loader);

        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.add_css_class("preview-pdf-list");
        list.set_hexpand(true);
        list.set_vexpand(true);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();

        scroll.add_css_class("preview-pdf-scroll");
        let weak_scroll = scroll.downgrade();
        let zoom_viewer = viewer.clone();
        self.zoom.bind(move |next| {
            if let Some(scroll) = weak_scroll.upgrade()
                && let Some(previous) = zoom_viewer.set_zoom(&scroll, next)
            {
                preserve_pdf_view_center(&scroll, next / previous);
            }
        });
        scroll.set_focusable(true);
        scroll.set_cursor_from_name(Some("grab"));
        install_pointer(&scroll, &viewer);
        install_keys(&scroll, &viewer);
        scroll.add_tick_callback(move |scroll, _| {
            viewer.follow_width(scroll);
            glib::ControlFlow::Continue
        });
        self.content.append(&scroll);
    }
}

/// A drag over text selects it; elsewhere it pans, and hovering shows which applies.
fn install_pointer(scroll: &gtk::ScrolledWindow, viewer: &Rc<PdfViewer>) {
    let pan = gtk::GestureDrag::new();
    pan.set_button(1);
    pan.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_scroll = scroll.downgrade();
    let viewer_for_begin = viewer.clone();
    pan.connect_drag_begin(move |gesture, x, y| {
        if let Some(scroll) = weak_scroll.upgrade() {
            viewer_for_begin.drag_begin(&scroll, gesture, (x, y));
        }
    });
    let weak_scroll = scroll.downgrade();
    let viewer_for_update = viewer.clone();
    pan.connect_drag_update(move |gesture, offset_x, offset_y| {
        if let Some(scroll) = weak_scroll.upgrade() {
            viewer_for_update.drag_update(&scroll, gesture, (offset_x, offset_y));
        }
    });
    let weak_scroll = scroll.downgrade();
    let viewer_for_end = viewer.clone();
    pan.connect_drag_end(move |_, _, _| {
        let panned = viewer_for_end.drag.replace(PdfDrag::Idle) == PdfDrag::Pan;
        if panned && let Some(scroll) = weak_scroll.upgrade() {
            scroll.set_cursor_from_name(Some("grab"));
        }
    });
    scroll.add_controller(pan);

    let motion = gtk::EventControllerMotion::new();
    let weak_scroll = scroll.downgrade();
    let viewer = viewer.clone();
    motion.connect_motion(move |_, x, y| {
        if viewer.drag.get() != PdfDrag::Idle {
            return;
        }
        if let Some(scroll) = weak_scroll.upgrade() {
            let over_text = viewer.text_at(&scroll, (x, y)).is_some();
            scroll.set_cursor_from_name(Some(if over_text { "text" } else { "grab" }));
        }
    });
    scroll.add_controller(motion);
}

fn install_keys(scroll: &gtk::ScrolledWindow, viewer: &Rc<PdfViewer>) {
    let keys = gtk::EventControllerKey::new();
    let weak_scroll = scroll.downgrade();
    let viewer = viewer.clone();
    keys::on_press(&keys, move |_, press| {
        let Some(scroll) = weak_scroll.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let handled = if press.is(SELECT_ALL) {
            viewer.select_all()
        } else if press.is(COPY) {
            viewer.copy_selection(&scroll)
        } else {
            false
        };
        if handled {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    scroll.add_controller(keys);
}

impl PdfViewer {
    fn new() -> Self {
        Self {
            zoom: Cell::new(1.0),
            page_width: Rc::new(Cell::new(0)),
            pages: RefCell::new(HashMap::new()),
            layers: Rc::new(RefCell::new(HashMap::new())),
            ranges: Rc::new(RefCell::new(HashMap::new())),
            drag: Cell::new(PdfDrag::Idle),
            drag_origin: Cell::new((0.0, 0.0)),
            anchor: Cell::new((-1, 0)),
            granularity: Cell::new(1),
            press: Cell::new(PdfPress {
                at: Instant::now(),
                x: f64::MAX,
                y: f64::MAX,
                page: -1,
                streak: 0,
            }),
        }
    }

    fn text_at(&self, scroll: &gtk::ScrolledWindow, point: (f64, f64)) -> Option<LayerPoint> {
        pdf_text_at(scroll, &self.pages.borrow(), &self.layers.borrow(), point)
    }

    fn select(&self, desired: PdfRanges) {
        pdf_apply_ranges(&self.ranges, &self.layers, &self.pages.borrow(), desired);
    }

    /// Returns the previous zoom when it changed.
    fn set_zoom(&self, scroll: &gtk::ScrolledWindow, next: f64) -> Option<f64> {
        let previous = self.zoom.replace(next);
        if (next - previous).abs() < f64::EPSILON {
            return None;
        }
        let width = pdf_page_width(scroll, next);
        self.page_width.set(width);
        resize_pdf_pages(&self.pages.borrow(), width);
        Some(previous)
    }

    fn follow_width(&self, scroll: &gtk::ScrolledWindow) {
        if scroll.width() > PDF_PAGE_GAP * 2 + 1 {
            let width = pdf_page_width(scroll, self.zoom.get());
            if width != self.page_width.replace(width) {
                resize_pdf_pages(&self.pages.borrow(), width);
            }
        }
    }

    fn drag_begin(&self, scroll: &gtk::ScrolledWindow, gesture: &gtk::GestureDrag, at: (f64, f64)) {
        let Some(hit) = self.text_at(scroll, at) else {
            self.begin_pan(scroll);
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        self.drag.set(PdfDrag::Select);
        scroll.grab_focus();
        let caret = pdf_text::caret_at(&hit.layer, hit.x, hit.y);
        let shift = gesture
            .current_event_state()
            .contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let streak = self.press_streak(hit.page, at, shift);
        let extend = shift && self.anchor.get().0 >= 0;
        self.granularity.set(if extend { 1 } else { streak });
        let desired = if extend {
            pdf_desired_ranges(
                &self.layers.borrow(),
                self.anchor.get(),
                (hit.page, caret),
                1,
            )
        } else {
            self.anchor_unit(&hit, caret, streak)
        };
        self.select(desired);
    }

    fn begin_pan(&self, scroll: &gtk::ScrolledWindow) {
        self.drag.set(PdfDrag::Pan);
        scroll.set_cursor_from_name(Some("grabbing"));
        scroll.grab_focus();
        self.drag_origin
            .set((scroll.hadjustment().value(), scroll.vadjustment().value()));
        self.anchor.set((-1, 0));
        self.select(HashMap::new());
    }

    /// Counts this press into a click streak: one selects a caret, two a word, three a line.
    fn press_streak(&self, page: i32, at: (f64, f64), shift: bool) -> u8 {
        let press = self.press.get();
        let streak = if !shift && press.continues(page, at) {
            (press.streak + 1).min(3)
        } else {
            1
        };
        self.press.set(PdfPress {
            at: Instant::now(),
            x: at.0,
            y: at.1,
            page,
            streak,
        });
        streak
    }

    fn anchor_unit(&self, hit: &LayerPoint, caret: usize, streak: u8) -> PdfRanges {
        let unit = match streak {
            2 => pdf_text::word_range(&hit.layer, caret),
            3 => pdf_text::line_range(&hit.layer, caret),
            _ => (caret, caret),
        };
        self.anchor.set((hit.page, unit.0));
        HashMap::from([(hit.page, unit)])
            .into_iter()
            .filter(|(_, r)| r.0 != r.1)
            .collect()
    }

    fn drag_update(
        &self,
        scroll: &gtk::ScrolledWindow,
        gesture: &gtk::GestureDrag,
        offset: (f64, f64),
    ) {
        if self.drag.get() == PdfDrag::Select {
            self.extend_selection(scroll, gesture, offset);
            return;
        }
        let (horizontal, vertical) = self.drag_origin.get();
        set_adjustment_value(&scroll.hadjustment(), horizontal - offset.0);
        set_adjustment_value(&scroll.vadjustment(), vertical - offset.1);
    }

    fn extend_selection(
        &self,
        scroll: &gtk::ScrolledWindow,
        gesture: &gtk::GestureDrag,
        (offset_x, offset_y): (f64, f64),
    ) {
        let Some((start_x, start_y)) = gesture.start_point() else {
            return;
        };
        let pages = self.pages.borrow();
        let layers = self.layers.borrow();
        let Some(point) = pdf_text_near(
            scroll,
            &pages,
            &layers,
            (start_x + offset_x, start_y + offset_y),
        ) else {
            return;
        };
        let caret = pdf_text::caret_at(&point.layer, point.x, point.y);
        let desired = pdf_desired_ranges(
            &layers,
            self.anchor.get(),
            (point.page, caret),
            self.granularity.get(),
        );
        drop(layers);
        pdf_apply_ranges(&self.ranges, &self.layers, &pages, desired);
    }

    fn select_all(&self) -> bool {
        let layers = self.layers.borrow();
        if layers.is_empty() {
            return false;
        }
        let desired = layers
            .iter()
            .map(|(page, layer)| (*page, (0, pdf_text::len(layer))))
            .collect();
        drop(layers);
        self.select(desired);
        true
    }

    fn copy_selection(&self, scroll: &gtk::ScrolledWindow) -> bool {
        let text = pdf_selected_text(&self.layers.borrow(), &self.ranges.borrow());
        if text.is_empty() {
            return false;
        }
        scroll.clipboard().set_text(&text);
        true
    }
}
