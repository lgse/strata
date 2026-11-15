// SPDX-License-Identifier: MIT

//! The drawn part: a thumbnail over its placeholder icon, and the C1e icon slot that draws it.

use std::hash::{Hash, Hasher};

use rune::{El, SurfaceFit, TextStyle, el, icon, row, text};
use strata_kit::{icons::IconName, look::Look};

use super::step::Step;
use super::store::{Thumb, ThumbnailStore};
use crate::{
    collection::element::{ICON, RowCx, Slot},
    contract::{Entry, EntryGlyph, EntryKind},
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
                let mark = mark_icon(rcx.state.marks);
                mark.hash(&mut h);
                icon_opacity(rcx, dim_files).to_bits().hash(&mut h);
                if mark.is_none()
                    && let Some(key) = shown_thumbnail(rcx.entry)
                {
                    store(s).version(key).hash(&mut h);
                }
                h.finish()
            },
            move |s: &S, rcx: &RowCx<'_>| {
                let glyph = placeholder(rcx, points, icon_opacity(rcx, dim_files));
                let st = store(s);
                match shown_thumbnail(rcx.entry) {
                    Some(key) if st.enabled() && mark_icon(rcx.state.marks).is_none() => {
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

/// The clipboard mark icon a cut or copied row draws in place of its thumbnail or entry icon, as
/// GTK's `ThumbnailSlot` draws `SCISSORS` and `COPY` (`src/ui/thumbnail/slot.rs`).
pub(crate) fn mark_icon(marks: crate::collection::element::RowMarks) -> Option<IconName> {
    if marks.cut {
        Some(IconName::Cut)
    } else if marks.copied {
        Some(IconName::Copy)
    } else {
        None
    }
}

/// The opacity of an entry's placeholder icon: a file's at `collection.file_icon.opacity`
/// where the view dims files, else full. A folder's never dims, nor a cut or copied entry's:
/// GTK's marked icon draws at full opacity (`update_state_opacity` in `src/ui/thumbnail/slot.rs`),
/// and C1e fades a cut row as a whole. GTK draws a hidden entry's icon at 0.65 over all of this,
/// folders included; C1e's row has no hidden flag yet.
///
/// Used by: [`icon_slot`], C3's tiles.
pub fn icon_opacity(rcx: &RowCx<'_>, dim_files: bool) -> f32 {
    let marked = mark_icon(rcx.state.marks).is_some();
    file_icon_opacity(rcx.look, rcx.entry.kind, marked, dim_files)
}

/// [`icon_opacity`]'s rule, for an entry of `kind`, `marked` (cut or copied) or not.
pub(crate) fn file_icon_opacity(
    look: &Look,
    kind: EntryKind,
    marked: bool,
    dim_files: bool,
) -> f32 {
    match dim_files && dims(kind, marked) {
        true => strata_kit::slots::collection::file_icon_opacity(look),
        false => 1.0,
    }
}

/// Whether an entry of `kind` draws a dimmed icon where its view dims files: an unmarked file.
fn dims(kind: EntryKind, marked: bool) -> bool {
    !kind.is_folder_like() && !marked
}

/// The thumbnail key an entry draws when thumbnails are on: none while a custom glyph is set.
pub(crate) fn shown_thumbnail(entry: Entry<'_>) -> Option<crate::contract::ThumbKey> {
    entry.thumbnail.filter(|_| entry.glyph.is_none())
}

/// Whether `entry` wears a custom icon or emoji inside a filled folder rather than replacing
/// its icon outright: a folder whose base icon is the folder itself (not a locked folder).
fn wears_folder_badge(look: &Look, entry: Entry<'_>) -> bool {
    entry.kind.is_folder_like()
        && (entry.icon.is_empty()
            || look
                .icon_named(entry.icon)
                .is_some_and(|art| std::ptr::eq(art, look.icon(IconName::Folder))))
}

/// Emoji presentation (`U+FE0F`) so text-default emoji draw in colour (`anomalyco/rune#102`).
fn drawn_emoji(emoji: &str) -> String {
    if emoji.contains('\u{fe0f}') {
        emoji.to_owned()
    } else {
        format!("{emoji}\u{fe0f}")
    }
}

/// A filled folder coloured `folder_color` with `badge` centred in its body, matching Strata's
/// `sized_folder_decoration_paintable` and `folder_emoji_texture` (`src/assets.rs`).
///
/// Used by: [`entry_glyph`], C8's sidebar rows.
pub fn folder_badge(
    look: &Look,
    badge: EntryGlyph<'_>,
    points: f32,
    folder_color: rune::Color,
) -> El {
    let base = icon(look.icon(IconName::FolderFilled), points)
        .color(folder_color)
        .a11y_hidden();
    let mark = match badge {
        EntryGlyph::Icon(name) => {
            let badge_pts = points * 0.54;
            let x = (points - badge_pts) * 0.5;
            let y = points * (6.8 / 24.0);
            let art = look
                .icon_named(name)
                .unwrap_or_else(|| look.icon(IconName::Folder));
            icon(art, badge_pts)
                .color(look.ink_on(folder_color))
                .absolute(x, y)
                .a11y_hidden()
        }
        EntryGlyph::Emoji(emoji) => {
            let font_pts = points * 0.48;
            let style = TextStyle {
                size: font_pts,
                ..look.kit.ty.ui
            }
            .lh(font_pts);
            let top = points * (16.0 / 96.0);
            row()
                .absolute(0.0, top)
                .w(points)
                .h(points - top)
                .center()
                .a11y_hidden()
                .child(
                    text(drawn_emoji(emoji))
                        .font(style)
                        .color(look.kit.c.text_base)
                        .a11y_hidden(),
                )
        }
    };
    el().w(points)
        .h(points)
        .a11y_hidden()
        .child(base)
        .child(mark)
}

/// `entry`'s icon or custom decoration at `points` square, keyed [`GLYPH`] and hidden from
/// assistive technology:
/// - a folder wearing [`Entry::glyph`] draws [`folder_badge`] in [`Entry::color`], else
///   `look.kit.c.icon_accent`;
/// - a file wearing [`EntryGlyph::Emoji`] draws the emoji in a `points × points` box;
/// - any other entry draws [`EntryGlyph::Icon`] (else [`Entry::icon`], else Folder/File) in
///   [`Entry::color`] (else `default_color`).
///
/// Used by: [`icon_slot`], C1e's default icon, C3's tiles, hosts (Customize preview).
pub fn entry_glyph(look: &Look, entry: Entry<'_>, points: f32, default_color: rune::Color) -> El {
    if let Some(badge) = entry.glyph
        && wears_folder_badge(look, entry)
    {
        let fill = entry.color.unwrap_or(look.kit.c.icon_accent);
        return folder_badge(look, badge, points, fill).key(GLYPH);
    }
    if let Some(EntryGlyph::Emoji(emoji)) = entry.glyph {
        let font_pts = points * 0.82;
        let style = TextStyle {
            size: font_pts,
            ..look.kit.ty.ui
        }
        .lh(font_pts);
        return row()
            .key(GLYPH)
            .w(points)
            .h(points)
            .center()
            .a11y_hidden()
            .child(
                text(drawn_emoji(emoji))
                    .font(style)
                    .color(look.kit.c.text_base)
                    .a11y_hidden(),
            );
    }
    let fallback = if entry.kind.is_folder_like() {
        IconName::Folder
    } else {
        IconName::File
    };
    let catalog_name = match entry.glyph {
        Some(EntryGlyph::Icon(custom)) => custom,
        _ => entry.icon,
    };
    let art = look
        .icon_named(catalog_name)
        .unwrap_or_else(|| look.icon(fallback));
    icon(art, points)
        .key(GLYPH)
        .color(entry.color.unwrap_or(default_color))
        .a11y_hidden()
}

/// The clipboard mark icon, else the entry's icon or custom glyph, as C1e's default icon draws it,
/// at `opacity`.
fn placeholder(rcx: &RowCx<'_>, points: f32, opacity: f32) -> El {
    let look = rcx.look;
    let glyph = match mark_icon(rcx.state.marks) {
        Some(mark) => icon(look.icon(mark), points)
            .key(GLYPH)
            .color(look.kit.c.icon_base)
            .a11y_hidden(),
        None => entry_glyph(look, rcx.entry, points, look.kit.c.icon_base),
    };
    match opacity < 1.0 {
        true => glyph.opacity(opacity),
        false => glyph,
    }
}
