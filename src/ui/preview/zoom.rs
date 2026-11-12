// SPDX-License-Identifier: MIT

//! Window-local preview magnification, independent of interface preferences.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

mod image;

pub(super) const MIN: f64 = 0.25;
pub(super) const MAX: f64 = 4.0;
type ZoomTarget = Rc<dyn Fn(f64)>;

pub(super) struct PreviewZoom {
    pub(super) row: gtk::Box,
    adjustment: gtk::Adjustment,
    percent: gtk::Label,
    css: gtk::CssProvider,
    display: gtk::gdk::Display,
    class: String,
    dpi_binding: RefCell<Option<(gtk::Settings, glib::SignalHandlerId)>>,
    target: RefCell<Option<ZoomTarget>>,
}

impl PreviewZoom {
    pub(super) fn new(content: &gtk::Box) -> Rc<Self> {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_margin_bottom(6);
        row.set_visible(false);
        let label = crate::i18n::tr("Preview zoom");
        let adjustment = gtk::Adjustment::new(1.0, MIN, MAX, 0.05, 0.25, 0.0);
        let slider = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&adjustment));
        slider.set_draw_value(false);
        slider.set_hexpand(true);
        super::super::accessibility::set_label(&slider, &label);
        let percent = gtk::Label::new(None);
        percent.set_width_chars(5);
        let reset = find::icon_button("Reset zoom", crate::assets::icons::UNDO_2);
        row.append(&slider);
        row.append(&percent);
        row.append(&reset);
        let css = gtk::CssProvider::new();
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let class = format!("preview-zoom-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        content.add_css_class(&class);
        let display = content.display();
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            super::super::theme::TOKEN_STYLE_PRIORITY,
        );
        let zoom = Rc::new(Self {
            row,
            adjustment,
            percent,
            css,
            display,
            class,
            dpi_binding: RefCell::new(None),
            target: RefCell::new(None),
        });
        zoom.apply();
        let weak = Rc::downgrade(&zoom);
        zoom.adjustment.connect_value_changed(move |_| {
            if let Some(zoom) = weak.upgrade() {
                zoom.apply();
            }
        });
        let weak = Rc::downgrade(&zoom);
        reset.connect_clicked(move |_| {
            if let Some(zoom) = weak.upgrade() {
                zoom.reset();
            }
        });
        zoom.bind_text_size();
        zoom
    }

    fn bind_text_size(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        super::super::preferences::PreferenceManager::shared().bind_preference(
            &self.row,
            |preferences| preferences.text_size(),
            move |_, _| {
                if let Some(zoom) = weak.upgrade() {
                    zoom.apply();
                }
            },
        );
        if let Some(settings) = gtk::Settings::default() {
            let weak = Rc::downgrade(self);
            let handler = settings.connect_gtk_xft_dpi_notify(move |_| {
                if let Some(zoom) = weak.upgrade() {
                    zoom.apply();
                }
            });
            self.dpi_binding.replace(Some((settings, handler)));
        }
    }

    pub(super) fn install(self: &Rc<Self>, pane: &gtk::Box) {
        self.install_reset(pane);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        wheel.connect_scroll(move |controller, _, dy| {
            let modifiers = controller.current_event_state();
            if !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                || modifiers.intersects(
                    gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SUPER_MASK,
                )
            {
                return glib::Propagation::Proceed;
            }
            if let Some(zoom) = weak.upgrade().filter(|zoom| zoom.row.is_visible()) {
                let steps = if controller.unit() == gtk::gdk::ScrollUnit::Surface {
                    dy / 10.0
                } else {
                    dy
                };
                zoom.adjustment
                    .set_value((zoom.value() * (-steps * 0.14).exp()).clamp(MIN, MAX));
            }
            glib::Propagation::Stop
        });
        pane.add_controller(wheel);
    }

    fn install_reset(self: &Rc<Self>, pane: &gtk::Box) {
        let reset = gio::SimpleAction::new("reset-zoom", None);
        let weak = Rc::downgrade(self);
        reset.connect_activate(move |_, _| {
            if let Some(zoom) = weak.upgrade() {
                zoom.reset();
            }
        });
        let actions = gio::SimpleActionGroup::new();
        actions.add_action(&reset);
        pane.insert_action_group("preview", Some(&actions));
    }

    pub(super) fn value(&self) -> f64 {
        self.adjustment.value()
    }

    pub(super) fn reset(&self) {
        self.adjustment.set_value(1.0);
    }

    pub(super) fn bind(&self, target: impl Fn(f64) + 'static) {
        target(self.value());
        self.target.replace(Some(Rc::new(target)));
    }

    pub(super) fn clear(&self) {
        self.target.borrow_mut().take();
        self.row.set_visible(false);
        self.css.load_from_string("");
    }

    pub(super) fn set_enabled(&self, enabled: bool) {
        self.row.set_visible(enabled);
        if enabled {
            self.apply();
        }
    }

    pub(super) fn image(
        &self,
        picture: &gtk::Picture,
        texture: &gtk::gdk::Texture,
    ) -> gtk::ScrolledWindow {
        let (scroll, image) = image::view(picture, texture);
        self.bind(move |zoom| image.set_zoom(zoom));
        scroll
    }

    fn apply(&self) {
        let value = (self.value() * 100.0).round() as i32;
        self.percent
            .set_text(&rust_i18n::t!("%{value}%", value = value));
        self.apply_text_size();
        if let Some(target) = self.target.borrow().as_ref() {
            target(self.value());
        }
    }

    fn apply_text_size(&self) {
        if !self.row.is_visible() {
            self.css.load_from_string("");
            return;
        }
        use super::super::preferences::{
            PreferenceManager, desktop_text_scale_factor, snapped_root_font_px,
        };
        let base = snapped_root_font_px(
            PreferenceManager::shared().text_size().root_font_px(),
            desktop_text_scale_factor(),
        );
        let pixels = base * self.value();
        let class = &self.class;
        self.css.load_from_string(&format!(
            ".{class}, .{class} textview, .{class} textview text {{ font-size: {pixels:.6}px; }}"
        ));
    }
}

impl Drop for PreviewZoom {
    fn drop(&mut self) {
        if let Some((settings, handler)) = self.dpi_binding.get_mut().take() {
            settings.disconnect(handler);
        }
        gtk::style_context_remove_provider_for_display(&self.display, &self.css);
    }
}
