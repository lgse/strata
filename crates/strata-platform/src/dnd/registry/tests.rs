use super::*;

struct S;

fn target() -> DropTarget<S> {
    DropTarget::new(|_: &mut S, _, _| {})
}

#[test]
fn a_hit_finds_the_innermost_target_first() {
    let mut r = Registry::<S>::default();
    r.put("pane", target());
    r.put("row-3", target());
    // Rune's path: keys, with `#n` for elements without one.
    let path = "#0/pane/#1/list/row-3/#0/label";
    assert_eq!(r.along(path), vec!["row-3", "pane"]);
    assert_eq!(r.along("#0/elsewhere"), Vec::<&str>::new());
}

#[test]
fn a_target_built_again_keeps_its_rect() {
    let mut r = Registry::<S>::default();
    let a = r.put("zone", target());
    a.set([1.0, 2.0, 3.0, 4.0]);
    let b = r.put("zone", target());
    assert_eq!(b.get(), [1.0, 2.0, 3.0, 4.0], "the same Live");
    assert_eq!(r.len(), 1);
}

#[test]
fn past_the_cap_the_oldest_targets_go() {
    let mut r = Registry::<S>::default();
    for i in 0..=CAP {
        r.put(&format!("row-{i}"), target());
    }
    assert!(r.len() <= CAP, "{}", r.len());
    assert!(r.get(&format!("row-{CAP}")).is_some(), "the newest stays");
    assert!(r.get("row-0").is_none(), "the oldest goes");
}

#[test]
fn a_scoped_target_is_found_only_under_its_scope() {
    let mut r = Registry::<S>::default();
    r.put(&scoped("list-a", "row-3"), target());
    r.put(&scoped("list-b", "row-3"), target());
    r.put("list-b", target());
    let in_b = "#0/pane/list-b/#0/viewport/rows/row-3/label";
    assert_eq!(
        r.along(in_b),
        vec![scoped("list-b", "row-3"), "list-b".to_owned()]
    );
    let in_a = "#0/pane/list-a/#0/viewport/rows/row-3";
    assert_eq!(r.along(in_a), vec![scoped("list-a", "row-3")]);
    // A row of that key outside both lists has no target.
    assert_eq!(r.along("#0/elsewhere/row-3"), Vec::<String>::new());
}
