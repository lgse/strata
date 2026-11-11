//! The heading strip: sortable headings with the sort arrow, a resize handle on each
//! heading's right edge, reordering by dragging a heading, and the columns menu on a
//! right-click.

use std::rc::Rc;

use rune::{Cursor, Drag, El, Phase, PointerPress, Tx, el, icon, row, text};
use strata_kit::icons::IconName;
use strata_kit::slots::list as tokens;

use super::Shared;
use super::column::Column;
use super::layout::{LayoutChange, LayoutWhy};
use super::resolve::{Placed, Resolved};
use super::sort::{SortDir, SortIntent, next_direction};
use super::state::{HeadingDrag, MenuAt, Resize};
use crate::collection::element::RowCx;
use crate::collection::element::RowState;
use crate::contract::{Point, Rect};

/// How far (points) a heading press travels sideways before it becomes a reorder drag, as
/// Strata's `REORDER_THRESHOLD`. A shorter press and release stays a sort click.
pub const REORDER_THRESHOLD: f32 = 10.0;

/// How close (points) to a heading's right edge a press resizes rather than reorders, as
/// Strata's `RESIZE_EDGE`: the resize handle's width.
pub const RESIZE_EDGE: f32 = 8.0;

/// How far (points) a press may move and still be a click of a run: Rune's own pairing
/// distance.
const CLICK_SLOP: f32 = 4.0;

/// The key of column `id`'s heading, under the strip.
///
/// Used by: C4, scripts.
pub fn heading_key(id: &str) -> String {
    format!("head-{id}")
}

/// The heading strip's key, under the table's key.
pub(super) fn strip_key(key: &str) -> String {
    format!("{key}-headings")
}

/// The strip, before the columns menu wraps it.
pub(super) fn strip<S: 'static>(shared: &Rc<Shared<S>>, resolved: &Rc<Resolved>, s: &S) -> El {
    let parts = (shared.parts)(s);
    let look = parts.look;
    let kit = &look.kit;
    let rule = rune_kit::TableStyle::from_kit(kit).rule;
    let layout_shared = shared.clone();
    let menu_shared = shared.clone();
    let mut strip = row()
        .key(strip_key(&shared.key))
        .role("row")
        .w_full()
        .h(tokens::heading_height(look))
        .pl(resolved.lead)
        .fill(kit.c.surface_base)
        .border_sides([0.0, 0.0, 1.0, 0.0], rule)
        .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
            (layout_shared.table_mut)(s).strip = Rect::from(r);
            tx.live_only();
        })
        .on_secondary_press(move |s: &mut S, at: PointerPress, _tx: &mut Tx| {
            (menu_shared.table_mut)(s).menu = Some(MenuAt::Headings(Some((at.x, at.y))));
        });
    for p in &resolved.cols {
        strip = strip.child(heading(shared, resolved, s, p));
    }
    strip
}

fn heading<S: 'static>(shared: &Rc<Shared<S>>, resolved: &Rc<Resolved>, s: &S, p: &Placed) -> El {
    let parts = (shared.parts)(s);
    let look = parts.look;
    let kit = &look.kit;
    let column = &shared.columns[p.def];
    let id = column.id.clone();
    let sorted = parts
        .sort
        .as_ref()
        .filter(|sort| sort.column == id)
        .map(|sort| sort.dir);
    let drag = parts.table.drag.as_ref().filter(|d| d.active);
    let mut title_row = row()
        .gap(look.space.xxs)
        .center_y()
        .min_w(0.0)
        .shrink()
        .child(
            text(column.title.clone())
                .a11y_hidden()
                .font(kit.ty.small_medium)
                .color(if sorted.is_some() {
                    kit.c.text_base
                } else {
                    kit.c.text_muted
                })
                .truncate()
                .shrink()
                .min_w(0.0),
        );
    if let Some(dir) = sorted {
        let glyph = match dir {
            SortDir::Ascending => look.icon(IconName::SortAscending),
            SortDir::Descending => look.icon(IconName::SortDescending),
        };
        title_row = title_row.child(
            icon(glyph, look.icon_size.sm)
                .key("sort")
                .color(kit.c.icon_base)
                .a11y_hidden(),
        );
    }
    let mut h = super::rows::justify(
        row()
            .key(heading_key(id.as_str()))
            .role("columnheader")
            .label(column.title.clone())
            .sorted(sorted.map(|d| d == SortDir::Ascending))
            .w(p.w)
            .min_w(p.w)
            .h_full()
            .px(tokens::heading_pad(look))
            .center_y()
            .border_sides([0.0, 1.0, 0.0, 0.0], kit.c.border_weak),
        column.align,
    )
    .child(title_row)
    .on_drag(reorder(shared, resolved, id.clone(), column.sortable));
    h = h.cursor(match drag {
        Some(_) => Cursor::Grabbing,
        None if column.sortable => Cursor::Pointer,
        None => Cursor::Default,
    });
    if column.sortable {
        let sort_shared = shared.clone();
        let sort_id = id.clone();
        h = h
            .hover(kit.c.hover)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                let st = (sort_shared.table_mut)(s);
                if st.suppress_press.replace(false) {
                    tx.live_only();
                    return;
                }
                let dir = next_direction((sort_shared.parts)(s).sort.as_ref(), &sort_id);
                if let Some(f) = &sort_shared.on_sort {
                    f(
                        s,
                        SortIntent {
                            column: sort_id.clone(),
                            dir,
                        },
                        tx,
                    );
                }
            });
    }
    if let Some(site) = drag
        .and_then(|d| d.site.as_ref())
        .filter(|site| site.target == id)
    {
        let (left, right) = if site.after {
            (None, Some(0.0))
        } else {
            (Some(0.0), None)
        };
        h = h.child(
            el().key("reorder-line")
                .pin(left, Some(0.0), right, Some(0.0))
                .w(tokens::reorder_width(look))
                .fill(kit.c.text_accent)
                .a11y_hidden(),
        );
    }
    h.child(resize_handle(shared, resolved, column, p))
}

/// The heading's drag: a sort click below the threshold (its `on_press` runs it), a reorder
/// past it.
fn reorder<S: 'static>(
    shared: &Rc<Shared<S>>,
    resolved: &Rc<Resolved>,
    id: super::ColumnId,
    sortable: bool,
) -> impl Fn(&mut S, &mut Drag, &mut Tx) + 'static {
    let shared = shared.clone();
    let resolved = resolved.clone();
    move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
        let [x, y] = d.window_point();
        let site_at = |s: &S| {
            let parts = (shared.parts)(s);
            let strip = parts.table.strip;
            strip
                .contains(Point::new(x, y))
                .then(|| resolved.drop_site(&id, x - strip.x - resolved.lead))
                .flatten()
        };
        match d.phase {
            Phase::Down => {
                let st = (shared.table_mut)(s);
                st.suppress_press.set(false);
                st.drag = Some(HeadingDrag {
                    column: id.clone(),
                    active: false,
                    site: None,
                });
                tx.live_only();
            }
            Phase::Move => {
                let Some(drag) = (shared.parts)(s).table.drag.clone() else {
                    tx.live_only();
                    return;
                };
                if !drag.active && d.dx.abs() < REORDER_THRESHOLD {
                    tx.live_only();
                    return;
                }
                let site = site_at(s);
                let changed = !drag.active || site != drag.site;
                (shared.table_mut)(s).drag = Some(HeadingDrag {
                    column: id.clone(),
                    active: true,
                    site,
                });
                if !changed {
                    tx.live_only();
                }
            }
            Phase::Up => {
                let site = site_at(s);
                let st = (shared.table_mut)(s);
                if st.drag.take().filter(|d| d.active).is_none() {
                    tx.live_only();
                    return;
                }
                // Rune presses the heading after a drag released on it (Rune draft R-C4-1):
                // that press isn't a sort click. Whether Rune presses depends on what it drew
                // (a heading the sideways scroll clips presses only where it shows), so the
                // table doesn't predict it: Rune runs that press in the same frame as this
                // release, before the view, and the next build clears a flag no press took
                // (`ListTableState::end_frame`). A press from assistive technology comes in a
                // later frame, and is never taken for this one.
                if d.cancelled() {
                    return;
                }
                st.suppress_press.set(sortable);
                let moved = site.and_then(|site| (shared.parts)(s).layout.moved(&id, &site));
                if let (Some(layout), Some(f)) = (moved, &shared.on_layout) {
                    f(
                        s,
                        LayoutChange {
                            layout,
                            why: LayoutWhy::Reorder,
                        },
                        tx,
                    );
                }
            }
        }
    }
}

/// The 8 pt strip on a heading's right edge: a drag resizes the column from its drawn width,
/// within its bounds; a double press fits it to the widest of its heading and the cells the
/// rows on screen show (Strata's autofit measures its bound cells).
fn resize_handle<S: 'static>(
    shared: &Rc<Shared<S>>,
    resolved: &Rc<Resolved>,
    column: &Column<S>,
    p: &Placed,
) -> El {
    let drag_shared = shared.clone();
    let fit_resolved = resolved.clone();
    let id = column.id.clone();
    let scale = resolved.scale;
    let drawn = p.w;
    let def = p.def;
    row()
        .key("resize")
        // Assistive technology can't move it (it takes no steps), so it would only add a
        // "Resize …" splitter under every column header: hidden. See the module docs.
        .a11y_hidden()
        .pin(None, Some(0.0), Some(0.0), Some(0.0))
        .w(RESIZE_EDGE)
        .cursor(Cursor::ResizeColumn)
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            let column = &drag_shared.columns[def];
            match d.phase {
                Phase::Down => {
                    // From the width drawn now, so the column follows the pointer with no jump.
                    let start = drawn / scale;
                    (drag_shared.table_mut)(s).resize = Some(Resize {
                        column: id.clone(),
                        start,
                        live: start,
                    });
                    tx.live_only();
                }
                Phase::Move => {
                    let st = (drag_shared.table_mut)(s);
                    let Some(r) = st.resize.as_mut() else {
                        tx.live_only();
                        return;
                    };
                    let live = column.bounded(r.start + d.dx / scale);
                    if live == r.live {
                        tx.live_only();
                        return;
                    }
                    r.live = live;
                }
                Phase::Up => {
                    let Some(r) = (drag_shared.table_mut)(s).resize.take() else {
                        tx.live_only();
                        return;
                    };
                    // The second click of a run fits the column (Rune counts the run, with its
                    // fixed 0.5 s on Linux: S5 U13).
                    let still = d.dx.abs() <= CLICK_SLOP && d.dy.abs() <= CLICK_SLOP;
                    if d.clicks == 2 && still && !d.cancelled() {
                        let w = fit(&drag_shared, &fit_resolved, s, def);
                        let widths = (drag_shared.parts)(s).widths.with(&id, w);
                        if let Some(f) = &drag_shared.on_widths {
                            f(s, widths, tx);
                        }
                        return;
                    }
                    let live = column.bounded(r.start + d.dx / scale);
                    // A cancelled drag puts the width back; a press that moved nothing (half
                    // a double press) changes nothing.
                    if d.cancelled() || live == r.start {
                        return;
                    }
                    let widths = (drag_shared.parts)(s).widths.with(&id, live);
                    if let Some(f) = &drag_shared.on_widths {
                        f(s, widths, tx);
                    }
                }
            }
        })
        .child(el().w(1.0).h_full().a11y_hidden())
}

/// The widest of column `def`'s heading and the cells of the rows on screen, in base points,
/// within its bounds. A cell that is an element can't be measured: the column's drawn width
/// stands in for it.
pub(super) fn fit<S: 'static>(shared: &Shared<S>, resolved: &Resolved, s: &S, def: usize) -> f32 {
    let parts = (shared.parts)(s);
    let look = parts.look;
    let kit = &look.kit;
    let column = &shared.columns[def];
    let measure = |t: &str, font| rune::text::measure(t, &[], font, None).0;
    let mut widest = measure(&column.title, kit.ty.small_medium)
        + 2.0 * tokens::heading_pad(look)
        + look.icon_size.sm
        + look.space.xxs;
    let pad = tokens::cell_pad(look);
    let mono = rune_kit::TableStyle::from_kit(kit).mono;
    let drawn = resolved.cols.iter().find(|p| p.def == def).map(|p| p.w);
    let src = parts.source;
    for index in parts.collection.visible_range() {
        if index >= src.len() {
            break;
        }
        let entry = src.entry(index);
        let w = match &column.cell {
            None => {
                look.icon_size.md + look.space.sm + measure(entry.name, kit.ty.ui_compact) + pad
            }
            Some(f) => {
                let rcx = RowCx {
                    index,
                    entry,
                    state: RowState::default(),
                    look,
                    scope: &shared.key,
                    hover_live: false,
                };
                let cell = f(s, &rcx);
                match cell.as_text() {
                    Some(t) => {
                        measure(t, if cell.mono { mono } else { kit.ty.ui_compact }) + 2.0 * pad
                    }
                    None => drawn.unwrap_or(0.0),
                }
            }
        };
        widest = widest.max(w);
    }
    column.bounded((widest / resolved.scale).ceil())
}
