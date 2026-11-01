// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use crate::services::{ModelEncoding, ModelMesh, ModelPreviewStage, ModelView};

use super::{
    ListingPosition, PreviewState,
    components::{self, DetailsHeader, navigation_button, navigation_row},
    media_layout,
    player_layout::{self, PlayerLayout},
};

pub(super) struct ModelPreview {
    pub(super) picture: gtk::Picture,
    pub(super) previous: gtk::Button,
    pub(super) next: gtk::Button,
    pub(super) camera: gtk::DrawingArea,
    spinner: gtk::Spinner,
    root: gtk::Box,
    layout: PlayerLayout,
    eyebrow: gtk::Label,
    pub(super) title: gtk::Label,
    pub(super) format: gtk::Label,
    pub(super) triangles: gtk::Label,
    camera_motion: Rc<CameraMotion>,
    error: gtk::Label,
    view: Cell<ModelView>,
    loading: Cell<bool>,
}

fn view_name(view: ModelView) -> String {
    crate::i18n::tr(match view {
        ModelView::FrontLeft => "Front left",
        ModelView::FrontRight => "Front right",
        ModelView::BackRight => "Back right",
        ModelView::BackLeft => "Back left",
    })
}

fn picture(texture: Option<&gdk::Texture>) -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_paintable(texture);
    picture.add_css_class("preview-image");
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_cursor_from_name(Some("grab"));
    crate::ui::accessibility::set_label(&picture, &crate::i18n::tr("Model preview"));
    picture
}

struct CameraMotion {
    view: Cell<ModelView>,
    angle: Cell<f64>,
    target: Cell<f64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

fn camera_angle(view: ModelView) -> f64 {
    let [x, y, _] = view.camera_direction();
    f64::from(-y).atan2(f64::from(x))
}

impl CameraMotion {
    fn new(view: ModelView) -> Self {
        let angle = camera_angle(view);
        Self {
            view: Cell::new(view),
            angle: Cell::new(angle),
            target: Cell::new(angle),
            tick: RefCell::new(None),
        }
    }

    fn finish(&self, camera: &gtk::DrawingArea) {
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        self.angle.set(self.target.get());
        camera.queue_draw();
    }

    fn set_view(self: &Rc<Self>, camera: &gtk::DrawingArea, view: ModelView) {
        let previous = self.view.replace(view);
        if previous == view {
            return;
        }
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        let turn = (camera_angle(view) - camera_angle(previous) + std::f64::consts::PI)
            .rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        // An unwrapped destination preserves direction through fast cached turns.
        let target = self.target.get() + turn;
        self.target.set(target);
        if !crate::ui::motion::animations_enabled() || !camera.is_mapped() {
            self.finish(camera);
            return;
        }
        let from = self.angle.get();
        let started = Cell::new(None);
        let motion = self.clone();
        let tick = camera.add_tick_callback(move |camera, clock| {
            let now = clock.frame_time();
            let start = started.get().unwrap_or_else(|| {
                started.set(Some(now));
                now
            });
            let progress = if crate::ui::motion::animations_enabled() {
                ((now - start) as f64 / 240_000.0).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if progress >= 1.0 {
                motion.angle.set(target);
                motion.tick.borrow_mut().take();
                camera.queue_draw();
                glib::ControlFlow::Break
            } else {
                motion.angle.set(
                    from + (target - from) * crate::ui::motion::emphasized_deceleration(progress),
                );
                camera.queue_draw();
                glib::ControlFlow::Continue
            }
        });
        self.tick.replace(Some(tick));
    }
}

fn camera_position(motion: Rc<CameraMotion>) -> gtk::DrawingArea {
    let camera = gtk::DrawingArea::builder()
        .content_width(64)
        .content_height(64)
        .accessible_role(gtk::AccessibleRole::Img)
        .build();
    camera.add_css_class("preview-media-center");
    camera.add_css_class("preview-model-camera");
    camera.set_can_target(false);
    crate::ui::accessibility::set_label(&camera, &crate::i18n::tr("Camera position"));
    let unmapped = motion.clone();
    camera.connect_unmap(move |camera| unmapped.finish(camera));
    camera.set_draw_func(move |area, context, width, height| {
        draw_camera_position(area, context, width, height, motion.angle.get());
    });
    camera
}

#[expect(
    deprecated,
    reason = "GTK exposes named theme colors through StyleContext"
)]
fn draw_camera_position(
    area: &gtk::DrawingArea,
    context: &gtk::cairo::Context,
    width: i32,
    height: i32,
    angle: f64,
) {
    let accent = area.color();
    let style = area.style_context();
    let background = style.lookup_color("strata_bg").unwrap_or(accent);
    let border = style.lookup_color("strata_border").unwrap_or(accent);
    let color = |color: &gdk::RGBA, opacity: f64| {
        context.set_source_rgba(
            f64::from(color.red()),
            f64::from(color.green()),
            f64::from(color.blue()),
            f64::from(color.alpha()) * opacity,
        );
    };
    let (x, y) = (f64::from(width) * 0.5, f64::from(height) * 0.5);
    // Leave room for the dot to straddle the circumference without clipping.
    let radius = (f64::from(width.min(height)) * 0.5 - 4.0).max(0.0);
    context.arc(x, y, radius, 0.0, std::f64::consts::TAU);
    color(&background, 0.72);
    let _ = context.fill_preserve();
    color(&border, 0.42);
    context.set_line_width(1.0);
    let _ = context.stroke();
    context.arc(x, y, 8.0, 0.0, std::f64::consts::TAU);
    color(&accent, 0.8);
    context.set_line_width(1.5);
    let _ = context.stroke();
    context.arc(
        x + angle.cos() * radius,
        y + angle.sin() * radius,
        3.5,
        0.0,
        std::f64::consts::TAU,
    );
    color(&accent, 1.0);
    let _ = context.fill();
}

impl ModelPreview {
    fn new(view: ModelView, spinner: gtk::Spinner, navigate: Rc<dyn Fn(i32)>) -> Self {
        let picture = picture(None);
        let DetailsHeader {
            root: header,
            eyebrow,
            title,
        } = DetailsHeader::new(None);
        let previous = navigation_button(
            crate::assets::icons::SKIP_BACK,
            "Previous model view",
            "Previous model view",
            None,
        );
        let next = navigation_button(
            crate::assets::icons::SKIP_FORWARD,
            "Next model view",
            "Next model view",
            None,
        );
        for (button, step) in [(&previous, -1), (&next, 1)] {
            button.set_valign(gtk::Align::Center);
            let navigate = navigate.clone();
            button.connect_clicked(move |_| navigate(step));
        }
        let camera_motion = Rc::new(CameraMotion::new(view));
        let camera = camera_position(camera_motion.clone());
        let row = navigation_row(&previous, &camera, &next, 16);
        let error = gtk::Label::new(None);
        error.add_css_class("preview-feedback-detail");
        error.set_wrap(true);
        error.set_justify(gtk::Justification::Center);
        error.set_visible(false);
        let badges = components::badge_row();
        badges.set_halign(gtk::Align::Start);
        let format = components::badge("");
        let triangles = components::badge("");
        triangles.set_visible(false);
        badges.append(&format);
        badges.append(&triangles);
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 12);
        controls.append(&row);
        controls.append(&error);
        let (root, layout) = player_layout::section(player_layout::Areas {
            content: picture.upcast_ref(),
            details: header.upcast_ref(),
            badges: Some(badges.upcast_ref()),
            timeline: None,
            controls: controls.upcast_ref(),
        });
        root.add_css_class("preview-model");
        layout.set_aspect(Some(1.0));
        let model = Self {
            picture,
            previous,
            next,
            camera,
            spinner,
            root,
            layout,
            eyebrow,
            title,
            format,
            triangles,
            camera_motion,
            error,
            view: Cell::new(view),
            loading: Cell::new(false),
        };
        model.set_camera_position(view);
        model
    }

    pub(super) fn view(&self) -> ModelView {
        self.view.get()
    }

    fn set_header(&self, filename: &str, position: Option<ListingPosition>) {
        self.title.set_text(filename);
        self.eyebrow
            .set_text(&position.map(ListingPosition::caption).unwrap_or_default());
        self.eyebrow.set_visible(position.is_some());
    }

    fn set_badges(&self, mesh: ModelMesh) {
        let format = match mesh.encoding {
            ModelEncoding::BinaryStl => crate::i18n::tr("Binary STL"),
            ModelEncoding::AsciiStl => crate::i18n::tr("ASCII STL"),
            ModelEncoding::ThreeMf => "3MF".to_owned(),
        };
        self.format.set_text(&format);
        self.set_triangles(mesh.triangles);
    }

    fn set_triangles(&self, triangles: usize) {
        self.triangles
            .set_text(&crate::i18n::count("triangles", triangles));
        self.triangles.set_visible(true);
    }

    fn set_camera_position(&self, view: ModelView) {
        let name = view_name(view);
        self.camera_motion.set_view(&self.camera, view);
        crate::ui::accessibility::set_description(&self.camera, Some(&name));
        self.camera.queue_draw();
    }

    pub(super) fn set_loading(&self, view: ModelView) {
        self.loading.set(true);
        self.set_camera_position(view);
        self.error.set_visible(false);
        let has_image = self.picture.paintable().is_some();
        self.previous.set_sensitive(has_image);
        self.next.set_sensitive(has_image);
        self.picture.set_opacity(0.3);
        self.picture.set_cursor_from_name(Some("wait"));
        crate::ui::accessibility::set_busy(&self.picture, true);
        crate::ui::accessibility::set_label(&self.spinner, &crate::i18n::tr("Loading preview…"));
        self.spinner.set_visible(true);
        self.spinner.start();
    }

    pub(super) fn set_progress(&self, stage: ModelPreviewStage) {
        if self.loading.get() {
            crate::ui::accessibility::set_label(&self.spinner, &stage.label());
            if let ModelPreviewStage::Rendering { triangles } = stage {
                self.set_triangles(triangles);
            }
        }
    }

    fn finish_loading(&self) {
        self.loading.set(false);
        self.spinner.stop();
        self.spinner.set_visible(false);
        let has_image = self.picture.paintable().is_some();
        self.previous.set_sensitive(has_image);
        self.next.set_sensitive(has_image);
        self.picture.set_opacity(1.0);
        self.picture.set_cursor_from_name(Some("grab"));
        crate::ui::accessibility::set_busy(&self.picture, false);
    }

    fn update(&self, texture: &gdk::Texture, mesh: ModelMesh) {
        let view = mesh.view;
        self.picture.set_paintable(Some(texture));
        self.layout.set_paintable(Some(texture.upcast_ref()));
        self.view.set(view);
        self.set_camera_position(view);
        self.set_badges(mesh);
        self.finish_loading();
        self.error.set_visible(false);
    }

    pub(super) fn show_error(&self, detail: &str) {
        self.set_camera_position(self.view.get());
        self.finish_loading();
        self.error.set_text(detail);
        self.error.set_visible(true);
    }
}

// The header spinner outlives this preview; a file switch mid-render must stop it.
impl Drop for ModelPreview {
    fn drop(&mut self) {
        self.spinner.stop();
        self.spinner.set_visible(false);
    }
}

#[cfg(test)]
mod tests;

impl PreviewState {
    pub(super) fn render_model(self: &Rc<Self>, png: Vec<u8>, mesh: Option<ModelMesh>) {
        let bytes = glib::Bytes::from_owned(png);
        let texture = match gdk::Texture::from_bytes(&bytes) {
            Ok(texture) => texture,
            Err(error) => {
                if let Some(model) = self
                    .model
                    .borrow()
                    .as_ref()
                    .filter(|model| model.picture.paintable().is_some())
                {
                    self.model_view.set(model.view());
                    model.show_error(&error.to_string());
                    return;
                }
                self.show_message(&crate::i18n::tr("Preview unavailable"), &error.to_string());
                return;
            }
        };
        let Some(mesh) = mesh else {
            let picture = picture(Some(&texture));
            super::install_preview_drag(&picture, self);
            self.content
                .append(&media_layout::section(&picture, &texture));
            return;
        };
        self.model_view.set(mesh.view);
        let filename = self
            .current
            .borrow()
            .as_ref()
            .map(|entry| entry.display_name.clone())
            .unwrap_or_default();
        let position = self.model_listing_position();
        if let Some(model) = self.model.borrow().as_ref() {
            model.update(&texture, mesh);
            model.set_header(&filename, position);
            return;
        }
        let model = self.create_model();
        model.update(&texture, mesh);
        self.model.replace(Some(model));
    }

    pub(super) fn show_model_loading(self: &Rc<Self>) -> bool {
        if !self
            .current
            .borrow()
            .as_ref()
            .is_some_and(crate::services::is_model)
        {
            return false;
        }
        let model = self.create_model();
        model.set_loading(self.model_view.get());
        self.model.replace(Some(model));
        true
    }

    fn create_model(self: &Rc<Self>) -> ModelPreview {
        let weak = Rc::downgrade(self);
        let model = ModelPreview::new(
            self.model_view.get(),
            self.model_spinner.clone(),
            Rc::new(move |step| {
                if let Some(state) = weak.upgrade() {
                    state.change_model_view(step);
                }
            }),
        );
        if let Some(entry) = self.current.borrow().as_ref() {
            model.set_header(&entry.display_name, self.model_listing_position());
            let format = std::path::Path::new(&entry.native_name)
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_ascii_uppercase();
            model.format.set_text(&format);
        }
        super::install_preview_drag(&model.picture, self);
        self.content.append(&model.root);
        model
    }

    fn model_listing_position(&self) -> Option<ListingPosition> {
        let view = self
            .keyboard_view
            .borrow()
            .as_ref()
            .and_then(crate::ui::browser::WeakBrowserView::upgrade)?;
        let depth = self.current_depth.get()?;
        let current = self.current.borrow().as_ref()?.location.clone();
        let files = view.displayed_entries_matching(depth, |entry| {
            entry.is_file() && crate::services::is_model(entry)
        });
        let index = files.iter().position(|entry| entry.location == current)?;
        Some(ListingPosition {
            position: index + 1,
            count: files.len(),
            results: view.results_replace_listing(),
        })
    }

    fn change_model_view(self: &Rc<Self>, step: i32) {
        if self
            .model
            .borrow()
            .as_ref()
            .is_none_or(|model| model.loading.get())
        {
            return;
        }
        let Some(entry) = self.current.borrow().clone() else {
            return;
        };
        self.model_view.set(self.model_view.get().step(step));
        self.load(entry, 0);
    }
}
