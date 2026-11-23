// SPDX-License-Identifier: MIT

mod indicator;
mod state;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cursor, Cx, El, Pixels, Tx, col, row, text};
use rune_kit::{IconButton, IconButtonVariant};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::contract::{Dnd, DragSource};
use crate::viewers::layout::{self, Areas};
use crate::viewers::viewport::{Interaction, Viewport, ViewportState};

pub use indicator::{SIZE as INDICATOR_SIZE, indicator};
pub use state::{CameraView, Encoding, Mesh, ModelFile, ModelKind, ModelLabels, ModelState, Stage};

/// How far the last picture fades while another view renders (Strata's 0.3).
const RENDERING_OPACITY: f32 = 0.3;

type CameraFn<S> = Rc<dyn Fn(&mut S, CameraView, &mut Tx)>;

pub struct ModelView<'a, S> {
    state: &'a ModelState,
    access: fn(&mut S) -> &mut ModelState,
    viewport: fn(&mut S) -> &mut ViewportState,
    key: String,
    pixels: Option<&'a Pixels>,
    on_camera: Option<CameraFn<S>>,
    drag: Option<(Dnd<S>, DragSource<S>)>,
}

impl<'a, S: 'static> ModelView<'a, S> {
    pub fn new(
        state: &'a ModelState,
        access: fn(&mut S) -> &mut ModelState,
        viewport: fn(&mut S) -> &mut ViewportState,
    ) -> Self {
        ModelView {
            state,
            access,
            viewport,
            key: "model".into(),
            pixels: None,
            on_camera: None,
            drag: None,
        }
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    /// The rendered picture.
    pub fn pixels(mut self, pixels: &'a Pixels) -> Self {
        self.pixels = Some(pixels);
        self
    }

    /// Called when a camera button turned the camera: render this view.
    pub fn on_camera(mut self, f: impl Fn(&mut S, CameraView, &mut Tx) + 'static) -> Self {
        self.on_camera = Some(Rc::new(f));
        self
    }

    /// Dragging the picture drags the file.
    pub fn drag_source(mut self, dnd: Dnd<S>, source: DragSource<S>) -> Self {
        self.drag = Some((dnd, source));
        self
    }

    pub fn build(self, look: &Look, cx: &mut Cx<S>) -> El {
        let ModelView {
            state: st,
            access,
            viewport,
            key,
            pixels,
            on_camera,
            drag,
        } = self;
        let k = &look.kit;
        let labels = st.labels();
        if let Some(message) = st.failed() {
            return failure(&key, &labels.unavailable, message, look);
        }

        let mut picture = Viewport::new(&st.viewport, viewport)
            .key(format!("{key}-picture"))
            .label(labels.picture.clone())
            .interaction(Interaction::Fixed);
        if let Some(px) = pixels.filter(|_| st.has_image()) {
            picture = picture.pixels(px);
        }
        if let Some((dnd, source)) = drag {
            picture = picture.drag_source(dnd, source);
        }
        let rendering = st.rendering();
        // Rune has no wait cursor (draft R-C14b-2, module docs): the default stands in for it.
        let picture = col()
            .w_full()
            .h_full()
            .cursor(if rendering {
                Cursor::Default
            } else {
                Cursor::Grab
            })
            .opacity(if rendering { RENDERING_OPACITY } else { 1.0 })
            .child(picture.build(look, cx).busy(st.busy().is_some()));

        if st.plain() {
            let aspect = st
                .viewport
                .content()
                .map_or(1.0, |[w, h]| if h > 0.0 { w / h } else { 1.0 });
            return col()
                .key(key)
                .w_full()
                .h_full()
                .min_h(0.0)
                .padding(look.space.lg)
                .center()
                .child(
                    picture
                        .aspect_fit(aspect)
                        .max_h(layout::MAX_CONTENT_WIDTH / aspect.max(f32::EPSILON)),
                );
        }

        let file = st.file();
        let details =
            layout::details_header(&key, file.position.map(labels.position), &file.name, look);
        let mut badges = vec![layout::badge(
            format!("{key}-format"),
            st.format_badge(),
            look,
        )];
        if let Some(triangles) = st.triangle_badge() {
            badges.push(layout::badge(format!("{key}-triangles"), triangles, look));
        }
        let badges = layout::badge_row(&key, badges, look);

        let accent_kit = look.accent_icon_kit();
        let button = |name: &str, icon: IconName, step: i32| {
            let on_camera = on_camera.clone();
            IconButton::new(look.icon(icon))
                .variant(IconButtonVariant::Ghost)
                .label(name.to_owned())
                .help(name.to_owned())
                .disabled(!st.can_step())
                .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                    if let Some(view) = access(s).step(step)
                        && let Some(f) = &on_camera
                    {
                        f(s, view, tx);
                    }
                })
                .build(&accent_kit)
                // The tooltip says what the name says: no description, so it isn't read twice.
                .description("")
        };
        let buttons = row()
            .gap(look.space.xl)
            .center_y()
            .child(button(&labels.previous, IconName::SkipBack, -1).key(format!("{key}-previous")))
            .child(indicator(
                format!("{key}-camera"),
                &labels.camera,
                (labels.view)(st.camera()),
                st.angle(),
                !st.fresh(),
                look,
            ))
            .child(button(&labels.next, IconName::SkipForward, 1).key(format!("{key}-next")));
        let mut controls = col()
            .key(format!("{key}-controls"))
            .gap(look.space.lg)
            .items_center()
            .child(buttons);
        if let Some(error) = st.error() {
            controls = controls.child(
                text(error.to_owned())
                    .key(format!("{key}-error"))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .wrap()
                    .align(rune::Align::Center),
            );
        }
        layout::section(
            key,
            Areas {
                content: picture,
                aspect: 1.0,
                details,
                badges: Some(badges),
                timeline: None,
                controls,
            },
            look,
        )
    }
}

/// A first load that failed: Strata's `show_message("Preview unavailable", error)`.
pub(crate) fn failure(key: &str, title: &str, message: &str, look: &Look) -> El {
    let k = &look.kit;
    col()
        .key(key.to_owned())
        .w_full()
        .h_full()
        .padding(look.space.xl)
        .gap(look.space.sm)
        .center()
        .child(
            text(title.to_owned())
                .key(format!("{key}-unavailable"))
                .font(look.ty.title)
                .color(k.c.text_base),
        )
        .child(
            text(message.to_owned())
                .key(format!("{key}-message"))
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .wrap()
                .align(rune::Align::Center),
        )
}
