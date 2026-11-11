//! The strip: one focus stop around a sideways scroll viewport of columns, with the strip's bar
//! over it. Keys go to the active column, and assistive technology hears that column's cursor
//! row as the strip's active descendant.

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
        .scroll_x()
        .on_scroll_offset(move |pos| {
            feed.set(pos);
            bar.follow(pos);
        })
        .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
            let p = (layout.parts_mut)(s);
            p.state.strip = Rect::from(r);
            if p.state.reveal_after_layout {
                p.state.reveal(p.look, tx);
            } else {
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
        .w_full()
        .h_full()
        .min_w(0.0)
        .on_key(move |s: &mut S, e, tx| strip_key(&keys_sh, s, e, tx))
        .child(with_bar::<S>(st, look, viewport));
    // The active column's cursor row, or the column itself while it has no cursor, so a change
    // of column is heard as the listbox it lands in.
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
