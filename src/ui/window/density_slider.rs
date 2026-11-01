// SPDX-License-Identifier: MIT

//! An Appearance menu density slider: any px value in range, with clickable
//! Compact and Airy labels under their tick marks that snap to those presets.

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, graphene, gsk, prelude::*, subclass::prelude::*};

use crate::ui::{browser_modes::DensityScale, preferences::PreferenceManager};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SnapLabels {
        pub(super) scale: glib::WeakRef<gtk::Scale>,
        pub(super) labels: RefCell<Vec<(u8, gtk::Button)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SnapLabels {
        const NAME: &'static str = "StrataDensitySnapLabels";
        type Type = super::SnapLabels;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SnapLabels {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    const GAP: i32 = 8;

    impl WidgetImpl for SnapLabels {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let labels = self.labels.borrow();
            let horizontal = orientation == gtk::Orientation::Horizontal;
            let gap = if horizontal {
                GAP * labels.len().saturating_sub(1) as i32
            } else {
                0
            };
            let (mut minimum, mut natural) = (gap, gap);
            for (_, label) in labels.iter() {
                let (child_minimum, child_natural, _, _) = label.measure(orientation, -1);
                if horizontal {
                    minimum += child_minimum;
                    natural += child_natural;
                } else {
                    minimum = minimum.max(child_minimum);
                    natural = natural.max(child_natural);
                }
            }
            (minimum, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let centers = self.mark_centers();
            let labels = self.labels.borrow();
            let widths: Vec<_> = labels
                .iter()
                .map(|(_, label)| label.measure(gtk::Orientation::Horizontal, -1).1.min(width))
                .collect();
            let mut remaining =
                widths.iter().sum::<i32>() + GAP * labels.len().saturating_sub(1) as i32;
            let mut left = 0;
            for (index, ((px, label), label_width)) in labels.iter().zip(widths).enumerate() {
                let center = centers
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| self.fallback_center(*px, width));
                let x = (center - label_width as f32 / 2.0)
                    .clamp(left as f32, (width - remaining).max(left) as f32);
                label.allocate(
                    label_width,
                    height,
                    -1,
                    Some(gsk::Transform::new().translate(&graphene::Point::new(x, 0.0))),
                );
                left = x.ceil() as i32 + label_width + GAP;
                remaining -= label_width + GAP;
            }
        }
    }

    impl SnapLabels {
        /// GTK places each `mark` node (a documented scale CSS node) centred on its
        /// value; marks are added in ascending order, matching `labels`.
        fn mark_centers(&self) -> Vec<f32> {
            let Some(scale) = self.scale.upgrade() else {
                return Vec::new();
            };
            let obj = self.obj();
            let mut centers = Vec::new();
            let mut pending = vec![scale.upcast::<gtk::Widget>()];
            while let Some(widget) = pending.pop() {
                if widget.css_name() == "mark" {
                    if let Some(bounds) = widget.compute_bounds(&*obj) {
                        centers.push(bounds.x() + bounds.width() / 2.0);
                    }
                    continue;
                }
                let mut child = widget.last_child();
                while let Some(current) = child {
                    child = current.prev_sibling();
                    pending.push(current);
                }
            }
            if centers.len() == self.labels.borrow().len() {
                centers.sort_by(f32::total_cmp);
                centers
            } else {
                Vec::new()
            }
        }

        fn fallback_center(&self, px: u8, width: i32) -> f32 {
            let max = self
                .scale
                .upgrade()
                .map_or(1.0, |scale| scale.adjustment().upper().max(1.0));
            (f64::from(px) / max * f64::from(width)) as f32
        }
    }
}

glib::wrapper! {
    pub struct SnapLabels(ObjectSubclass<imp::SnapLabels>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SnapLabels {
    fn new(scale: &gtk::Scale, labels: Vec<(u8, gtk::Button)>) -> Self {
        let widget: Self = glib::Object::new();
        widget.add_css_class("appearance-density-snaps");
        widget.imp().scale.set(Some(scale));
        for (_, label) in &labels {
            label.set_parent(&widget);
        }
        widget.imp().labels.replace(labels);
        widget
    }
}

pub(super) fn density_slider<D: DensityScale>(
    preferences: &Rc<PreferenceManager>,
    name: &str,
    get: fn(&PreferenceManager) -> D,
    set: fn(&PreferenceManager, D),
) -> gtk::Box {
    let name = crate::i18n::tr(name);
    let control = gtk::Box::new(gtk::Orientation::Vertical, 0);
    control.add_css_class("appearance-density");

    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title = gtk::Label::new(Some(&name));
    title.add_css_class("appearance-density-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let value = gtk::Label::new(None);
    value.add_css_class("appearance-density-value");
    heading.append(&title);
    heading.append(&value);

    let scale =
        gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, f64::from(D::MAX_PX), 1.0);
    scale.set_increments(1.0, f64::from(D::AIRY_PX - D::COMPACT_PX));
    scale.set_round_digits(0);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.add_css_class("appearance-density-scale");
    super::super::browser_modes::disable_scale_long_press_zoom(&scale);
    pass_wheel_to_menu(&scale);
    super::super::accessibility::set_label(&scale, &name);
    for px in [D::COMPACT_PX, D::AIRY_PX] {
        scale.add_mark(f64::from(px), gtk::PositionType::Bottom, None);
    }
    {
        let preferences = preferences.clone();
        scale.connect_value_changed(move |scale| {
            let px = scale.value().round().clamp(0.0, f64::from(D::MAX_PX)) as u8;
            if get(&preferences).px() != px {
                set(&preferences, D::from_px(px));
            }
        });
    }

    let snaps: Vec<(u8, gtk::Button)> = [(D::COMPACT_PX, "Compact"), (D::AIRY_PX, "Airy")]
        .into_iter()
        .map(|(px, label)| {
            let button = gtk::Button::with_label(&crate::i18n::tr(label));
            button.set_has_frame(false);
            button.add_css_class("appearance-density-snap");
            let preferences = preferences.clone();
            button.connect_clicked(move |_| set(&preferences, D::from_px(px)));
            (px, button)
        })
        .collect();
    for (px, button) in &snaps {
        let px = *px;
        preferences.bind_preference(button, get, move |widget, density: D| {
            let selected = density.px() == px;
            if selected {
                widget.add_css_class("selected");
            } else {
                widget.remove_css_class("selected");
            }
            super::super::accessibility::set_pressed(widget, selected);
        });
    }
    let snap_labels = SnapLabels::new(&scale, snaps);

    preferences.bind_preference(&value, get, |widget, density: D| {
        widget
            .downcast_ref::<gtk::Label>()
            .expect("density value label")
            .set_text(&density_value_text(density));
    });
    preferences.bind_preference(&scale, get, |widget, density: D| {
        let scale = widget.downcast_ref::<gtk::Scale>().expect("density scale");
        scale.set_value(f64::from(density.px()));
        scale.update_property(&[gtk::accessible::Property::ValueText(&density_value_text(
            density,
        ))]);
    });

    control.append(&heading);
    control.append(&scale);
    control.append(&snap_labels);
    control
}

/// The menu scrolls, so a wheel passing over a slider scrolls it instead of moving the value.
fn pass_wheel_to_menu(scale: &gtk::Scale) {
    let controllers = scale.observe_controllers();
    let wheels: Vec<gtk::EventControllerScroll> = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index)?.downcast().ok())
        .collect();
    for wheel in wheels {
        scale.remove_controller(&wheel);
    }
}

fn density_value_text<D: DensityScale>(density: D) -> String {
    let px = density.px();
    if px == D::COMPACT_PX {
        rust_i18n::t!("Compact · %{px} px", px = px)
    } else if px == D::AIRY_PX {
        rust_i18n::t!("Airy · %{px} px", px = px)
    } else {
        rust_i18n::t!("Custom · %{px} px", px = px)
    }
    .into_owned()
}
