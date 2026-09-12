// SPDX-License-Identifier: MIT

use gtk::{glib, graphene, gsk, prelude::*, subclass::prelude::*};

use super::{
    ICONS_CARD_CAPTION_GAP, ICONS_CARD_LABEL_CHARS, ICONS_CARD_LABEL_LINE_PX,
    ICONS_CARD_LABEL_LINES,
};

fn caption_label(widget: &gtk::Widget) -> Option<gtk::Inscription> {
    widget.first_child()?.downcast().ok()
}

fn caption_editor(widget: &gtk::Widget) -> Option<gtk::Widget> {
    widget
        .last_child()
        .filter(|child| child.is::<gtk::Entry>() && child.is_visible())
}

#[expect(
    deprecated,
    reason = "GTK4 exposes CSS box insets only through StyleContext"
)]
fn label_insets(label: &gtk::Inscription) -> (i32, i32) {
    let style = label.style_context();
    let padding = style.padding();
    let border = style.border();
    (
        i32::from(padding.left())
            + i32::from(padding.right())
            + i32::from(border.left())
            + i32::from(border.right()),
        i32::from(padding.top())
            + i32::from(padding.bottom())
            + i32::from(border.top())
            + i32::from(border.bottom()),
    )
}

fn preferred_caption_width(label: &gtk::Inscription) -> i32 {
    let metrics = label.pango_context().metrics(None, None);
    let chars = metrics
        .approximate_char_width()
        .saturating_mul(ICONS_CARD_LABEL_CHARS);
    (chars + gtk::pango::SCALE - 1) / gtk::pango::SCALE + label_insets(label).0
}

fn caption_extent(label: &gtk::Inscription, width: i32) -> (i32, i32) {
    let (horizontal, vertical) = label_insets(label);
    let layout = label.create_pango_layout(label.text().as_deref());
    layout.set_attributes(label.attributes().as_ref());
    layout.set_width(
        (width - horizontal)
            .max(1)
            .saturating_mul(gtk::pango::SCALE),
    );
    layout.set_wrap(label.wrap_mode());
    layout.set_alignment(gtk::pango::Alignment::Center);
    layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
    layout.set_height(-ICONS_CARD_LABEL_LINES);
    let (text_width, text_height) = layout.pixel_size();
    (text_width + horizontal, text_height + vertical)
}

fn reserved_caption_height(label: &gtk::Inscription, width: i32) -> i32 {
    // Measure independently of visibility so entering rename cannot shrink the row.
    let lines = std::iter::repeat_n("M", ICONS_CARD_LABEL_LINES as usize)
        .collect::<Vec<_>>()
        .join("\n");
    let layout = label.create_pango_layout(Some(&lines));
    layout.set_attributes(label.attributes().as_ref());
    (layout.pixel_size().1 + label_insets(label).1)
        .max(ICONS_CARD_LABEL_LINE_PX * ICONS_CARD_LABEL_LINES)
        .max(caption_extent(label, width).1)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct IconCellLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for IconCellLayout {
        const NAME: &'static str = "StrataIconCellLayout";
        type Type = super::IconCellLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for IconCellLayout {}

    impl LayoutManagerImpl for IconCellLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let (Some(icon), Some(caption)) = (widget.first_child(), widget.last_child()) else {
                return (0, 0, -1, -1);
            };
            let icon = icon.measure(orientation, for_size);
            let caption = caption.measure(orientation, for_size);
            let (minimum, natural) = if orientation == gtk::Orientation::Horizontal {
                (icon.0.max(caption.0), icon.1.max(caption.1))
            } else {
                (
                    icon.0 + caption.0 + ICONS_CARD_CAPTION_GAP,
                    icon.1 + caption.1 + ICONS_CARD_CAPTION_GAP,
                )
            };
            (minimum, natural, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let (Some(icon), Some(caption)) = (widget.first_child(), widget.last_child()) else {
                return;
            };
            let icon_width = icon.measure(gtk::Orientation::Horizontal, -1).1;
            let icon_height = icon.measure(gtk::Orientation::Vertical, -1).1;
            icon.allocate(
                icon_width,
                icon_height,
                -1,
                Some(gsk::Transform::new().translate(&graphene::Point::new(
                    ((width - icon_width) / 2) as f32,
                    0.0,
                ))),
            );
            let caption_y = icon_height + ICONS_CARD_CAPTION_GAP;
            caption.allocate(
                width,
                (height - caption_y).max(0),
                -1,
                Some(gsk::Transform::new().translate(&graphene::Point::new(0.0, caption_y as f32))),
            );
        }
    }

    #[derive(Default)]
    pub struct CaptionLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for CaptionLayout {
        const NAME: &'static str = "StrataIconCaptionLayout";
        type Type = super::CaptionLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for CaptionLayout {}

    impl LayoutManagerImpl for CaptionLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let Some(label) = caption_label(widget) else {
                return (0, 0, -1, -1);
            };
            if orientation == gtk::Orientation::Horizontal {
                let width = preferred_caption_width(&label);
                return (width, width, -1, -1);
            }
            let width = if for_size < 0 {
                preferred_caption_width(&label)
            } else {
                for_size
            };
            let height = reserved_caption_height(&label, width)
                .max(caption_editor(widget).map_or(0, |field| field.measure(orientation, width).1));
            (height, height, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let Some(label) = caption_label(widget) else {
                return;
            };
            if label.is_visible() {
                let (caption_width, caption_height) = caption_extent(&label, width);
                let caption_width = caption_width.min(width);
                label.allocate(
                    caption_width,
                    caption_height.min(height),
                    -1,
                    Some(gsk::Transform::new().translate(&graphene::Point::new(
                        ((width - caption_width) / 2) as f32,
                        0.0,
                    ))),
                );
            }
            if let Some(field) = caption_editor(widget) {
                let field_height = field
                    .measure(gtk::Orientation::Vertical, width)
                    .1
                    .min(height);
                field.allocate(width, field_height, -1, None);
            }
        }
    }
}

glib::wrapper! {
    pub struct IconCellLayout(ObjectSubclass<imp::IconCellLayout>)
        @extends gtk::LayoutManager;
}

glib::wrapper! {
    pub struct CaptionLayout(ObjectSubclass<imp::CaptionLayout>)
        @extends gtk::LayoutManager;
}

pub(super) fn install(card: &gtk::Box, label: &gtk::Inscription) {
    card.set_layout_manager(Some(glib::Object::new::<IconCellLayout>()));
    if let Some(caption) = label.parent() {
        caption.set_layout_manager(Some(glib::Object::new::<CaptionLayout>()));
    }
    label.connect_text_notify(|label| {
        if let Some(caption) = label.parent() {
            caption.queue_resize();
        }
    });
}
