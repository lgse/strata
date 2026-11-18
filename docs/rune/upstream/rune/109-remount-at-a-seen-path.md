# 109. A node mounted where one just left takes its id, and loses its scene entry with the old subtree

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A19 of the parity plan (`docs/rune/parity/packages/A19.md`), for A4's `tabs/hovered-tab-takes-drop` |
| Affects | Any element mounted under a new parent at a path whose element leaves in the same commit with its subtree: `strata-app`'s hidden drag stand-in (`dnd::keep_alive`), which keeps Rune's held press alive while a spring-loaded tab hides the drag's source |
| Retires | `StandIn` in `crates/strata-app/src/dnd/drag.rs` and `Wiring::poll_stand_in` in `crates/strata-app/src/dnd/router.rs`: the stand-in waits a commit after its source leaves, and that commit unmounts the source with `Tx::instant` |
| Checked | By reading at `631e8a67` (2026-11-17): `resolve_path_and_id` (`runtime/commit/flatten.rs`) gives a new node at a path in `path_ids` that path's old `NodeId`; the apply loop over old nodes (`runtime/commit/apply.rs`, around line 748) skips an old node whose id is in the new `node_index`, so no scene entry is made for the new node, which keeps the old one; `Runtime::node` (`runtime/input/keys.rs:232`) skips a node in an unmounted subtree. By hosted repro in `strata-app::dnd::tests` (`hovering_inactive_tab_springs_tab_and_keeps_drag_alive_until_drop`): with the workaround reverted the drop is lost in 5 of 6 runs, with it 20 of 20 pass. The scene parent the new node inherits is our reading, not traced |
| Status | Drafted, not filed: to file at the owner's filing pass |

Paste from here.

---

**Title:** A node mounted at a path another node leaves in the same commit takes its id and is lost with the old subtree

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. A drag's source can
be hidden while its press is held (a spring-loaded tab shows another folder), so Strata mounts
a hidden stand-in at the source's path elsewhere in the tree, for the held press to keep
reaching an `on_drag` handler.

When the stand-in mounts in the commit that removes the source's subtree, `resolve_path_and_id`
(`runtime/commit/flatten.rs`) gives it the source's `NodeId` from `path_ids`. The apply loop over
old nodes (`runtime/commit/apply.rs`) then skips the old node, its id being in the new
`node_index`, and the stand-in keeps the old node's scene entry, which still sits under the
removed ancestors. When they are removed or unmounted, the entry goes with them: `Runtime::node`
counts the stand-in unmounted, and the held press's `Action::Drag` moves and release reach no
handler. Whether it happens varies with the commit's timing, so it shows as a flaky lost drop.

**Repro sketch** (headless, `631e8a67`):
1. A view with `col().key("a").child(el().key("src").on_drag(..))` while `s.hide` is false; while
   it is true, `col().key("b")` and, as a sibling, an element whose keys make the same path as
   `a/src` (or, with a stable path, `a` removed and `src` remounted under another keyed parent).
2. `pointer_down` on `src`, move past the threshold, set `s.hide = true` and commit.
3. `pointer_move` and `pointer_up`. Expected: the remounted element's `on_drag` hears `Move` and
   `Up`. Actual: at times neither arrives.

**Ask.** Give a node whose old parent is gone a fresh scene entry under its new parent (or a fresh
id), so a node at a reused path is mounted where it now is.
