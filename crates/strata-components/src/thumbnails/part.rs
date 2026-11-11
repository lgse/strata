//! The drawn part: a thumbnail over its placeholder icon, and the C1e icon slot that draws it.

use std::hash::{Hash, Hasher};

use rune::{El, SurfaceFit, el, icon, row};
use strata_kit::icons::IconName;

use super::step::Step;
use super::store::{Thumb, ThumbnailStore};
use crate::collection::element::{ICON, RowCx, Slot};

/// The key of a part's image, under the part ([`ICON`] in a row).
pub const THUMB: &str = "thumb";

/// `thumb` over `icon` in a `w × h` box: the icon centred underneath, faded out through the
/// handle's opacity while an image is held, and the image fitted (letterboxed, centred) over
/// it. Neither changes the box, so nothing lays out again when a thumbnail lands.
///
/// The part is hidden from assistive technology, like an icon: the row's name says what it
/// is. The caller keys it (C1e's [`ICON`], so a motion effect finds it).
///
/// Used by: C3 (tiles), [`icon_slot`].
pub fn picture(thumb: &Thumb, icon: El, w: f32, h: f32) -> El {
    el().w(w)
        .h(h)
        .a11y_hidden()
        .child(
            row()
                .w(w)
                .h(h)
                .center()
                .live_opacity(thumb.icon_alpha())
                .child(icon),
        )
        .child(
            el().key(THUMB)
                .absolute(0.0, 0.0)
                .w(w)
                .h(h)
                .pixels_fit(thumb.pixels(), SurfaceFit::Fit),
        )
}

/// A C1e icon slot that draws each entry's thumbnail at `points` square from `store`, at
/// `step` (see [`Step::for_tile`]), over the entry's icon. An entry with no thumbnail key,
/// and every entry while thumbnails are off, draws the icon alone.
///
/// The slot's version is the store's [`ThumbnailStore::version`] and the step, so a row
/// builds again only when the key's handles change or thumbnails are turned on or off, never
/// when a thumbnail lands. Build it in the view, where the step is known.
///
/// Used by: C4, C2, the C25 gallery page, hosts.
pub fn icon_slot<S: 'static>(store: fn(&S) -> &ThumbnailStore, step: Step, points: f32) -> Slot<S> {
    Slot::new(
        move |s: &S, rcx: &RowCx<'_>| {
            let mut h = std::hash::DefaultHasher::new();
            step.hash(&mut h);
            points.to_bits().hash(&mut h);
            if let Some(key) = rcx.entry.thumbnail {
                store(s).version(key).hash(&mut h);
            }
            h.finish()
        },
        move |s: &S, rcx: &RowCx<'_>| {
            let glyph = placeholder(rcx, points);
            let st = store(s);
            match rcx.entry.thumbnail {
                Some(key) if st.enabled() => {
                    picture(&st.thumb(key), glyph, points, points).key(ICON)
                }
                _ => row()
                    .key(ICON)
                    .w(points)
                    .h(points)
                    .center()
                    .a11y_hidden()
                    .child(glyph),
            }
        },
    )
}

/// The entry's catalog icon, else the Folder or File icon, as C1e's default icon draws it.
fn placeholder(rcx: &RowCx<'_>, points: f32) -> El {
    let look = rcx.look;
    let fallback = if rcx.entry.kind.is_folder_like() {
        IconName::Folder
    } else {
        IconName::File
    };
    let glyph = look
        .icon_named(rcx.entry.icon)
        .unwrap_or_else(|| look.icon(fallback));
    icon(glyph, points)
        .color(look.kit.c.icon_base)
        .a11y_hidden()
}
