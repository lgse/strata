//! Size steps: the pixel size a thumbnail is held at, one of the look's thumbnail classes.

use strata_kit::look::Look;

/// The long edge, in device pixels, a thumbnail is held at: one of the look's thumbnail
/// classes (`Look::thumbnails`, W2), so an image is rendered once per class, not once per
/// tile size.
///
/// A view asks for the class that covers what it draws, [`Step::for_tile`]. Rune draws a
/// streamed `Pixels` bilinearly from the full image, with no mipmaps, so an image held far
/// larger than it draws aliases (Rune draft U-C25-1, in the module docs).
///
/// Used by: C3, C4, C25, hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Step(u32);

impl Step {
    /// A step of `px` device pixels (at least 1). Views use [`Step::for_tile`]; this is for
    /// hosts and tests that know their class.
    pub const fn new(px: u32) -> Self {
        Self(if px == 0 { 1 } else { px })
    }

    /// The class that covers a tile `points` wide at `scale` device pixels per point: the
    /// smallest of the look's classes at least that large, else the largest.
    pub fn for_tile(look: &Look, points: f32, scale: f32) -> Self {
        let classes = &look.thumbnails;
        let px = classes.sizes()[classes.for_size(points * scale)];
        Self::new(px.round().max(1.0) as u32)
    }

    /// The long edge in device pixels.
    pub const fn px(self) -> u32 {
        self.0
    }

    /// What an image at this step holds at most, in bytes (a square RGBA image).
    pub(super) fn max_bytes(self) -> usize {
        (self.0 as usize)
            .saturating_mul(self.0 as usize)
            .saturating_mul(4)
    }
}
