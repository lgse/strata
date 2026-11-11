//! The demo host, headless: its window keys and its springs. (What crosses processes is in
//! `tests/x2.rs`, under W3.)

use std::path::PathBuf;

use rune::{Env, Runtime};
use strata_components::collection::element::DropOver;
use strata_components::contract::EntryId;
use strata_components::contract::dnd::DropHover;
use strata_components::sidebar::{DropSite, ItemId};

use super::*;

/// A fixture folder under the gallery's target directory (never the desktop's /tmp).
fn fixture(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp/x2-host")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    for d in ["Documents/Reports", "Music", "pinned", "usb"] {
        std::fs::create_dir_all(root.join(d)).expect("fixture folder");
    }
    std::fs::write(root.join("notes.txt"), "n").expect("fixture file");
    root.canonicalize().expect("the root")
}

fn mount(name: &str) -> Runtime<Win> {
    let root = fixture(name);
    let wiring = Wiring::new(strata_platform::dnd::Dnd::new());
    let w = Win::new("main".into(), root, false, wiring);
    let mut rt = Runtime::new(w, view, (1100.0, 700.0), Env::new(false, 1.0));
    rt.poll = Some(poll);
    rt.frame(0.0);
    rt
}

/// Frames of 1/60 s (`Runtime::frame` takes the step) until the host's clock reaches `t`.
fn run_until(rt: &mut Runtime<Win>, t: f64) {
    while rt.state.clock < t {
        rt.frame(1.0 / 60.0);
    }
}

/// Every target a binder finds by key must be keyed apart: no element in any tab's window
/// repeats a key among its siblings (Rune renames a repeat `key#dup1`), the inactive panes
/// kept for a running drag included.
#[test]
fn no_key_repeats_in_any_tab_or_with_every_pane_kept() {
    let mut rt = mount("keys");
    for tab in [1, 2, 3] {
        rt.update(|w, tx| w.select_tab(TabId(tab), "test", tx));
        let t = rt.state.clock + 0.5;
        run_until(&mut rt, t);
        let dups: Vec<String> = rt
            .elements()
            .into_iter()
            .filter_map(|e| e.key)
            .filter(|k| k.contains("#dup"))
            .collect();
        assert!(dups.is_empty(), "tab {tab}: {dups:?}");
    }
    // A drag from tab 1 keeps its pane mounted under tab 3's.
    rt.state.out = Some(Out {
        tab: TabId(1),
        ids: Vec::new(),
    });
    rt.update(|_, tx| tx.wake_after(Duration::ZERO));
    let t = rt.state.clock + 0.5;
    run_until(&mut rt, t);
    let keys: Vec<String> = rt.elements().into_iter().filter_map(|e| e.key).collect();
    assert!(keys.iter().any(|k| k == "pane-1") && keys.iter().any(|k| k == "pane-3"));
    assert!(!keys.iter().any(|k| k.contains("#dup")), "{keys:?}");
}

/// Steps a resting window: a frame only at each wake it asked for, none in between, until the
/// clock passes `until` or nothing more is asked.
fn rest(rt: &mut Runtime<Win>, until: f64) {
    for _ in 0..200 {
        let Some(deadline) = rt.wake_deadline() else {
            break;
        };
        let now = rt.state.clock;
        if deadline > until {
            break;
        }
        rt.frame((deadline - now).max(0.001));
    }
}

/// A hover change at time `t`: the window rests until then (stepping only its own wakes),
/// then `change` runs, as a zone's hover handler would, and the window draws one frame (the
/// highlight changed) and polls.
fn hovered_at(rt: &mut Runtime<Win>, t: f64, change: impl FnOnce(&mut Win)) {
    rest(rt, t);
    change(&mut rt.state);
    rt.frame((t - rt.state.clock).max(0.001));
}

/// operations/drag-and-drop: "Holding a file drag over a folder row or sidebar row for 750 ms
/// navigates there"; leaving first cancels it. The host's spring, in a window at rest: it
/// polls only at the wakes the host asked for, and when a hover changes.
#[test]
fn a_folder_row_springs_open_after_750_ms_and_a_leave_cancels_it() {
    let mut rt = mount("spring-row");
    rt.update(|w, tx| w.select_tab(TabId(2), "test", tx));
    run_until(&mut rt, 0.5);
    let docs = rt.state.ids.of(&rt.state.root.join("Documents"));
    let music = rt.state.ids.of(&rt.state.root.join("Music"));
    let over = |rt: &mut Runtime<Win>, id: Option<EntryId>, t: f64| {
        hovered_at(rt, t, |w| {
            w.flat_mut(TabId(2))
                .coll
                .set_drop_over(id.map(DropOver::Row))
        });
    };
    // Over Music for 0.5 s, away for 0.2 s, back for 0.5 s: its wait started over, nothing opens.
    over(&mut rt, Some(music), 1.0);
    over(&mut rt, None, 1.5);
    over(&mut rt, Some(music), 1.7);
    over(&mut rt, None, 2.2);
    rest(&mut rt, 4.0);
    assert_eq!(
        rt.state.folder_of(TabId(2)),
        rt.state.root,
        "a leave restarts the wait"
    );
    // Over Documents: it opens once 0.75 s have passed, not before.
    over(&mut rt, Some(docs), 5.0);
    rest(&mut rt, 5.7);
    assert_eq!(rt.state.folder_of(TabId(2)), rt.state.root, "not yet");
    rest(&mut rt, 6.5);
    assert_eq!(
        rt.state.folder_of(TabId(2)),
        rt.state.root.join("Documents")
    );
}

#[test]
fn a_sidebar_row_springs_open_its_place() {
    let mut rt = mount("spring-place");
    run_until(&mut rt, 0.2);
    let music = ItemId::Place(super::path_id(&rt.state.root.join("Music")));
    hovered_at(&mut rt, 0.3, |w| {
        w.sidebar
            .set_drop_hover(DropSite::Item(music), DropHover::Accepting)
    });
    rest(&mut rt, 1.5);
    assert_eq!(
        rt.state.folder_of(rt.state.active),
        rt.state.root.join("Music")
    );
}

/// trash: "Dropping files onto the sidebar Trash row moves them to Trash"; "Web addresses
/// dropped on the sidebar Trash row, alone or with files, are refused".
#[test]
fn the_trash_takes_files_and_refuses_web_addresses() {
    let both = DropActions::COPY | DropActions::MOVE;
    let offer = |uris: Option<&[&str]>| {
        let o = DropOffer::new(vec![URI_LIST.into()], both);
        match uris {
            Some(u) => o.uris(u.iter().map(|u| (*u).to_owned()).collect()),
            None => o,
        }
    };
    let file = "file:///home/someone/notes.txt";
    let web = "https://example.org/page";
    assert_eq!(trash_action(&offer(Some(&[file]))), Some(DropAction::Move));
    assert_eq!(
        trash_action(&offer(None)),
        Some(DropAction::Move),
        "not read yet"
    );
    assert_eq!(trash_action(&offer(Some(&[web]))), None);
    assert_eq!(trash_action(&offer(Some(&[file, web]))), None);
    let text = DropOffer::new(vec!["text/plain".into()], both);
    assert_eq!(trash_action(&text), None, "content");
    let copy_only = DropOffer::new(vec![URI_LIST.into()], DropActions::COPY);
    assert_eq!(trash_action(&copy_only), None);
}

/// The folders Columns tab 1 shows, column by column.
fn columns(rt: &Runtime<Win>) -> Vec<PathBuf> {
    let (listings, state) = rt.state.cols(TabId(1));
    assert_eq!(listings.0.len(), state.len(), "listings and columns agree");
    listings.0.iter().map(|l| l.folder.clone()).collect()
}

/// P1 keeps a drag's source mounted until the drag ends: while a drag from Columns' second
/// column runs, a spring on a sibling folder, a crumb jump and a Close leave that column be.
#[test]
fn a_running_drag_keeps_its_source_column() {
    let mut rt = mount("source-column");
    let root = rt.state.root.clone();
    let (docs, music) = (root.join("Documents"), root.join("Music"));
    rt.update(|w, tx| w.navigate(TabId(1), &docs, tx));
    rt.frame(1.0 / 60.0);
    assert_eq!(columns(&rt), [root.clone(), docs.clone()]);
    // A drag of Reports, in the Documents column (P1 can't start one headless).
    let reports = rt.state.ids.of(&docs.join("Reports"));
    rt.state.out = Some(Out {
        tab: TabId(1),
        ids: vec![reports],
    });
    let music_id = rt.state.ids.of(&music);
    rt.update(|w, tx| {
        let look = w.look.clone();
        w.open_in_columns(TabId(1), music_id, &look, tx);
    });
    rt.update(|w, tx| w.navigate(TabId(1), &music, tx));
    rt.update(|w, tx| w.navigate(TabId(1), &root, tx));
    rt.update(|w, tx| w.columns_intent(TabId(1), ColumnsIntent::CloseFrom(1), tx));
    rt.frame(1.0 / 60.0);
    assert_eq!(
        columns(&rt),
        [root.clone(), docs.clone()],
        "the source is kept"
    );
    // Deeper is fine: Reports opens after the source.
    rt.update(|w, tx| w.navigate(TabId(1), &docs.join("Reports"), tx));
    rt.frame(1.0 / 60.0);
    assert_eq!(
        columns(&rt),
        [root.clone(), docs.clone(), docs.join("Reports")]
    );
    // Once the drag ends, the spring goes through.
    rt.state.out = None;
    rt.update(|w, tx| w.navigate(TabId(1), &music, tx));
    rt.frame(1.0 / 60.0);
    assert_eq!(columns(&rt), [root, music]);
}
