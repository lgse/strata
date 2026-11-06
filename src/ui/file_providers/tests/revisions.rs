// SPDX-License-Identifier: MIT
use super::*;

#[test]
fn scoped_revisions_make_progress_under_continuous_unrelated_activity() {
    let mut freshness = Freshness::default();
    let selected = vec!["/account-a/folder/file".into()];
    for revision in 1..=100 {
        freshness.invalidate(Some(&["/account-b".into()]), Some(revision));
        assert!(freshness.accept(&selected, Some(0)));
        assert!(freshness.current(&selected, 0));
    }
    freshness.invalidate(Some(&["/account-a/folder".into()]), Some(101));
    let accepted = [
        freshness.accept(&selected, Some(100)),
        freshness.accept(&selected, Some(101)),
    ];
    assert_eq!(
        accepted,
        [false, true],
        "accepts (revision 100, revision 101)"
    );
    let current = [
        freshness.current(&selected, 0),
        freshness.current(&selected, freshness.generation()),
        freshness.current(&["/account-a/folder-other".into()], 0),
        freshness.current(&["/account-a".into()], 0),
    ];
    assert_eq!(
        current,
        [false, true, true, false],
        "current (selected at 0, selected now, sibling folder at 0, parent at 0)"
    );
}

#[test]
fn legacy_snapshot_replies_survive_refresh_hints_without_claiming_freshness() {
    let mut freshness = Freshness::default();
    let selected = vec!["/file".into()];
    for _ in 0..100 {
        let generation = freshness.generation();
        freshness.invalidate(None, None);
        assert!(freshness.accept(&selected, None));
        assert!(!freshness.current(&selected, generation));
    }
    freshness.invalidate(None, Some(101));
    assert!(!freshness.accept(&selected, None));
    assert!(!freshness.accept(&selected, Some(100)));
    assert!(freshness.accept(&selected, Some(101)));
}
