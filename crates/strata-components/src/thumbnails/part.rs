// SPDX-License-Identifier: MIT

//! The drawn part: a thumbnail over its placeholder icon, and the C1e icon slot that draws it.

use std::hash::{Hash, Hasher};

use rune::{El, SurfaceFit, el, icon, row};
use strata_kit::{icons::IconName, look::Look};

use super::step::Step;
use super::store::{Thumb, ThumbnailStore};
use crate::{
    collection::element::{ICON, RowCx, Slot},
    contract::EntryKind,
};

/// The key of a part's image, under the part ([`ICON`] in a row).
pub const THUMB: &str = "thumb";

/// The key of a part's placeholder icon, under the part.
pub const GLYPH: &str = "glyph";

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
/// and every entry while thumbnails are off, draws the icon alone. [`IconSlot::dim_files`]
/// draws a file's icon fainter than a folder's, as Strata's Columns rows do.
///
/// The slot's version is the store's [`ThumbnailStore::version`] and the step, so a row
/// builds again only when the key's handles change or thumbnails are turned on or off, never
/// when a thumbnail lands. Build it in the view, where the step is known.
///
/// Used by: C4, C2, the C25 gallery page, hosts.
pub fn icon_slot<S: 'static>(
    store: fn(&S) -> &ThumbnailStore,
    step: Step,
    points: f32,
) -> IconSlot<S> {
    IconSlot {
        store,
        step,
        points,
        dim_files: false,
    }
}

/// [`icon_slot`]'s settings, made into C1e's [`Slot`] where the collection takes it.
pub struct IconSlot<S> {
    store: fn(&S) -> &ThumbnailStore,
    step: Step,
    points: f32,
    dim_files: bool,
}

impl<S: 'static> IconSlot<S> {
    /// Draws a file's icon at `collection.file_icon.opacity`, a folder's at full opacity: Strata's
    /// Columns rows. A thumbnail is never dimmed, nor the icon of a cut entry (its row fades
    /// as a whole). Strata's List rows leave it off.
    pub fn dim_files(mut self) -> Self {
        self.dim_files = true;
        self
    }
}

impl<S: 'static> From<IconSlot<S>> for Slot<S> {
    fn from(slot: IconSlot<S>) -> Self {
        let IconSlot {
            store,
            step,
            points,
            dim_files,
        } = slot;
        Slot::new(
            move |s: &S, rcx: &RowCx<'_>| {
                let mut h = std::hash::DefaultHasher::new();
                step.hash(&mut h);
                points.to_bits().hash(&mut h);
                icon_opacity(rcx, dim_files).to_bits().hash(&mut h);
                if let Some(key) = rcx.entry.thumbnail {
                    store(s).version(key).hash(&mut h);
                }
                h.finish()
            },
            move |s: &S, rcx: &RowCx<'_>| {
                let glyph = placeholder(rcx, points, icon_opacity(rcx, dim_files));
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
}

/// The opacity of an entry's placeholder icon: a file's at `collection.file_icon.opacity`
/// where the view dims files, else full. A folder's never dims, nor a cut entry's: GTK's
/// marked icon draws at full opacity (`update_state_opacity` in `src/ui/thumbnail/slot.rs`),
/// and C1e fades the cut row as a whole.
///
/// Used by: [`icon_slot`], C3's tiles.
pub fn icon_opacity(rcx: &RowCx<'_>, dim_files: bool) -> f32 {
    file_icon_opacity(rcx.look, rcx.entry.kind, rcx.state.marks.cut, dim_files)
}

/// [`icon_opacity`]'s rule, for an entry of `kind`, `cut` or not.
pub(crate) fn file_icon_opacity(look: &Look, kind: EntryKind, cut: bool, dim_files: bool) -> f32 {
    match dim_files && dims(kind, cut) {
        true => strata_kit::slots::collection::file_icon_opacity(look),
        false => 1.0,
    }
}

/// Whether an entry of `kind` draws a dimmed icon where its view dims files: a file not cut.
fn dims(kind: EntryKind, cut: bool) -> bool {
    !kind.is_folder_like() && !cut
}

/// The entry's catalog icon, else the Folder or File icon, as C1e's default icon draws it, at
/// `opacity`.
fn placeholder(rcx: &RowCx<'_>, points: f32, opacity: f32) -> El {
    let look = rcx.look;
    let fallback = if rcx.entry.kind.is_folder_like() {
        IconName::Folder
    } else {
        IconName::File
    };
    let glyph = look
        .icon_named(rcx.entry.icon)
        .unwrap_or_else(|| look.icon(fallback));
    let glyph = icon(glyph, points)
        .key(GLYPH)
        .color(look.kit.c.icon_base)
        .a11y_hidden();
    match opacity < 1.0 {
        true => glyph.opacity(opacity),
        false => glyph,
    }
}
