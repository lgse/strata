// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::{Cell, RefCell};

use gtk::{glib, prelude::*, subclass::prelude::*};

const CODE_HORIZONTAL_PADDING: i32 = 10;
pub(super) const DOCUMENT_PANEL_MARGIN: i32 = 6;
pub(super) const CODE_TOP_SPACE: i32 = 14;
pub(super) const CODE_BOTTOM_SPACE: i32 = 18;

#[derive(Clone, Debug)]
pub(super) struct DocumentCodeBlock {
    pub start: i32,
    pub end: i32,
    pub first: bool,
    pub last: bool,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DocumentTextView {
        pub(super) code_blocks: RefCell<Vec<CodeRange>>,
        pub code_fill: RefCell<Option<gtk::gdk::RGBA>>,
        pub code_border: RefCell<Option<gtk::gdk::RGBA>>,
        pub selection: RefCell<Option<gtk::gdk::RGBA>>,
        pub selection_range: Cell<Option<(i32, i32)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DocumentTextView {
        const NAME: &'static str = "StrataDocumentTextView";
        type Type = super::DocumentTextView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for DocumentTextView {}
    impl WidgetImpl for DocumentTextView {}
    impl ScrollableImpl for DocumentTextView {}

    impl TextViewImpl for DocumentTextView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            if layer == gtk::TextViewLayer::BelowText {
                self.snapshot_code_blocks(&snapshot);
                self.snapshot_selection(&snapshot);
            }
            self.parent_snapshot_layer(layer, snapshot);
        }
    }

    impl DocumentTextView {
        fn snapshot_code_blocks(&self, snapshot: &gtk::Snapshot) {
            let view = self.obj();
            let Some(fill) = self.code_fill.borrow().as_ref().copied() else {
                return;
            };
            let Some(border) = self.code_border.borrow().as_ref().copied() else {
                return;
            };
            let buffer = view.buffer();
            let visible = view.visible_rect();
            let visible_bottom = visible.y() + visible.height();
            let content_right = view
                .hadjustment()
                .map_or(f64::from(visible.x() + visible.width()), |adjustment| {
                    adjustment.upper()
                }) as f32
                - view.right_margin() as f32;

            for range in self.code_blocks.borrow().iter() {
                let start = buffer.iter_at_offset(range.start);
                let end = buffer.iter_at_offset(range.end.saturating_sub(1));
                let (first_y, _) = view.line_yrange(&start);
                let (last_y, last_height) = view.line_yrange(&end);
                if last_y + last_height < visible.y() || first_y > visible_bottom {
                    continue;
                }
                let text_x = view.iter_location(&start).x();
                let Some([x, y, width, height]) = code_panel_bounds(
                    text_x,
                    first_y,
                    last_y,
                    last_height,
                    content_right,
                    range.first,
                    range.last,
                ) else {
                    continue;
                };
                let bounds = gtk::graphene::Rect::new(x, y, width, height);
                let zero = gtk::graphene::Size::new(0.0, 0.0);
                let round = gtk::graphene::Size::new(5.0, 5.0);
                let outline = gtk::gsk::RoundedRect::new(
                    bounds,
                    if range.first { round } else { zero },
                    if range.first { round } else { zero },
                    if range.last { round } else { zero },
                    if range.last { round } else { zero },
                );
                snapshot.push_rounded_clip(&outline);
                snapshot.append_color(&fill, &bounds);
                snapshot.pop();
                snapshot.append_border(&outline, &[1.0; 4], &[border; 4]);
            }
        }

        fn snapshot_selection(&self, snapshot: &gtk::Snapshot) {
            let view = self.obj();
            let buffer = view.buffer();
            let Some((start, end)) = self.selection_range.get() else {
                return;
            };
            let selection_start = buffer.iter_at_offset(start.min(buffer.char_count()));
            let selection_end = buffer.iter_at_offset(end.min(buffer.char_count()));
            let Some(color) = self.selection.borrow().as_ref().copied() else {
                return;
            };
            let visible = view.visible_rect();
            let visible_bottom = visible.y() + visible.height();
            let mut line_start = if view.iter_location(&selection_start).y() < visible.y() {
                view.line_at_y(visible.y()).0
            } else {
                selection_start
            };
            view.backward_display_line_start(&mut line_start);

            while line_start.offset() < selection_end.offset() {
                let mut line_end = line_start;
                view.forward_display_line_end(&mut line_end);
                let start =
                    buffer.iter_at_offset(line_start.offset().max(selection_start.offset()));
                let end = buffer.iter_at_offset(line_end.offset().min(selection_end.offset()));
                let start_rect = view.iter_location(&start);
                if start_rect.y() > visible_bottom {
                    break;
                }
                let end_rect = view.iter_location(&end);
                if let Some([x, y, width, height]) = selection_span_bounds(
                    start_rect.x(),
                    end_rect.x(),
                    start_rect.y(),
                    start_rect.height(),
                ) {
                    snapshot.append_color(&color, &gtk::graphene::Rect::new(x, y, width, height));
                }

                let previous = line_start.offset();
                if !view.forward_display_line(&mut line_start) || line_start.offset() <= previous {
                    break;
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct DocumentTextView(ObjectSubclass<imp::DocumentTextView>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl DocumentTextView {
    pub(super) fn new(buffer: &gtk::TextBuffer, code_blocks: Vec<DocumentCodeBlock>) -> Self {
        let view: Self = glib::Object::builder().property("buffer", buffer).build();
        view.set_code_blocks(code_blocks);
        view
    }

    pub(super) fn set_code_blocks(&self, code_blocks: Vec<DocumentCodeBlock>) {
        self.imp().code_blocks.replace(
            code_blocks
                .into_iter()
                .map(|block| CodeRange {
                    start: block.start,
                    end: block.end,
                    first: block.first,
                    last: block.last,
                })
                .collect(),
        );
        self.queue_draw();
    }

    pub(super) fn set_selection_range(&self, range: Option<(i32, i32)>) {
        if self.imp().selection_range.replace(range) != range {
            self.queue_draw();
        }
    }

    pub(super) fn set_colors(
        &self,
        fill: gtk::gdk::RGBA,
        border: gtk::gdk::RGBA,
        selection: gtk::gdk::RGBA,
    ) {
        self.imp().code_fill.replace(Some(fill));
        self.imp().code_border.replace(Some(border));
        self.imp().selection.replace(Some(selection));
        self.queue_draw();
    }
}

fn selection_span_bounds(start_x: i32, end_x: i32, y: i32, height: i32) -> Option<[f32; 4]> {
    let x = start_x.min(end_x);
    let width = (end_x - start_x).unsigned_abs();
    (width > 0 && height > 0).then_some([x as f32, y as f32, width as f32, height as f32])
}

fn code_panel_bounds(
    text_x: i32,
    first_y: i32,
    last_y: i32,
    last_height: i32,
    content_right: f32,
    first: bool,
    last: bool,
) -> Option<[f32; 4]> {
    let x = (text_x - CODE_HORIZONTAL_PADDING).max(0) as f32;
    let top = if first { DOCUMENT_PANEL_MARGIN } else { 0 };
    let bottom = if last { DOCUMENT_PANEL_MARGIN } else { 0 };
    let y = (first_y + top) as f32;
    let width = content_right - x;
    let height = (last_y + last_height - first_y - top - bottom) as f32;
    (width > 0.0 && height > 0.0).then_some([x, y, width, height])
}

#[derive(Clone, Debug)]
pub(super) struct CodeRange {
    start: i32,
    end: i32,
    first: bool,
    last: bool,
}

#[cfg(test)]
mod tests;
