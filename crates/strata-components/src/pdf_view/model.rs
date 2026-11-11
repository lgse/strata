//! The data the host hands the view and the requests it asks back: the document's pages and
//! sizes, a page's text layer, a render's reply, and the renderer's limits.

use std::rc::Rc;

use super::layer::Layer;

/// The most pages a document shows, as Strata's PDF view clamps them (`render_pdf_viewer`).
pub const MAX_PAGES: usize = 10_000;

/// A page's text layer as Strata's renderer sends it (`PdfTextLayer`): the page's text in
/// reading order, and for each character its box `[x1, y1, x2, y2]` in pixels of the image
/// rendered with it, which is `width` × `height`. A character with an empty box (a line
/// break) has no ink.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayer {
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub glyphs: Vec<[f32; 4]>,
}

impl TextLayer {
    pub fn new(width: f32, height: f32, text: impl Into<String>, glyphs: Vec<[f32; 4]>) -> Self {
        TextLayer {
            width,
            height,
            text: text.into(),
            glyphs,
        }
    }
}

/// A document: how many pages it has, and what's known of their sizes. Strata's renderer says
/// a page's size only when it renders it, so a page with no size of its own takes the
/// default: the size of the page the preview opened on. Only a size's shape matters.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfDocument {
    pages: usize,
    default_size: [f32; 2],
    sizes: Vec<Option<[f32; 2]>>,
}

impl PdfDocument {
    /// `pages` pages (at most [`MAX_PAGES`]), each `default_size` until told otherwise.
    pub fn new(pages: usize, default_size: [f32; 2]) -> Self {
        let pages = pages.min(MAX_PAGES);
        PdfDocument {
            pages,
            default_size: sane(default_size).unwrap_or([612.0, 792.0]),
            sizes: vec![None; pages],
        }
    }

    /// Page `page`'s own size, for a host that knows it before rendering.
    pub fn with_size(mut self, page: usize, size: [f32; 2]) -> Self {
        if let Some(s) = self.sizes.get_mut(page) {
            *s = sane(size);
        }
        self
    }

    pub fn pages(&self) -> usize {
        self.pages
    }

    /// The size page `page` lays out at.
    pub fn size(&self, page: usize) -> [f32; 2] {
        self.sizes
            .get(page)
            .copied()
            .flatten()
            .unwrap_or(self.default_size)
    }

    pub fn default_size(&self) -> [f32; 2] {
        self.default_size
    }

    /// Sets page `page`'s size. True when its shape changed by more than a rounding.
    pub(crate) fn set_size(&mut self, page: usize, size: [f32; 2]) -> bool {
        let Some(size) = sane(size) else {
            return false;
        };
        let old = self.size(page);
        let Some(slot) = self.sizes.get_mut(page) else {
            return false;
        };
        *slot = Some(size);
        let (a, b) = (old[1] / old[0], size[1] / size[0]);
        (a - b).abs() > a * 0.005
    }
}

fn sane(size: [f32; 2]) -> Option<[f32; 2]> {
    (size[0].is_finite() && size[1].is_finite() && size[0] > 0.0 && size[1] > 0.0).then_some(size)
}

/// What a render request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RenderKind {
    /// The page's image, at the request's box, and its text layer.
    Page,
    /// Only the page's text layer, for find: the host may render it as small as it likes.
    TextOnly,
}

/// A request's ticket: what the host hands back with the reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ticket {
    /// Which [`PdfState::open`](super::PdfState::open) it belongs to.
    pub(crate) generation: u32,
    pub(crate) serial: u64,
    pub(crate) page: u32,
    pub(crate) kind: RenderKind,
}

impl Ticket {
    /// The page it asks for, from 0.
    pub fn page(&self) -> usize {
        self.page as usize
    }
}

/// A page to render: Strata's `page:WxH` request, with the box the image should fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderRequest {
    pub ticket: Ticket,
    /// From 0.
    pub page: usize,
    /// The box to fit, in pixels: the width wanted and the renderer's height cap, as Strata's
    /// `PdfRenderSize::for_viewport_width` asks.
    pub box_px: [u32; 2],
    pub kind: RenderKind,
}

/// The renderer's reply. The host decodes the PNG (off the UI thread) into RGBA.
#[derive(Clone, Debug, PartialEq)]
pub enum PageReply {
    /// `width × height` straight-alpha sRGB RGBA pixels, rows top first, and the page's text
    /// layer when it has one (a scan has none).
    Rendered {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        text: Option<TextLayer>,
    },
    /// The page didn't render: it shows "Unable to render this PDF page".
    Failed,
}

/// The renderer's limits (Strata's `PdfRenderSize`): a request never asks past them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCaps {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
}

impl Default for RenderCaps {
    fn default() -> Self {
        RenderCaps {
            max_width: 1_400,
            max_height: 1_800,
            max_pixels: 2_500_000,
        }
    }
}

impl RenderCaps {
    /// The width worth asking for a page of `aspect` (height over width) drawn `wanted` pixels
    /// wide: no wider than the caps let a page of that shape be.
    pub fn width_for(&self, wanted: f32, aspect: f32) -> u32 {
        let aspect = aspect.max(f32::EPSILON);
        let by_height = self.max_height as f32 / aspect;
        let by_pixels = (self.max_pixels as f32 / aspect).sqrt();
        let w = wanted
            .min(self.max_width as f32)
            .min(by_height)
            .min(by_pixels);
        (w.floor() as u32).max(16)
    }
}

/// What the view shows instead of pages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Content {
    /// The pages.
    #[default]
    Pages,
    /// "Preview unavailable" and "This PDF is password protected.", with Open (Strata: no
    /// password field).
    Locked,
    /// "Preview unavailable" and this message, with no Open (a damaged PDF).
    Failed(String),
}

/// A held text layer: the host's, read into lines once. The UI thread's only.
pub(crate) type Shared = Rc<Layer>;
