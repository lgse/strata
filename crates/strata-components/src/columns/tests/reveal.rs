//! The strip's reveal with the look's animations off, in a host that leaves the system's
//! reduced motion alone: Strata's own animations switch is not the desktop's reduced motion,
//! and under Rune's reduced motion every glide lands at once anyway (`scroll_to_with` snaps), so
//! only a host like this one shows the view choosing the instant scroll itself.

use rune::{Cx, El, Env, Runtime, Tx, col};
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
