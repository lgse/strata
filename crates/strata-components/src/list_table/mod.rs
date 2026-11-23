// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod column;
mod groups;
mod header;
mod layout;
mod menu;
mod resolve;
mod rows;
mod sort;
mod state;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use rune::{El, ScrollPos, Tx, col, el};
use strata_kit::look::Look;

pub use column::{Cell, Column, ColumnId, WIDTH_CEILING};
pub use groups::groups_from_runs;
pub use header::{REORDER_THRESHOLD, RESIZE_EDGE, heading_key};
pub use layout::{ColumnLayout, ColumnWidths, DropSite, LayoutChange, LayoutWhy};
pub use rows::cell_key;
pub use sort::{SortDir, SortIntent, SortShown, next_direction};
pub use state::{ListTableState, MenuAt};

use crate::collection::element::{Collection, CollectionState, Roles, RowCx, Slot, viewport_path};
use crate::collection::pointer::PointerSettings;
use crate::contract::EntrySource;
use crate::scroll::{BarLook, Target, anchor};
use resolve::{Inputs, Resolved, resolve};

/// The words of the columns menu and its button, which the host translates; the defaults are English.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ListTableLabels {
    /// The Columns button's name and help ("Columns").
    pub button: String,
    /// The menu's heading ("COLUMNS").
    pub heading: String,
    /// Why the primary column's item is disabled.
    pub fixed: Option<String>,
    /// The item that restores the default columns ("Reset to default columns").
    pub reset: String,
}

impl Default for ListTableLabels {
    fn default() -> Self {
        Self {
            button: "Columns".into(),
            heading: "COLUMNS".into(),
            fixed: None,
            reset: "Reset to default columns".into(),
        }
    }
}

/// What the table reads from the host's state, in one accessor.
pub struct ListParts<'a> {
    /// The listing, as the collection shows it.
    pub source: &'a dyn EntrySource,
    /// The collection's state.
    pub collection: &'a CollectionState,
    /// The table's own state.
    pub table: &'a ListTableState,
    /// The column order and visibility, shared by every window and the chooser.
    pub layout: &'a ColumnLayout,
    /// The widths given by hand, in this scope.
    pub widths: &'a ColumnWidths,
    /// The host's sort, as the headings show it.
    pub sort: Option<SortShown>,
    /// The look.
    pub look: &'a Look,
}

type PartsFn<S> = Rc<dyn for<'a> Fn(&'a S) -> ListParts<'a>>;
type TableMutFn<S> = Rc<dyn Fn(&mut S) -> &mut ListTableState>;
type VersionFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> u64>;
type TooltipFn<S> = Rc<dyn Fn(&S, &RowCx<'_>, &dyn Fn() -> bool) -> Option<String>>;
type SortFn<S> = Rc<dyn Fn(&mut S, SortIntent, &mut Tx)>;
type LayoutFn<S> = Rc<dyn Fn(&mut S, LayoutChange, &mut Tx)>;
type WidthsFn<S> = Rc<dyn Fn(&mut S, ColumnWidths, &mut Tx)>;
type SidewaysFn<S> = Rc<dyn Fn(&S) -> Box<dyn Fn(ScrollPos)>>;

/// What the table was built with, shared by its parts' handlers.
pub(crate) struct Shared<S> {
    key: Rc<str>,
    columns: Rc<[Column<S>]>,
    parts: PartsFn<S>,
    table_mut: TableMutFn<S>,
    cells_version: Option<VersionFn<S>>,
    tooltip: Option<TooltipFn<S>>,
    label: Option<Slot<S>>,
    on_sort: Option<SortFn<S>>,
    on_layout: Option<LayoutFn<S>>,
    on_widths: Option<WidthsFn<S>>,
    sideways: Vec<SidewaysFn<S>>,
    labels: Rc<ListTableLabels>,
}

impl<S> Clone for Shared<S> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            columns: self.columns.clone(),
            parts: self.parts.clone(),
            table_mut: self.table_mut.clone(),
            cells_version: self.cells_version.clone(),
            tooltip: self.tooltip.clone(),
            label: self.label.clone(),
            on_sort: self.on_sort.clone(),
            on_layout: self.on_layout.clone(),
            on_widths: self.on_widths.clone(),
            sideways: self.sideways.clone(),
            labels: self.labels.clone(),
        }
    }
}

/// The factor widths are drawn at: the interface text size over base size.
pub fn text_scale(look: &Look) -> f32 {
    let f = look.kit.ty.ui.size / strata_kit::typography::BASE_PX;
    if f.is_finite() && f > 0.0 { f } else { 1.0 }
}

/// The List view's table: see the module docs.
pub struct ListTable<S> {
    shared: Shared<S>,
}

impl<S: 'static> ListTable<S> {
    /// A table keyed `key` (its heading strip and menus' keys start there), of `columns` in their default order.
    pub fn new(
        key: &str,
        columns: impl Into<Rc<[Column<S>]>>,
        parts: impl for<'a> Fn(&'a S) -> ListParts<'a> + 'static,
        table_mut: impl Fn(&mut S) -> &mut ListTableState + 'static,
    ) -> Self {
        Self {
            shared: Shared {
                key: key.into(),
                columns: columns.into(),
                parts: Rc::new(parts),
                table_mut: Rc::new(table_mut),
                cells_version: None,
                tooltip: None,
                label: None,
                on_sort: None,
                on_layout: None,
                on_widths: None,
                sideways: Vec::new(),
                labels: Rc::new(ListTableLabels::default()),
            },
        }
    }

    /// A version of what row `rcx`'s cells read beyond its entry (its size, date, type).
    pub fn cells_version(mut self, f: impl Fn(&S, &RowCx<'_>) -> u64 + 'static) -> Self {
        self.shared.cells_version = Some(Rc::new(f));
        self
    }

    /// Hover tooltip text for a row's primary (Name) cell.
    pub fn tooltip(
        mut self,
        f: impl Fn(&S, &RowCx<'_>, &dyn Fn() -> bool) -> Option<String> + 'static,
    ) -> Self {
        self.shared.tooltip = Some(Rc::new(f));
        self
    }

    /// The words of the columns menu and its button, translated.
    pub fn labels(mut self, labels: ListTableLabels) -> Self {
        self.shared.labels = Rc::new(labels);
        self
    }

    /// The primary column's label (matched text, say), instead of the middle-truncated name.
    pub fn label(mut self, slot: Slot<S>) -> Self {
        self.shared.label = Some(slot);
        self
    }

    /// Hears a sort click: the host sorts.
    pub fn on_sort(mut self, f: impl Fn(&mut S, SortIntent, &mut Tx) + 'static) -> Self {
        self.shared.on_sort = Some(Rc::new(f));
        self
    }

    /// Hears a new column order or visibility: the host keeps it and shares it.
    pub fn on_layout(mut self, f: impl Fn(&mut S, LayoutChange, &mut Tx) + 'static) -> Self {
        self.shared.on_layout = Some(Rc::new(f));
        self
    }

    /// Hears new widths, once per resize or fit: the host keeps them for this scope.
    pub fn on_widths(mut self, f: impl Fn(&mut S, ColumnWidths, &mut Tx) + 'static) -> Self {
        self.shared.on_widths = Some(Rc::new(f));
        self
    }

    /// Hears the sideways scroll's position on every frame it moves, with no view commit, as `El::on_scroll_offset` would.
    pub fn follow_sideways<F: Fn(ScrollPos) + 'static>(
        mut self,
        f: impl Fn(&S) -> F + 'static,
    ) -> Self {
        self.shared.sideways.push(Rc::new(move |s: &S| {
            Box::new(f(s)) as Box<dyn Fn(ScrollPos)>
        }));
        self
    }

    /// The sideways scroll as autoscroll moves it.
    pub fn sideways_target(&self, s: &S) -> Target {
        Target::horizontal(
            self.scroll_key(),
            (self.shared.parts)(s).table.sideways_feed.clone(),
        )
    }

    /// The sideways scroll's key.
    fn scroll_key(&self) -> String {
        format!("{}-scroll", self.shared.key)
    }

    /// The columns, in their default order.
    pub fn columns(&self) -> &[Column<S>] {
        &self.shared.columns
    }

    /// A saved layout made whole for this table's columns (`ColumnLayout::normalize` with their default order and the primary column): what a host loads `list_column_order` and `list_hidden_columns` with.
    pub fn normalize<'a>(
        &self,
        order: impl IntoIterator<Item = &'a str>,
        hidden: impl IntoIterator<Item = &'a str>,
    ) -> ColumnLayout {
        let defaults: Vec<ColumnId> = self.shared.columns.iter().map(|c| c.id.clone()).collect();
        let primary = self
            .shared
            .columns
            .iter()
            .find(|c| c.is_primary())
            .map(|c| &c.id);
        ColumnLayout::normalize(&defaults, primary, order, hidden)
    }

    /// The default layout: every column, in the order given, shown.
    pub fn default_layout(&self) -> ColumnLayout {
        ColumnLayout::new(self.shared.columns.iter().map(|c| c.id.clone()))
    }

    /// The columns as `s` lays them out now.
    fn resolved(&self, s: &S) -> Rc<Resolved> {
        let parts = (self.shared.parts)(s);
        let look = parts.look;
        let lead = strata_kit::slots::collection::row_inset(look) + look.space.xs;
        let live = parts.table.resize.as_ref().map(|r| (&r.column, r.live));
        Rc::new(resolve(
            &self.shared.columns,
            &Inputs {
                layout: parts.layout,
                widths: parts.widths,
                live,
                viewport: parts.table.width,
                lead,
                scale: text_scale(look),
            },
        ))
    }

    /// `base` with the table's parts: its roles, row height, label, cells and hit regions.
    pub fn collection(&self, s: &S, base: Collection<S>) -> Collection<S> {
        let shared = Rc::new(self.shared.clone());
        let resolved = self.resolved(s);
        self.collection_with(s, base, &shared, &resolved)
    }

    /// [`ListTable::collection`] with the columns as this build laid them out.
    fn collection_with(
        &self,
        s: &S,
        base: Collection<S>,
        shared: &Rc<Shared<S>>,
        resolved: &Rc<Resolved>,
    ) -> Collection<S> {
        let region_shared = shared.clone();
        let region_resolved = resolved.clone();
        let table = (self.shared.parts)(s).table;
        let (bar, feed) = (table.bar.clone(), table.feed.clone());
        let edge_bar = bar.clone();
        let (bar_key, target) = (
            format!("{}-bar", self.shared.key),
            viewport_path(base.scope()),
        );
        let edge_shared = shared.clone();
        let pointer = base
            .setting::<PointerSettings>()
            .copied()
            .unwrap_or_default()
            .drag_selected_rows(false);
        base.roles(Roles::LIST)
            .settings(pointer)
            .on_scroll(move |p| {
                feed.set(p);
                bar.follow(p);
            })
            // Inside the sideways scroll, kept to its visible edge (`Bar::follow_across`).
            .edge(move |s: &S| {
                let look = BarLook::list((edge_shared.parts)(s).look);
                Some(edge_bar.build::<S>(&bar_key, &target, look))
            })
            .row_height(|_: &S, look: &Look| strata_kit::slots::list::row_height(look))
            .row_gap_y(1.0)
            .label(self.shared.label.clone().unwrap_or_else(rows::label))
            .content(rows::content(
                shared,
                resolved,
                rows::Fonts::of((self.shared.parts)(s).look),
            ))
            .regions(move |s: &S, index, local| {
                let parts = (region_shared.parts)(s);
                if index >= parts.source.len() {
                    return crate::collection::element::Region::Gutter;
                }
                let name = parts.source.entry(index).name;
                rows::region(&region_resolved, parts.look, name, local)
            })
    }

    /// The table: `base` with its parts and the heading strip.
    pub fn build(&self, s: &S, base: Collection<S>, layer: &mut Vec<El>) -> El {
        (self.shared.parts)(s).table.end_frame();
        let shared = Rc::new(self.shared.clone());
        let resolved = self.resolved(s);
        let strip = header::strip(&shared, &resolved, s);
        let strip = menu::headings(&shared, s, strip, layer);
        let header = Rc::new(RefCell::new(Some(strip)));
        let parts = (shared.parts)(s);
        let width = parts.table.width;
        let (bar, sideways_bar) = (parts.table.bar.clone(), parts.table.sideways_bar.clone());
        let sideways_look = BarLook::app(parts.look).horizontal();
        let sideways_overflows =
            width >= sideways_look.min_length && resolved.row_width() - width > 0.5;
        let bottom_pad = if sideways_overflows {
            sideways_look.edge + sideways_look.band()
        } else {
            0.0
        };
        let list = self
            .collection_with(s, base, &shared, &resolved)
            .header(move |_: &S| header.borrow_mut().take())
            .viewport_bottom_pad(bottom_pad)
            .build(s);
        let sideways = sideways_bar.build::<S>(
            &format!("{}-sideways-bar", self.shared.key),
            &self.scroll_key(),
            sideways_look,
        );
        let scroll = self.sideways_scroll(s, &shared, resolved.row_width().max(width), list);
        let content = col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .min_w(0.0)
            .clip_soft(0.0)
            .child(self.backdrop(parts.look))
            .child(scroll)
            .child(anchor(0.0, 0.0, sideways));
        bar.area(sideways_bar.area(content))
    }

    fn sideways_scroll(&self, s: &S, shared: &Rc<Shared<S>>, inner_width: f32, list: El) -> El {
        let parts = (shared.parts)(s);
        let (follow_bar, follow_sideways) =
            (parts.table.bar.clone(), parts.table.sideways_bar.clone());
        let sideways_feed = parts.table.sideways_feed.clone();
        let followers: Vec<Box<dyn Fn(ScrollPos)>> = shared.sideways.iter().map(|f| f(s)).collect();
        let layout_shared = shared.clone();
        let inner = col().w(inner_width).h_full().min_h(0.0).instant_changes();
        col()
            .key(self.scroll_key())
            .w_full()
            .h_full()
            .min_h(0.0)
            .scroll_x()
            .on_scroll_offset(move |p: ScrollPos| {
                sideways_feed.set(p);
                follow_sideways.follow(p);
                follow_bar.follow_across(p);
                for f in &followers {
                    f(p);
                }
            })
            .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
                let st = (layout_shared.table_mut)(s);
                if st.width == r[2] {
                    tx.live_only();
                } else {
                    st.width = r[2];
                }
            })
            .child(inner.child(list))
    }

    fn backdrop(&self, look: &Look) -> El {
        let kit = &look.kit;
        let heading_h = strata_kit::slots::list::heading_height(look);
        col()
            .key(format!("{}-backdrop", self.shared.key))
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .a11y_hidden()
            .inert()
            .child(
                el().key(format!("{}-header-backdrop", self.shared.key))
                    .w_full()
                    .h(heading_h)
                    .fill(kit.c.bg_deep)
                    .border_sides([0.0, 0.0, 1.0, 0.0], kit.c.border_base)
                    .a11y_hidden()
                    .inert(),
            )
            .child(
                el().key(format!("{}-body-backdrop", self.shared.key))
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .fill(kit.c.bg_base)
                    .a11y_hidden()
                    .inert(),
            )
    }

    /// The Columns button for the pane header, with the columns menu under it (mounted into `layer`).
    pub fn columns_button(&self, s: &S, layer: &mut Vec<El>) -> El {
        menu::button(&Rc::new(self.shared.clone()), s, layer)
    }

    /// The width the column `id` would fit its heading and the cells on screen at, in base points.
    pub fn fit_width(&self, s: &S, id: &ColumnId) -> Option<f32> {
        let def = self.shared.columns.iter().position(|c| c.id == *id)?;
        Some(header::fit(&self.shared, &self.resolved(s), s, def))
    }
}
