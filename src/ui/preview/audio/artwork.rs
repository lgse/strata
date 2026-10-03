// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    f32::consts::TAU,
};

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use super::palette::{Palette, follow_theme, mix, palette, with_alpha};

/// Width over height: the square cover plus room for the disc to slide out.
pub(super) const ASPECT: f32 = 1.3;
const DISC: f32 = 0.94;
const TUCKED: f32 = 0.06;
const SLID_OUT: f32 = 0.3;
const SLIDE_RATE: f32 = 7.0;
/// Tucking away before a cover change is brisk so the new art is not kept waiting.
const TUCK_RATE: f32 = 14.0;
/// The crossfade starts once the record is this close to tucked, in pixels.
const TUCK_SETTLED: f32 = 2.0;
const FADE_SECONDS: f32 = 0.24;
/// The record fades on its own, never through a translucent cover.
const DISC_FADE_SECONDS: f32 = 0.18;
const SECONDS_PER_TURN: f32 = 2.4;
const LABEL: f32 = 0.34;
/// Room around the disc for its shadow, as a share of the radius.
const SHADOW_MARGIN: f32 = 0.2;
/// The CPU renderer repaints every pixel itself, so the label turns at film rate.
const SOFTWARE_SPIN_INTERVAL_US: i64 = 33_333;

/// A cover change runs in sequence so the record and the art never blend: the
/// record tucks behind the sleeve (or fades away when there is none), the art
/// fades, then the record slides out (or fades in when the art is gone).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Change {
    #[default]
    Idle,
    Tucking,
    Fading,
}

/// Room around a cover for its shadow, as a share of its side.
const SLEEVE_MARGIN: f32 = 0.12;

/// A cover pre-scaled with its shadow and rounded corners, so fades only blend
/// a finished image instead of re-filtering the source every frame.
struct Sleeve {
    source: gdk::Texture,
    side_pixels: i32,
    scale: i32,
    palette: Palette,
    rendered: gdk::Texture,
}

/// The record rendered once: a static body, and a label that only needs rotating.
struct DiscTextures {
    radius_pixels: i32,
    scale: i32,
    palette: Palette,
    body: gdk::Texture,
    label: gdk::Texture,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Artwork {
        pub(super) cover: RefCell<Option<gdk::Texture>>,
        pub(super) previous: RefCell<Option<Option<gdk::Texture>>>,
        pub(super) fade: Cell<f32>,
        pub(super) change: Cell<Change>,
        pub(super) incoming: RefCell<Option<Option<gdk::Texture>>>,
        pub(super) awaiting: Cell<bool>,
        pub(super) disc_opacity: Cell<f32>,
        /// Set while art appears or disappears, so the record stays out of the fade.
        pub(super) disc_hidden: Cell<bool>,
        /// Nothing shown yet: the first art fades in alone rather than after a
        /// placeholder record.
        pub(super) fresh: Cell<bool>,
        pub(super) playing: Cell<bool>,
        /// How far the record sits right of the cover, as a share of the cover's
        /// side, so it stays put however the art is resized.
        pub(super) disc_offset: Cell<Option<f32>>,
        pub(super) angle: Cell<f32>,
        pub(super) tick: RefCell<Option<gtk::TickCallbackId>>,
        pub(super) last_frame: Cell<Option<i64>>,
        pub(super) last_spin_draw: Cell<i64>,
        pub(super) disc: RefCell<Option<DiscTextures>>,
        pub(super) sleeves: RefCell<Vec<Sleeve>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Artwork {
        const NAME: &'static str = "StrataAudioArtwork";
        type Type = super::Artwork;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for Artwork {}

    impl WidgetImpl for Artwork {
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn map(&self) {
            self.parent_map();
            self.obj().wake();
        }

        fn unmap(&self) {
            if let Some(id) = self.tick.borrow_mut().take() {
                id.remove();
            }
            self.last_frame.set(None);
            self.parent_unmap();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct Artwork(ObjectSubclass<imp::Artwork>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Artwork {
    pub(super) fn new() -> Self {
        let artwork: Self = glib::Object::new();
        artwork.add_css_class("preview-audio-artwork");
        artwork.imp().fade.set(1.0);
        artwork.imp().fresh.set(true);
        artwork.update_property(&[gtk::accessible::Property::Label("Album art")]);
        follow_theme(&artwork);
        artwork
    }

    /// Keeps the record tucked while the next track's art loads, so it does not
    /// slide out only to tuck away again when the art arrives.
    pub(super) fn await_cover(&self) {
        self.imp().awaiting.set(true);
        self.wake();
    }

    pub(super) fn set_cover(&self, cover: Option<gdk::Texture>) {
        let imp = self.imp();
        imp.awaiting.set(false);
        let animated = crate::ui::motion::animations_enabled();
        if imp.fresh.replace(false) && animated {
            // Art fades in on its own, then the record slides out from behind
            // it; without art the record simply fades in.
            let art = cover.is_some();
            imp.disc_opacity.set(0.0);
            imp.cover.replace(cover);
            imp.previous.replace(art.then_some(None));
            imp.fade.set(if art { 0.0 } else { 1.0 });
            imp.change
                .set(if art { Change::Fading } else { Change::Idle });
            imp.disc_hidden.set(art);
            self.queue_draw();
            self.wake();
            return;
        }
        if !animated || !self.is_mapped() {
            imp.incoming.replace(None);
            imp.change.set(Change::Idle);
            imp.previous.replace(None);
            imp.fade.set(1.0);
            imp.disc_hidden.set(false);
            imp.disc_opacity.set(1.0);
            imp.cover.replace(cover);
            self.queue_draw();
            self.wake();
            return;
        }
        match imp.change.get() {
            Change::Fading => {
                if *imp.cover.borrow() != cover {
                    let previous = imp.cover.replace(cover);
                    imp.previous.replace(Some(previous));
                    imp.fade.set(0.0);
                }
            }
            Change::Idle if *imp.cover.borrow() == cover => {}
            Change::Idle | Change::Tucking => {
                imp.incoming.replace(Some(cover));
                imp.change.set(Change::Tucking);
            }
        }
        self.wake();
    }

    pub(super) fn set_playing(&self, playing: bool) {
        self.imp().playing.set(playing);
        self.wake();
    }

    fn cover_size(&self) -> f32 {
        (self.height() as f32).min(self.width() as f32 / ASPECT)
    }

    fn disc_target(&self) -> f32 {
        let imp = self.imp();
        // Without motion there is no tucking while art loads: the record stays
        // put, then appears where the new track places it.
        if imp.awaiting.get()
            && !crate::ui::motion::animations_enabled()
            && let Some(offset) = imp.disc_offset.get()
        {
            return offset;
        }
        let has_cover = imp.cover.borrow().is_some();
        let sleeve = has_cover
            || matches!(imp.incoming.borrow().as_ref(), Some(Some(_)))
            || matches!(imp.previous.borrow().as_ref(), Some(Some(_)));
        let changing = imp.awaiting.get() || imp.change.get() != Change::Idle;
        if changing && sleeve {
            return TUCKED;
        }
        if !has_cover {
            return (ASPECT - 1.0) / 2.0;
        }
        if imp.playing.get() { SLID_OUT } else { TUCKED }
    }

    fn disc_visible(&self) -> bool {
        let imp = self.imp();
        if imp.fresh.get() {
            return false;
        }
        match imp.change.get() {
            Change::Tucking => imp.cover.borrow().is_some(),
            Change::Fading => !imp.disc_hidden.get(),
            Change::Idle => true,
        }
    }

    fn begin_fade(&self) {
        let imp = self.imp();
        let Some(cover) = imp.incoming.take() else {
            imp.change.set(Change::Idle);
            return;
        };
        if *imp.cover.borrow() == cover {
            imp.change.set(Change::Idle);
            return;
        }
        let arriving = cover.is_some();
        let previous = imp.cover.replace(cover);
        if previous.is_some() != arriving {
            imp.disc_hidden.set(true);
            imp.disc_opacity.set(0.0);
        }
        imp.previous.replace(Some(previous));
        imp.fade.set(0.0);
        imp.change.set(Change::Fading);
    }

    fn wake(&self) {
        let imp = self.imp();
        if imp.tick.borrow().is_some() || !self.is_mapped() {
            return;
        }
        let id = self.add_tick_callback(|artwork, clock| {
            if artwork.advance(clock.frame_time()) {
                glib::ControlFlow::Continue
            } else {
                artwork.imp().tick.borrow_mut().take();
                artwork.imp().last_frame.set(None);
                glib::ControlFlow::Break
            }
        });
        imp.tick.replace(Some(id));
    }

    fn advance(&self, frame_time: i64) -> bool {
        let imp = self.imp();
        let elapsed = imp
            .last_frame
            .replace(Some(frame_time))
            .map_or(0.0, |last| (frame_time - last) as f32 / 1_000_000.0)
            .min(0.1);
        let animated = crate::ui::motion::animations_enabled();
        let size = self.cover_size().max(1.0);
        let has_cover = imp.cover.borrow().is_some();
        let visible = self.disc_visible();
        let previous_opacity = imp.disc_opacity.get();
        let step = if animated {
            elapsed / DISC_FADE_SECONDS
        } else {
            1.0
        };
        let opacity = if visible {
            (previous_opacity + step).min(1.0)
        } else {
            (previous_opacity - step).max(0.0)
        };
        imp.disc_opacity.set(opacity);

        let target = self.disc_target();
        let previous_offset = imp.disc_offset.get();
        let previous_fade = imp.fade.get();
        let rate = if imp.change.get() == Change::Idle && !imp.awaiting.get() {
            SLIDE_RATE
        } else {
            TUCK_RATE
        };
        let offset = if opacity <= 0.0 {
            // Hidden, it waits where it reappears; behind art that is tucked, so it
            // can then slide out.
            if has_cover { TUCKED } else { target }
        } else if !visible {
            previous_offset.unwrap_or(target)
        } else {
            let offset = previous_offset.unwrap_or(target);
            let moved = if animated {
                offset + (target - offset) * (1.0 - (-rate * elapsed).exp())
            } else {
                target
            };
            if (target - moved).abs() * size < 0.5 {
                target
            } else {
                moved
            }
        };
        imp.disc_offset.set(Some(offset));
        if imp.change.get() == Change::Tucking {
            let ready = if has_cover {
                (target - offset).abs() * size <= TUCK_SETTLED
            } else {
                opacity <= 0.0
            };
            if ready {
                self.begin_fade();
            }
        }
        let fade = if animated {
            (imp.fade.get() + elapsed / FADE_SECONDS).min(1.0)
        } else {
            1.0
        };
        imp.fade.set(fade);
        if fade >= 1.0 {
            imp.previous.replace(None);
            if imp.change.get() == Change::Fading {
                imp.change.set(Change::Idle);
                imp.disc_hidden.set(false);
            }
        }
        // With a cover the label is hidden behind it and grooves look the same at
        // any angle, so turning would only repaint the cover every frame.
        let spinning = animated && imp.playing.get() && imp.cover.borrow().is_none();
        let mut redraw =
            previous_offset != Some(offset) || previous_fade != fade || previous_opacity != opacity;
        if spinning {
            imp.angle
                .set((imp.angle.get() + TAU * elapsed / SECONDS_PER_TURN) % TAU);
            let interval = if self.software_rendered() {
                SOFTWARE_SPIN_INTERVAL_US
            } else {
                0
            };
            if frame_time - imp.last_spin_draw.get() >= interval {
                imp.last_spin_draw.set(frame_time);
                redraw = true;
            }
        }
        if redraw {
            self.queue_draw();
        }
        // A phase may have just ended, so judge the remaining motion afresh.
        spinning
            || offset != self.disc_target()
            || fade < 1.0
            || opacity != if self.disc_visible() { 1.0 } else { 0.0 }
            || imp.change.get() != Change::Idle
            || imp.awaiting.get()
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let (width, height) = (self.width() as f32, self.height() as f32);
        let size = height.min(width / ASPECT);
        if size < 8.0 {
            return;
        }
        let colors = palette();
        let left = (width - size * ASPECT) / 2.0;
        let top = (height - size) / 2.0;
        let offset = imp.disc_offset.get().unwrap_or_else(|| self.disc_target()) * size;
        let disc_opacity = imp.disc_opacity.get();
        if disc_opacity > 0.0 {
            snapshot.push_opacity(f64::from(disc_opacity));
            self.draw_disc(
                snapshot,
                &colors,
                graphene::Point::new(left + size / 2.0 + offset, top + size / 2.0),
                size * DISC / 2.0,
            );
            snapshot.pop();
        }
        let frame = graphene::Rect::new(left, top, size, size);
        let fade = imp.fade.get();
        if let Some(Some(texture)) = imp.previous.borrow().as_ref() {
            // Under an incoming cover the old one stays opaque so the crossfade
            // never dips; with nothing incoming it fades out to the bare record.
            let opacity = if imp.cover.borrow().is_some() {
                1.0
            } else {
                1.0 - fade
            };
            self.draw_cover(snapshot, &colors, texture, &frame, opacity);
        }
        if let Some(texture) = imp.cover.borrow().as_ref() {
            self.draw_cover(snapshot, &colors, texture, &frame, fade);
        }
    }

    fn draw_cover(
        &self,
        snapshot: &gtk::Snapshot,
        colors: &Palette,
        texture: &gdk::Texture,
        frame: &graphene::Rect,
        opacity: f32,
    ) {
        if opacity <= 0.0 {
            return;
        }
        snapshot.push_opacity(f64::from(opacity));
        let size = frame.width();
        let reach = size * SLEEVE_MARGIN;
        if let Some(sleeve) = self.sleeve(colors, texture, size) {
            snapshot.append_texture(
                &sleeve,
                &graphene::Rect::new(
                    frame.x() - reach,
                    frame.y() - reach,
                    size + reach * 2.0,
                    size + reach * 2.0,
                ),
            );
        } else {
            paint_cover(snapshot, colors, texture, frame);
        }
        snapshot.pop();
    }

    fn sleeve(&self, colors: &Palette, texture: &gdk::Texture, size: f32) -> Option<gdk::Texture> {
        let imp = self.imp();
        let scale = self.scale_factor();
        let side_pixels = (size * scale as f32).round() as i32;
        if let Some(sleeve) = imp.sleeves.borrow().iter().find(|sleeve| {
            sleeve.source == *texture
                && sleeve.side_pixels == side_pixels
                && sleeve.scale == scale
                && sleeve.palette == *colors
        }) {
            return Some(sleeve.rendered.clone());
        }
        let renderer = self.native()?.renderer()?;
        let reach = size * SLEEVE_MARGIN;
        let rendered = render(&renderer, size + reach * 2.0, scale, |snapshot| {
            paint_cover(
                snapshot,
                colors,
                texture,
                &graphene::Rect::new(reach, reach, size, size),
            );
        })?;
        let current = imp.cover.borrow().clone();
        let previous = imp.previous.borrow().clone().flatten();
        let mut sleeves = imp.sleeves.borrow_mut();
        // Only the covers on screen are worth keeping.
        sleeves.retain(|sleeve| {
            Some(&sleeve.source) == current.as_ref() || Some(&sleeve.source) == previous.as_ref()
        });
        sleeves.retain(|sleeve| sleeve.source != *texture);
        sleeves.push(Sleeve {
            source: texture.clone(),
            side_pixels,
            scale,
            palette: *colors,
            rendered: rendered.clone(),
        });
        Some(rendered)
    }

    fn software_rendered(&self) -> bool {
        self.native()
            .and_then(|native| native.renderer())
            .is_some_and(|renderer| renderer.is::<gsk::CairoRenderer>())
    }

    fn draw_disc(
        &self,
        snapshot: &gtk::Snapshot,
        colors: &Palette,
        center: graphene::Point,
        radius: f32,
    ) {
        let angle = self.imp().angle.get().to_degrees();
        let label_radius = radius * LABEL;
        let Some((body, label)) = self.disc_textures(colors, radius) else {
            paint_disc(snapshot, colors, center, radius);
            snapshot.save();
            snapshot.translate(&center);
            snapshot.rotate(angle);
            paint_label(snapshot, colors, radius);
            snapshot.restore();
            return;
        };
        let reach = radius * (1.0 + SHADOW_MARGIN);
        snapshot.append_texture(
            &body,
            &graphene::Rect::new(
                center.x() - reach,
                center.y() - reach,
                reach * 2.0,
                reach * 2.0,
            ),
        );
        snapshot.save();
        snapshot.translate(&center);
        snapshot.rotate(angle);
        snapshot.append_texture(
            &label,
            &graphene::Rect::new(
                -label_radius,
                -label_radius,
                label_radius * 2.0,
                label_radius * 2.0,
            ),
        );
        snapshot.restore();
    }

    /// Renders the record at this size, scale and theme once; spinning then only
    /// rotates the small label texture.
    fn disc_textures(&self, colors: &Palette, radius: f32) -> Option<(gdk::Texture, gdk::Texture)> {
        let imp = self.imp();
        let scale = self.scale_factor();
        let radius_pixels = (radius * scale as f32).round() as i32;
        if let Some(disc) = imp.disc.borrow().as_ref().filter(|disc| {
            disc.radius_pixels == radius_pixels && disc.scale == scale && disc.palette == *colors
        }) {
            return Some((disc.body.clone(), disc.label.clone()));
        }
        let renderer = self.native()?.renderer()?;
        let reach = radius * (1.0 + SHADOW_MARGIN);
        let body = render(&renderer, reach * 2.0, scale, |snapshot| {
            paint_disc(snapshot, colors, graphene::Point::new(reach, reach), radius);
        })?;
        let label_radius = radius * LABEL;
        let label = render(&renderer, label_radius * 2.0, scale, |snapshot| {
            snapshot.translate(&graphene::Point::new(label_radius, label_radius));
            paint_label(snapshot, colors, radius);
        })?;
        imp.disc.replace(Some(DiscTextures {
            radius_pixels,
            scale,
            palette: *colors,
            body: body.clone(),
            label: label.clone(),
        }));
        Some((body, label))
    }
}

fn render(
    renderer: &gsk::Renderer,
    side: f32,
    scale: i32,
    paint: impl FnOnce(&gtk::Snapshot),
) -> Option<gdk::Texture> {
    let snapshot = gtk::Snapshot::new();
    snapshot.scale(scale as f32, scale as f32);
    paint(&snapshot);
    let pixels = (side * scale as f32).ceil();
    Some(renderer.render_texture(
        snapshot.to_node()?,
        Some(&graphene::Rect::new(0.0, 0.0, pixels, pixels)),
    ))
}

/// The darker and lighter of the theme's background and text. The record is
/// dark in every theme, so its grooves need the lighter one to show; in light
/// themes the text is the dark colour.
fn shades(colors: &Palette) -> (gdk::RGBA, gdk::RGBA) {
    let luminance =
        |color: gdk::RGBA| color.red() * 0.2126 + color.green() * 0.7152 + color.blue() * 0.0722;
    if luminance(colors.background) <= luminance(colors.text) {
        (colors.background, colors.text)
    } else {
        (colors.text, colors.background)
    }
}

fn paint_cover(
    snapshot: &gtk::Snapshot,
    colors: &Palette,
    texture: &gdk::Texture,
    frame: &graphene::Rect,
) {
    let size = frame.width();
    let rounded = gsk::RoundedRect::from_rect(*frame, (size * 0.035).clamp(4.0, 14.0));
    snapshot.append_outset_shadow(
        &rounded,
        &with_alpha(shades(colors).0, 0.55),
        0.0,
        size * 0.03,
        0.0,
        size * 0.07,
    );
    // Fill the square like a record sleeve: crop rather than letterbox odd shapes.
    let (texture_width, texture_height) = (texture.width() as f32, texture.height() as f32);
    let scale = (size / texture_width).max(size / texture_height);
    let (drawn_width, drawn_height) = (texture_width * scale, texture_height * scale);
    snapshot.push_rounded_clip(&rounded);
    snapshot.append_scaled_texture(
        texture,
        gsk::ScalingFilter::Trilinear,
        &graphene::Rect::new(
            frame.x() + (size - drawn_width) / 2.0,
            frame.y() + (size - drawn_height) / 2.0,
            drawn_width,
            drawn_height,
        ),
    );
    snapshot.pop();
    snapshot.append_border(&rounded, &[1.0; 4], &[with_alpha(colors.text, 0.08); 4]);
}

fn circle(center: graphene::Point, radius: f32) -> gsk::RoundedRect {
    gsk::RoundedRect::from_rect(
        graphene::Rect::new(
            center.x() - radius,
            center.y() - radius,
            radius * 2.0,
            radius * 2.0,
        ),
        radius,
    )
}

fn paint_disc(snapshot: &gtk::Snapshot, colors: &Palette, center: graphene::Point, radius: f32) {
    let (dark, light) = shades(colors);
    let body = mix(dark, light, 0.06);
    let outline = circle(center, radius);
    snapshot.append_outset_shadow(
        &outline,
        &with_alpha(dark, 0.5),
        0.0,
        radius * 0.04,
        0.0,
        radius * 0.12,
    );
    snapshot.push_rounded_clip(&outline);
    snapshot.append_color(&body, outline.bounds());
    let groove = with_alpha(light, 0.05);
    let mut ring = 0.42;
    while ring < 0.97 {
        snapshot.append_border(&circle(center, radius * ring), &[1.0; 4], &[groove; 4]);
        ring += 0.055;
    }
    // The light catching the grooves stays put while the record turns under it.
    let glint = with_alpha(light, 0.09);
    let clear = with_alpha(light, 0.0);
    snapshot.append_conic_gradient(
        outline.bounds(),
        &center,
        30.0,
        &[
            gsk::ColorStop::new(0.0, clear),
            gsk::ColorStop::new(0.08, glint),
            gsk::ColorStop::new(0.16, clear),
            gsk::ColorStop::new(0.5, clear),
            gsk::ColorStop::new(0.58, glint),
            gsk::ColorStop::new(0.66, clear),
            gsk::ColorStop::new(1.0, clear),
        ],
    );
    snapshot.pop();
    snapshot.append_border(&outline, &[1.0; 4], &[with_alpha(light, 0.14); 4]);
}

/// Paints the label around the origin; the stripe makes its turning visible.
fn paint_label(snapshot: &gtk::Snapshot, colors: &Palette, radius: f32) {
    let label = radius * LABEL;
    let origin = graphene::Point::new(0.0, 0.0);
    let label_circle = circle(origin, label);
    snapshot.push_rounded_clip(&label_circle);
    snapshot.append_color(&colors.accent, label_circle.bounds());
    snapshot.append_color(
        &colors.accent_bright,
        &graphene::Rect::new(-label, -label * 0.12, label * 2.0, label * 0.24),
    );
    snapshot.pop();
    snapshot.append_border(
        &circle(origin, label * 0.72),
        &[1.0; 4],
        &[with_alpha(colors.background, 0.25); 4],
    );
    let hole = circle(origin, (radius * 0.035).max(1.5));
    snapshot.push_rounded_clip(&hole);
    snapshot.append_color(&colors.background, hole.bounds());
    snapshot.pop();
}
