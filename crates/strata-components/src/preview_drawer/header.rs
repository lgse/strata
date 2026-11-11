//! The header: the previewed item (icon, name, details, badges), then the controls the hosted
//! view supports, in Strata's order: the Rendered/Source switch, Find, the busy spinner, Open,
//! Wrap and Close. There is never a Print button.

use rune::{El, Tx, col, el, icon, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Spinner, State};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::{DocumentView, DrawerIntent, DrawerState, Parts, Shared, find};

pub(super) fn header<S: 'static>(
    shared: &Shared<S>,
    st: &DrawerState,
    parts: &Parts<'_, S>,
    look: &Look,
) -> El {
    let k = &look.kit;
    let caps = parts.caps;
    let labels = &shared.labels;
    let size = control_size(look.frame.control);
    let button = |name: IconName, label: &str| {
        IconButton::new(look.icon(name))
            .variant(IconButtonVariant::GhostMuted)
            .control_size(size)
            .label(label.to_owned())
    };

    let mut controls: Vec<El> = Vec::new();
    if let Some(view) = caps.view {
        let (name, label) = match view {
            DocumentView::Rendered => (IconName::Source, &labels.view_source),
            DocumentView::Source => (IconName::Rendered, &labels.view_rendered),
        };
        let sh = shared.clone();
        controls.push(
            button(name, label)
                .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                    sh.emit(s, DrawerIntent::ShowView(view.other()), tx)
                })
                .build(k)
                .key("preview-view-switch"),
        );
    }
    if caps.searchable {
        let sh = shared.clone();
        controls.push(
            button(IconName::Search, &labels.find)
                .expanded(st.find.open)
                .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                    if (sh.access)(s).find.open {
                        find::close(&sh, s, tx);
                    } else {
                        find::open(&sh, s);
                    }
                })
                .build(k)
                .key("preview-find"),
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
        controls.push(
            button(IconName::Open, &labels.open)
                .on_press_tx(move |s: &mut S, tx: &mut Tx| sh.emit(s, DrawerIntent::Open, tx))
                .build(k)
                .key("preview-open"),
        );
    }
    if let Some(on) = caps.wrap {
        let sh = shared.clone();
        let wrap = button(IconName::Wrap, &labels.wrap)
            // On, it keeps the pressed fill, as the open Find button does.
            .state(if on { State::Pressed } else { State::Rest })
            .on_press_tx(move |s: &mut S, tx: &mut Tx| sh.emit(s, DrawerIntent::SetWrap(!on), tx));
        controls.push(wrap.build(k).key("preview-wrap").checked(on));
    }
    let sh = shared.clone();
    controls.push(
        button(IconName::ClosePanel, &labels.close)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                (sh.access)(s).close_with(sh.animations, tx);
                sh.emit(s, DrawerIntent::Closed, tx);
            })
            .build(k)
            .key("preview-close"),
    );

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
        heading = heading.child(
            text(info.details.join(" · "))
                .key("preview-details")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .max_lines(1),
        );
    }
    let mut handle = row()
        .key("preview-handle")
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space.sm)
        .items_center()
        .children(info.icon.map(|i| {
            icon(i, look.icon_size.md)
                .color(k.c.icon_muted)
                .a11y_hidden()
        }))
        .child(heading);
    if let Some((dnd, source)) = parts.drag {
        handle = dnd.drag_source_keyed(handle, "preview-handle", source.clone());
    }

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
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted)
        .child(handle)
        .children(controls)
}

/// The kit's button size nearest the look's bar control size (`frame.control`). rune-kit's
/// icon buttons come in 16, 20, 24, 28 and 36 point squares, each with its own glyph size, so
/// a control size between them rounds to the nearest.
fn control_size(points: f32) -> ControlSize {
    [
        (16.0, ControlSize::Mini),
        (20.0, ControlSize::Small),
        (24.0, ControlSize::Regular),
        (28.0, ControlSize::Large),
        (36.0, ControlSize::ExtraLarge),
    ]
    .into_iter()
    .min_by(|a, b| (a.0 - points).abs().total_cmp(&(b.0 - points).abs()))
    .map_or(ControlSize::Regular, |(_, size)| size)
}
