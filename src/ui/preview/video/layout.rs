// SPDX-License-Identifier: MIT

//! Player layout: rows above the frame stack at the top, rows below it at the
//! bottom, and the frame fits the remaining height to the paintable's aspect,
//! resting on the timeline so the slack opens between header and picture.

use std::cell::Cell;

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use crate::ui::preview::media_layout::{MAX_CONTENT_WIDTH, fitted_size};

/// The frame is the second child: the header comes first, controls follow.
const FRAME_INDEX: usize = 1;

mod imp {
    use super::*;

    pub struct PlayerLayout {
        pub(super) paintable: glib::WeakRef<gdk::Paintable>,
        pub(super) margin: Cell<i32>,
    }

    impl Default for PlayerLayout {
        fn default() -> Self {
            Self {
                paintable: glib::WeakRef::default(),
                margin: Cell::new(0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PlayerLayout {
        const NAME: &'static str = "StrataVideoPlayerLayout";
        type Type = super::PlayerLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for PlayerLayout {}

    impl LayoutManagerImpl for PlayerLayout {
        fn request_mode(&self, _: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let mut minimum = 0;
            let mut natural = 0;
            for (index, child) in children(widget).iter().enumerate() {
                if index == FRAME_INDEX || !child.should_layout() {
                    continue;
                }
                let (min, nat, _, _) = child.measure(orientation, for_size.min(MAX_CONTENT_WIDTH));
                if orientation == gtk::Orientation::Horizontal {
                    minimum = minimum.max(min);
                    natural = natural.max(nat);
                } else {
                    minimum += min;
                    natural += nat;
                }
            }
            (minimum, natural, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _: i32) {
            let children = children(widget);
            let Some(frame) = children.get(FRAME_INDEX) else {
                return;
            };
            let section_width = width.min(MAX_CONTENT_WIDTH);
            let x = (width - section_width) / 2;
            let row_height = |child: &gtk::Widget| {
                if child.should_layout() {
                    child.measure(gtk::Orientation::Vertical, section_width).1
                } else {
                    0
                }
            };
            let above: Vec<(gtk::Widget, i32)> = children[..FRAME_INDEX]
                .iter()
                .map(|child| (child.clone(), row_height(child)))
                .collect();
            let below: Vec<(gtk::Widget, i32)> = children[FRAME_INDEX + 1..]
                .iter()
                .map(|child| (child.clone(), row_height(child)))
                .collect();
            let rows_height: i32 = above.iter().chain(&below).map(|(_, height)| height).sum();
            let frame_height = (height - rows_height).max(0);
            let mut y = 0;
            for (child, height) in &above {
                allocate_at(child, section_width, *height, x, y);
                y += height;
            }
            let (intrinsic_width, intrinsic_height) =
                self.paintable.upgrade().map_or((0, 0), |paintable| {
                    (paintable.intrinsic_width(), paintable.intrinsic_height())
                });
            let margin = self.margin.get();
            let (fitted_width, fitted_height) = fitted_size(
                section_width - margin * 2,
                frame_height - margin * 2,
                intrinsic_width,
                intrinsic_height,
            );
            let outer_width = (fitted_width + margin * 2).min(section_width);
            let outer_height = (fitted_height + margin * 2).min(frame_height);
            allocate_at(
                frame,
                outer_width,
                outer_height,
                (width - outer_width) / 2,
                y + frame_height - outer_height,
            );
            y += frame_height;
            for (child, height) in &below {
                allocate_at(child, section_width, *height, x, y);
                y += height;
            }
        }
    }
}

fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        children.push(current);
    }
    children
}

fn allocate_at(widget: &gtk::Widget, width: i32, height: i32, x: i32, y: i32) {
    widget.allocate(
        width,
        height,
        -1,
        Some(gsk::Transform::new().translate(&graphene::Point::new(x as f32, y as f32))),
    );
}

glib::wrapper! {
    pub struct PlayerLayout(ObjectSubclass<imp::PlayerLayout>) @extends gtk::LayoutManager;
}

impl PlayerLayout {
    pub(super) fn new() -> Self {
        glib::Object::new()
    }

    /// The frame is fitted to this paintable's aspect; `None` lets it fill.
    pub(super) fn set_paintable(&self, paintable: Option<&gdk::Paintable>) {
        self.imp().paintable.set(paintable);
    }

    /// Space kept around the fitted frame on every side.
    pub(super) fn set_margin(&self, margin: i32) {
        if self.imp().margin.replace(margin) != margin {
            self.layout_changed();
        }
    }
}
