// SPDX-License-Identifier: MIT

//! The video stack, laid out like the audio view: the frame is the hero, the
//! header sits right under it, then the timeline and transport, and the whole
//! group is centred in the pane.

use std::cell::{Cell, RefCell};

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use crate::ui::preview::media_layout::{MAX_CONTENT_WIDTH, fitted_size};

const PADDING: i32 = 16;
const GAP: i32 = 12;
const CONTROLS_GAP: i32 = 4;
const PANEL_MIN: i32 = 200;
/// A frame shorter than this is not worth showing beside the controls.
const FRAME_MIN: i32 = 48;
const DEFAULT_ASPECT: f64 = 16.0 / 9.0;

struct Parts {
    frame: gtk::Widget,
    header: gtk::Widget,
    timeline: gtk::Widget,
    transport: gtk::Widget,
}

impl Parts {
    fn of(widget: &gtk::Widget) -> Option<Self> {
        let frame = widget.first_child()?;
        let header = frame.next_sibling()?;
        let timeline = header.next_sibling()?;
        let transport = timeline.next_sibling()?;
        Some(Self {
            frame,
            header,
            timeline,
            transport,
        })
    }

    fn panel_minimum(&self) -> i32 {
        [&self.header, &self.timeline, &self.transport]
            .iter()
            .map(|part| part.measure(gtk::Orientation::Horizontal, -1).0)
            .max()
            .unwrap_or(0)
            .max(PANEL_MIN)
    }

    fn height(widget: &gtk::Widget, width: i32) -> i32 {
        let width = width.max(widget.measure(gtk::Orientation::Horizontal, -1).0);
        widget.measure(gtk::Orientation::Vertical, width).1
    }

    fn fixed_height(&self, width: i32) -> i32 {
        Self::height(&self.header, width)
            + GAP
            + Self::height(&self.timeline, width)
            + CONTROLS_GAP
            + Self::height(&self.transport, width)
    }
}

mod imp {
    use super::*;

    pub struct PlayerLayout {
        pub(super) paintable: glib::WeakRef<gdk::Paintable>,
        pub(super) aspect: Cell<Option<f64>>,
        pub(super) margin: Cell<i32>,
        pub(super) on_allocate: RefCell<Option<Box<dyn Fn()>>>,
    }

    impl Default for PlayerLayout {
        fn default() -> Self {
            Self {
                paintable: glib::WeakRef::default(),
                aspect: Cell::new(None),
                margin: Cell::new(0),
                on_allocate: RefCell::new(None),
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
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            _: i32,
        ) -> (i32, i32, i32, i32) {
            let Some(parts) = Parts::of(widget) else {
                return (0, 0, -1, -1);
            };
            let panel_min = parts.panel_minimum();
            if orientation == gtk::Orientation::Horizontal {
                let minimum = panel_min + 2 * PADDING;
                return (minimum, minimum, -1, -1);
            }
            let minimum = parts.fixed_height(panel_min) + 2 * PADDING;
            (minimum, minimum + FRAME_MIN + GAP, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _: i32) {
            let Some(parts) = Parts::of(widget) else {
                return;
            };
            let content_width = (width - 2 * PADDING).clamp(0, MAX_CONTENT_WIDTH);
            let panel_min = parts.panel_minimum();
            let fixed = parts.fixed_height(content_width.max(panel_min));
            let free = height - 2 * PADDING - fixed - GAP;
            let margin = self.margin.get();
            let (frame_width, frame_height) =
                self.frame_size(content_width - 2 * margin, free - 2 * margin);
            let (outer_width, outer_height) = if frame_height >= FRAME_MIN {
                (frame_width + 2 * margin, frame_height + 2 * margin)
            } else {
                (0, 0)
            };
            let panel_width = outer_width.max(panel_min).min(content_width.max(panel_min));
            let stack = fixed
                + if outer_height > 0 {
                    outer_height + GAP
                } else {
                    0
                };
            let top = (height - stack) / 2;
            allocate_at(
                &parts.frame,
                (width - outer_width) / 2,
                top,
                outer_width,
                outer_height,
            );
            let x = (width - panel_width) / 2;
            let mut y = top
                + if outer_height > 0 {
                    outer_height + GAP
                } else {
                    0
                };
            let natural =
                |part: &gtk::Widget| part.measure(gtk::Orientation::Vertical, panel_width).1;
            for (part, gap) in [
                (&parts.header, GAP),
                (&parts.timeline, CONTROLS_GAP),
                (&parts.transport, 0),
            ] {
                let part_height = natural(part);
                allocate_at(part, x, y, panel_width, part_height);
                y += part_height + gap;
            }
            if let Some(on_allocate) = self.on_allocate.borrow().as_ref() {
                on_allocate();
            }
        }
    }

    impl PlayerLayout {
        /// The picture's size inside `width` × `height`: the paintable's aspect
        /// (enlarged at most twice) once known, otherwise the announced aspect.
        fn frame_size(&self, width: i32, height: i32) -> (i32, i32) {
            let (width, height) = (width.max(0), height.max(0));
            if let Some(paintable) = self.paintable.upgrade()
                && paintable.intrinsic_width() > 0
                && paintable.intrinsic_height() > 0
            {
                return fitted_size(
                    width,
                    height,
                    paintable.intrinsic_width(),
                    paintable.intrinsic_height(),
                );
            }
            let aspect = self.aspect.get().unwrap_or(DEFAULT_ASPECT);
            let by_width = (f64::from(width) / aspect).floor() as i32;
            if by_width <= height {
                (width, by_width)
            } else {
                ((f64::from(height) * aspect).floor() as i32, height)
            }
        }
    }
}

fn allocate_at(widget: &gtk::Widget, x: i32, y: i32, width: i32, height: i32) {
    widget.set_child_visible(width > 0 && height > 0);
    widget.allocate(
        width.max(0),
        height.max(0),
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

    /// The frame follows this paintable's size once it has one.
    pub(super) fn set_paintable(&self, paintable: Option<&gdk::Paintable>) {
        self.imp().paintable.set(paintable);
        self.layout_changed();
    }

    /// The aspect to reserve before a frame exists, from the poster or probe.
    pub(super) fn set_aspect(&self, aspect: Option<f64>) {
        let aspect = aspect.filter(|aspect| aspect.is_finite() && *aspect > 0.0);
        if self.imp().aspect.replace(aspect) != aspect {
            self.layout_changed();
        }
    }

    /// Space kept around the picture on every side, inside the frame.
    pub(super) fn set_margin(&self, margin: i32) {
        if self.imp().margin.replace(margin) != margin {
            self.layout_changed();
        }
    }

    /// Runs after each allocation, so followers of the pane's size need no
    /// frame-clock tick.
    pub(super) fn set_on_allocate(&self, callback: impl Fn() + 'static) {
        self.imp().on_allocate.replace(Some(Box::new(callback)));
    }
}
