# 108. `Tx::element_at*` callbacks run inside `Runtime::frame` see `tx.clock() == 0.0`

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A19 of the parity plan (`docs/rune/parity/packages/A19.md`), for spring-loaded drop zones on tabs and breadcrumbs |
| Affects | Any `Tx::element_at*` callback queued from an input or gesture handler inside `Runtime::frame` that reads `tx.clock()` or schedules `tx.wake_at(tx.clock() + delay)`: `strata-components`' tab strip (`tabs/drops.rs`) and location bar (`location_bar/drops.rs`) arm their spring-load timers from drop-zone `on_hover` callbacks invoked by P1's and A19's drag binders via `Tx::element_at_with` |
| Retires | `Spring::since: Option<f64>` and `tx.wake_after` in `crates/strata-components/src/tabs/{state.rs,drops.rs}` and `crates/strata-components/src/location_bar/{state.rs,drops.rs}`, which defer stamping `since` to the next `tick(now)` when `tx.clock() == 0.0` |
| Checked | By reading at `631e8a67` (2026-11-16): `Runtime::frame_step` (`runtime/frame.rs`, lines 483, 491, 637) takes `let mut tx = std::mem::take(&mut self.pending);` (whose `tx.clock` is `0.0`), advances `self.clock` without updating `tx.clock`, merges per-action `one` transactions (whose `clock` is `self.clock`, not copied by `merge`), and passes `&mut tx` to `self.run_pick_reads(&mut tx)` (`runtime/elements.rs`, lines 433-438). By hosted repro in `strata-app::dnd::tests`: a folder drag crossing an inactive tab at `clock = 2.5` s stamped `Spring.since = 0.0` and sprung the tab on the very next poll (`0 ms` instead of `450 ms`) |
| Status | Drafted, not filed: to file at the owner's filing pass |

Paste from here.

---

**Title:** `Tx::element_at*` callbacks inside `Runtime::frame` read `tx.clock() == 0.0` instead of the frame's clock

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. Its drag-and-drop
binders hit-test drop zones with `Tx::element_at_with`, and tab/crumb drop zones arm spring-load
timers from `tx.clock()` inside that callback.

In `Runtime::update_inner`, `one.clock` is set to `self.now_clock()` before `run_pick_reads(&mut one)`
runs. In `Runtime::frame_step` (`runtime/frame.rs`), `let mut tx = std::mem::take(&mut self.pending);`
leaves `tx.clock` at `0.0`: `self.clock` advances on line 491, each queued action runs with its own
`self.transaction()` (`one.clock = self.clock`) whose `pick_reads` merge into `tx`, and
`self.run_pick_reads(&mut tx)` on line 637 invokes the `Tx::element_at*` callbacks with `&mut tx`
still carrying `tx.clock == 0.0`. Any timer stamped from `tx.clock()` inside that callback appears
seconds overdue on the next `poll`.

**Repro sketch** (headless, `631e8a67`):
1. Build a `Runtime` with an element carrying `on_pointer_move(|_, _, tx| tx.element_at(10.0, 10.0, |s, _, tx| s.seen_clock = tx.clock()))`.
2. Step `rt.frame(2.5)`, then `rt.pointer_move(10.0, 10.0)` and `rt.frame(0.016)`.
3. Expected: `rt.state.seen_clock` is `~2.516` (`rt.clock`). Actual: `rt.state.seen_clock` is `0.0`.

**Ask.** Set `tx.clock = self.clock;` once `self.clock` advances in `Runtime::frame_step` (or stamp
`tx.clock = self.now_clock()` in `run_pick_reads`), so `Tx::element_at*` callbacks read the same
clock as the action that queued them.
