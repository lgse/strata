// SPDX-License-Identifier: MIT

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{El, Pixels, SurfaceFit, TextStyle, el, icon, row, text};
use strata_kit::{icons::IconName, look::Look};

use super::step::Step;
use super::store::{Thumb, ThumbnailStore};
use crate::{
    collection::element::{ICON, RowCx, Slot},
    contract::{Entry, EntryGlyph, EntryId, EntryKind},
};

/// The key of a part's image, under the part ([`ICON`] in a row).
pub const THUMB: &str = "thumb";

/// The key of a part's placeholder icon, under the part.
pub const GLYPH: &str = "glyph";

/// The key of an optional corner badge over the part.
pub const BADGE: &str = "badge";

/// Corner badge side for a `points × points` slot, matching Strata's `ThumbnailSlot`.
fn badge_size(points: f32) -> f32 {
    (points * 0.55).clamp(10.0, 24.0)
}

/// Overlays `badge` in the bottom-right corner of a `(inset, slot_points)` box.
pub(crate) fn overlay_badge(el: El, badge: Option<Pixels>, (inset, slot_points): (f32, f32)) -> El {
    let Some(badge_px) = badge else {
        return el;
    };
    let size = badge_size(slot_points);
    let edge = inset + slot_points - size;
    el.child(
        rune::el()
            .key(BADGE)
            .absolute(edge, edge)
            .w(size)
            .h(size)
            .a11y_hidden()
            .pixels_fit(&badge_px, SurfaceFit::Fit),
    )
}

type BadgeAccess<S> = (fn(&S, EntryId) -> u64, fn(&S, EntryId) -> Option<Pixels>);
type DragPredicate<S> = Rc<dyn Fn(&S) -> bool>;

/// `thumb` over `icon` in a `w × h` box: the icon centred underneath.
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

/// A icon slot that draws each entry's thumbnail at `points` square from `store`, at `step` (see `Step::for_tile`).
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
        badge: None,
        dragging: None,
    }
}

/// `icon_slot`'s settings, made into `Slot` where the collection takes it.
pub struct IconSlot<S> {
    store: fn(&S) -> &ThumbnailStore,
    step: Step,
    points: f32,
    dim_files: bool,
    badge: Option<BadgeAccess<S>>,
    dragging: Option<DragPredicate<S>>,
}

impl<S: 'static> IconSlot<S> {
    /// Draws a file's icon at `collection.file_icon.opacity`, a folder's at full opacity: Strata's Columns rows.
    pub fn dim_files(mut self) -> Self {
        self.dim_files = true;
        self
    }

    /// Overlays an optional corner badge (`Pixels`) in the bottom-right corner of the slot.
    pub fn badge(
        mut self,
        version: fn(&S, EntryId) -> u64,
        pixels: fn(&S, EntryId) -> Option<Pixels>,
    ) -> Self {
        self.badge = Some((version, pixels));
        self
    }

    /// While `is_dragging` returns true (a scrollbar thumb drag is actively deferring thumbnails).
    pub fn dragging(mut self, is_dragging: impl Fn(&S) -> bool + 'static) -> Self {
        self.dragging = Some(Rc::new(is_dragging));
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
            badge,
            dragging,
        } = slot;
        let version_dragging = dragging.clone();
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
                    let st = store(s);
                    let ver = if version_dragging.as_ref().is_some_and(|f| f(s)) {
                        st.existing_version(key)
                    } else {
                        st.version(key)
                    };
                    ver.hash(&mut h);
                }
                badge
                    .map_or(0, |(ver, _)| ver(s, rcx.entry.id))
                    .hash(&mut h);
                h.finish()
            },
            move |s: &S, rcx: &RowCx<'_>| {
                let glyph = placeholder(rcx, points, icon_opacity(rcx, dim_files));
                let st = store(s);
                let thumb = shown_thumbnail(rcx.entry)
                    .filter(|_| st.enabled() && mark_icon(rcx.state.marks).is_none())
                    .and_then(|key| {
                        if dragging.as_ref().is_some_and(|f| f(s)) {
                            st.existing_thumb(key)
                        } else {
                            Some(st.thumb(key))
                        }
                    });
                let built = match thumb {
                    Some(thumb) => picture(&thumb, glyph, points, points).key(ICON),
                    None => row()
                        .key(ICON)
                        .w(points)
                        .h(points)
                        .center()
                        .a11y_hidden()
                        .child(glyph),
                };
                let px = badge.and_then(|(_, px_fn)| px_fn(s, rcx.entry.id));
                overlay_badge(built, px, (0.0, points))
            },
        )
    }
}

pub(crate) fn mark_icon(marks: crate::collection::element::RowMarks) -> Option<IconName> {
    if marks.cut {
        Some(IconName::Cut)
    } else if marks.copied {
        Some(IconName::Copy)
    } else {
        None
    }
}

/// The opacity of an entry's placeholder icon.
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

/// A filled folder coloured `folder_color` with `badge` centred in its body.
pub fn folder_badge(
    look: &Look,
    badge: EntryGlyph<'_>,
    points: f32,
    folder_color: rune::Color,
) -> El {
    folder_badge_hover(look, badge, points, (folder_color, None))
}

pub(crate) fn folder_badge_hover(
    look: &Look,
    badge: EntryGlyph<'_>,
    points: f32,
    (folder_color, hover_color): (rune::Color, Option<rune::Color>),
) -> El {
    let mut base = icon(look.icon(IconName::FolderFilled), points)
        .color(folder_color)
        .a11y_hidden();
    if let Some(hover) = hover_color {
        base = base.group_color(hover);
    }
    let mark = match badge {
        EntryGlyph::Icon(name) => {
            let badge_pts = points * 0.54;
            let x = (points - badge_pts) * 0.5;
            let y = points * (6.8 / 24.0);
            let art = look
                .icon_named(name)
                .unwrap_or_else(|| look.icon(IconName::Folder));
            let mut mark = icon(art, badge_pts)
                .color(look.ink_on(folder_color))
                .absolute(x, y)
                .a11y_hidden();
            if let Some(hover) = hover_color {
                mark = mark.group_color(look.ink_on(hover));
            }
            mark
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

/// `entry`'s icon or custom decoration at `points` square, keyed `GLYPH` and hidden from assistive technology.
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

fn placeholder(rcx: &RowCx<'_>, points: f32, opacity: f32) -> El {
    let look = rcx.look;
    let glyph = match mark_icon(rcx.state.marks) {
        Some(mark) => icon(look.icon(mark), points)
            .key(GLYPH)
            .color(look.kit.c.icon_accent)
            .a11y_hidden(),
        None => entry_glyph(look, rcx.entry, points, look.kit.c.icon_accent),
    };
    match opacity < 1.0 {
        true => glyph.opacity(opacity),
        false => glyph,
    }
}
