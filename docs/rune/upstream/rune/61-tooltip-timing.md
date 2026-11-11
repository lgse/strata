# 61. Per-element tooltip timing

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C3-2 |
| Affects | C3 (icon names wait 900 ms; buttons the toolkit's) |
| Retires | C3's deviation "tooltips via Rune's `.help` at the host's delay" |
| Checked | Re-checked at `2a44156` with probe `rv_61` and the source. The probe sets the window's delay to 0.9 s with `Cx::help_look`; two `.help` elements both show nothing at 0.7 s and their tooltip by 1.0 s, and a 20 pt move inside the element at 0.5 s still shows it at 1.0 s, so the wait didn't restart. Source: the delay is the look's or the runtime's (`Delays { open: look.delay.unwrap_or(self.help_delay), .. }`, `runtime/input/help.rs:239`; `Cx::help_look`, `runtime/cx.rs:174`; `HelpLook::delay`, `runtime/input/help_look.rs:156`), and a frame over the same target changes nothing (`help.rs:115`). The tooltip state machine (`0cde17ec`, #383) and `El::help_row` (`e723f63b`, #452) add neither a per-element delay nor a restart distance. Gallery: C3's deviation stands (`icons_grid/mod.rs:127`); `c3::the_tooltip_waits_for_the_pages_delay` passes. Earlier: by source at `5d421fd` (`help.rs:179`) and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `El::help` has one delay per window, and a move inside the element never restarts the wait

Strata's (lgse/strata, a file manager on Rune) icon names show their full name after 900 ms, while
its buttons use the toolkit's delay; GTK restarts the wait after 4 px of movement. `El::help`'s
delay is the window's (`HelpLook::delay`, `runtime/input/help.rs:239` at `2a44156`), and a move
inside the element never restarts it (`help.rs:115`).

**Ask.** `El::help_delay(secs)`, and a restart distance.
