// SPDX-License-Identifier: MIT

use rune::{El, Key, KeyEvent, PointerPress, Side, Tx, icon, row, text};
use rune_kit::{Keybind, Spinner};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::sidebar as slot;

use super::data::{Device, ItemId, LockState, PlaceKind, RowIcon, SidebarLabels};
use super::drag::{self, Block, Landing, RowDrag};
use super::intent::{DropSite, SidebarIntent, Via};
use super::state::SidebarState;
use super::{ActionFn, Emit, trash_target_key};
use crate::contract::dnd::{Dnd, DropHover};

pub(crate) const ACTION_SLOT: f32 = 20.0;

pub(crate) fn action_icon_size(look: &Look) -> f32 {
    ((look.icon_size.sm + look.icon_size.md) / 2.0).round()
}

fn row_shell(look: &Look, icon: El, selected: bool) -> El {
    let size = slot::ICON_BOX;
    let r = row()
        .w_full()
        .h(slot::row_height(look))
        .pl(slot::ROW_INSET)
        .center_y()
        .radius(look.kit.radius.md)
        .motion(look.kit.motion.control)
        .hover_group()
        .hover_motion(rune::Anim::INSTANT, rune::Anim::INSTANT)
        .focusable(true)
        .focus_selected(selected)
        .child(
            row()
                .key("icon")
                .size(size, size)
                .center()
                .child(icon)
                .a11y_hidden(),
        );
    look.kit.interactive(r)
}

/// A rail row: the icon alone, where the full row has it, so a step moves no icon.
fn rail_row(look: &Look, icon: El, selected: bool) -> El {
    row_shell(look, icon, selected)
}

fn full_row(look: &Look, icon: El, label: &str, selected: bool, trailing: Option<El>) -> El {
    let label = text(label)
        .key("label")
        .font(look.kit.ty.ui_compact)
        .color(look.kit.c.text_base)
        .truncate()
        .grow(1.0)
        .shrink()
        .a11y_hidden();
    let mut r = row_shell(look, icon, selected)
        .pr(super::build::full_row_trailing_pad(look))
        .gap(look.space.md)
        .child(label);
    if let Some(t) = trailing {
        r = r.child(t);
    }
    r
}

/// The busy spinner shown in the release column while a device is being ejected/unmounted.
fn busy_slot(look: &Look) -> El {
    row()
        .key("status")
        .size(ACTION_SLOT, ACTION_SLOT)
        .center()
        .child(
            Spinner::new()
                .size(action_icon_size(look))
                .color(look.kit.c.icon_accent)
                .build(&look.kit),
        )
        .a11y_hidden()
}

/// What every row builder shares.
pub(crate) struct Ctx<'a, S> {
    pub key: &'a str,
    pub look: &'a Look,
    pub state: &'a SidebarState,
    pub access: fn(&mut S) -> &mut SidebarState,
    pub emit: Emit<S>,
    pub dnd: &'a Dnd<S>,
    pub content: &'static [&'static str],
    pub action: Option<&'a ActionFn<S>>,
    pub reorder: bool,
    pub rail: bool,
    pub new_tabs: bool,
    pub tenxer: bool,
    pub labels: SidebarLabels<'a>,
}

/// What one row shows and does.
pub(crate) struct Spec<'a> {
    pub item: ItemId,
    pub label: &'a str,
    pub selected: bool,
    pub accepts_drops: bool,
    /// A middle click opens a new tab.
    pub new_tab: bool,
    pub keycap: Option<&'a str>,
    /// The row's accessible description, if the host gave one.
    pub description: Option<&'a str>,
    /// The row's place in its reorderable list: index and count.
    pub reorder: Option<(usize, usize)>,
    /// The row is lifted by its reorder drag.
    pub lifted: bool,
}

pub(crate) fn over(top: rune::Color, bottom: rune::Color) -> rune::Color {
    use rune::theme::{blends_srgb, srgb_decode, srgb_encode};
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return rune::Color::TRANSPARENT;
    }
    type Channel = fn(f32) -> f32;
    let (enc, dec): (Channel, Channel) = if blends_srgb() {
        (srgb_encode, srgb_decode)
    } else {
        (|c| c, |c| c)
    };
    let mix = |t: f32, b: f32| dec((enc(t) * top.a + enc(b) * bottom.a * (1.0 - top.a)) / a);
    rune::Color {
        r: mix(top.r, bottom.r),
        g: mix(top.g, bottom.g),
        b: mix(top.b, bottom.b),
        a,
    }
}

pub(crate) fn row_icon(look: &Look, i: &RowIcon, tint: Option<rune::Color>, selected: bool) -> El {
    let badge_fill = tint.unwrap_or(look.kit.c.icon_accent);
    match i {
        RowIcon::Catalog(name) => crate::thumbnails::folder_badge(
            look,
            crate::contract::EntryGlyph::Icon(name),
            look.icon_size.md,
            badge_fill,
        ),
        RowIcon::Emoji(emoji) => crate::thumbnails::folder_badge(
            look,
            crate::contract::EntryGlyph::Emoji(emoji),
            look.icon_size.md,
            badge_fill,
        ),
        RowIcon::Name(n) => {
            let colour = tint.unwrap_or_else(|| {
                if selected {
                    slot::selected_icon(look)
                } else {
                    look.kit.c.icon_accent
                }
            });
            icon(look.icon(*n), look.icon_size.md).color(colour)
        }
    }
}

pub(crate) fn trash_icon(
    key: &str,
    look: &Look,
    full: bool,
    selected: bool,
    hook: Option<Box<dyn FnOnce(El) -> El + '_>>,
) -> El {
    let name = if full {
        IconName::TrashFull
    } else {
        IconName::Trash
    };
    let glyph = row_icon(look, &RowIcon::Name(name), None, selected);
    let size = slot::ICON_BOX;
    let target = row()
        .key(trash_target_key(key))
        .size(size, size)
        .center()
        .child(glyph);
    match hook {
        Some(f) => f(target),
        None => target,
    }
}

fn activate<S: 'static>(ctx: &Ctx<'_, S>, el: El, item: ItemId, new_tab: bool) -> El {
    let access = ctx.access;
    let press_emit = ctx.emit.clone();
    let menu_emit = ctx.emit.clone();
    let press_item = item.clone();
    let menu_item = item.clone();
    let laid_out = item.clone();
    let el = el
        .on_layout_tx(move |s: &mut S, rect, tx: &mut Tx| {
            access(s).boxes.insert(laid_out.clone(), rect);
            tx.live_only();
        })
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            let cancelled = access(s).cancelled_press.take();
            if !tx.by_keys() && cancelled.as_ref() == Some(&press_item) {
                return;
            }
            let via = if tx.by_keys() {
                Via::Keyboard
            } else {
                Via::Pointer
            };
            press_emit(
                s,
                SidebarIntent::Open {
                    item: press_item.clone(),
                    via,
                },
                tx,
            );
        })
        .on_secondary_press(move |s: &mut S, point: PointerPress, tx: &mut Tx| {
            menu_emit(
                s,
                SidebarIntent::ContextMenu {
                    item: Some(menu_item.clone()),
                    point,
                },
                tx,
            );
        });
    optional_row_handlers(ctx, el, item, new_tab)
}

fn optional_row_handlers<S: 'static>(
    ctx: &Ctx<'_, S>,
    mut el: El,
    item: ItemId,
    new_tab: bool,
) -> El {
    if new_tab {
        let middle_emit = ctx.emit.clone();
        let middle_item = item.clone();
        el = el.on_middle_press(move |s: &mut S, tx: &mut Tx| {
            middle_emit(
                s,
                SidebarIntent::OpenInNewTab {
                    item: middle_item.clone(),
                },
                tx,
            );
        });
    }
    if ctx.tenxer {
        let key_emit = ctx.emit.clone();
        el = el.on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let open = |s: &mut S, tx: &mut Tx| {
                key_emit(
                    s,
                    SidebarIntent::Open {
                        item: item.clone(),
                        via: Via::Keyboard,
                    },
                    tx,
                );
                true
            };
            tenxer_key(s, ev, tx, &key_emit, open)
        });
    }
    el
}

pub(crate) fn tenxer_key<S: 'static>(
    s: &mut S,
    ev: &KeyEvent,
    tx: &mut Tx,
    emit: &Emit<S>,
    activate: impl FnOnce(&mut S, &mut Tx) -> bool,
) -> bool {
    let step = |tx: &mut Tx, key| {
        tx.send_key(KeyEvent { key, mods: ev.mods });
        true
    };
    match &ev.key {
        Key::Text(t) if t == "j" => step(tx, Key::Down),
        Key::Text(t) if t == "k" => step(tx, Key::Up),
        Key::Text(t) if t == "l" => activate(s, tx),
        Key::Text(t) if t == "h" => {
            emit(s, SidebarIntent::LeaveToFiles, tx);
            true
        }
        Key::Left | Key::Backspace => {
            emit(s, SidebarIntent::LeaveToFiles, tx);
            true
        }
        _ => false,
    }
}

fn named(el: El, spec: &Spec<'_>) -> El {
    let el = el.role("button").label(spec.label);
    match spec.description {
        Some(description) => el.description(description),
        None => el,
    }
}

fn dress<S>(ctx: &Ctx<'_, S>, el: El, spec: &Spec<'_>) -> El {
    let look = ctx.look;
    let c = &look.kit.c;
    let dragging = ctx.state.dragging();
    let hovered = matches!(
        ctx.state.drop_over(),
        Some((DropSite::Item(i), DropHover::Accepting)) if *i == spec.item
    );
    let mut el = named(el.key(spec.item.key(ctx.key)), spec);
    if spec.selected && spec.lifted {
        let lift = over(c.pressed, c.bg_deep);
        el = el.fill(over(slot::selected_fill(look), lift));
    } else if spec.selected {
        el = el.fill(slot::selected_fill(look));
    } else if hovered && !spec.lifted {
        el = el.fill(slot::hover_fill(look));
    }
    if hovered {
        el = el.border(1.0, c.border_focus);
    }
    el = el.hover(if dragging {
        rune::Color::TRANSPARENT
    } else {
        slot::hover_fill(look)
    });
    if dragging {
        // No focus ring while a drag is over the sidebar.
        el = el.focus_ring(0.0, 0.0, rune::Color::TRANSPARENT);
    }
    if ctx.rail {
        el = el.help(spec.label).help_side(Side::Right);
    }
    el
}

pub(crate) type Reordering<'a> = (
    &'a RowDrag,
    Block,
    std::rc::Rc<dyn Fn(usize, Landing) -> SidebarIntent>,
);

/// A built-in place or a pin. Both widths drag to reorder ([`super::drag`]).
pub(crate) fn folder_row<S: 'static>(
    ctx: &Ctx<'_, S>,
    spec: Spec<'_>,
    icon: El,
    reorder: Option<Reordering<'_>>,
) -> El {
    let look = ctx.look;
    let built = if ctx.rail {
        rail_row(look, icon, spec.selected)
    } else {
        let trailing = match (spec.keycap, ctx.state.keycaps) {
            (Some(cap), true) => Some(
                Keybind::new(&[cap])
                    .keep_case()
                    .build(&look.kit)
                    .a11y_hidden(),
            ),
            _ => None,
        };
        full_row(look, icon, spec.label, spec.selected, trailing)
    };
    let built = match (spec.reorder, reorder) {
        (Some((index, count)), Some((state, block, intent))) => drag::attach(
            built,
            drag::AttachSpec {
                look,
                state,
                block,
                access: ctx.access,
                emit: ctx.emit.clone(),
                item: spec.item.clone(),
                index,
                count,
                intent,
            },
        ),
        _ => built,
    };
    let el = activate(ctx, built, spec.item.clone(), spec.new_tab);
    let el = dress(ctx, el, &spec);
    let el = match spec.reorder {
        Some((index, count)) => el.position_in_set(index + 1, count),
        None => el,
    };
    attach(ctx, el, &spec)
}

fn attach<S: 'static>(ctx: &Ctx<'_, S>, el: El, spec: &Spec<'_>) -> El {
    if spec.accepts_drops {
        let zone = super::drops::row(ctx.access, &ctx.emit, ctx.action, spec.item.clone())
            .takes_content(ctx.content);
        ctx.dnd.drop_zone_keyed(el, spec.item.key(ctx.key), zone)
    } else {
        el
    }
}

pub(crate) fn device_row<S: 'static>(ctx: &Ctx<'_, S>, d: &Device, selected: bool) -> El {
    let look = ctx.look;
    let item = d.item();
    let icon = row_icon(look, &RowIcon::Name(d.icon), None, selected);
    let spec = Spec {
        item: item.clone(),
        label: &d.label,
        selected,
        accepts_drops: d.accepts_drops && d.mounted && !d.pending,
        new_tab: ctx.new_tabs && d.mounted,
        keycap: None,
        description: None,
        reorder: None,
        lifted: false,
    };
    let el = if ctx.rail {
        rail_row(look, icon, selected)
    } else {
        let trailing = if d.pending {
            Some(busy_slot(look))
        } else if d.lock.is_some() || d.release.is_some() {
            Some(device_actions(ctx, d))
        } else {
            None
        };
        full_row(look, icon, d.label.as_str(), selected, trailing)
    };
    if d.pending {
        return dress(ctx, el, &spec)
            .description(ctx.labels.device.releasing)
            .disabled(true);
    }
    let el = activate(ctx, el, item, spec.new_tab);
    let el = dress(ctx, el, &spec).description(d.description.clone());
    attach(ctx, el, &spec)
}

fn device_actions<S: 'static>(ctx: &Ctx<'_, S>, d: &Device) -> El {
    let look = ctx.look;
    let icon_size = action_icon_size(look);
    let mut actions = row().gap(look.space.xxs).center_y();
    let button = |key: String, glyph: IconName, label: &str, intent: SidebarIntent| {
        let emit = ctx.emit.clone();
        let glyph = icon(look.icon(glyph), icon_size)
            .key("icon")
            .color(look.kit.c.icon_accent)
            .group_opacity(0.75, 1.0)
            .opacity_motion(rune::Anim::INSTANT)
            .a11y_hidden();
        let btn = row()
            .key(key)
            .role("button")
            .label(label)
            .help(label)
            .size(ACTION_SLOT, ACTION_SLOT)
            .center()
            .radius(look.kit.radius.sm)
            .hover_group()
            .hover_motion(rune::Anim::INSTANT, rune::Anim::INSTANT)
            .focusable(false)
            .preserve_focus_on_press(true)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| emit(s, intent.clone(), tx))
            .child(glyph);
        look.kit.interactive(btn)
    };
    let base = d.item().key(ctx.key);
    let words = ctx.labels.device;
    if let Some(lock) = d.lock {
        let (glyph, label, intent) = match lock {
            LockState::Locked => (
                IconName::Lock,
                words.unlock,
                SidebarIntent::Unlock(d.id.clone()),
            ),
            LockState::Unlocked => (
                IconName::Unlock,
                words.lock,
                SidebarIntent::Lock(d.id.clone()),
            ),
        };
        actions = actions.child(button(format!("{base}-lock"), glyph, label, intent));
    }
    if let Some(release) = d.release {
        actions = actions.child(button(
            format!("{base}-eject"),
            IconName::Eject,
            words.release(release),
            SidebarIntent::Eject(d.id.clone()),
        ));
    }
    actions
}

/// Whether a place is the Trash, and full.
pub(crate) fn trash_full(kind: PlaceKind) -> Option<bool> {
    match kind {
        PlaceKind::Trash { full } => Some(full),
        _ => None,
    }
}
