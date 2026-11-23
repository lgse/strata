// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{El, Tx, col};

use super::keys;
use super::view::{Shared, collection, columns_row, strip_key, with_bar};
use crate::contract::Rect;

pub(super) fn build<S: 'static>(sh: &Rc<Shared<S>>, s: &S) -> El {
    let p = (sh.parts)(s);
    let st = p.state;
    let look = p.look;
    let (feed, bar) = (st.strip_feed.clone(), st.strip_bar.clone());
    let layout = sh.clone();
    let viewport = col()
        .key(keys::strip(&st.key))
        .w_full()
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .scroll_x()
        .drawn_rect_into(&st.strip_drawn)
        .on_scroll_offset(move |pos| {
            feed.set(pos);
            bar.follow(pos);
        })
        .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
            let p = (layout.parts_mut)(s);
            p.state.strip = Rect::from(r);
            let clipped_changed = p.state.sync_clipped(p.look);
            let overflow_changed = p.state.sync_strip_overflow(p.look);
            if p.state.reveal_after_layout {
                p.state.reveal(p.look, tx);
            } else if !clipped_changed && !overflow_changed {
                tx.live_only();
            }
        })
        .child(columns_row(sh, s));
    let keys_sh = sh.clone();
    let mut root = col()
        .key(st.key.clone())
        .role("group")
        .label(sh.labels.strip.clone())
        .focusable(true)
        .no_focus_ring()
        .w_full()
        .h_full()
        .min_w(0.0)
        .on_key(move |s: &mut S, e, tx| strip_key(&keys_sh, s, e, tx))
        .child(with_bar::<S>(st, look, viewport));
    let n = st.columns.len().min(p.sources.len());
    if n > 0 {
        let active = &st.columns[st.active.min(n - 1)];
        let path = collection(sh, s, active.serial)
            .cursor_path(s)
            .unwrap_or_else(|| keys::column(&st.key, active.serial));
        root = root.active_descendant(path);
    }
    root
}
