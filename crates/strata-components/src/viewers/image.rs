//! The image view: C14a's viewport (zoom and pan, the checkerboard behind transparent images)
//! with the image's name and its dimensions under it.
//!
//! ```ignore
//! ImageView::new(&s.image, |s: &mut App| &mut s.image)
//!     .pixels(&s.photo)
//!     .name("harbour.png")
//!     .size_px([1200, 800])
//!     .build(&s.look, cx)
//! ```
//!
//! **New vs Strata** (coordinator ruling): Strata's image preview shows the picture alone. The
//! work package asks for the name and dimensions, so a caption shows them ("harbour.png",
//! "1200 × 800"), and the picture's accessible description reads the dimensions too.
//!
//! The host gives the size in pixels with [`ImageView::size_px`] and sets the content on its
//! [`ViewportState`] itself (C14a: the viewport never reads the `Pixels`' own size).

use rune::{Cx, El, Pixels, Tx, col, text};
use strata_kit::look::Look;

use crate::contract::{Dnd, DragSource};
use crate::viewers::viewport::{Viewport, ViewportState, ZoomReport};

/// The words the image view shows and reads.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ImageLabels {
    /// "1200 × 800".
    pub dimensions: fn([u32; 2]) -> String,
}

impl Default for ImageLabels {
    fn default() -> Self {
        ImageLabels {
            dimensions: |[w, h]| format!("{w} × {h}"),
        }
    }
}

type ZoomFn<S> = Box<dyn Fn(&mut S, ZoomReport, &mut Tx)>;

/// The image view: `ImageView::new(&s.image, access) … .build(look, cx)`. It fills its parent.
pub struct ImageView<'a, S> {
    state: &'a ViewportState,
    access: fn(&mut S) -> &mut ViewportState,
    key: String,
    pixels: Option<&'a Pixels>,
    name: String,
    size_px: Option<[u32; 2]>,
    transparent: bool,
    labels: ImageLabels,
    on_zoom: Option<ZoomFn<S>>,
    drag: Option<(Dnd<S>, DragSource<S>)>,
}

impl<'a, S: 'static> ImageView<'a, S> {
    pub fn new(state: &'a ViewportState, access: fn(&mut S) -> &mut ViewportState) -> Self {
        ImageView {
            state,
            access,
            key: "image".into(),
            pixels: None,
            name: String::new(),
            size_px: None,
            transparent: true,
            labels: ImageLabels::default(),
            on_zoom: None,
            drag: None,
        }
    }

    /// The view's key ("image" unless set): the viewport is `<key>-picture`, the caption's
    /// lines `<key>-name` and `<key>-dimensions`.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    pub fn pixels(mut self, pixels: &'a Pixels) -> Self {
        self.pixels = Some(pixels);
        self
    }

    /// The file's display name: the caption, and the picture's accessible name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The image's size in pixels, as the decoder reported it.
    pub fn size_px(mut self, size: [u32; 2]) -> Self {
        self.size_px = Some(size);
        self
    }

    /// Whether a checkerboard shows behind the image (on unless set).
    pub fn transparent(mut self, on: bool) -> Self {
        self.transparent = on;
        self
    }

    pub fn labels(mut self, labels: ImageLabels) -> Self {
        self.labels = labels;
        self
    }

    /// As [`Viewport::on_zoom`]: a sharper render for the new level, say.
    pub fn on_zoom(mut self, f: impl Fn(&mut S, ZoomReport, &mut Tx) + 'static) -> Self {
        self.on_zoom = Some(Box::new(f));
        self
    }

    /// As [`Viewport::drag_source`].
    pub fn drag_source(mut self, dnd: Dnd<S>, source: DragSource<S>) -> Self {
        self.drag = Some((dnd, source));
        self
    }

    pub fn build(self, look: &Look, cx: &mut Cx<S>) -> El {
        let k = &look.kit;
        let dimensions = self.size_px.map(self.labels.dimensions);
        let mut viewport = Viewport::new(self.state, self.access)
            .key(format!("{}-picture", self.key))
            .label(self.name.clone())
            .transparent(self.transparent);
        if let Some(px) = self.pixels {
            viewport = viewport.pixels(px);
        }
        if let Some(f) = self.on_zoom {
            viewport = viewport.on_zoom(f);
        }
        if let Some((dnd, source)) = self.drag {
            viewport = viewport.drag_source(dnd, source);
        }
        let mut picture = viewport.build(look, cx);
        if let Some(d) = &dimensions {
            picture = picture.description(d.clone());
        }
        let mut caption = col()
            .key(format!("{}-caption", self.key))
            .w_full()
            .px(look.space.xl)
            .py(look.space.md)
            .gap(look.space.xxs)
            .items_center()
            .child(
                text(self.name)
                    .key(format!("{}-name", self.key))
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base)
                    .truncate_middle()
                    .a11y_hidden(),
            );
        if let Some(d) = dimensions {
            caption = caption.child(
                text(d)
                    .key(format!("{}-dimensions", self.key))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .a11y_hidden(),
            );
        }
        col()
            .key(self.key)
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(
                col()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .padding(look.space.lg)
                    .child(picture),
            )
            .child(caption)
    }
}
