// SPDX-License-Identifier: MIT

//! The header: the previewed item (icon, name, details, badges), then the controls the hosted
//! view supports, in Strata's order: the Rendered/Source switch, Find, the busy spinner, Open,
//! Wrap and Close. There is never a Print button.

use rune::{El, Tx, col, el, icon, row, text};
use rune_kit::Spinner;
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::{DocumentView, DrawerIntent, DrawerState, Parts, Shared, find};

/// Strata's `.preview-header-action` corner radius at 100 % roundness (`src/style.css`).
const ACTION_RADIUS: f32 = 5.0;
/// Strata's `.preview-header-action:disabled` opacity (`src/style.css`).
const DISABLED_OPACITY: f32 = 0.46;
/// Strata's `.preview-header-action:focus-visible` ring opacity (`src/style.css`).
const FOCUS_RING_OPACITY: f32 = 0.52;

/// Configuration for a preview header or find bar icon button (`.preview-header-action`).
pub(super) struct Action<'a> {
    pub key: &'a str,
    pub icon: IconName,
    pub label: &'a str,
    pub active: bool,
    pub disabled: bool,
}

/// An accented ghost icon button matching GTK's `.preview-header-action`: `icon_accent` at rest
/// and hover, `pressed` fill when active/checked, `look.round(5.0)` corners, and muted dimmed
/// ink when disabled.
pub(super) fn action_button<S: 'static>(
    spec: Action<'_>,
    look: &Look,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let k = &look.kit;
    let side = look.frame.control;
    let ink = if spec.disabled {
        k.c.icon_muted
    } else {
        k.c.icon_accent
    };
    let mut button = row()
        .key(spec.key)
        .role("button")
        .label(spec.label.to_owned())
        .help(spec.label.to_owned())
        .size(side, side)
        .center()
        .radius(look.round(ACTION_RADIUS))
        .child(
            icon(look.icon(spec.icon), look.icon_size.md)
                .key("icon")
                .color(ink)
                .a11y_hidden(),
        );
    if spec.disabled {
        return button
            .disabled(true)
            .focusable(false)
            .opacity(DISABLED_OPACITY);
    }
    if spec.active {
        button = button.fill(k.c.pressed);
    }
    button
        .hover(k.c.hover)
        .press(k.c.pressed)
        .focusable(true)
        .focus_ring(2.0, 1.0, k.alpha(k.c.border_focus, FOCUS_RING_OPACITY))
        .on_press_tx(on_press)
        .on_key(super::return_as_space::<S>)
}

pub(super) fn header<S: 'static>(
    shared: &Shared<S>,
    st: &DrawerState,
    parts: &Parts<'_, S>,
    look: &Look,
) -> El {
    let k = &look.kit;
    let labels = &shared.labels;
    let controls = header_controls(shared, st, parts, look);
    let handle = header_handle(parts, &labels.detail_descriptions, look);

    // Sized like the window's header bar, as Strata's `.preview-header` is: the bar's height
    // and its Frame-density inset, no vertical padding, 8 points before and 6 after.
    row()
        .key("preview-header")
        .w_full()
        .h(look.frame.bar + 2.0 * look.frame.inset)
        .pl(look.space.md)
        .pr(look.space.sm)
        .gap(look.space.xs)
        .items_center()
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_base)
        .child(handle)
        .children(controls)
}

fn header_controls<S: 'static>(
    shared: &Shared<S>,
    st: &DrawerState,
    parts: &Parts<'_, S>,
    look: &Look,
) -> Vec<El> {
    let k = &look.kit;
    let caps = parts.caps;
    let labels = &shared.labels;
    let mut controls: Vec<El> = Vec::new();
    if let Some(view) = caps.view {
        let (icon_name, label) = match view {
            DocumentView::Rendered => (IconName::Source, &labels.view_source),
            DocumentView::Source => (IconName::Rendered, &labels.view_rendered),
        };
        let sh = shared.clone();
        controls.push(action_button(
            Action {
                key: "preview-view-switch",
                icon: icon_name,
                label,
                active: false,
                disabled: false,
            },
            look,
            move |s: &mut S, tx: &mut Tx| sh.emit(s, DrawerIntent::ShowView(view.other()), tx),
        ));
    }
    if caps.searchable {
        let sh = shared.clone();
        controls.push(
            action_button(
                Action {
                    key: "preview-find",
                    icon: IconName::Search,
                    label: &labels.find,
                    active: st.find.open,
                    disabled: false,
                },
                look,
                move |s: &mut S, tx: &mut Tx| {
                    if (sh.access)(s).find.open {
                        find::close(&sh, s, tx);
                    } else {
                        find::open(&sh, s, tx);
                    }
                },
            )
            .expanded(st.find.open),
        );
    }
    if let Some(busy) = &caps.busy {
        controls.push(
            el().key("preview-busy")
                .role("progressbar")
                .label(busy.clone())
                .child(
                    Spinner::new()
                        .size(look.icon_size.md)
                        .build(k)
                        .a11y_hidden(),
                ),
        );
    }
    if parts.open_allowed {
        let sh = shared.clone();
        controls.push(action_button(
            Action {
                key: "preview-open",
                icon: IconName::Open,
                label: &labels.open,
                active: false,
                disabled: parts.open_disabled,
            },
            look,
            move |s: &mut S, tx: &mut Tx| sh.emit(s, DrawerIntent::Open, tx),
        ));
    }
    if let Some(on) = caps.wrap {
        let sh = shared.clone();
        controls.push(
            action_button(
                Action {
                    key: "preview-wrap",
                    icon: IconName::Wrap,
                    label: &labels.wrap,
                    active: on,
                    disabled: false,
                },
                look,
                move |s: &mut S, tx: &mut Tx| sh.emit(s, DrawerIntent::SetWrap(!on), tx),
            )
            .checked(on),
        );
    }
    let sh = shared.clone();
    controls.push(action_button(
        Action {
            key: "preview-close",
            icon: IconName::ClosePanel,
            label: &labels.close,
            active: false,
            disabled: false,
        },
        look,
        move |s: &mut S, tx: &mut Tx| {
            (sh.access)(s).close_with(sh.animations, tx);
            sh.emit(s, DrawerIntent::Closed, tx);
        },
    ));
    controls
}

fn header_handle<S: 'static>(parts: &Parts<'_, S>, descriptions: &[String; 3], look: &Look) -> El {
    let k = &look.kit;
    let info = parts.header;
    let mut heading = col().grow(1.0).min_w(0.0);
    if !info.title.is_empty() {
        heading = heading.child(
            text(info.title.clone())
                .key("preview-title")
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .max_lines(1),
        );
    }
    if !info.details.is_empty() {
        heading = heading.child(details_row(&info.details, descriptions, look));
    }
    let mut handle = row()
        .key("preview-handle")
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space.sm)
        .items_center()
        .children(info.icon.map(|i| {
            icon(i, look.icon_size.md)
                .color(k.c.icon_accent)
                .a11y_hidden()
        }))
        .child(heading);
    if let Some((dnd, source)) = parts.drag {
        handle = dnd.drag_source_keyed(handle, "preview-handle", source.clone());
    }
    handle
}

fn details_row(details: &[String], descriptions: &[String; 3], look: &Look) -> El {
    let k = &look.kit;
    let mut line = row()
        .key("preview-details")
        .w_full()
        .min_w(0.0)
        .gap(look.space.xxs)
        .items_center();
    for (index, detail) in details.iter().enumerate() {
        if index > 0 {
            line = line.child(text("·").font(k.ty.small).color(k.c.text_muted));
        }
        let mut item = text(detail.clone())
            .label(detail.clone())
            .key(format!("preview-detail-{index}"))
            .font(k.ty.small)
            .color(k.c.text_muted)
            .max_lines(1);
        if let Some(desc) = descriptions.get(index).filter(|desc| !desc.is_empty()) {
            item = item.description(desc.clone());
        }
        if index + 1 == details.len() {
            item = item.grow(1.0).min_w(0.0);
        }
        line = line.child(item);
    }
    line
}
