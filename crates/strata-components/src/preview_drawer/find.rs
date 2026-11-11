//! The find bar under the header: the query field, the status ("Match 1 of N", "No
//! matches", hidden while the query is empty), Previous, Next and Close find. The hosted view
//! answers the search through [`HostedPreview`](super::HostedPreview).

use rune::{El, Key, KeyEvent, LiveRegion, Tx, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, TextInput};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::{Caps, DrawerState, FIND_MAX_CHARS, Shared};

/// The query field's key.
const FIELD_KEY: &str = "preview-find-field";

pub(super) fn bar<S: 'static>(
    shared: &Shared<S>,
    st: &DrawerState,
    caps: &Caps,
    look: &Look,
) -> El {
    let k = &look.kit;
    let labels = &shared.labels;
    let access = shared.access;
    let status = caps.status.unwrap_or_default();
    // No status while the query is empty, or while the view hasn't answered yet.
    let show_status = !st.find.query.text.is_empty() && caps.status.is_some();

    let on_change = shared.clone();
    let on_key = shared.clone();
    let field = TextInput::new(&st.find.query)
        .label(labels.find_field.clone())
        .placeholder(labels.find_field.clone())
        .width(None)
        .on_edit(move |s: &mut S| &mut access(s).find.query)
        .on_change(move |s: &mut S, _tx: &mut Tx| {
            let query = &mut (on_change.access)(s).find.query;
            if query.text.chars().count() > FIND_MAX_CHARS {
                let cut: String = query.text.chars().take(FIND_MAX_CHARS).collect();
                query.replace_text(cut);
            }
            let q = query.text.clone();
            on_change.with_view(s, |v| v.find(&q));
        })
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            // The field would blur on Escape; it closes find instead, as Escape anywhere in
            // the drawer does.
            if ev.key == Key::Escape {
                close(&on_key, s, tx);
                return true;
            }
            // Enter and Shift+Enter step only while focus is in the field.
            if ev.key == Key::Enter && !ev.mods.cmd && !ev.mods.ctrl && !ev.mods.alt {
                let forward = !ev.mods.shift;
                on_key.with_view(s, |v| v.find_step(forward));
                return true;
            }
            false
        })
        .build(k)
        .key(FIELD_KEY)
        .grow(1.0)
        .min_w(0.0);

    let step = |name: IconName, label: &str, key: &'static str, forward: bool| {
        let sh = shared.clone();
        IconButton::new(look.icon(name))
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(label.to_owned())
            .disabled(status.total == 0)
            .on_press(move |s: &mut S| sh.with_view(s, |v| v.find_step(forward)))
            .build(k)
            .key(key)
    };
    let close_sh = shared.clone();
    let close = IconButton::new(look.icon(IconName::Close))
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label(labels.find_close.clone())
        .on_press_tx(move |s: &mut S, tx: &mut Tx| close(&close_sh, s, tx))
        .build(k)
        .key("preview-find-close");

    let mut bar = row()
        .key("preview-find-bar")
        .w_full()
        .px(look.space.md)
        .py(look.space.xs)
        .gap(look.space.xs)
        .items_center()
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted)
        .child(field);
    if show_status {
        let words = match status.current {
            Some(n) if status.total > 0 => (labels.match_of)(n, status.total),
            _ if status.total == 0 => labels.no_matches.clone(),
            _ => (labels.match_of)(0, status.total),
        };
        bar = bar.child(
            text(words)
                .key("preview-find-status")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .live_region(LiveRegion::Polite),
        );
    }
    bar.child(step(
        IconName::Up,
        &labels.find_previous,
        "preview-find-previous",
        false,
    ))
    .child(step(
        IconName::Down,
        &labels.find_next,
        "preview-find-next",
        true,
    ))
    .child(close)
}

/// Closes the find bar: the view keeps its selected match, and keys go back to the document.
pub(super) fn close<S: 'static>(shared: &Shared<S>, s: &mut S, _tx: &mut Tx) {
    let st = (shared.access)(s);
    st.find.open = false;
    st.find.query.focused = false;
    st.focus_content();
    shared.with_view(s, |v| v.find_end());
}

/// Opens the find bar, and runs the query it kept on the view, as Strata's find refreshes
/// on reopening.
pub(super) fn open<S: 'static>(shared: &Shared<S>, s: &mut S) {
    let st = (shared.access)(s);
    st.open_find();
    let query = st.find.query.text.clone();
    if !query.is_empty() {
        shared.with_view(s, |v| v.find(&query));
    }
}
