// SPDX-License-Identifier: MIT

//! The strip's reveal with the look's animations off, in a host that leaves the system's
//! reduced motion alone: Strata's own animations switch is not the desktop's reduced motion,
//! and under Rune's reduced motion every glide lands at once anyway (`scroll_to_with` snaps), so
//! only a host like this one shows the view choosing the instant scroll itself.

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, Tx, col};
use strata_kit::look::Look;

use super::super::*;
use crate::collection::selection::Listing;
use crate::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource};
use crate::scroll::Autoscroll;

const W: f32 = 500.0;
const DT: f64 = 1.0 / 60.0;

struct Sources(Vec<ListSource>);

impl ColumnSources for Sources {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn source(&self, depth: usize) -> &dyn EntrySource {
        &self.0[depth]
    }
    fn version(&self, _depth: usize) -> u64 {
        1
    }
    fn title(&self, _depth: usize) -> &str {
        "Folder"
    }
    fn open_child(&self, depth: usize) -> Option<EntryId> {
        (depth + 1 < self.0.len()).then(|| self.0[depth].id_at(0))
    }
}

struct Host {
    sources: Sources,
    columns: ColumnsState,
    look: Look,
    /// Whether the host's hook takes each key ahead of the columns.
    takes_keys: bool,
}

fn listing(depth: u64) -> ListSource {
    ListSource::new(
        (0..5)
            .map(|i| {
                ListEntry::new(
                    EntryId::new(depth * 100 + i + 1),
                    format!("entry {i}"),
                    EntryKind::Folder,
                    "",
                )
            })
            .collect(),
    )
}

fn view_of() -> ColumnsView<Host> {
    ColumnsView::new(
        |s: &Host| ColumnsParts {
            sources: &s.sources,
            state: &s.columns,
            look: &s.look,
        },
        |s: &mut Host| ColumnsPartsMut {
            sources: &s.sources,
            state: &mut s.columns,
            look: &s.look,
        },
    )
    .collection(|_, _, column| column.before_keys(|s: &mut Host, _, _| s.takes_keys))
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    col().size(W, 400.0).child(view_of().build(s))
}

/// Three columns in a 500 pt strip, the first active, the strip at its start; the look's
/// animations as `animations`, the system's reduced motion off.
fn host(animations: bool) -> Runtime<Host> {
    let mut look = Look::fallback(false);
    look.animations = animations;
    let mut host = Host {
        sources: Sources(Vec::new()),
        columns: ColumnsState::new("t", Autoscroll::new()),
        look,
        takes_keys: false,
    };
    let mut tx = Tx::default();
    for depth in 0..3 {
        host.sources.0.push(listing(depth));
        let source = &host.sources.0[depth as usize];
        host.columns.push(
            Opening::new(source, Listing::Complete),
            Focus::No,
            &host.look,
            &mut tx,
        );
    }
    let mut rt = Runtime::new(host, view, (W, 400.0), Env::new(false, 1.0));
    rt.set_system_reduced_motion(false);
    rt.poll = Some(|s: &mut Host, t, tx| view_of().tick(s, t, tx));
    rt.frame(0.0);
    for _ in 0..10 {
        rt.frame(DT);
    }
    rt
}

/// The strip's offset over the frames after column 2 becomes active (900 points of columns in
/// a 500 point strip: the strip goes to 400).
fn offsets_after_reveal(animations: bool) -> Vec<f32> {
    let mut rt = host(animations);
    assert_eq!(rt.state.columns.strip_offset(), 0.0);
    rt.update(|s, tx| {
        let look = s.look.clone();
        s.columns.set_active(2, &look, tx);
    });
    (0..4)
        .map(|_| {
            rt.frame(DT);
            rt.state.columns.strip_offset()
        })
        .collect()
}

#[test]
fn with_animations_off_the_reveal_lands_at_once() {
    // browser/view-modes/columns: A keyboard move reveals a column clipped by up to 4 px fully
    // (with animations off, at once: the strip makes no glide).
    let off = offsets_after_reveal(false);
    assert_eq!(off[0], 400.0, "{off:?}");
    let on = offsets_after_reveal(true);
    assert!(
        on[0] > 0.0 && on[0] < 400.0,
        "the same reveal glides with them on: {on:?}"
    );
}

/// The strip's offset after Down in a clipped third column that became active without a
/// reveal, with the host's hook taking the key or leaving it to the column.
fn offset_after_down(host_takes: bool) -> (f32, Option<EntryId>) {
    let mut rt = host(false);
    rt.update(|s, tx| {
        s.takes_keys = host_takes;
        s.columns.set_active_quiet(2);
        tx.focus("t", false);
    });
    rt.frame(DT);
    assert_eq!(rt.state.columns.strip_offset(), 0.0);

    rt.key(KeyEvent {
        key: Key::Down,
        mods: Mods::default(),
    });
    rt.frame(DT);
    rt.frame(DT);

    let cursor = rt
        .state
        .columns
        .column(2)
        .and_then(|c| c.collection.selection.cursor());
    (rt.state.columns.strip_offset(), cursor)
}

/// Strata reveals a column when its cursor or the active column changes, not for a key
/// (`reveal_column` in `strip_scroll.rs`), so a key the host's hook took leaves the strip.
#[test]
fn a_key_the_hosts_hook_took_moves_no_cursor_and_does_not_reveal_the_column() {
    let (offset, cursor) = offset_after_down(false);
    assert!(cursor.is_some(), "the column's own Down places the cursor");
    assert_eq!(offset, 400.0, "and the strip shows the column");

    assert_eq!(offset_after_down(true), (0.0, None));
}
