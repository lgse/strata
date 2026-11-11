//! The card: rune-kit's `HoverCard` holding the folder's name, its first rows, and the more,
//! empty, loading or error line.

use std::rc::Rc;
use std::time::Duration;

use rune::{Align, El, LiveRegion, col, el, icon, row, text};
use rune_kit::{HoverCard, Placement, Spinner};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::{collection as rows_tokens, columns as tokens};

use super::place::WIDTH;
use super::{More, Origin, PeekStatus, PeekTarget, Shared};
use crate::contract::Entry;
use crate::thumbnails::picture;

/// `HoverCard`'s padding inside its surface on each side (`overlay.rs`, `padding(4.0)`): the
/// content is this much narrower than the card, so the card is [`WIDTH`] wide.
const CARD_PADDING: f32 = 4.0;

/// How many rows tall the body is at least, so the loading, empty and error states don't
/// shrink the card to its header: Strata's `presentation.stack.set_size_request(-1, 120)` is
/// four of its 30 pt `.file-row`s.
const BODY_MIN_ROWS: f32 = 4.0;

/// The line under the header as a share of the border colour: Strata's
/// `.peek-popover .column-header { border-bottom: 1px solid alpha(@strata_border, 0.24) }`
/// (`src/style.css`).
const HEADER_LINE_OPACITY: f32 = 0.24;

/// The key of the card's group, which assistive technology names "Folder peek".
pub const GROUP: &str = "folder-peek";

/// The open peek's card, anchored to its folder's row.
pub(super) fn card<S: 'static>(sh: &Rc<Shared<S>>, s: &S) -> Option<El> {
    let target = (sh.get)(s).target()?.clone();
    let look = (sh.look)(s);
    let content = body(sh, s, &target, look);
    let mut layer = Vec::new();
    // The trigger is the zero-width part C22 pins in the folder's row (`attach`), keyed as
    // `HoverCard` names its anchor; the wrapper it returns is not mounted.
    let _ = HoverCard::new(format!("{}-peek", target.collection))
        .placement(Placement::RightStart)
        .child(content)
        .open(true)
        .build(&look.kit, &mut layer);
    let surface = layer.pop()?;
    let hover = sh.clone();
    let shown = (sh.get)(s).lives.shown.clone();
    Some(
        surface
            .live_opacity(&shown)
            .hover_delays(Duration::ZERO, sh.behavior.close)
            .on_hover_change(move |s: &mut S, inside, tx| {
                let change = (hover.get_mut)(s).card_rest(inside);
                hover.changed(s, change, tx);
            }),
    )
}

fn body<S: 'static>(sh: &Rc<Shared<S>>, s: &S, target: &PeekTarget, look: &Look) -> El {
    let k = &look.kit;
    let c = (sh.contents)(s);
    let labels = &sh.labels;
    let loading = *c.status == PeekStatus::Loading;

    // The header and the body are plain containers: assistive technology hears the heading
    // and the list inside them, not two unnamed groups.
    let mut header = row()
        .w_full()
        .h(look.row.columns)
        .px(tokens::header_inset(look))
        .gap(look.space.xs)
        .items_center()
        .border_sides(
            [0.0, 0.0, tokens::edge_width(look), 0.0],
            tokens::fade(look, k.c.std_border, HEADER_LINE_OPACITY),
        )
        .child(
            text(c.title.to_owned())
                .key("peek-heading")
                .role("heading")
                .font(k.ty.ui_compact)
                .color(k.c.text_muted)
                .truncate_middle()
                .shrink()
                .grow(1.0)
                .min_w(0.0),
        );
    if loading {
        header = header.child(
            el().key("peek-loading")
                .role("progressbar")
                .label(labels.loading.clone())
                .child(
                    Spinner::new()
                        .size(look.icon_size.sm)
                        .build(k)
                        .a11y_hidden(),
                ),
        );
    }

    let n = c.entries.len();
    let shown = n.min(sh.behavior.rows);
    let body_min = BODY_MIN_ROWS * look.row.columns + 2.0 * look.space.xs;
    let mut list = col().w_full().min_h(body_min).py(look.space.xs);
    if shown > 0 {
        let rows = (0..shown).map(|i| entry_row(sh, s, look, c.entries.entry(i), i, shown));
        list = list.child(col().key("peek-rows").role("list").w_full().children(rows));
        let more = c
            .more
            .or_else(|| (n > shown).then(|| More::Exactly(n - shown)))
            .filter(|m| matches!(m, More::Exactly(n) | More::AtLeast(n) if *n > 0));
        if let Some(more) = more {
            list = list.child(
                text((labels.more)(more))
                    .key("peek-more")
                    .font(k.ty.ui_compact)
                    .color(k.c.text_muted)
                    .truncate()
                    .px(rows_tokens::row_inset(look) + look.space.sm)
                    .py(look.space.xxs),
            );
        }
    } else {
        match c.status {
            PeekStatus::Loading => {}
            PeekStatus::Ready => {
                list = list.child(
                    message(look, body_min).child(
                        text(labels.empty.clone())
                            .key("peek-empty")
                            .font(k.ty.ui_compact)
                            .color(k.c.text_muted),
                    ),
                );
            }
            PeekStatus::Error(why) => {
                let line = |key: &str, t: String| {
                    text(t)
                        .key(key)
                        .font(k.ty.ui_compact)
                        .color(k.c.danger.fg)
                        .align(Align::Center)
                        .wrap()
                        .shrink()
                };
                list = list.child(
                    message(look, body_min)
                        .child(line("peek-error", labels.unreadable.clone()))
                        .child(line("peek-error-message", why.clone())),
                );
            }
        }
    }

    let open = sh.clone();
    let mut group = col()
        .key(GROUP)
        .role("group")
        .label(labels.peek.clone())
        .w(WIDTH - 2.0 * CARD_PADDING)
        .child(header)
        .child(list)
        // A click anywhere on the card opens the folder; hidden from assistive technology so
        // the card reads as a group (a pressable group is a button to Rune).
        .child(
            el().key("peek-open")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .a11y_hidden()
                .on_press_tx(move |s: &mut S, tx| {
                    let target = (open.get)(s).target().cloned();
                    (open.get_mut)(s).close();
                    open.changed(s, super::Change::Closed, tx);
                    if let (Some(target), Some(f)) = (target, &open.on_open) {
                        f(s, target, tx);
                    }
                }),
        );
    if target.origin == Origin::Keyboard {
        group = group.live_region(LiveRegion::Polite);
    }
    group
}

/// Lines of text centred in the body: a plain container, so assistive technology hears the
/// lines, not an unnamed group around them.
fn message(look: &Look, body_min: f32) -> El {
    col()
        .w_full()
        .min_h(body_min - 2.0 * look.space.xs)
        .px(look.space.md)
        .gap(look.space.xxs)
        .center()
}

fn entry_row<S>(
    sh: &Shared<S>,
    s: &S,
    look: &Look,
    entry: Entry<'_>,
    i: usize,
    shown: usize,
) -> El {
    let k = &look.kit;
    let folder = entry.kind.is_folder_like();
    let size = look.icon_size.md;
    let glyph = look.icon_named(entry.icon).unwrap_or_else(|| {
        look.icon(if folder {
            IconName::Folder
        } else {
            IconName::File
        })
    });
    // Files a little fainter than folders, as Strata's 0.82 icon opacity.
    let mark = icon(glyph, size)
        .color(if folder {
            k.c.icon_base
        } else {
            k.c.icon_muted
        })
        .a11y_hidden();
    let mark = match (&sh.thumbs, entry.thumbnail) {
        (Some(store), Some(key)) if store(s).enabled() => {
            picture(&store(s).thumb(key), mark, size, size)
        }
        _ => row().w(size).h(size).center().a11y_hidden().child(mark),
    };
    let mut r = row()
        .key(format!("peek-row-{}", entry.id.get()))
        .role("listitem")
        .label(entry.name.to_owned())
        .position_in_set(i + 1, shown)
        .w_full()
        .h(look.row.columns)
        .px(rows_tokens::row_inset(look) + look.space.sm)
        .gap(look.space.sm)
        .items_center()
        .child(mark)
        .child(
            text(entry.name.to_owned())
                .a11y_hidden()
                .font(k.ty.ui_compact)
                .color(k.c.text_base)
                .truncate_middle()
                .shrink()
                .grow(1.0)
                .min_w(0.0),
        );
    if folder {
        r = r.child(
            icon(look.icon(IconName::ChevronRight), look.icon_size.sm)
                .color(k.c.icon_muted)
                .a11y_hidden(),
        );
    }
    r
}
