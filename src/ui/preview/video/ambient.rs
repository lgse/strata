// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use crate::ui::{
    media::ambient::{CELLS, EdgeGrid, GRID_HEIGHT, GRID_WIDTH},
    preview::audio::palette::{follow_theme, palette, with_alpha},
};

pub(super) const BAND: i32 = 24;
const OPACITY: f64 = 0.6;
const UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const SMOOTHING: f32 = 0.35;

mod imp {
    use super::*;

    pub struct Glow {
        pub(super) shown: RefCell<[[f32; 3]; CELLS]>,
        pub(super) texture: RefCell<Option<gdk::Texture>>,
        pub(super) updated: Cell<Option<Instant>>,
    }

    impl Default for Glow {
        fn default() -> Self {
            Self {
                shown: RefCell::new([[0.0; 3]; CELLS]),
                texture: RefCell::default(),
                updated: Cell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Glow {
        const NAME: &'static str = "StrataVideoGlow";
        type Type = super::Glow;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Glow {
        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.add_css_class("preview-video-glow");
            widget.set_can_target(false);
            widget.set_can_focus(false);
            widget.set_accessible_role(gtk::AccessibleRole::Presentation);
            follow_theme(&*widget);
        }
    }

    impl WidgetImpl for Glow {
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(texture) = self.texture.borrow().clone() else {
                return;
            };
            let widget = self.obj();
            let (width, height) = (widget.width() as f32, widget.height() as f32);
            let band = BAND as f32;
            if width <= band * 2.0 || height <= band * 2.0 {
                return;
            }
            let bounds = graphene::Rect::new(0.0, 0.0, width, height);
            snapshot.push_opacity(OPACITY);
            snapshot.append_scaled_texture(&texture, gsk::ScalingFilter::Linear, &bounds);
            snapshot.pop();
            let surface = palette().surface;
            let solid = with_alpha(surface, 1.0);
            let clear = with_alpha(surface, 0.0);
            let strips = [
                (
                    graphene::Rect::new(0.0, 0.0, width, band),
                    graphene::Point::new(0.0, 0.0),
                    graphene::Point::new(0.0, band),
                ),
                (
                    graphene::Rect::new(0.0, height - band, width, band),
                    graphene::Point::new(0.0, height),
                    graphene::Point::new(0.0, height - band),
                ),
                (
                    graphene::Rect::new(0.0, 0.0, band, height),
                    graphene::Point::new(0.0, 0.0),
                    graphene::Point::new(band, 0.0),
                ),
                (
                    graphene::Rect::new(width - band, 0.0, band, height),
                    graphene::Point::new(width, 0.0),
                    graphene::Point::new(width - band, 0.0),
                ),
            ];
            for (rect, start, end) in strips {
                snapshot.append_linear_gradient(
                    &rect,
                    &start,
                    &end,
                    &[
                        gsk::ColorStop::new(0.0, solid),
                        gsk::ColorStop::new(1.0, clear),
                    ],
                );
            }
        }
    }
}

glib::wrapper! {
    pub struct Glow(ObjectSubclass<imp::Glow>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Glow {
    pub(super) fn new() -> Self {
        glib::Object::new()
    }

    pub(super) fn update(&self, grid: &EdgeGrid) {
        let imp = self.imp();
        if imp
            .updated
            .get()
            .is_some_and(|updated| updated.elapsed() < UPDATE_INTERVAL)
        {
            return;
        }
        imp.updated.set(Some(Instant::now()));
        let fresh = imp.texture.borrow().is_none();
        let mut shown = imp.shown.borrow_mut();
        let mut bytes = Vec::with_capacity(CELLS * 3);
        for (current, target) in shown.iter_mut().zip(grid.cells.iter()) {
            for (channel, value) in current.iter_mut().enumerate() {
                let wanted = f32::from(target[channel]);
                *value = if fresh {
                    wanted
                } else {
                    *value + (wanted - *value) * SMOOTHING
                };
                bytes.push(value.round().clamp(0.0, 255.0) as u8);
            }
        }
        drop(shown);
        let texture = gdk::MemoryTexture::new(
            GRID_WIDTH as i32,
            GRID_HEIGHT as i32,
            gdk::MemoryFormat::R8g8b8,
            &glib::Bytes::from_owned(bytes),
            GRID_WIDTH * 3,
        );
        imp.texture.replace(Some(texture.upcast()));
        self.queue_draw();
    }

    pub(super) fn clear(&self) {
        let imp = self.imp();
        imp.texture.borrow_mut().take();
        imp.updated.set(None);
        self.queue_draw();
    }

    pub(in crate::ui::preview) fn is_lit(&self) -> bool {
        self.imp().texture.borrow().is_some()
    }
}
