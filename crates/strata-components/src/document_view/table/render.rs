// SPDX-License-Identifier: MIT

//! Interactive table rendering inside [`DocumentView`](super::super::DocumentView): sortable
//! column headers with accent sort indicators, column resize dividers (including the last
//! column), rich inline-styled selectable body cells in sorted row order, and a horizontal
//! scroll viewport with Strata's C24 overlay scrollbar.

use std::rc::Rc;

use rune::{Cursor, Drag, El, Phase, TextStyle, Tx, col, icon, row, text};
use rune_kit::SortDir;

use super::super::model::Cell;
use super::super::render::{Ctx, styled, styled_text};
use super::super::state::DocumentState;
use super::{CELL_PAD_X, CELL_PAD_Y, MAX_WIDTH, MIN_WIDTH, TableFonts};
use crate::scroll::{Bar, anchor};

/// Snapshot of a table's resolved layout and sort state for one build pass.
struct TableBuild<'a, S> {
    ctx: &'a Rc<Ctx<S>>,
    unit: usize,
    scroll_gen: u64,
    row_key: String,
    rows: &'a [Vec<Cell>],
    header: bool,
    widths: Vec<f32>,
    sort: Vec<(usize, SortDir)>,
    order: Vec<u32>,
    bar: Bar,
}

impl<S: 'static> TableBuild<'_, S> {
    fn total_width(&self) -> f32 {
        self.widths.iter().sum()
    }

    fn header_font(&self) -> TextStyle {
        TextStyle {
            weight: 700.0,
            ..self.ctx.paint.body
        }
    }

    fn column_title(&self, column: usize) -> (String, Option<&Cell>) {
        let cell = self
            .header
            .then(|| self.rows[0].get(column))
            .flatten()
            .filter(|c| !c.text.trim().is_empty());
        let name = match cell {
            Some(c) => c.text.clone(),
            None => fill_placeholder(&self.ctx.labels.column, &(column + 1).to_string()),
        };
        (name, cell)
    }
}

pub(crate) fn build<S: 'static>(
    ctx: &Rc<Ctx<S>>,
    st: &DocumentState,
    unit: usize,
    room: Option<f32>,
) -> El {
    let super::super::model::UnitKind::Table { rows, .. } = &st.document().units[unit].kind else {
        return col().role("table").w_full();
    };
    let p = &ctx.paint;
    let header_font = TextStyle {
        weight: 700.0,
        ..p.body
    };
    let inner_room = room.map(|w| (w - 2.0).max(0.0));
    let fonts = TableFonts {
        body: p.body,
        header: header_font,
    };
    let Some(snap) = st.prepare_table(unit, fonts, inner_room) else {
        return col().role("table").w_full();
    };
    if snap.widths.is_empty() {
        return col().role("table").w_full();
    }
    let tb = TableBuild {
        ctx,
        unit,
        scroll_gen: snap.scroll_gen,
        row_key: st.row_key(unit),
        rows,
        header: snap.header,
        widths: snap.widths,
        sort: snap.sort,
        order: snap.order,
        bar: snap.bar,
    };
    let total_w = tb.total_width();
    let mut grid = col().role("table").w(total_w);
    grid = grid.child(header_row(&tb));
    for (dr, &src_offset) in tb.order.iter().enumerate() {
        let src = usize::from(tb.header) + src_offset as usize;
        if let Some(cells) = tb.rows.get(src) {
            grid = grid.child(body_row(&tb, dr, src, cells));
        }
    }
    wrap_scrollable(&tb, grid)
}

fn header_row<S: 'static>(tb: &TableBuild<'_, S>) -> El {
    let p = &tb.ctx.paint;
    let bottom = if tb.order.is_empty() { 0.0 } else { 1.0 };
    let mut line = row()
        .role("row")
        .w(tb.total_width())
        .fill(p.table_head_fill)
        .border_sides([0.0, 0.0, bottom, 0.0], p.table_rule);
    for c in 0..tb.widths.len() {
        line = line.child(header_cell(tb, c));
    }
    line
}

fn header_cell<S: 'static>(tb: &TableBuild<'_, S>, c: usize) -> El {
    let p = &tb.ctx.paint;
    let w = tb.widths[c];
    let (name, cell) = tb.column_title(c);
    let place = tb.sort.iter().position(|&(col, _)| col == c);
    let primary = (place == Some(0)).then(|| tb.sort[0].1);
    let title_row = header_label_row(tb, &name, cell, primary);
    let right_border = if c + 1 < tb.widths.len() { 1.0 } else { 0.0 };
    let mut el = col()
        .key(format!("{}-th{c}", tb.row_key))
        .role("columnheader")
        .label(name.clone())
        .sorted(primary.map(|d| d == SortDir::Ascending))
        .w(w)
        .px(CELL_PAD_X)
        .py(CELL_PAD_Y + 1.0)
        .justify_center()
        .border_sides([0.0, right_border, 0.0, 0.0], p.table_rule)
        .cursor(Cursor::Pointer)
        .hover_group()
        .hover(p.table_head_hover)
        .focusable(true)
        .on_key(crate::preview_drawer::return_as_space::<S>)
        .child(title_row)
        .child(divider(tb, c, w, &name));
    if let Some(k) = place.filter(|&k| k > 0) {
        let dir = match tb.sort[k].1 {
            SortDir::Ascending => &tb.ctx.labels.ascending,
            SortDir::Descending => &tb.ctx.labels.descending,
        };
        let rank = fill_placeholder(&tb.ctx.labels.sort_key, &(k + 1).to_string());
        el = el.description(format!("{rank}, {dir}"));
    }
    let get_mut = tb.ctx.get_mut;
    let unit = tb.unit;
    el.on_press_tx(move |s: &mut S, tx: &mut Tx| {
        get_mut(s).click_table_header(unit, c, tx);
    })
}

fn header_label_row<S: 'static>(
    tb: &TableBuild<'_, S>,
    name: &str,
    cell: Option<&Cell>,
    primary: Option<SortDir>,
) -> El {
    let p = &tb.ctx.paint;
    let st = styled(cell.map_or(&[][..], |c| &c.spans[..]), p);
    let color = if primary.is_some() { p.accent } else { p.text };
    let label = text(name.to_owned())
        .spans(st.spans)
        .strikes(st.strikes)
        .font(tb.header_font())
        .color(color)
        .group_color(p.accent)
        .truncate()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .help_when_truncated(name.to_owned())
        .a11y_hidden();
    let mut line = row().w_full().min_w(0.0).gap(4.0).center_y().child(label);
    if let Some(dir) = primary {
        let glyph = match dir {
            SortDir::Ascending => p.sort_asc,
            SortDir::Descending => p.sort_desc,
        };
        line = line.child(
            icon(glyph, p.icon)
                .key("sort")
                .color(p.icon_accent)
                .a11y_hidden(),
        );
    }
    line
}

fn divider<S: 'static>(tb: &TableBuild<'_, S>, c: usize, w: f32, name: &str) -> El {
    let get_mut = tb.ctx.get_mut;
    let unit = tb.unit;
    let last = c + 1 == tb.widths.len();
    let (right_inset, handle_w) = if last { (0.0, 8.0) } else { (-4.0, 9.0) };
    row()
        .key(format!("{}-th{c}-resize", tb.row_key))
        .role("separator")
        .label(fill_placeholder(&tb.ctx.labels.resize, name))
        .range_value(
            f64::from(w.round()),
            f64::from(MIN_WIDTH),
            f64::from(MAX_WIDTH),
        )
        .a11y_value(format!("{}", w.round()))
        .pin(None, Some(0.0), Some(right_inset), Some(0.0))
        .w(handle_w)
        .cursor(Cursor::ResizeColumn)
        .keeps_text_selection()
        .on_drag(move |s: &mut S, d: &mut Drag, _tx: &mut Tx| {
            let st = get_mut(s);
            match d.phase {
                Phase::Down => d.data = st.begin_table_resize(unit, c),
                Phase::Move if d.dx != 0.0 => st.set_table_column_width(unit, c, d.data + d.dx),
                Phase::Move => {}
                Phase::Up if d.cancelled() || d.dx == 0.0 => st.cancel_table_resize(unit),
                Phase::Up => st.finish_table_resize(unit, c, d.data + d.dx),
            }
        })
        .on_double_press(move |s: &mut S| get_mut(s).fit_table_column(unit, c))
}

fn body_row<S: 'static>(tb: &TableBuild<'_, S>, dr: usize, src: usize, cells: &[Cell]) -> El {
    let p = &tb.ctx.paint;
    let mut line = row()
        .role("row")
        .w(tb.total_width())
        .hover(p.table_row_hover);
    if dr > 0 {
        line = line.border_sides([1.0, 0.0, 0.0, 0.0], p.table_rule);
    }
    let cols = tb.widths.len();
    for (c, &w) in tb.widths.iter().enumerate() {
        let cell = cells.get(c);
        let content = cell.map_or_else(String::new, |c| c.text.clone());
        let st = styled(cell.map_or(&[][..], |c| &c.spans[..]), p);
        let is_header = cell.is_some_and(|c| c.header);
        let (style, color) = if is_header {
            (tb.header_font(), p.accent)
        } else {
            (p.body, p.text)
        };
        let key = format!("r{dr}c{c}");
        let scope = format!("{}/r{src}c{c}", tb.unit);
        let t = styled_text(tb.ctx, content, st, (style, color), &key, scope, true);
        let right_border = if c + 1 < cols { 1.0 } else { 0.0 };
        line = line.child(
            col()
                .w(w)
                .px(CELL_PAD_X)
                .py(CELL_PAD_Y)
                .border_sides([0.0, right_border, 0.0, 0.0], p.table_rule)
                .role(if is_header { "columnheader" } else { "cell" })
                .child(t),
        );
    }
    line
}

fn wrap_scrollable<S: 'static>(tb: &TableBuild<'_, S>, grid: El) -> El {
    let p = &tb.ctx.paint;
    let scroll_key = format!("{}-tscroll-{}", tb.row_key, tb.scroll_gen);
    let bar_key = format!("{}-thbar-{}", tb.row_key, tb.scroll_gen);
    let bar_el = tb.bar.build::<S>(&bar_key, &scroll_key, p.bar_look);
    let reveal_bar = tb.bar.clone();
    let viewport = col()
        .key(scroll_key)
        .w_full()
        .scroll_x()
        .on_scroll_offset(tb.bar.watch())
        .child(grid);
    col()
        .key(format!("{}-table", tb.row_key))
        .w_full()
        .border(1.0, p.table_rule)
        .radius(p.radius)
        .fill(p.surface)
        .clip_soft(0.0)
        .on_pointer_move_any(move |_, tx| {
            tx.live_only();
            reveal_bar.reveal();
        })
        .child(viewport)
        .child(anchor(0.0, 0.0, bar_el))
}

fn fill_placeholder(template: &str, value: &str) -> String {
    template.replacen("{}", value, 1)
}
