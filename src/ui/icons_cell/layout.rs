// SPDX-License-Identifier: MIT

use gtk::{glib, graphene, gsk, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct CenteredIconLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for CenteredIconLayout {
        const NAME: &'static str = "StrataCenteredIconLayout";
        type Type = super::CenteredIconLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for CenteredIconLayout {}

    impl LayoutManagerImpl for CenteredIconLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let mut minimum = 0;
            let mut natural = 0;
            let mut child = widget.first_child();
            while let Some(current) = child {
                if current.should_layout() {
                    let (child_minimum, child_natural, _, _) =
                        current.measure(orientation, for_size);
                    if orientation == gtk::Orientation::Horizontal {
                        minimum = minimum.max(child_minimum);
                        natural = natural.max(child_natural);
                    } else {
                        minimum += child_minimum;
                        natural += child_natural;
                    }
                }
                child = current.next_sibling();
            }
            if orientation == gtk::Orientation::Vertical {
                let caption = widget
                    .last_child()
                    .map_or(0, |labels| {
                        labels.measure(gtk::Orientation::Vertical, for_size).1
                    })
                    .max(
                        super::super::rename_field(widget)
                            .filter(gtk::prelude::WidgetExt::is_visible)
                            .map_or(0, |field| {
                                field.measure(gtk::Orientation::Vertical, for_size).1
                            }),
                    );
                let reserved = natural.max(widget.height_request()).max(
                    super::super::parts(widget).map_or(0, |(icon, _)| icon.slot_size())
                        + caption
                        + super::super::ICONS_CARD_PAD_Y
                        + 3,
                );
                return (reserved, reserved, -1, -1);
            }
            (minimum, natural, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let Some((icon, label)) = super::super::parts(widget) else {
                return;
            };
            let Some(labels) = widget.last_child() else {
                return;
            };
            let caption_height = if label.is_visible() {
                let layout = label.create_pango_layout(label.text().as_deref());
                layout.set_attributes(label.attributes().as_ref());
                layout.set_width(width.max(1).saturating_mul(gtk::pango::SCALE));
                layout.set_wrap(label.wrap_mode());
                layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
                layout.set_height(-super::super::ICONS_CARD_LABEL_LINES);
                layout.pixel_size().1
            } else {
                super::super::rename_field(widget)
                    .filter(gtk::prelude::WidgetExt::is_visible)
                    .map_or(0, |field| {
                        field.measure(gtk::Orientation::Vertical, width).1
                    })
            };
            let slot = icon.slot_size();
            let top = ((height - slot - caption_height) / 2).max(0);
            icon.allocate(
                slot,
                slot,
                -1,
                Some(gsk::Transform::new().translate(&graphene::Point::new(
                    ((width - slot) / 2) as f32,
                    top as f32,
                ))),
            );
            labels.allocate(
                width,
                caption_height,
                -1,
                Some(
                    gsk::Transform::new()
                        .translate(&graphene::Point::new(0.0, (top + slot) as f32)),
                ),
            );
        }
    }
}

glib::wrapper! {
    pub struct CenteredIconLayout(ObjectSubclass<imp::CenteredIconLayout>)
        @extends gtk::LayoutManager;
}

pub(super) fn install(card: &gtk::Box, label: &gtk::Inscription) {
    card.set_layout_manager(Some(glib::Object::new::<CenteredIconLayout>()));
    let weak_card = card.downgrade();
    label.connect_text_notify(move |_| {
        if let Some(card) = weak_card.upgrade() {
            // Inscription's fixed requisition does not change when its text wraps.
            card.queue_allocate();
        }
    });
}
