# 89. Presence motion takes a long frame's whole step

| | |
| --- | --- |
| Kind | New issue |
| Drafts | C15's "a frame longer than 33 ms" gap |
| Affects | C15 (dialog open and close) |
| Retires | C15's known gap for the bullet "A frame longer than 33 ms…" |
| Checked | Re-checked at `2a44156` with a headless probe (`repros-head/src/bin/rv_89.rs`; the program below, `rv_89_body.rs`, printed the Output block as is): a 200 ms fade at 0.44 jumps to 0.99 on one 150 ms frame, against 0.72 on a 33 ms one. By source: a window's clock charges running motion the whole gap up to `MAX_GAP`, 250 ms (`frame/clock.rs:28`, `split` at `:47–52`), and `FrameClock::presented` holds only motion the late frame started (`aea4606d`, before the pin; `:111–117`). The module docs make keeping real time through a hitch the design, as Core Animation does (`frame/clock.rs:4–8`). No later commit changes it. Gallery: C15's known gap is still there (`dialog/mod.rs:89–92`, coverage row `:117`). Earlier: from C15's notes at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Running presence motion takes a long frame's whole step; clamp it (to 33 ms, as GTK does)

Context: Strata (lgse/strata), a file manager on Rune.

Since `aea4606`, motion that a late frame starts holds its first pose for the overrun instead of
jumping. Motion already running still takes the whole step, up to 250 ms (`MAX_GAP`,
`frame/clock.rs:28` at `2a44156`), so a dialog's 200 ms open that meets a 150 ms frame jumps
most of the way. We know keeping real time through a hitch is the design, as Core Animation
does (`frame/clock.rs:4`); GTK instead clamps a frame's step to 33 ms for its animations, and a
port that matches GTK's look needs that.

**Repro** (headless, at `2a44156`; `Runtime::frame(dt)` steps running motion by `dt`, as a
window's clock does up to `MAX_GAP`):

```rust
use rune::{Cx, El, Env, Runtime, Spring, SpringPair, Transition, col, el};
#[derive(Default)] struct S { open: bool }
fn view(s: &S, _: &mut Cx<S>) -> El {
    let spring = Spring::new(0.2, 0.0); // a dialog's 200 ms open
    let dialog = el().key("dialog").size(200.0, 100.0)
        .transition(Transition::opacity().springs(SpringPair::new(spring, spring)));
    col().w_full().h_full().children(s.open.then_some(dialog))
}
fn run(long: f64) -> String {
    let mut rt = Runtime::new(S::default(), view, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.016);
    rt.update(|s, _| s.open = true);
    let mut t = 0.0;
    let mut out = Vec::new();
    for dt in [0.016, 0.016, 0.016, long] {
        rt.frame(dt);
        t += dt;
        let opacity = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("dialog")).unwrap().opacity;
        out.push(format!("{:.0} ms {opacity:.2}", t * 1000.0));
    }
    out.join(", ")
}
fn main() {
    println!("a 150 ms frame: {}", run(0.150));
    println!("a  33 ms frame: {}", run(0.033));
}
```

**Output:**

```
a 150 ms frame: 16 ms 0.09, 32 ms 0.27, 48 ms 0.44, 198 ms 0.99
a  33 ms frame: 16 ms 0.09, 32 ms 0.27, 48 ms 0.44, 81 ms 0.72
```

**Ask.** Clamp a running transition's step to a maximum (33 ms), or make it an option.
