# 107. A `Runtime` an embedder builds can't take `App::on_modifiers`

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A4 of the parity plan (`docs/rune/parity/packages/A4.md`), for the tab strip's number badges |
| Affects | Strata's offscreen host (`crates/strata-app/src/host/offscreen.rs`), which builds a `Runtime` with `Runtime::with_text` to run the in-process tests and the QA mode, and so any part that reads the modifiers held (`shell::held_modifiers`) there |
| Retires | `Offscreen::modifiers`: the host calls the shell's hook itself each time it feeds `PhysicalInput::Modifiers` (`qa/staged.rs`, `test_support::send_key`), as it feeds them rather than in the input queue's order |
| Checked | By reading at `2a44156` (2026-11-14): `Runtime::on_mods` is `pub(crate)` (`runtime.rs`, line 394), as are `on_focus` (392) and `on_visibility` (397), where `poll` and `on_key` are public (433, 443). Only the desktop window (`desktop/window.rs`, line 132), the web host (`platform/web.rs`, line 454) and the headless tool (`tooling/headless.rs`, line 113) copy `App::on_modifiers` onto their runtime. Rune's README has no section for `on_modifiers`, so a host that embeds `Runtime` can't find the hook: it is documented only on the method (`app.rs`, lines 270-283) and in `changelog/unreleased/2026-11-08-held-modifiers.md` |
| Status | Drafted, not filed: to file at the owner's filing pass |

Paste from here.

---

**Title:** Let a `Runtime` built directly take `on_modifiers` (and `on_focus`)

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. Its tests and its
QA mode run the interface on a `Runtime` they build themselves (`Runtime::with_text`), step
it in real time beside GLib, and set its `poll` and `on_key` fields.

`App::on_modifiers` hears the modifiers held on every change, wherever focus is. A window, the
web host and the headless tool copy it onto their runtime. A runtime built directly can't take
it: `Runtime::on_mods` is `pub(crate)`, as are `on_focus` and `on_visibility`, while `poll` and
`on_key` are public fields. `PhysicalInput::Modifiers` fed to such a runtime updates its held
modifiers, and no hook hears them.

**Repro sketch** (headless, `2a44156`):
1. `let mut rt = Runtime::new(state, view, (400.0, 300.0), Env::new(true, 1.0));`
2. Try to set a modifiers hook: `rt.on_mods = Some(hook);` does not compile outside the crate,
   and nothing else takes one.
3. `rt.physical_input(PhysicalInput::Modifiers(Mods { shift: true, ..Mods::default() }))`,
   then `rt.frame(0.016)`: no handler hears Shift held.

**Ask.** A public way to give a built runtime the app-level hooks, such as
`Runtime::on_modifiers(f)` and `Runtime::on_focus(f)` setters, or public fields as `poll` and
`on_key` have; and a line on `on_modifiers` in the README beside "Modifier keys alone", so an
embedder finds it.
