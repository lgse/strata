//! [`DockView`]: the dock's region in the overlay layer, its cards and its notices, and the
//! progress bar the dialog shares.

use std::rc::Rc;

use rune::{Align, El, LoopSpec, ScrollSpec, TextStyle, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::model::{Dock, DockIntent, DockNotice, EndKind, Op, Placement};

/// The card's and the notice's corner radius before roundness (`.file-operation-card`,
/// `.overlay-notice`: 12 px, `src/style.css`).
pub const CARD_RADIUS: f32 = 12.0;

/// A card's and a check notice's minimum width (`min-width: 280px`, `src/style.css`).
pub const CARD_MIN_WIDTH: f32 = 280.0;

/// A card's widest: Strata wraps the card's info and destination at 42 characters
/// (`wrapped_label(42)`, `src/ui/progress_dock.rs`), about 340 pt with its 16 px padding at
/// Strata's 0.85 em meta text.
pub const CARD_MAX_WIDTH: f32 = 340.0;

/// The dock's scroll height cap (`max_content_height(320)`, `src/ui/progress_dock.rs`).
pub const DOCK_MAX_HEIGHT: f32 = 320.0;

/// The dock's distance from the window's bottom edge (`margin_bottom(48)`), clear of the
/// footer. The host can move it with [`DockView::bottom`].
pub const DOCK_BOTTOM: f32 = 48.0;

/// A card's bar height (`.file-operation-card .modal-progress`: 4 px) and the dialog's
/// (`.modal-progress`: 8 px).
pub const CARD_BAR: f32 = 4.0;
pub const DIALOG_BAR: f32 = 8.0;

/// The trough's radius before roundness (`.modal-progress trough`: 5 px).
pub const BAR_RADIUS: f32 = 5.0;

/// The glow Strata draws around cards and notices (`box-shadow: 0 0 10px alpha(@strata_glow,
/// 0.18)`), as a share of the look's standard glow: its radius and opacity scale by these.
const GLOW_RADIUS: f32 = 10.0 / 14.0;
const GLOW_OPACITY: f32 = 0.18 / 0.24;

/// The trough's fill and border alphas (`.modal-progress trough`).
const TROUGH_FILL: f32 = 0.72;
const TROUGH_BORDER: f32 = 0.42;

type Intent<S> = Rc<dyn Fn(&mut S, DockIntent, &mut Tx)>;

/// The dock: running operations as cards, and brief notices, stacked in the bottom-right
/// corner in the order they appeared. The window under it stays usable: it is no modal and
/// draws no backdrop.
///
/// ```ignore
/// DockView::new("dock", |s: &App| &s.dock, |s: &mut App| &mut s.dock)
///     .on_intent(|s: &mut App, intent, tx| s.ops.handle(intent, tx))
///     .build(s, &s.look, &mut layer);
/// ```
///
/// Keys: `<key>` (the region), `<key>-card-<id>` with `-title`, `-bar`, `-track`, `-fill`,
/// `-count`, `-meta`, `-percent`, `-phase`, `-cancel`, `-pin` and `-complete` after it, and
/// `<key>-notice-<n>`.
pub struct DockView<S> {
    key: String,
    get: fn(&S) -> &Dock,
    get_mut: fn(&mut S) -> &mut Dock,
    on_intent: Intent<S>,
    bottom: Option<f32>,
}

impl<S: 'static> DockView<S> {
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &Dock,
        get_mut: fn(&mut S) -> &mut Dock,
    ) -> Self {
        DockView {
            key: key.into(),
            get,
            get_mut,
            on_intent: Rc::new(|_, _, _| {}),
            bottom: None,
        }
    }

    /// Hears Cancel and dismissals ([`DockIntent`]).
    pub fn on_intent(mut self, f: impl Fn(&mut S, DockIntent, &mut Tx) + 'static) -> Self {
        self.on_intent = Rc::new(f);
        self
    }

    /// The dock's distance from the window's bottom, [`DOCK_BOTTOM`] by default.
    pub fn bottom(mut self, points: f32) -> Self {
        self.bottom = Some(points);
        self
    }

    /// Pushes the dock onto `layer`, the overlays `rune_kit::root` mounts, unless it has
    /// nothing to show.
    pub fn build(&self, s: &S, look: &Look, layer: &mut Vec<El>) {
        let dock = (self.get)(s);
        if dock.is_empty() {
            return;
        }
        enum Row<'a> {
            Card(&'a Op),
            Notice(&'a DockNotice),
        }
        let mut rows: Vec<(u64, Row)> = dock
            .ops
            .iter()
            .filter(|o| o.revealed && o.spec.placement == Placement::Dock)
            .map(|o| (o.order, Row::Card(o)))
            .chain(dock.notices.iter().map(|n| (n.order, Row::Notice(n))))
            .collect();
        rows.sort_by_key(|(order, _)| *order);
        let children: Vec<El> = rows
            .into_iter()
            .map(|(_, row)| match row {
                Row::Card(op) => self.card(op, dock, look),
                Row::Notice(n) => notice(look, &format!("{}-notice-{}", self.key, n.id), n),
            })
            .collect();
        let space = look.space;
        layer.push(
            col()
                .key(self.key.clone())
                .role("region")
                .label(dock.words().region.clone())
                .pin(
                    None,
                    None,
                    Some(space.lg),
                    Some(self.bottom.unwrap_or(DOCK_BOTTOM)),
                )
                .max_h(DOCK_MAX_HEIGHT)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .items_end()
                        .gap(space.md)
                        // Room for the cards' glow inside the scroll's clip.
                        .padding(space.xl)
                        .children(children),
                ),
        );
    }

    fn card(&self, op: &Op, dock: &Dock, look: &Look) -> El {
        let k = &look.kit;
        let c = &k.c;
        let space = look.space;
        let words = dock.words();
        let shown = &op.shown;
        let id = op.id;
        let key = format!("{}-card-{id}", self.key);
        let get_mut = self.get_mut;
        let intent = self.on_intent.clone();
        let ended = shown.ended.as_ref();
        let small = k.ty.small;

        let mut header = row()
            .w_full()
            .gap(space.sm)
            .items_start()
            .child(
                el().size(look.icon_size.sm, small.line_height_or_default())
                    .center()
                    .a11y_hidden()
                    .child(
                        icon(
                            look.icon(shown.icon.unwrap_or(IconName::Info)),
                            look.icon_size.sm,
                        )
                        .color(c.icon_base),
                    ),
            )
            .child(
                text(shown.title.clone())
                    .key(format!("{key}-title"))
                    .font(TextStyle {
                        weight: 700.0,
                        ..k.ty.ui
                    })
                    .color(c.text_base)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0)
                    // The card carries the title as its name.
                    .a11y_hidden(),
            );
        if let Some(end) = ended {
            let pin = IconButton::new(look.icon(IconName::Pin))
                .variant(IconButtonVariant::Ghost)
                .control_size(ControlSize::Mini)
                .label(words.keep.clone())
                .help(words.keep.clone())
                .on_press_tx(move |s: &mut S, tx: &mut Tx| get_mut(s).toggle_pin(id, tx))
                .build(k)
                .key(format!("{key}-pin"))
                .checked(end.pinned);
            header = header.child(pin);
        }
        let x = if ended.is_some() {
            let intent = intent.clone();
            IconButton::new(look.icon(IconName::Close))
                .variant(IconButtonVariant::Ghost)
                .control_size(ControlSize::Mini)
                .label(words.dismiss.clone())
                .help(words.dismiss.clone())
                .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                    if get_mut(s).dismiss(id) {
                        intent(s, DockIntent::Dismissed(id), tx);
                    }
                })
        } else {
            let intent = intent.clone();
            IconButton::new(look.icon(IconName::Close))
                .variant(IconButtonVariant::Ghost)
                .control_size(ControlSize::Mini)
                .label(op.spec.cancel_label.clone())
                .help(words.cancel_operation.clone())
                .disabled(!shown.cancel_enabled)
                .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                    if get_mut(s).cancel(id, tx) {
                        intent(s, DockIntent::Cancel(id), tx);
                    }
                })
        };
        header = header.child(x.build(k).key(format!("{key}-cancel")));

        let mut details = row().w_full().gap(space.md).items_start().child(
            text(shown.info.clone())
                .key(format!("{key}-info"))
                .font(small)
                .color(c.text_base)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        );
        if let Some(count) = &shown.count {
            details = details.child(
                // The count's box is its committed words, the widest it reads at that total.
                text(count.clone())
                    .key(format!("{key}-count"))
                    .live_text(&op.live.count)
                    .align(Align::End)
                    .font(small)
                    .color(c.text_muted)
                    .a11y_hidden(),
            );
        }

        let mut card = col()
            .key(key.clone())
            .role("group")
            .label(shown.title.clone())
            .min_w(CARD_MIN_WIDTH)
            .max_w(CARD_MAX_WIDTH)
            .padding(space.xl)
            .gap(space.xs)
            .radius(look.round(CARD_RADIUS))
            .fill(c.layer_01)
            .border(1.0, c.border_base)
            .focus_within_ring(1.0, -1.0, c.text_accent)
            .child(header)
            .child(details);
        card = glow(look, card);
        if let Some(d) = &op.spec.destination_description {
            card = card.description(d.clone());
        }
        if let Some(dest) = &op.spec.destination {
            card = card.child(
                text(dest.clone())
                    .key(format!("{key}-destination"))
                    .font(small)
                    .color(c.text_muted)
                    .wrap()
                    .w_full()
                    .mb(space.xs)
                    .a11y_hidden(),
            );
        }
        let bar = Bar {
            key: key.clone(),
            height: CARD_BAR,
            label: shown.title.clone(),
        };
        card = card.child(match ended {
            Some(end) => bar.countdown(look, op, end.left, &words.countdown_description, get_mut),
            None => bar.progress(look, op, dock.animations(), get_mut),
        });

        let mut footer = row().w_full().gap(space.sm).items_start();
        if let Some(end) = ended {
            let intent = intent.clone();
            let label = match end.kind {
                EndKind::Complete => words.complete_label.clone(),
                EndKind::Notice => words.dismiss.clone(),
            };
            footer = footer
                .child(
                    Button::new(words.complete.clone())
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Mini)
                        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                            if get_mut(s).dismiss(id) {
                                intent(s, DockIntent::Dismissed(id), tx);
                            }
                        })
                        .build(k)
                        .key(format!("{key}-complete"))
                        .label(label),
                )
                .child(
                    text(format!("· {}", end.words))
                        .key(format!("{key}-meta"))
                        .font(small)
                        .color(c.text_muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                );
        } else {
            footer = footer.child(
                text(" ")
                    .key(format!("{key}-meta"))
                    .live_text(&op.live.meta)
                    .font(small)
                    .color(c.text_muted)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate_fade()
                    .a11y_hidden(),
            );
        }
        if ended.is_none_or(|e| e.kind == EndKind::Complete) {
            footer = footer.child(
                text((words.percent)(100))
                    .key(format!("{key}-percent"))
                    .live_text(&op.live.percent)
                    .align(Align::End)
                    .font(small)
                    .color(c.text_accent)
                    .ml(space.md)
                    .a11y_hidden(),
            );
        }
        card = card.child(footer).child(
            // What the card's phase says ("Copy complete", "Cancelling transfer…"), announced
            // when it changes; the percentages are never announced.
            el().key(format!("{key}-phase"))
                .role("status")
                .label(shown.title.clone())
                .absolute(0.0, 0.0)
                .size(0.0, 0.0),
        );
        let hover = self.get_mut;
        card = card.on_hover_change(move |s: &mut S, h, tx: &mut Tx| {
            hover(s).hover(id, h, tx);
            // The view reads only what the dock shows, which the tick this asks for updates
            // (and commits); the hover itself draws nothing.
            tx.live_only();
        });
        if let Some(fade) = &op.fade {
            card = card.live_opacity(fade);
        }
        card
    }
}

/// A progress bar: role `progressbar` with its value, over a track whose fill follows the
/// op's live fraction.
pub(crate) struct Bar {
    pub key: String,
    pub height: f32,
    pub label: String,
}

impl Bar {
    fn track(&self, look: &Look) -> El {
        let k = &look.kit;
        row()
            .key(format!("{}-track", self.key))
            .w_full()
            .h(self.height)
            .radius(look.round(BAR_RADIUS))
            .fill(k.alpha(k.c.bg_base, TROUGH_FILL))
            .border(1.0, k.alpha(k.c.std_border, TROUGH_BORDER))
            .clip_soft(0.0)
            .a11y_hidden()
    }

    fn fill(&self, look: &Look) -> El {
        el().key(format!("{}-fill", self.key))
            .h_full()
            .radius(look.round(BAR_RADIUS - 1.0))
            .fill(look.kit.c.border_active)
    }

    fn wrapper(&self, value: Option<(f32, String)>, description: &str) -> El {
        let mut e = col()
            .key(format!("{}-bar", self.key))
            .role("progressbar")
            .label(self.label.clone())
            .w_full();
        if !description.is_empty() {
            e = e.description(description.to_owned());
        }
        if let Some((f, v)) = value {
            e = e
                .range_value((f64::from(f) * 1e4).round() / 1e4, 0.0, 1.0)
                .a11y_value(v);
        }
        e
    }

    /// The op's bar: the live fraction once the track has a size, a static one before, and a
    /// sliding segment while it pulses.
    pub(crate) fn progress<S: 'static>(
        &self,
        look: &Look,
        op: &Op,
        animations: bool,
        get_mut: fn(&mut S) -> &mut Dock,
    ) -> El {
        self.progress_described(look, op, animations, get_mut, &op.shown.description)
    }

    /// [`Bar::progress`] with its own description.
    pub(crate) fn progress_described<S: 'static>(
        &self,
        look: &Look,
        op: &Op,
        animations: bool,
        get_mut: fn(&mut S) -> &mut Dock,
        description: &str,
    ) -> El {
        let shown = &op.shown;
        let fill = self.fill(look);
        let fill = match (op.shown.fraction, op.track) {
            (Some(_), Some(_)) => fill.w_full().live_rect(&op.live.fill),
            (Some(f), None) => fill.w_frac(f),
            (None, _) if animations => {
                fill.w_frac(0.25)
                    .looping(LoopSpec::slide(1.4, [-1.0, 0.0], [4.0, 0.0]))
            }
            (None, _) => fill.w_frac(0.25),
        };
        let id = op.id;
        let track = self
            .track(look)
            .child(fill)
            .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
                if !get_mut(s).set_track(id, r[2], r[3]) {
                    tx.live_only();
                }
            });
        let value = shown.fraction.zip(shown.value.clone());
        self.wrapper(value, description).child(track)
    }

    /// A finished card's bar: the time left before it closes, shrinking by a tween while it
    /// counts and still while it is paused.
    fn countdown<S: 'static>(
        &self,
        look: &Look,
        op: &Op,
        left: f32,
        description: &str,
        get_mut: fn(&mut S) -> &mut Dock,
    ) -> El {
        let fill = self.fill(look);
        let paused = op.shown.ended.as_ref().is_none_or(|e| e.paused);
        let fill = match (&op.countdown_bar, paused) {
            (Some(bar), false) => fill.w_full().live_rect(bar),
            _ => fill.w_frac(left),
        };
        let id = op.id;
        let track = self
            .track(look)
            .child(fill)
            .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
                if !get_mut(s).set_track(id, r[2], r[3]) {
                    tx.live_only();
                }
            });
        let percent = (left * 100.0).round() as u32;
        self.wrapper(Some((left, format!("{percent}%"))), description)
            .child(track)
    }
}

/// A zero-size element that asks for a wake once it is laid out: a timer a handler without a
/// `Tx` started then gets its tick in a resting window ([`Dock::cancel_untimed`]). A new
/// `generation` mounts a new one, so the commit that mounts it lays it out.
pub(crate) fn wake_beacon<S: 'static>(key: &str, generation: u64) -> El {
    el().key(format!("{key}-wake-{generation}"))
        .absolute(0.0, 0.0)
        .size(0.0, 0.0)
        .a11y_hidden()
        .on_layout_tx(|_: &mut S, _, tx: &mut Tx| {
            tx.wake_after(std::time::Duration::ZERO);
            tx.live_only();
        })
}

/// The glow around a card or a notice when the look's glow is on, else the kit's overlay
/// shadow, as C15's panel.
fn glow(look: &Look, e: El) -> El {
    let k = &look.kit;
    if look.glow.on {
        e.shadow(
            0.0,
            look.glow.radius * GLOW_RADIUS,
            look.glow.color.alpha(look.glow.opacity * GLOW_OPACITY),
        )
    } else {
        k.shadow_layers(&k.elevation.overlay)
            .fold(e, |e, s| e.box_shadow(s))
    }
}

/// A notice row: an optional check, a bold heading and text cut to three lines. No controls,
/// and a press on it does nothing.
fn notice(look: &Look, key: &str, n: &DockNotice) -> El {
    let k = &look.kit;
    let c = &k.c;
    let space = look.space;
    let notice = &n.notice;
    let mut column = col().gap(space.xxs).a11y_hidden();
    if notice.check {
        column = column.grow(1.0).min_w(0.0);
    }
    if let Some(h) = &notice.heading {
        column = column.child(
            text(h.clone())
                .font(TextStyle {
                    weight: 700.0,
                    ..k.ty.ui
                })
                .color(c.text_base)
                .wrap(),
        );
    }
    let body = text(notice.text.clone())
        .key(format!("{key}-text"))
        .font(k.ty.ui)
        .color(c.text_base);
    // A wrapping text in a row without a width lays out at its narrowest, so a check notice
    // takes the card's width; a status notice is one short line and fits its words.
    column = column.child(if notice.check {
        body.max_lines(3)
    } else {
        body
    });
    let mut r = row()
        .key(key.to_owned())
        .role("status")
        .label(
            notice
                .heading
                .clone()
                .unwrap_or_else(|| notice.text.clone()),
        )
        .gap(space.md)
        .py(space.lg)
        .px(space.xl)
        .radius(look.round(CARD_RADIUS))
        .fill(c.layer_01)
        .border(1.0, c.border_base);
    if notice.heading.is_some() {
        r = r.description(notice.text.clone());
    }
    if notice.check {
        r = r.w(CARD_MAX_WIDTH).child(
            icon(look.icon(IconName::Check), look.icon_size.md)
                .color(c.icon_base)
                .a11y_hidden(),
        );
    }
    glow(look, r.child(column))
}

trait LineHeight {
    fn line_height_or_default(&self) -> f32;
}

impl LineHeight for TextStyle {
    /// The style's line height in points: Rune's 1.25 × size when it is 0.
    fn line_height_or_default(&self) -> f32 {
        if self.line_height > 0.0 {
            self.line_height
        } else {
            self.size * 1.25
        }
    }
}
