// SPDX-License-Identifier: MIT

mod checker;
mod state;
pub mod zoom;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cursor, Cx, Drag, El, KeyEvent, MagnifyEvent, Pixels, SurfaceFit, Tx, WheelEvent, el};
use strata_kit::look::Look;

use crate::contract::{Dnd, DragSource};

pub use checker::checkerboard;
pub use state::ViewportState;
pub use zoom::{
    Mode, ZoomKey, ZoomLimits, ZoomReport, ctrl_zero_key, default_steps, no_keys, zoom_key,
};

/// How much input the viewport takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Interaction {
    /// Zoom, pan and the keys; focusable.
    #[default]
    Full,
    /// Shows the content in its mode and takes no input at all.
    Fixed,
}

/// When a primary drag pans (with [`Interaction::Full`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PanDrag {
    #[default]
    WhenOverflowing,
    /// Always: a drag never drags the file.
    Always,
    /// Never: drags are the host's; the wheel and the arrows still pan.
    Never,
}

/// The text the viewport shows assistive technology.
#[derive(Clone, Copy, Debug)]
pub struct Labels {
    /// The accessible value for a level (device pixels per content pixel): "56%".
    pub level: fn(f32) -> String,
}

impl Default for Labels {
    fn default() -> Labels {
        Labels {
            level: |level| format!("{:.0}%", level * 100.0),
        }
    }
}

/// Where content pixels land inside the content element, for [`Viewport::overlay`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentMap {
    /// Layout points per content pixel at the current level.
    pub points_per_px: f32,
    /// The content's drawn size in points.
    pub size: [f32; 2],
}

impl ContentMap {
    /// The point for content pixel `px`, from the content's top-left corner.
    pub fn point(&self, px: [f32; 2]) -> [f32; 2] {
        [px[0] * self.points_per_px, px[1] * self.points_per_px]
    }

    /// The rect `[x, y, w, h]` for a rect in content pixels.
    pub fn rect(&self, px: [f32; 4]) -> [f32; 4] {
        let k = self.points_per_px;
        [px[0] * k, px[1] * k, px[2] * k, px[3] * k]
    }
}

type ZoomFn<S> = Rc<dyn Fn(&mut S, ZoomReport, &mut Tx)>;
type OverlayFn<'a> = Box<dyn FnOnce(&ContentMap) -> El + 'a>;

pub struct Viewport<'a, S> {
    state: &'a ViewportState,
    access: fn(&mut S) -> &mut ViewportState,
    key: String,
    pixels: Option<&'a Pixels>,
    label: Option<String>,
    transparent: bool,
    interaction: Interaction,
    pan_drag: PanDrag,
    overlay: Option<OverlayFn<'a>>,
    on_zoom: Option<ZoomFn<S>>,
    labels: Labels,
    keys: fn(&KeyEvent) -> Option<ZoomKey>,
    drag: Option<(Dnd<S>, DragSource<S>)>,
}

impl<'a, S: 'static> Viewport<'a, S> {
    /// The viewport for `state`, which `access` finds in the app's state.
    pub fn new(state: &'a ViewportState, access: fn(&mut S) -> &mut ViewportState) -> Self {
        Viewport {
            state,
            access,
            key: "viewport".into(),
            pixels: None,
            label: None,
            transparent: false,
            interaction: Interaction::Full,
            pan_drag: PanDrag::WhenOverflowing,
            overlay: None,
            on_zoom: None,
            labels: Labels::default(),
            keys: zoom_key,
            drag: None,
        }
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    pub fn pixels(mut self, pixels: &'a Pixels) -> Self {
        self.pixels = Some(pixels);
        self
    }

    /// The image's accessible name: a file name, "Route preview".
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn transparent(mut self, on: bool) -> Self {
        self.transparent = on;
        self
    }

    pub fn interaction(mut self, interaction: Interaction) -> Self {
        self.interaction = interaction;
        self
    }

    pub fn pan_drag(mut self, pan_drag: PanDrag) -> Self {
        self.pan_drag = pan_drag;
        self
    }

    pub fn overlay(mut self, f: impl FnOnce(&ContentMap) -> El + 'a) -> Self {
        self.overlay = Some(Box::new(f));
        self
    }

    /// Called when the level changes, with where the content now draws.
    pub fn on_zoom(mut self, f: impl Fn(&mut S, ZoomReport, &mut Tx) + 'static) -> Self {
        self.on_zoom = Some(Rc::new(f));
        self
    }

    pub fn labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }

    /// Which keys zoom, reset and pan while the viewport has focus ([`zoom_key`] unless set).
    pub fn zoom_keys(mut self, keys: fn(&KeyEvent) -> Option<ZoomKey>) -> Self {
        self.keys = keys;
        self
    }

    pub fn drag_source(mut self, dnd: Dnd<S>, source: DragSource<S>) -> Self {
        self.drag = Some((dnd, source));
        self
    }

    pub fn build(self, look: &Look, cx: &mut Cx<S>) -> El {
        let Viewport {
            state: st,
            access,
            key,
            pixels,
            label,
            transparent,
            interaction,
            pan_drag,
            overlay,
            on_zoom,
            labels,
            keys,
            drag,
        } = self;
        // Handlers are built here, so each one carries the scale this frame draws at.
        let scale = cx.env().scale;
        let full = interaction == Interaction::Full;
        let placed = match (st.frame(), st.content(), st.level_at(scale)) {
            (Some(frame), Some(content), Some(level)) => Some((frame, content, level)),
            _ => None,
        };

        let mut frame = el()
            .key(key.clone())
            .w_full()
            .h_full()
            .min_w(0.0)
            .min_h(0.0)
            .clip_soft(0.0)
            .instant_changes()
            // Centres the stand-in drawn before the first layout; the rest is pinned.
            .center()
            .role("image");
        if let Some(label) = label {
            frame = frame.label(label);
        }
        {
            let on_zoom = on_zoom.clone();
            frame = frame.on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
                let st = access(s);
                let size = [r[2], r[3]];
                if st.frame() == Some(size) && st.scale() == scale {
                    // Only moved (a scroll or a sibling around it): nothing the view reads.
                    tx.live_only();
                    return;
                }
                let report = st.set_frame(size, scale);
                emit(&on_zoom, s, report, tx);
            });
        }

        // Rune fires `on_layout` only when a rect changes in points, not on a display-scale-only
        // change; mount a scale-keyed probe element to trigger `sync_scale`.
        if st.frame().is_some() && st.scale() != scale {
            let on_zoom = on_zoom.clone();
            frame = frame.child(
                el().key(format!("{key}-rescale-{:08x}", scale.to_bits()))
                    .role("presentation")
                    .pin(Some(0.0), Some(0.0), None, None)
                    .size(0.0, 0.0)
                    .on_layout_tx(move |s: &mut S, _r: [f32; 4], tx: &mut Tx| {
                        let report = access(s).sync_scale(scale);
                        emit(&on_zoom, s, report, tx);
                    }),
            );
        }

        match placed {
            Some((frame_size, content, level)) => {
                let drawn = zoom::drawn_size(content, level, scale);
                let visible = [drawn[0].min(frame_size[0]), drawn[1].min(frame_size[1])];
                if transparent && visible[0] > 0.0 && visible[1] > 0.0 {
                    frame = frame.child(checkerboard(format!("{key}-checker"), visible, look).pin(
                        Some((frame_size[0] - visible[0]) / 2.0),
                        Some((frame_size[1] - visible[1]) / 2.0),
                        None,
                        None,
                    ));
                }
                let map = ContentMap {
                    points_per_px: level / scale,
                    size: drawn,
                };
                let mut body = el()
                    .key(format!("{key}-content"))
                    .role("presentation")
                    .pin(
                        Some((frame_size[0] - drawn[0]) / 2.0),
                        Some((frame_size[1] - drawn[1]) / 2.0),
                        None,
                        None,
                    )
                    .size(drawn[0], drawn[1])
                    .live_offset(&st.live_pan);
                if let Some(px) = pixels {
                    body = body.pixels_fit(px, SurfaceFit::Fit);
                }
                if let Some(overlay) = overlay {
                    body = body.child(
                        el().key(format!("{key}-overlay"))
                            .role("presentation")
                            .pin(Some(0.0), Some(0.0), None, None)
                            .size(drawn[0], drawn[1])
                            .child(overlay(&map)),
                    );
                }
                frame = frame.child(body);
                if full {
                    frame = frame.a11y_value((labels.level)(level));
                }
            }
            None if matches!(st.mode(), Mode::Fit | Mode::ScaleDown) => {
                if let (Some(px), Some(content)) = (pixels, st.content()) {
                    let mut stand_in = el()
                        .key(format!("{key}-content"))
                        .role("presentation")
                        .w_full()
                        .h_full()
                        .pixels_fit(px, SurfaceFit::Fit);
                    if st.mode() == Mode::ScaleDown {
                        stand_in = stand_in.max_w(content[0] / scale).max_h(content[1] / scale);
                    }
                    frame = frame.child(stand_in);
                }
            }
            None => {}
        }

        let overflow = st.overflow();
        let pans = full
            && match pan_drag {
                PanDrag::Always => true,
                PanDrag::WhenOverflowing => overflow[0] || overflow[1],
                PanDrag::Never => false,
            };
        if !pans && let Some((dnd, source)) = drag {
            frame = dnd.drag_source_keyed(frame, key.clone(), source);
        }
        if !full {
            return frame;
        }
        frame = frame
            .focusable(true)
            .no_focus_ring()
            .on_key(key_handler(
                access,
                on_zoom.clone(),
                keys,
                scale,
                look.space.xxl,
            ))
            .on_wheel(wheel_handler(access, on_zoom.clone(), scale));
        let on_zoom_drag = on_zoom.clone();
        if cfg!(target_os = "macos") {
            frame = frame.on_magnify(magnify_handler(access, on_zoom, scale));
        }
        if pans {
            let on_zoom = on_zoom_drag;
            frame =
                frame
                    .cursor(Cursor::Grab)
                    .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
                        let rescaled = access(s).sync_scale(scale);
                        let commits = rescaled.is_some();
                        emit(&on_zoom, s, rescaled, tx);
                        access(s).drag(d);
                        if !commits {
                            tx.live_only();
                        }
                    });
        }
        frame
    }
}

fn emit<S>(f: &Option<ZoomFn<S>>, s: &mut S, report: Option<ZoomReport>, tx: &mut Tx) {
    if let (Some(f), Some(report)) = (f, report) {
        f(s, report, tx);
    }
}

fn key_handler<S: 'static>(
    access: fn(&mut S) -> &mut ViewportState,
    on_zoom: Option<ZoomFn<S>>,
    keys: fn(&KeyEvent) -> Option<ZoomKey>,
    scale: f32,
    step: f32,
) -> impl Fn(&mut S, &KeyEvent, &mut Tx) -> bool + 'static {
    move |s: &mut S, e: &KeyEvent, tx: &mut Tx| {
        let Some(k) = keys(e) else { return false };
        let st = access(s);
        let rescaled = st.sync_scale(scale);
        if st.level().is_none() {
            return false;
        }
        let take_anyway = rescaled.is_some();
        // A new scale changed the level: this event commits, whatever it does next.
        let rescaled = rescaled.is_some_and(|r| {
            emit(&on_zoom, s, Some(r), tx);
            true
        });
        let st = access(s);
        let report = match k {
            ZoomKey::In => st.zoom_step(1),
            ZoomKey::Out => st.zoom_step(-1),
            ZoomKey::Reset => st.reset(),
            ZoomKey::Pan(x, y) => {
                let o = st.overflow();
                let along = |on: bool, dir: i8| if on { -f32::from(dir) * step } else { 0.0 };
                let d = [along(o[0], x), along(o[1], y)];
                // At the edge already, the key passes on (to the listing, say).
                if !st.pan_by(d) {
                    return take_anyway;
                }
                if !rescaled {
                    tx.live_only();
                }
                return true;
            }
        };
        emit(&on_zoom, s, report, tx);
        true
    }
}

fn wheel_handler<S: 'static>(
    access: fn(&mut S) -> &mut ViewportState,
    on_zoom: Option<ZoomFn<S>>,
    scale: f32,
) -> impl Fn(&mut S, WheelEvent, &mut Tx) -> bool + 'static {
    move |s: &mut S, e: WheelEvent, tx: &mut Tx| {
        let st = access(s);
        let rescaled = st.sync_scale(scale);
        if st.level().is_none() {
            return false;
        }
        // A new scale changed the level: this event commits, whatever it does next.
        let rescaled = rescaled.is_some_and(|r| {
            emit(&on_zoom, s, Some(r), tx);
            true
        });
        let st = access(s);
        if e.mods.ctrl {
            // Ctrl+Shift+wheel is Strata's thumbnail size; Ctrl+Alt is nobody's here.
            if e.mods.shift || e.mods.alt {
                return false;
            }
            let report = st.zoom_by(zoom::wheel_factor(e.dy), Some([e.x, e.y]));
            emit(&on_zoom, s, report, tx);
            return true;
        }
        let o = st.overflow();
        let d = [if o[0] { e.dx } else { 0.0 }, if o[1] { e.dy } else { 0.0 }];
        if d == [0.0, 0.0] {
            // Nothing to pan that way: the scroll view around the viewport scrolls.
            return false;
        }
        st.pan_by(d);
        if !rescaled {
            tx.live_only();
        }
        true
    }
}

fn magnify_handler<S: 'static>(
    access: fn(&mut S) -> &mut ViewportState,
    on_zoom: Option<ZoomFn<S>>,
    scale: f32,
) -> impl Fn(&mut S, MagnifyEvent, &mut Tx) -> bool + 'static {
    move |s: &mut S, e: MagnifyEvent, tx: &mut Tx| {
        let st = access(s);
        let rescaled = st.sync_scale(scale);
        if st.level().is_none() {
            return false;
        }
        emit(&on_zoom, s, rescaled, tx);
        let st = access(s);
        let report = st.zoom_by(zoom::magnify_factor(e.delta), Some([e.x, e.y]));
        emit(&on_zoom, s, report, tx);
        true
    }
}
