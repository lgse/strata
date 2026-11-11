//! Rows: built-in places, pins and devices, on rune-kit's `SidebarRow`, in the full sidebar
//! and the rail. The rows are rune-kit's (their reorder drag, selection, focus and spacing);
//! the sidebar sets the look Strata's rows have over the built row: the accent tint and
//! icon, the neutral hover, the rail's tooltip and the device actions.

use rune::{El, Key, KeyEvent, PointerButton, PointerPress, Tx, icon, row};
use rune_kit::shell::{RowStatus, SidebarRow};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Keybind};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::sidebar as slot;

use super::data::{Device, ItemId, LockState, PlaceKind, RowIcon};
use super::intent::{DropSite, SidebarIntent, Via};
use super::state::SidebarState;
use super::{ActionFn, Emit, ReorderAccess, trash_target_key};
use crate::contract::dnd::{Dnd, DropHover};

/// The accessible description of a device whose release is still running.
pub const PENDING_RELEASE: &str = "Writing data to the drive. Do not unplug it.";

/// A rail row: the icon alone, where the full row has it, so a step moves no icon. rune-kit's
/// `SidebarRow` keeps a label slot and its gap even with an empty label, which don't fit the
/// rail (U18), so the rail builds its own row: a list item that takes focus, selects and
/// hovers as `SidebarRow` does, without its reorder drag.
fn rail_row(look: &Look, icon: El, selected: bool) -> El {
    let size = slot::ICON_BOX;
    let r = row()
        .w_full()
        .h(slot::row_height(look))
        .pl(slot::ROW_INSET)
        .center_y()
        .radius(look.kit.radius.md)
        .focusable(true)
        .focus_selected(selected)
        .hover_motion(rune::Anim::INSTANT, rune::Anim::INSTANT)
        .child(
            row()
                .key("icon")
                .size(size, size)
                .center()
                .child(icon)
                .a11y_hidden(),
        );
    look.kit.interactive(r).role("listitem")
}

/// What every row builder shares.
pub(crate) struct Ctx<'a, S> {
    pub key: &'a str,
    pub look: &'a Look,
    pub state: &'a SidebarState,
    pub access: fn(&mut S) -> &mut SidebarState,
    pub emit: Emit<S>,
    pub dnd: &'a Dnd<S>,
    pub action: Option<&'a ActionFn<S>>,
    pub reorder: Option<(ReorderAccess<S>, ReorderAccess<S>)>,
    pub rail: bool,
    pub new_tabs: bool,
    pub tenxer: bool,
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
    /// The row's place in its reorderable list: index and count.
    pub reorder: Option<(usize, usize)>,
    /// The row is lifted by its reorder drag.
    pub lifted: bool,
}

/// Source-over of straight-alpha colours, mixed as the window blends: rune-kit's own `over`,
/// which isn't public (U22).
fn over(top: rune::Color, bottom: rune::Color) -> rune::Color {
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

/// Reports a reorder's parked move, if any, before an activation acts (U21). True when the
/// move came from `item`: Rune's press on the release that ended the drag, which reports the
/// move and nothing else.
pub(crate) fn flush_reordered<S>(
    s: &mut S,
    tx: &mut Tx,
    access: fn(&mut S) -> &mut SidebarState,
    emit: &Emit<S>,
    item: Option<&ItemId>,
) -> bool {
    match access(s).reordered.take() {
        Some((source, moved)) => {
            emit(s, moved, tx);
            item == Some(&source)
        }
        None => false,
    }
}

/// A row's icon: the semantic or catalog icon, in the accent on the current row, else the
/// folder's tint, else the muted icon colour.
pub(crate) fn row_icon(look: &Look, i: &RowIcon, tint: Option<rune::Color>, selected: bool) -> El {
    let glyph = match i {
        RowIcon::Name(n) => look.icon(*n),
        RowIcon::Catalog(name) => look
            .icon_named(name)
            .unwrap_or_else(|| look.icon(IconName::Folder)),
    };
    let colour = if selected {
        slot::selected_icon(look)
    } else {
        tint.unwrap_or(look.kit.c.icon_muted)
    };
    icon(glyph, look.icon_size.md).color(colour)
}

/// The Trash row's icon, keyed for C23's flight and passed through the host's hook.
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

/// Activation shared by every row: a click or a middle click, Enter or Space, the 10xer
/// keys, and the context menu.
fn activate<S: 'static>(ctx: &Ctx<'_, S>, el: El, item: ItemId, new_tab: bool) -> El {
    let access = ctx.access;
    let tenxer = ctx.tenxer;
    let press_emit = ctx.emit.clone();
    let key_emit = ctx.emit.clone();
    let menu_emit = ctx.emit.clone();
    let press_item = item.clone();
    let key_item = item.clone();
    let mut el = el
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            // Rune presses a row on the release that ends its reorder drag (U21): that
            // release reports the move, not an open. A move parked by another row is
            // reported first, then this press acts.
            if flush_reordered(s, tx, access, &press_emit, Some(&press_item)) {
                return;
            }
            let item = press_item.clone();
            if tx.button() == PointerButton::Middle {
                if new_tab {
                    press_emit(s, SidebarIntent::OpenInNewTab { item }, tx);
                }
                return;
            }
            press_emit(
                s,
                SidebarIntent::Open {
                    item,
                    via: Via::Pointer,
                },
                tx,
            );
        })
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let open = |s: &mut S, tx: &mut Tx| {
                flush_reordered(s, tx, access, &key_emit, None);
                key_emit(
                    s,
                    SidebarIntent::Open {
                        item: key_item.clone(),
                        via: Via::Keyboard,
                    },
                    tx,
                );
                true
            };
            match &ev.key {
                Key::Enter => open(s, tx),
                Key::Text(t) if t == " " => open(s, tx),
                _ if tenxer => tenxer_key(s, ev, tx, &key_emit, open),
                _ => false,
            }
        })
        .on_secondary_press(move |s: &mut S, point: PointerPress, tx: &mut Tx| {
            flush_reordered(s, tx, access, &menu_emit, None);
            menu_emit(
                s,
                SidebarIntent::ContextMenu {
                    item: Some(item.clone()),
                    point,
                },
                tx,
            );
        });
    if ctx.new_tabs {
        el = el.pointer_buttons([PointerButton::Middle]);
    }
    el
}

/// 10xer keys on a row or heading: `j` and `k` step like Down and Up, `l` activates, and `h`,
/// Left and Backspace hand focus back to the files.
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

/// The look over a built row: the accent tint on the current row, the neutral hover (none
/// while a drag is over the sidebar), and in the rail its name as a tooltip. A row whose body
/// takes the drag shows the hover fill and an accent outline.
fn dress<S>(ctx: &Ctx<'_, S>, el: El, spec: &Spec<'_>) -> El {
    let look = ctx.look;
    let c = &look.kit.c;
    let dragging = ctx.state.dragging();
    let hovered = matches!(
        ctx.state.drop_over(),
        Some((DropSite::Item(i), DropHover::Accepting)) if *i == spec.item
    );
    let mut el = el.key(spec.item.key(ctx.key)).label(spec.label);
    if spec.selected && spec.lifted {
        // Lifted, the row keeps rune-kit's opaque lift fill under the tint, so the rows it
        // passes don't show through.
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
        el = el.help(spec.label);
    }
    el
}

/// A row's drag to reorder: the list's drag, its accessor, and the intent a move from one
/// index to another makes.
pub(crate) type Reordering<'a, S> = (
    &'a rune_kit::shell::Reorder,
    ReorderAccess<S>,
    std::rc::Rc<dyn Fn(usize, usize) -> SidebarIntent>,
);

/// A built-in place or a pin.
pub(crate) fn folder_row<S: 'static>(
    ctx: &Ctx<'_, S>,
    spec: Spec<'_>,
    icon: El,
    reorder: Option<Reordering<'_, S>>,
) -> El {
    let look = ctx.look;
    if ctx.rail {
        let mut row = rail_row(look, icon, spec.selected);
        if let Some((index, count)) = spec.reorder {
            row = row.position_in_set(index + 1, count);
        }
        let el = activate(ctx, row, spec.item.clone(), spec.new_tab);
        let el = dress(ctx, el, &spec);
        return attach(ctx, el, &spec);
    }
    let mut r = SidebarRow::new(spec.label)
        .avatar(icon)
        .selected(spec.selected)
        .height(slot::row_height(look));
    if let (Some(cap), true) = (spec.keycap, ctx.state.keycaps) {
        r = r.trailing(Keybind::new(&[cap]).build(&look.kit).a11y_hidden());
    }
    if let (Some((index, count)), Some((state, access, intent))) = (spec.reorder, reorder) {
        let sidebar = ctx.access;
        // The drop runs without a `Tx`, and Rune presses the row on the same release (U21):
        // keep the move for that press to report, instead of opening the row.
        let source = spec.item.clone();
        r = r.reorder(state, index, count, access, move |s: &mut S, from, to| {
            sidebar(s).reordered = Some((source.clone(), intent(from, to)));
        });
    }
    let el = activate(ctx, r.build(&look.kit), spec.item.clone(), spec.new_tab);
    let el = dress(ctx, el, &spec);
    let el = match spec.reorder {
        Some((index, count)) => el.position_in_set(index + 1, count),
        None => el,
    };
    attach(ctx, el, &spec)
}

fn attach<S: 'static>(ctx: &Ctx<'_, S>, el: El, spec: &Spec<'_>) -> El {
    if spec.accepts_drops {
        let zone = super::drops::row(ctx.access, &ctx.emit, ctx.action, spec.item.clone());
        ctx.dnd.drop_zone_keyed(el, spec.item.key(ctx.key), zone)
    } else {
        el
    }
}

/// A device row: in the full sidebar its lock and release buttons, and while a release runs
/// a spinner in the release column, disabled.
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
        reorder: None,
        lifted: false,
    };
    let el = if ctx.rail {
        rail_row(look, icon, selected)
    } else {
        let mut r = SidebarRow::new(d.label.as_str())
            .avatar(icon)
            .selected(selected)
            .height(slot::row_height(look));
        if d.pending {
            r = r.status(RowStatus::Busy);
        } else if d.lock.is_some() || d.release.is_some() {
            r = r.trailing(device_actions(ctx, d));
        }
        r.build(&look.kit)
    };
    if d.pending {
        return dress(ctx, el, &spec)
            .description(PENDING_RELEASE)
            .disabled(true);
    }
    let el = activate(ctx, el, item, spec.new_tab);
    let el = dress(ctx, el, &spec).description(d.description.clone());
    attach(ctx, el, &spec)
}

/// The lock button, then the release button: a row with only a lock button gives it the
/// release column. They are pointer-only, as rune-kit's row controls are: the list's arrows
/// step rows, and the row's menu (C21) has the same actions.
fn device_actions<S: 'static>(ctx: &Ctx<'_, S>, d: &Device) -> El {
    let look = ctx.look;
    let access = ctx.access;
    let mut actions = row().gap(look.space.xxs).center_y();
    let button = |key: String, glyph: IconName, label: &str, intent: SidebarIntent| {
        let emit = ctx.emit.clone();
        IconButton::new(look.icon(glyph))
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(label)
            .help(label)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                flush_reordered(s, tx, access, &emit, None);
                emit(s, intent.clone(), tx)
            })
            .build(&look.kit)
            .key(key)
            .focusable(false)
            .preserve_focus_on_press(true)
    };
    let base = d.item().key(ctx.key);
    if let Some(lock) = d.lock {
        let (glyph, label, intent) = match lock {
            LockState::Locked => (
                IconName::Lock,
                "Unlock",
                SidebarIntent::Unlock(d.id.clone()),
            ),
            LockState::Unlocked => (IconName::Unlock, "Lock", SidebarIntent::Lock(d.id.clone())),
        };
        actions = actions.child(button(format!("{base}-lock"), glyph, label, intent));
    }
    if let Some(release) = d.release {
        actions = actions.child(button(
            format!("{base}-eject"),
            IconName::Eject,
            release.label(),
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
