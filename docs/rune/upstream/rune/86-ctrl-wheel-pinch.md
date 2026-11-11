# 86. Off macOS, Ctrl+wheel over `on_magnify` becomes an inverted pinch with no fallback

| | |
| --- | --- |
| Kind | New issue |
| Drafts | C14a's "Ctrl+wheel turned into a pinch zooms the wrong way" draft |
| Affects | C14a (image viewport), C12 (PDF zoom) |
| Retires | C14a's `on_magnify` on macOS only, with Ctrl+wheel handled in `on_wheel` elsewhere |
| Checked | Re-checked at `2a44156` by source: `pinch_wheel` still sends `MagnifyEvent::new(-dy / PINCH_WHEEL)` (`runtime/input/gesture.rs:111–115`, `PINCH_WHEEL` 200 at `:19`) to any `on_magnify` up the chain (`gesture_chain`, `:146`); `MagnifyEvent` has no modifiers (`input/canvas_input.rs:54–62`); the window's wheel takes the pinch whenever a handler is there, declined or not (`window/events.rs:427`; the web's `platform/web/input.rs:228` likewise). No change to it past the pin. Gallery: C14a's Ctrl+wheel in `on_wheel` and its tests pass (`mise run test ctrl_wheel`: 5 tests in strata-components, c12, c14a). Earlier: by source at `5d421fd` (`:106`) and `04b38ae` (`:113`). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Off macOS, a Ctrl+wheel over any `on_magnify` (ancestors included) becomes a pinch: inverted, without modifiers, and with no fallback

Context: Strata (lgse/strata), a file manager on Rune.

`pinch_wheel` (`runtime/input/gesture.rs:111` at `2a44156`) turns a Ctrl+wheel over an
`on_magnify` handler into `MagnifyEvent::new(-dy / 200)`:
- the zoom is inverted against the wheel convention (a wheel turned away zooms out);
- the event has no modifiers, so Ctrl+Shift+wheel can't be told from a pinch;
- a declined pinch doesn't fall back to the wheel;
- it applies to an `on_magnify` anywhere up the chain, so an ancestor's zoom takes Ctrl+wheel
  from a viewport inside it.

On Linux winit 0.30.13 delivers no pinch at all, so this conversion is the only zoom path there.

**Ask.** Keep the wheel's direction, carry the modifiers, fall back to `on_wheel` when the pinch
is declined, and limit it to the innermost handler.
