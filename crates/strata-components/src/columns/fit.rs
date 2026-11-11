//! What the strip tells C9's drawer about itself in Columns ([`ColumnsFit`]).

use strata_kit::look::Look;

use super::sources::ColumnSources;
use super::state::ColumnsState;
use super::width;
use crate::preview_drawer::{ColumnsFit, DrawerRules};

/// The strip's fit for the drawer's rules: the columns from the root through the active one,
/// the width that keeps the active one whole (at most one standard column for the
/// narrow-window test), and the columns after it, which borrow the empty slot's width; one
/// standard column at the text size.
pub(super) fn columns_fit(st: &ColumnsState, look: &Look, rules: &DrawerRules) -> ColumnsFit {
    let spans = st.spans(look);
    let standard = width::drawn(width::STANDARD, width::scale(look));
    let Some(&(left, right)) = spans.get(st.active.min(spans.len().saturating_sub(1))) else {
        return ColumnsFit::new(rules, 0.0, 0.0).standard_column(standard);
    };
    let total = spans.last().map_or(0.0, |s| s.1);
    let active = right - left;
    ColumnsFit::new(rules, right, active)
        .keep_focused_standard(active.min(standard))
        .trailing(total - right)
        .standard_column(standard)
}

/// How wide column `depth`'s content is drawn, in points: the widest of its header's title
/// and the rows on screen (Strata measures its bound rows' natural widths). A row is its
/// insets, its icon and its name; the badge sits over the name, as Strata's overlay does, so
/// it adds nothing. The host's header actions are elements and can't be measured.
pub(super) fn content_width(
    sources: &dyn ColumnSources,
    st: &ColumnsState,
    look: &Look,
    depth: usize,
) -> f32 {
    use strata_kit::slots::{collection, columns as tokens};
    let measure = |t: &str, font| rune::text::measure(t, &[], font, None).0;
    let Some(column) = st.columns.get(depth) else {
        return 0.0;
    };
    if depth >= sources.len() {
        return 0.0;
    }
    // The header's padding either side of its title; its line is a bottom border.
    let title = tokens::header_inset(look)
        + measure(sources.title(depth), tokens::header_title_font(look))
        + look.space.xs;
    let row_chrome =
        2.0 * collection::row_inset(look) + 2.0 * look.space.xs + look.icon_size.md + look.space.sm;
    let source = sources.source(depth);
    let rows = column.collection.visible_range();
    let names = (rows.start..rows.end.min(source.len()))
        .map(|i| row_chrome + measure(source.entry(i).name, look.kit.ty.ui_compact));
    names.fold(title, f32::max)
}
