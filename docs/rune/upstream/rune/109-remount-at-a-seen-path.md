# 109. A node mounted where one just left takes its id, and loses its scene entry with the old subtree

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A19 of the parity plan (`docs/rune/parity/packages/A19.md`), for A4's `tabs/hovered-tab-takes-drop` |
| Affects | Any element mounted under a new parent at a path whose element leaves in the same commit with its parent: `strata-app`'s hidden drag stand-in (`dnd::keep_alive`), which keeps Rune's held press alive while a spring-loaded tab hides the drag's source |
| Retires | `StandIn` in `crates/strata-app/src/dnd/drag.rs` and `Wiring::poll_stand_in` in `crates/strata-app/src/dnd/router.rs`: the stand-in waits a commit after its source leaves, and that commit unmounts the source with `Tx::instant` |
| Checked | By reading at `631e8a67` (2026-11-18), traced step by step below. By headless repro at `631e8a67` (2026-11-18, a scratch test against the pinned `rune`, the sketch below): the stand-in hears no `Move` and no `Up` in every run, with or without the earlier mount; with the old parent kept mounted, or the stand-in mounted one commit later, it hears both. By hosted repro in `strata-app::dnd::tests` (`hovering_inactive_tab_springs_tab_and_keeps_drag_alive_until_drop`): with the workaround reverted the drop is lost in 5 of 6 runs, with it 20 of 20 pass |
| Status | Drafted, not filed: to file at the owner's filing pass |

Paste from here.

---

**Title:** A node mounted at a path another node leaves in the same commit takes its id, and is lost with the old parent

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. A drag's source can
be hidden while its press is held (a spring-loaded tab shows another folder), so Strata mounts
a hidden stand-in at the source's path elsewhere in the tree, for the held press to keep
reaching an `on_drag` handler.

When the stand-in mounts in the commit that removes the source's parent, it is never live:

1. `resolve_path_and_id` (`runtime/commit/flatten.rs:151-187`) gives the stand-in the source's
   `NodeId`, from `path_ids`, since the path is the same.
2. The apply loop asks `self.scene.contains(id)` (`runtime/commit/apply.rs:374`), which is
   true, and calls `insert_delayed` (`apply.rs:409`). `Scene::insert_delayed`
   (`rune-motion/src/scene/presence.rs:22-38`) finds the id's slot and returns early: the entry
   keeps its old scene parent, the source's parent.
3. The loop over old nodes skips the source, its id being in the new `node_index`
   (`apply.rs:748`), and removes the source's parent, which a commit no input caused unmounts at
   once. `Scene::remove_now` (`rune-motion/src/scene.rs:1001-1008`) removes the parent's scene
   children first, the stand-in's entry among them.
4. `Runtime::node` (`runtime/input/keys.rs:232`) skips a node whose subtree is unmounted
   (`subtree_unmounted`, `runtime/input/hit.rs:119`: not in `scene.contains`), so the held
   press's queued `Action::Drag` moves and release find no handler, and the drag never ends.

**Repro** (headless, `631e8a67`; fails every run):

```rust
#[derive(Default)]
struct S { hide: bool, stand_in: bool, heard: Vec<(&'static str, Phase)> }

fn view(s: &S, _: &mut Cx<S>) -> El {
    let mut root = col().w_full().h_full();
    root = match s.hide {
        true => root.child(col().key("b").size(100.0, 40.0)),
        false => root.child(col().key("a").child(
            el().key("src").size(100.0, 40.0).on_drag(|s: &mut S, d, _| s.heard.push(("src", d.phase))),
        )),
    };
    if s.stand_in {
        // The same path as `a/src`, under the root.
        root = root.child(el().key("a/src").size(0.0, 0.0).on_drag(|s: &mut S, d, _| s.heard.push(("stand-in", d.phase))));
    }
    root
}

let mut rt = Runtime::new(S::default(), view, (300.0, 300.0), Env::new(false, 1.0));
rt.frame(0.0);
// Mount and unmount at the path once.
rt.update(|s, _| { s.hide = true; s.stand_in = true; });
rt.frame(1.0);
rt.update(|s, _| { s.hide = false; s.stand_in = false; });
rt.frame(1.0);
// Press and drag `src`, then remount at its path in the commit that removes `a`.
rt.pointer_move(10.0, 10.0);
rt.pointer_down(10.0, 10.0);
rt.pointer_move(40.0, 10.0);
rt.frame(0.016);
rt.update(|s, _| { s.hide = true; s.stand_in = true; });
rt.frame(1.0);
rt.pointer_move(60.0, 10.0);
rt.frame(0.016);
rt.pointer_up(60.0, 10.0);
rt.frame(0.016);
assert!(rt.state.heard.contains(&("stand-in", Phase::Up))); // fails: only src's Down and Move
```

It fails without the first mount and unmount too. It passes when `a` stays mounted (an empty
`col().key("a")` in place of `b`), or when the stand-in mounts one commit after `a` leaves.

**Ask.** When `insert_delayed` finds an id already in the scene under a parent other than the
new node's, move the entry under its new parent (or give the node a fresh id), so a node at a
reused path is mounted where it now is.
