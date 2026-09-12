// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::Cell;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct WrapRow {
        pub spacing: Cell<i32>,
        pub end_align: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WrapRow {
        const NAME: &'static str = "StrataSettingsWrapRow";
        type Type = super::WrapRow;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for WrapRow {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for WrapRow {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let children = self.obj().children();
            if orientation == gtk::Orientation::Horizontal {
                let mut minimum = 0;
                let mut natural = 0;
                for (index, child) in children.iter().enumerate() {
                    let (min, nat, _, _) = child.measure(orientation, -1);
                    minimum = minimum.max(min);
                    natural += nat + if index == 0 { 0 } else { self.spacing.get() };
                }
                (minimum, natural, -1, -1)
            } else {
                let width = if for_size < 0 {
                    self.measure(gtk::Orientation::Horizontal, -1).1
                } else {
                    for_size
                };
                let layout = self.obj().layout(width.max(1));
                let height = layout
                    .iter()
                    .map(|(_, _, y, _, h)| y + h)
                    .max()
                    .unwrap_or(0);
                (height, height, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let layout = self.obj().layout(width.max(1));
            let used = layout
                .iter()
                .map(|(_, _, y, _, h)| y + h)
                .max()
                .unwrap_or(0);
            let offset = (height - used).max(0) / 2;
            for (child, x, y, width, height) in layout {
                let transform = gtk::gsk::Transform::new()
                    .translate(&gtk::graphene::Point::new(x as f32, (y + offset) as f32));
                child.allocate(width, height, -1, Some(transform));
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            for child in self.obj().children() {
                self.obj().snapshot_child(&child, snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct WrapRow(ObjectSubclass<imp::WrapRow>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl WrapRow {
    pub(super) fn new(spacing: i32) -> Self {
        let row: Self = glib::Object::new();
        row.imp().spacing.set(spacing);
        row
    }

    pub(super) fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.set_parent(self);
    }

    pub(super) fn spacing(&self) -> i32 {
        self.imp().spacing.get()
    }

    pub(super) fn set_end_align(&self, end: bool) {
        if self.imp().end_align.replace(end) != end {
            self.queue_allocate();
        }
    }

    fn children(&self) -> Vec<gtk::Widget> {
        let mut children = Vec::new();
        let mut child = self.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.is_visible() {
                children.push(widget);
            }
        }
        children
    }

    fn layout(&self, available: i32) -> Vec<(gtk::Widget, i32, i32, i32, i32)> {
        let (mut x, mut y) = (0, 0);
        let mut layout = Vec::new();
        let mut line_start = 0;
        for child in self.children() {
            let (minimum, natural, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
            // Let ellipsizing labels yield width before wrapping their metadata.
            let width = if child.hexpands() { minimum } else { natural }.min(available);
            if x > 0 && x + width > available {
                let height = align_line(
                    &mut layout[line_start..],
                    available,
                    self.imp().end_align.get(),
                );
                line_start = layout.len();
                x = 0;
                y += height + self.spacing();
            }
            layout.push((child, x, y, width, 0));
            x += width + self.spacing();
        }
        align_line(
            &mut layout[line_start..],
            available,
            self.imp().end_align.get(),
        );
        layout
    }
}

fn align_line(line: &mut [(gtk::Widget, i32, i32, i32, i32)], width: i32, end: bool) -> i32 {
    let spare = line
        .last()
        .map(|(_, x, _, w, _)| width - x - w)
        .unwrap_or(0);
    let count = line.iter().filter(|(child, ..)| child.hexpands()).count() as i32;
    let mut shift = if end && count == 0 { spare } else { 0 };
    let mut remainder = if count > 0 { spare % count } else { 0 };
    let mut height = 0;
    for (child, x, _, width, child_height) in line.iter_mut() {
        *x += shift;
        if child.hexpands() {
            let extra = spare / count + i32::from(remainder > 0);
            remainder = (remainder - 1).max(0);
            *width += extra;
            shift += extra;
        }
        *child_height = child.measure(gtk::Orientation::Vertical, *width).1;
        height = height.max(*child_height);
    }
    for (_, _, y, _, child_height) in line {
        *y += (height - *child_height) / 2;
    }
    height
}
