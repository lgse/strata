//! How the grid draws: the header (corner, column letters and titles, dividers), the rows
//! (a sticky row number, then the cells) and the look they take from the Kit.

use std::rc::Rc;

use rune::{Color, Cursor, Drag, El, Phase, TextStyle, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, SortDir, TableStyle};
use strata_kit::look::Look;

use super::model::{column_letters, shown, title_shown};
use super::range::Columns;
use super::state::{SheetGridState, SheetLabels, SheetView, fill};
use super::widths::{MAX_WIDTH, MIN_WIDTH};

/// The look, from rune-kit's `TableStyle` and the colours C11's document tables use.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Paint {
    pub style: TableStyle,
    pub header_h: f32,
    pub letters: TextStyle,
    pub numbers: TextStyle,
    pub head_fill: Color,
    pub rule: Color,
    pub selection: Color,
    pub focus: Color,
    pub hover: Color,
    pub caret_up: &'static rune::Icon,
    pub caret_down: &'static rune::Icon,
}

impl Paint {
    pub fn from_look(look: &Look) -> Paint {
        let k = &look.kit;
        let style = TableStyle::from_kit(k);
        let letters = k.ty.text_small;
        Paint {
            style,
            // The letters' line above the titles' row.
            header_h: style.header_height + (letters.size * 1.3).ceil(),
            letters,
            numbers: style.mono,
            head_fill: look.fill.muted,
            rule: k.c.border_muted,
            selection: k.selection(),
            focus: k.c.border_focus,
            hover: k.c.hover,
            caret_up: k.glyph.chevron_up,
            caret_down: k.glyph.chevron_down,
        }
    }

    /// The row numbers' width for a sheet whose last row is numbered `last`.
    pub fn gutter(&self, last: usize) -> f32 {
        let digits = last.max(1).ilog10() as usize + 1;
        (rune::text::advance(&"0".repeat(digits.max(2)), self.numbers) + 2.0 * self.style.cell_pad)
            .ceil()
    }
}

pub(crate) struct Ctx<S> {
    pub get: fn(&S) -> &SheetGridState,
    pub get_mut: fn(&mut S) -> &mut SheetGridState,
    pub paint: Paint,
    pub labels: SheetLabels,
    pub cols: Columns,
    /// The columns built: those near what shows, on a wide sheet.
    pub built: std::ops::Range<usize>,
}

/// A column's name: its title, else its letters ("Column C").
pub(crate) fn column_name(v: &SheetView, labels: &SheetLabels, j: usize) -> String {
    let letters = column_letters(j);
    match v.data.as_ref().and_then(|d| d.title(j)) {
        Some(t) if !t.trim().is_empty() => title_shown(t).into_owned(),
        _ => fill(&labels.column, &letters),
    }
}

/// The header: the corner over the row numbers, then each column's letters and title, which
/// sort on a click, with a divider on its trailing edge. The titles move sideways with the
/// rows through `strip_x`, the body's offset (probe P1: the same frame).
pub(crate) fn header<S: 'static>(
    ctx: &Rc<Ctx<S>>,
    st: &SheetGridState,
    v: &SheetView,
    strip_x: &rune::Live<[f32; 2]>,
) -> El {
    let p = &ctx.paint;
    let key = &st.key;
    let mut strip = row()
        .key(format!("{key}-strip"))
        .role("none")
        .h_full()
        .live_offset(strip_x);
    let sorted = |j: usize| v.spec.iter().position(|&(c, _)| c == j);
    strip = strip.child(lead(&ctx.cols, &ctx.built));
    for j in ctx.built.clone() {
        let (a, b) = ctx.cols.span(j);
        let w = b - a;
        let name = column_name(v, &ctx.labels, j);
        let letters = column_letters(j);
        let titled = v.data.as_ref().and_then(|d| d.title(j)).is_some();
        let mut about = Vec::new();
        if titled {
            about.push(fill(&ctx.labels.column, &letters));
        }
        let place = sorted(j);
        if let Some(k) = place.filter(|&k| k > 0) {
            let dir = match v.spec[k].1 {
                SortDir::Ascending => &ctx.labels.ascending,
                SortDir::Descending => &ctx.labels.descending,
            };
            about.push(format!(
                "{}, {dir}",
                fill(&ctx.labels.sort_key, &(k + 1).to_string())
            ));
        }
        let primary = (place == Some(0)).then(|| v.spec[0].1);
        let title = titled.then(|| {
            text(name.clone())
                .key("title")
                .font(p.style.header_font)
                .color(if primary.is_some() {
                    p.style.text
                } else {
                    p.style.header_text
                })
                .truncate()
                .shrink()
                .min_w(0.0)
                .help_when_truncated(name.clone())
        });
        let mut line = row().key("line").gap(2.0).center_y().min_w(0.0).w_full();
        if let Some(t) = title {
            line = line.child(t);
        }
        if let Some(d) = primary {
            let glyph = if d == SortDir::Ascending {
                p.caret_up
            } else {
                p.caret_down
            };
            line = line.child(icon(glyph, 14.0).key("sort").color(p.style.text));
        }
        let inner = col()
            .key("inner")
            .w_full()
            .min_w(0.0)
            .child(
                text(letters)
                    .key("letters")
                    .font(p.letters)
                    .color(p.style.muted),
            )
            .child(line)
            .a11y_hidden();
        let mut cell = col()
            .key(format!("{key}-h{j}"))
            .role("columnheader")
            .label(name.clone())
            .sorted(primary.map(|d| d == SortDir::Ascending))
            .w(w)
            .h_full()
            .px(p.style.cell_pad)
            .justify_center()
            .border_sides([0.0, 1.0, 0.0, 0.0], p.rule)
            .cursor(Cursor::Pointer)
            .hover(p.hover)
            .child(inner)
            .child(divider(ctx, &format!("{key}-h{j}-resize"), j, w, &name));
        if !about.is_empty() {
            cell = cell.description(about.join(", "));
        }
        let get_mut = ctx.get_mut;
        cell = cell.on_press_tx(move |s: &mut S, tx: &mut Tx| get_mut(s).click_title(j, tx));
        strip = strip.child(cell);
    }
    let corner = col()
        .key(format!("{key}-corner"))
        .role("columnheader")
        .label(ctx.labels.row.clone())
        .w(ctx.cols.gutter)
        .h_full()
        .border_sides([0.0, 1.0, 0.0, 0.0], p.rule);
    row()
        .key(format!("{key}-header"))
        .role("row")
        .w_full()
        .h(p.header_h)
        .fill(p.head_fill)
        .border_sides([0.0, 0.0, 1.0, 0.0], p.rule)
        .clip_soft(0.0)
        .child(corner)
        .child(
            row()
                .key(format!("{key}-titles"))
                .role("none")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .clip_soft(0.0)
                .child(strip),
        )
}

/// Column `j`'s divider: a drag resizes it (every column, the last included), a double click
/// puts it back to its fitted width (D7). Its value is the width, so assistive technology and
/// scripts hear a resize.
fn divider<S: 'static>(ctx: &Rc<Ctx<S>>, key: &str, j: usize, w: f32, name: &str) -> El {
    let get_mut = ctx.get_mut;
    row()
        .key(key.to_owned())
        .role("separator")
        .label(fill(&ctx.labels.resize, name))
        .range_value(
            f64::from(w.round()),
            f64::from(MIN_WIDTH),
            f64::from(MAX_WIDTH),
        )
        .a11y_value(format!("{}", w.round()))
        .pin(None, Some(0.0), Some(-4.0), Some(0.0))
        .w(9.0)
        .cursor(Cursor::ResizeColumn)
        .on_drag(move |s: &mut S, d: &mut Drag, _tx: &mut Tx| {
            let st = get_mut(s);
            match d.phase {
                Phase::Down => d.data = st.column_width(j).unwrap_or(MIN_WIDTH),
                Phase::Move | Phase::Up => {
                    let to = if d.cancelled() { d.data } else { d.data + d.dx };
                    st.set_column_width(j, to);
                }
            }
        })
        .on_double_press(move |s: &mut S| get_mut(s).fit_column(j))
}

/// Display row `i`: its row number, kept at the left edge as the rows scroll sideways, then
/// its cells.
pub(crate) fn build_row<S: 'static>(ctx: &Ctx<S>, s: &S, i: usize) -> El {
    let st = (ctx.get)(s);
    let p = &ctx.paint;
    let Some(v) = st.view() else {
        return row();
    };
    let Some(data) = v.data.as_ref() else {
        return row();
    };
    let src = v.order[i] as usize;
    let cells = &data.body()[src];
    let number = src + 1 + usize::from(data.header);
    let range = v.range();
    let cursor = v.sel.map(|s| s.cursor);
    let row_key = &v.row_keys[i];
    let mut r = row()
        .key(row_key.clone())
        .role("row")
        .w(ctx.cols.width())
        .h(p.style.row_height)
        .border_sides([0.0, 0.0, 1.0, 0.0], p.rule)
        .child(
            row()
                .key(format!("{row_key}-n"))
                .role("rowheader")
                .label(number.to_string())
                .w(ctx.cols.gutter)
                .h_full()
                .px(p.style.cell_pad)
                .center_y()
                .justify_end()
                .fill(p.head_fill)
                .border_sides([0.0, 1.0, 0.0, 0.0], p.rule)
                .sticky_left(0.0)
                .child(
                    text(number.to_string())
                        .font(p.numbers)
                        .color(p.style.muted)
                        .a11y_hidden(),
                ),
        );
    r = r.child(lead(&ctx.cols, &ctx.built));
    for j in ctx.built.clone() {
        let (a, b) = ctx.cols.span(j);
        let here = super::range::CellPos::new(i, j);
        let selected = range.is_some_and(|r| r.contains(here));
        // Keys unique in the window, so the grid's active descendant names one cell.
        let mut c = row()
            .key(format!("{row_key}-c{j}"))
            .role("gridcell")
            .w(b - a)
            .h_full()
            .px(p.style.cell_pad)
            .center_y()
            .border_sides([0.0, 1.0, 0.0, 0.0], p.rule)
            .focus_selected(selected);
        if selected {
            c = c.fill(p.selection);
        }
        if cursor == Some(here) {
            c = c.outline(1.0, -1.0, p.focus);
        }
        if let Some(t) = cells.get(j).filter(|t| !t.is_empty()) {
            let (show, cut) = shown(t);
            let show = show.into_owned();
            if cut {
                c = c.description(ctx.labels.shortened.clone());
            }
            c = c.child(
                text(show)
                    .font(p.style.font)
                    .color(p.style.text)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        }
        r = r.child(c);
    }
    r
}

/// The room the columns before the built ones take.
fn lead(cols: &Columns, built: &std::ops::Range<usize>) -> El {
    let w = cols.edges.get(built.start).map_or(0.0, |x| x - cols.gutter);
    rune::el().w(w).h_full().a11y_hidden()
}

/// A sheet that couldn't be read, in the grid's place: Strata's "Preview unavailable" over
/// the reason, and Try again, which asks for it once more.
pub(crate) fn failure<S: 'static>(
    look: &Look,
    key: &str,
    labels: &SheetLabels,
    reason: &str,
    retry: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let k = &look.kit;
    col()
        .key(format!("{key}-failed"))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .center()
        .gap(look.space.sm)
        .px(look.space.md)
        .child(
            text(labels.failed.clone())
                .font(k.ty.ui)
                .color(k.c.text_base),
        )
        .child(
            text(reason.to_owned())
                .wrap()
                .font(k.ty.text_small)
                .color(k.c.text_muted),
        )
        .child(
            Button::new(labels.retry.clone())
                .variant(ButtonVariant::Neutral)
                .on_press_tx(retry)
                .build(k)
                .key(format!("{key}-retry")),
        )
}

/// A line in the grid's place: the sheet loading, or empty.
pub(crate) fn placeholder(look: &Look, key: String, what: &str) -> El {
    let k = &look.kit;
    col().key(key).w_full().grow(1.0).min_h(0.0).center().child(
        text(what.to_owned())
            .font(k.ty.text_small)
            .color(k.c.text_muted),
    )
}
