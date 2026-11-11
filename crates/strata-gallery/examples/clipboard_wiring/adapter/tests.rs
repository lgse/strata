//! Strata's clipboard rules on [`Board`], without P2: marks, the badge and the cut's
//! remainder. (What crosses processes is in `tests/x3.rs`, under W3.)

use std::path::PathBuf;

use super::{After, Board, Mark};

fn p(name: &str) -> PathBuf {
    PathBuf::from("/root").join(name)
}

#[test]
fn a_copy_and_a_cut_mark_their_entries_and_show_the_badge() {
    // The marks only; which icon a row shows for its mark is the PNG goldens' and the eye's.
    let mut b = Board::<u32>::default();
    b.copied(1, vec![p("a"), p("b")], false);
    assert_eq!(
        (b.mark(&p("a")), b.mark(&p("c"))),
        (Mark::Copied, Mark::None)
    );
    assert!(b.badge());
    b.copied(2, vec![p("b")], true);
    assert_eq!((b.mark(&p("a")), b.mark(&p("b"))), (Mark::None, Mark::Cut));
    // The earlier copy's loss (replaced by ours) changes nothing.
    b.lost(1);
    assert_eq!(
        (b.mark(&p("b")), b.badge(), b.held()),
        (Mark::Cut, true, Some(2))
    );
}

#[test]
fn another_apps_copy_or_an_empty_clipboard_clears_every_mark() {
    // operations/clipboard: When another application takes the clipboard, copy and cut marks clear in every window.
    let mut b = Board::<u32>::default();
    b.copied(1, vec![p("a")], true);
    b.lost(1);
    assert_eq!(
        (b.mark(&p("a")), b.badge(), b.held()),
        (Mark::None, false, None)
    );
    b.copied(2, vec![p("a")], false);
    assert_eq!(b.offered(false), None, "text: no read");
    assert_eq!((b.mark(&p("a")), b.badge()), (Mark::None, false));
    b.copied(3, vec![p("a")], false);
    b.emptied();
    assert_eq!((b.mark(&p("a")), b.badge()), (Mark::None, false));
}

#[test]
fn the_badge_follows_another_apps_file_list_and_drops_a_late_read() {
    let mut b = Board::<u32>::default();
    let read = b.offered(true).expect("files may be offered: read them");
    assert!(!b.badge(), "off until the read answers");
    b.files_read(read, true);
    assert!(b.badge());
    let empty = b.offered(true).unwrap();
    b.files_read(empty, false);
    assert!(!b.badge(), "an empty file list shows no badge");
    // A read that answers after a newer change is dropped.
    let late = b.offered(true).unwrap();
    b.offered(false);
    b.files_read(late, true);
    assert!(!b.badge());
    let late = b.offered(true).unwrap();
    b.copied(7, vec![p("a")], false);
    b.lost(7);
    b.files_read(late, true);
    assert!(!b.badge());
}

#[test]
fn only_what_we_cut_pastes_as_our_cut() {
    // operations/clipboard/file-manager-formats: A cut made in Strata still pastes as a move when the clipboard carries no cut marker, because the paste also checks Strata's own cut list.
    let mut b = Board::<u32>::default();
    assert!(!b.is_our_cut(&[p("a")]));
    b.copied(1, vec![p("a"), p("b")], true);
    assert!(b.is_our_cut(&[p("b"), p("a")]));
    assert!(!b.is_our_cut(&[p("a")]));
    assert!(!b.is_our_cut(&[]));
    b.copied(2, vec![p("a"), p("b")], false);
    assert!(!b.is_our_cut(&[p("a"), p("b")]), "a copy is no cut");
}

#[test]
fn a_cut_paste_takes_the_moved_entries_off_the_clipboard() {
    // operations/clipboard: Completing a cut-paste removes the moved items from the clipboard; text copied after the cut stays on the clipboard.
    let mut b = Board::<u32>::default();
    b.copied(1, vec![p("a"), p("b")], true);
    assert_eq!(b.moved(&[p("c")]), After::Keep, "none of ours");
    // Part of the cut: the rest is to be cut again (the formats that copy offers are read in
    // tests/x3.rs, `a_partial_cut_paste_keeps_the_rest_cut`).
    assert_eq!(b.moved(&[p("a")]), After::Recut(1, vec![p("b")]));
    assert_eq!((b.mark(&p("a")), b.mark(&p("b"))), (Mark::None, Mark::Cut));
    assert_eq!(b.moved(&[p("b")]), After::Clear(1));
    assert_eq!(
        (b.mark(&p("b")), b.badge(), b.held()),
        (Mark::None, false, None)
    );
    // Another app's copy came after the cut: nothing of ours to clear.
    b.copied(2, vec![p("a")], true);
    b.lost(2);
    assert_eq!(b.moved(&[p("a")]), After::Keep);
    // A copy (not a cut) stays after its files are pasted.
    b.copied(3, vec![p("a")], false);
    assert_eq!(b.moved(&[p("a")]), After::Keep);
    assert_eq!(b.release(), Some(3));
    assert_eq!(
        (b.mark(&p("a")), b.badge(), b.release()),
        (Mark::None, false, None)
    );
}
