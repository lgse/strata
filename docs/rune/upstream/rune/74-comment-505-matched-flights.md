# 74. Comment on #505: matched flights that start late or arc, and rotation in a beat

| | |
| --- | --- |
| Kind | Comment on [#505](https://github.com/anomalyco/rune/issues/505) (timed curves for presence transitions, matched moves and the press scale) |
| Drafts | U-C23-1, U-C23-2 |
| Affects | C23 (trash flight, shudder) |
| Retires | C23's deviations 1 (a straight path), 2 (stagger by arrival) and 4 (a scale-only shake) |
| Checked | Re-checked at `2a44156` (2026-11-10) by source: `El::motion` takes a `Spring` (`el/motion.rs:89`); a flight takes `tx.motion`, else the element's matched spring, else `REFLOW`, into `LayoutAnim::new(spring)` with no delay or path (`runtime/motion/flights.rs:194-197`, launches `:296-297`); `Tx::motion_curve` (`el/tx.rs:536`) turns a curve into a zero-bounce spring for matched flights, so its `Anim::delay` doesn't reach them; `Beat` is `at`, `opacity`, `scale`, `easing` (`rune-motion/src/sequence.rs:5-11`); `El::rotate` changes on commits (`el/visuals.rs:381`, curve from `rotate_motion` `:389`, both at the pin). The flight commits since the pin (`f7bf9e04`, `5a9f64e8` for #413, `8d96f0ee`) fix where a flight starts and keep it through an instant change; none adds a delay, a path or a rotating beat. #505 is open with no comments (`gh issue view`, 2026-11-10). Gallery: `mise run test -- c23::` (4 behaviour tests plus a11y, png and scripts) passes; deviations 1, 2 and 4 stand (`motion/mod.rs:88-99`, `motion/pulse.rs:67`). Earlier: by source at `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

Two more needs for matched moves, from Strata's (lgse/strata, a file manager on Rune) "move to
Trash" flight (icons fly from the list into the Trash button):

- **A start delay and a path.** Two `El::matched` flyers swapped in one commit can differ only by
  their springs: a staggered departure takes a commit per flyer, and an arcing path can't be
  had. A delay on `.motion()` (or a per-element delay for matched moves) and a path bend
  (`matched_arc(height)`) would cover it, with timed curves as asked here.
- **Rotation in a finite beat.** A shudder (±8° with a scale) can't be a `Beat`, which has
  opacity and scale only; `El::rotate` changes on commits instead (on its spring or
  `rotate_motion`'s curve), a commit per swing.
