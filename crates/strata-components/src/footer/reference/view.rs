//! The reference's panel: the header with the search, the categories, the results and the note.

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{
    Align, El, GrowthSpec, KeyEvent, LiveRegion, ScrollSpec, Tx, col, el, row, text, text_input,
};
use strata_kit::look::Look;

use super::keys::Route;
use super::search::{Index, Line, Lines, fold_query};
use super::{RESULTS_HEIGHT, SIDEBAR_WIDTH, ShortcutReference};
use crate::contract::MatchedText;
use crate::filter_bar::keycap;

/// Strata's reference look (`src/style.css`): a chosen category on the accent at 16 % with a
/// 3 px accent bar at its left, a hovered one at 10 %; compact pills with a border at 40 % of the
/// border colour, a chosen pill on the accent at 20 % with an accent border. No Kit or
/// strata-kit token holds them (a W2 `footer.*` slot is a follow-up).
pub const CATEGORY_CHOSEN_ALPHA: f32 = 0.16;
pub const CATEGORY_HOVER_ALPHA: f32 = 0.1;
pub const CATEGORY_BAR: f32 = 3.0;
pub const PILL_CHOSEN_ALPHA: f32 = 0.2;
pub const PILL_BORDER_ALPHA: f32 = 0.4;
/// The gap between the two columns of rows (Strata's grid `column_spacing(48)`).
pub const COLUMN_GAP: f32 = 48.0;
/// A line's height before it measures, for the virtual rows.
const LINE_ESTIMATE: f32 = 28.0;

/// The panel's content: one column, whose key handler routes the categories' and the results'
/// keys.
pub(super) fn panel<S: 'static>(r: &ShortcutReference<S>, s: &S, look: &Look) -> Vec<El> {
    let state = (r.get)(s);
    let data = (r.data)(s);
    let k = &look.kit;
    let compact = state.width.is_none_or(|w| w < r.compact_below);
    let lines = state.lines(data, if compact { 1 } else { 2 });
    let index = state.index();
    let chosen = state.section(data);
    let key = r.key.clone();
    let line_key: Rc<dyn Fn(usize) -> String> = {
        let key = key.clone();
        let lines = lines.clone();
        Rc::new(move |i| line_key(&key, &lines, i))
    };
    let route = Rc::new(Route {
        get_mut: r.get_mut,
        keys: r.keys.clone(),
        categories: format!("{key}-category-"),
        search: r.search_key(),
        // Category 0 is All, then each section in order.
        chosen_category: r.category_key(chosen.map_or(0, |c| c + 1)),
        results: r.results_key(),
        line_key,
        lines: lines.clone(),
    });

    let header = header(r, s, look, compact, &route);
    let categories = categories(r, s, look, compact, chosen);
    let results = results(r, s, look, compact, lines, index, &route);
    let body = if compact {
        col()
            .key(format!("{key}-body"))
            .w_full()
            .h(RESULTS_HEIGHT)
            .shrink()
            .min_h(0.0)
            .child(categories)
            .child(results)
    } else {
        row()
            .key(format!("{key}-body"))
            .w_full()
            .h(RESULTS_HEIGHT)
            .shrink()
            .min_h(0.0)
            .child(categories)
            .child(results)
    };
    let note = note(r, look);
    let on_key = route.clone();
    let get_mut = r.get_mut;
    vec![
        col()
            .key(format!("{key}-content"))
            .w_full()
            .shrink()
            .min_h(0.0)
            .child(header)
            .child(body)
            .child(note)
            .on_layout(move |s: &mut S, rect: [f32; 4]| {
                let st = get_mut(s);
                if st.height != Some(rect[3]) {
                    st.height = Some(rect[3]);
                }
            })
            .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| on_key.panel_key(s, ev, tx))
            .fill(k.c.layer_01),
    ]
}

fn line_key(key: &str, lines: &Lines, i: usize) -> String {
    match lines.lines.get(i) {
        Some(Line::Heading { section, .. }) => format!("{key}-heading-{section}"),
        Some(Line::Rows { section, first, .. }) => format!("{key}-line-{section}-{first}"),
        None => format!("{key}-line-none-{i}"),
    }
}

/// The title and the search field: side by side, or stacked when compact. Reports the panel's
/// width, which picks the layout.
fn header<S: 'static>(
    r: &ShortcutReference<S>,
    s: &S,
    look: &Look,
    compact: bool,
    route: &Rc<Route<S>>,
) -> El {
    let k = &look.kit;
    let state = (r.get)(s);
    let get_mut = r.get_mut;
    let title = text(r.labels.title.clone())
        .font(look.ty.title)
        .color(k.c.text_base)
        .a11y_hidden();
    let on_key = route.clone();
    let field = text_input(&state.search, k.ty.ui)
        .single_line()
        .key(r.search_key())
        .label(r.labels.search.clone())
        .placeholder(r.labels.search.clone(), k.c.text_faint)
        .color(k.c.text_base)
        .caret(k.c.text_base, k.selection())
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .on_edit(move |s: &mut S| &mut get_mut(s).search)
        .on_input_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| on_key.field_key(s, ev, tx));
    let frame = row()
        .key(format!("{}-frame", r.key))
        .h(look.frame.control)
        .min_w(SEARCH_MIN_WIDTH)
        .grow(1.0)
        .px(look.space.sm)
        .items_center()
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .child(field);
    let frame = if state.search.focused {
        frame.outline(1.0, -1.0, k.c.border_focus)
    } else {
        k.elevate(frame, k.elevation.input())
    };
    let base = if compact { col() } else { row().items_center() };
    let get_mut = r.get_mut;
    base.key(format!("{}-header", r.key))
        .w_full()
        .gap(look.space.lg)
        .px(look.space.xl + look.space.xs)
        .py(look.space.lg + look.space.xxs)
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_base)
        .on_layout(move |s: &mut S, rect: [f32; 4]| {
            let st = get_mut(s);
            if st.width != Some(rect[2]) {
                st.width = Some(rect[2]);
            }
        })
        .child(title.grow(if compact { 0.0 } else { 1.0 }))
        .child(frame)
}

/// The compact category pills' height limits (`src/ui/shortcut_footer/reference.rs`).
const PILLS_MIN: f32 = 60.0;
const PILLS_MAX: f32 = 160.0;

/// The search field's narrowest width (`.shortcut-reference-search`'s `min-width: 150px`).
const SEARCH_MIN_WIDTH: f32 = 150.0;

/// All, then each section: a column of rows with counts beside the results, or wrapping pills
/// above them when compact. A tab list: arrows, Home and End choose, and Tab visits the chosen
/// one only.
fn categories<S: 'static>(
    r: &ShortcutReference<S>,
    s: &S,
    look: &Look,
    compact: bool,
    chosen: Option<usize>,
) -> El {
    let k = &look.kit;
    let data = (r.data)(s);
    let total: usize = data.sections.iter().map(|s| s.rows.len()).sum();
    let entries = std::iter::once((r.labels.all.clone(), total, None)).chain(
        data.sections
            .iter()
            .enumerate()
            .map(|(i, s)| (s.title.clone(), s.rows.len(), Some(i))),
    );
    let tabs = entries.enumerate().map(|(i, (title, count, section))| {
        let selected = section == chosen;
        let get_mut = r.get_mut;
        let results = r.results_key();
        let pick = section.map(|si| data.sections[si].title.clone());
        let base = row()
            .key(r.category_key(i))
            .role("tab")
            .label(title.clone())
            .description((r.labels.count)(count))
            .focusable(true)
            .focus_selected(selected)
            .items_center()
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                get_mut(s).set_category(pick.clone());
                tx.scroll_to_instant(results.clone(), 0.0, 0.0);
            });
        if compact {
            base.h(look.frame.control - look.space.xxs)
                .px(look.space.md + look.space.xxs / 2.0)
                .radius(look.frame.control)
                .fill(if selected {
                    k.alpha(k.c.text_accent, PILL_CHOSEN_ALPHA)
                } else {
                    k.c.layer_01
                })
                .border(
                    1.0,
                    if selected {
                        k.c.text_accent
                    } else {
                        k.alpha(k.c.border_base, PILL_BORDER_ALPHA)
                    },
                )
                .hover(k.alpha(k.c.text_accent, CATEGORY_HOVER_ALPHA))
                .child(
                    text(title)
                        .font(k.ty.small)
                        .color(if selected {
                            k.c.text_base
                        } else {
                            k.c.text_muted
                        })
                        .a11y_hidden(),
                )
        } else {
            let tab = base
                .w_full()
                .px(look.space.xl + look.space.xxs)
                .py(look.space.md + 1.0)
                .gap(look.space.md)
                .hover(k.alpha(k.c.text_accent, CATEGORY_HOVER_ALPHA))
                .child(
                    text(title)
                        .font(k.ty.small)
                        .color(if selected {
                            k.c.text_base
                        } else {
                            k.c.text_muted
                        })
                        .grow(1.0)
                        .a11y_hidden(),
                )
                .child(
                    text(count.to_string())
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .a11y_hidden(),
                );
            if selected {
                tab.fill(k.alpha(k.c.text_accent, CATEGORY_CHOSEN_ALPHA))
                    .border_sides([0.0, 0.0, 0.0, CATEGORY_BAR], k.c.text_accent)
            } else {
                tab
            }
        }
    });
    let list = if compact { row().flex_wrap() } else { col() };
    let list = list
        .key(format!("{}-categories", r.key))
        .role("tablist")
        .label(r.labels.categories.clone())
        .focus_group()
        .children(tabs);
    if compact {
        // At most a quarter of the panel's height, 60 to 160 pt, scrolling past it (Strata's
        // `max_sidebar_height`: a quarter of the window, 60 to 160 px).
        let cap = (r.get)(s)
            .height
            .map_or(PILLS_MAX, |h| (h / 4.0).clamp(PILLS_MIN, PILLS_MAX));
        col()
            .w_full()
            .max_h(cap)
            .shrink()
            .min_h(0.0)
            .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_base)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                list.w_full()
                    .gap(look.space.md)
                    .px(look.space.lg)
                    .py(look.space.md + look.space.xxs),
            )
    } else {
        // A column of its own that scrolls when the categories outgrow it.
        col()
            .w(SIDEBAR_WIDTH)
            .h_full()
            .min_h(0.0)
            .border_sides([0.0, 1.0, 0.0, 0.0], k.c.border_base)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(list.w_full().py(look.space.lg))
    }
}

/// The results: a focusable table of lines (a heading, or one or two rows), in virtual rows,
/// with the announced count; or "No matching shortcuts".
fn results<S: 'static>(
    r: &ShortcutReference<S>,
    s: &S,
    look: &Look,
    compact: bool,
    lines: Rc<Lines>,
    index: Rc<Index>,
    route: &Rc<Route<S>>,
) -> El {
    let k = &look.kit;
    let state = (r.get)(s);
    let results_key = r.results_key();
    let (px, py) = if compact {
        (look.space.xl, look.space.lg)
    } else {
        (look.space.xxl, look.space.xl + look.space.xs)
    };
    let view = col()
        .key(results_key.clone())
        .role("table")
        .label(r.labels.results.clone())
        .focusable(true)
        .no_focus_ring()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    // The announced count. Silent while the query is empty: the categories describe their own
    // counts, so choosing one is heard once.
    let quiet = state.query().trim().is_empty();
    let status = |words: String| {
        let e = el()
            .key(format!("{}-status", r.key))
            .role("status")
            .live_region(LiveRegion::Polite)
            .absolute(0.0, 0.0)
            .w(0.0)
            .h(0.0);
        if quiet { e } else { e.label(words) }
    };
    if lines.matches == 0 {
        let empty = col().w_full().h_full().min_h(0.0).center().px(px).child(
            text(r.labels.no_match.clone())
                .font(k.ty.small)
                .color(k.c.text_muted)
                .wrap()
                .align(Align::Center)
                .a11y_hidden(),
        );
        let _ = route;
        return frame(view.child(empty), status(r.labels.no_match.clone()));
    }
    let count = lines.lines.len();
    let mut version = std::collections::hash_map::DefaultHasher::new();
    (r.data)(s).revision.hash(&mut version);
    fold_query(state.query()).hash(&mut version);
    state.section((r.data)(s)).hash(&mut version);
    compact.hash(&mut version);
    let version = version.finish();
    let query = fold_query(state.query());
    let look = Rc::new(look.clone());
    let data = r.data;
    let key = r.key.clone();
    let keys_lines = lines.clone();
    let get_mut = r.get_mut;
    let words = r.labels.count.clone();
    let rows = col()
        .key(format!("{}-rows", r.key))
        .role("rowgroup")
        .w_full()
        .px(px)
        .py(py)
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            LINE_ESTIMATE,
            move |_s: &S, i| line_key(&key, &keys_lines, i).into(),
            {
                let key = r.key.clone();
                move |s: &S, i| {
                    let at = Placed {
                        key: &key,
                        look: &look,
                        query: &query,
                        compact,
                    };
                    line(&at, (data)(s), &lines, &index, &*words, i)
                }
            },
        )
        .virtual_rows_version(version)
        .on_visible_rows(move |s: &mut S, visible| {
            get_mut(s).visible = visible;
            false
        });
    frame(
        view.child(rows),
        status((r.labels.count)(route.lines.matches)),
    )
}

/// The results' viewport beside its announced count, which stays out of the scroll content (a
/// growth stack must be its viewport's only child).
fn frame(viewport: El, status: El) -> El {
    col()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .child(viewport)
        .child(status)
}

/// What every line shares: the reference's key, the look, the folded query and the layout.
struct Placed<'a> {
    key: &'a str,
    look: &'a Look,
    query: &'a str,
    compact: bool,
}

/// Line `i`: a heading, or its rows.
fn line(
    at: &Placed<'_>,
    data: &super::ShortcutData,
    lines: &Lines,
    index: &Index,
    words: &dyn Fn(usize) -> String,
    i: usize,
) -> El {
    let Placed {
        key, look, compact, ..
    } = *at;
    let k = &look.kit;
    match lines.lines[i] {
        Line::Heading { section, shown } => {
            let title = &data.sections[section].title;
            let top = if i == 0 { 0.0 } else { look.space.xxl };
            row()
                .key(format!("{key}-heading-{section}"))
                .role("none")
                .w_full()
                .pt(top)
                .pb(look.space.xl)
                .gap(look.space.lg)
                .items_center()
                .child(
                    text(title.to_uppercase())
                        .key(format!("{key}-heading-{section}-title"))
                        .role("rowheader")
                        .label(title.clone())
                        .description(words(shown))
                        .font(look.ty.heading)
                        .color(k.c.text_accent),
                )
                .child(
                    text(shown.to_string())
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .a11y_hidden(),
                )
                .child(el().h(1.0).grow(1.0).fill(k.c.text_accent).a11y_hidden())
        }
        Line::Rows {
            section,
            first,
            second,
        } => {
            let one = |ri: usize| {
                let ordinal = lines.ordinal[section][ri].unwrap_or(0);
                shortcut(
                    at,
                    &data.sections[section].rows[ri],
                    &index.sections[section][ri],
                    (section, ri),
                    (ordinal + 1, lines.matches),
                )
            };
            let line = row()
                .key(format!("{key}-line-{section}-{first}"))
                .role("none")
                .w_full()
                .pb(look.space.lg)
                .equal_columns(if compact { 1 } else { 2 })
                .gaps(0.0, COLUMN_GAP)
                .child(one(first));
            match second {
                Some(ri) => line.child(one(ri)),
                None => line,
            }
        }
    }
}

/// One row: its action, then its keys (beside it, or under it when compact).
fn shortcut(
    at: &Placed<'_>,
    row_data: &super::Shortcut,
    row_index: &super::search::RowIndex,
    (section, ri): (usize, usize),
    (position, size): (usize, usize),
) -> El {
    let Placed {
        key,
        look,
        query,
        compact,
    } = *at;
    let k = &look.kit;
    let action = MatchedText::new(
        row_data.action.clone(),
        row_index.action.ranges(&row_data.action, query),
    )
    .font(k.ty.small)
    .color(k.c.text_base)
    .build(k)
    .role("cell")
    .wrap()
    .grow(1.0)
    .shrink()
    .min_w(0.0);
    let mut keys = row()
        .role("cell")
        .label(row_data.key_text())
        .flex_wrap()
        .items_center()
        .gap(look.space.xs);
    if let Some(c) = &row_data.context
        && !c.before.is_empty()
    {
        keys = keys.child(context(look, &c.before, &row_index.before, query));
    }
    let alternatives: Vec<&Vec<String>> =
        row_data.keys.0.iter().filter(|a| !a.is_empty()).collect();
    for (ai, alt) in alternatives.iter().enumerate() {
        if ai > 0 {
            keys = keys.child(
                text("/")
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .a11y_hidden(),
            );
        }
        for step in alt.iter() {
            keys = keys.child(keycap(look, step));
        }
    }
    if let Some(c) = &row_data.context
        && !c.after.is_empty()
    {
        keys = keys.child(context(look, &c.after, &row_index.after, query));
    }
    let base = if compact {
        col().gap(look.space.xs)
    } else {
        row().gap(look.space.sm).items_center()
    };
    base.key(format!("{key}-row-{section}-{ri}"))
        .role("row")
        .position_in_set(position, size)
        .w_full()
        .min_w(0.0)
        .child(action)
        .child(keys.shrink().min_w(0.0))
}

fn context(look: &Look, words: &str, folded: &super::search::Folded, query: &str) -> El {
    let k = &look.kit;
    MatchedText::new(words.to_owned(), folded.ranges(words, query))
        .font(k.ty.small)
        .color(k.c.text_muted)
        .build(k)
        .a11y_hidden()
}

/// The note: hint by hint, wrapping only between hints, each but the last followed by "·".
fn note<S: 'static>(r: &ShortcutReference<S>, look: &Look) -> El {
    let k = &look.kit;
    let n = r.labels.note.len();
    row()
        .key(format!("{}-note", r.key))
        .role("note")
        .label(r.labels.note.join(" \u{b7} "))
        .w_full()
        .flex_wrap()
        .gaps(0.0, look.space.sm)
        .px(look.space.xl + look.space.xs)
        .py(look.space.lg + look.space.xxs)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base)
        .children(r.labels.note.iter().enumerate().map(|(i, hint)| {
            let words = if i + 1 < n {
                format!("{hint} \u{b7}")
            } else {
                hint.clone()
            };
            text(words)
                .font(k.ty.small)
                .color(k.c.text_muted)
                .a11y_hidden()
        }))
}
