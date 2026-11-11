# 20. Mipmaps for a streamed `Pixels`

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C25-1 (and C14a's "`Pixels::set` builds no mips" note) |
| Affects | C25 (thumbnail store), W2d (thumbnail size classes), C12 (zoomed-out pages), C14a (images shrunk to fit) |
| Retires | W2d's 32/64/128/256 px thumbnail classes held at most twice their drawn size; hosts' use of `ImageCache` for stills only |
| Checked | Re-checked at `2a44156`, by a new headless probe (`rv_20`, the repro below, `--snap --scale 1`): the streamed square measures grey 7 to 248 (mean 127, standard deviation 37 of 255), the cached one an even 188 (deviation 0). By source, `Pixels::set` (`pixels.rs:90`) still clears the mips (`:98`; its docs, `:88`, say "no decoding or mip generation"), and the only path that fills them is crate-private (`set_mips`, `:103`, called from `image.rs:80`); `ImageCache` has no call that fills an existing handle (`image.rs:183–258`). In the gallery, W2d's size classes (`thumbnails/step.rs:10–11`, `thumbnails/mod.rs:134`) and strata-kit's 2:1 note (`look.rs:507`) are still there; the `thumbnails::` tests (46) passed. Earlier: by source at `5d421fd`; C25's run at the pin measured 7 to 255, deviation 21, against an even 188. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `Pixels::set` builds no mipmaps, so a streamed image drawn well below its size aliases

Context: Strata (lgse/strata), a file manager on Rune, streams thumbnails into one `Pixels` handle
per file (the handle stays, the image arrives later) and draws them at whatever size the grid uses.

**What's missing** (`5d421fd`, unchanged at `2a44156`). `Pixels::set` is the streaming path and
builds no mip levels (`pixels.rs:90`), so a 256 px thumbnail drawn at 37 pt samples bilinearly
and aliases. `ImageCache` images get mips, but `ImageCache::rgba` makes a new handle for each
call (keyed by the bytes) and fills its mips later, so a store that keeps one handle per file
can't use it: each landed image would be a new handle, and the view would commit to swap it in.

**Repro** (headless; ran at `2a44156` with `--snap out.png --scale 1 --at 0.5`): a 256 × 256
one-pixel checkerboard drawn at 37 × 37 pt, streamed through `Pixels::set` (left) and through
`ImageCache::rgba` of the same bytes (right).

```rust
use rune::{App, AppSpec, Cx, El, Image, ImageCache, Pixels, Waker, el, row};
struct S { streamed: Pixels, cached: Option<Image> }
fn board() -> Vec<u8> {
    (0..256u32 * 256).flat_map(|i| { let v = if (i % 256 + i / 256) % 2 == 0 { 0 } else { 255 }; [v, v, v, 255] }).collect()
}
fn init() -> S { let p = Pixels::new(); p.set(256, 256, board()); S { streamed: p, cached: None } }
fn setup(s: &mut S, waker: Waker) {
    let img = ImageCache::new(waker).rgba(256, 256, board());
    let _ = img.wait();
    s.cached = Some(img);
}
fn view(s: &S, _: &mut Cx<S>) -> El {
    let mut r = row().w_full().h_full().child(el().size(37.0, 37.0).pixels(&s.streamed)).child(el().size(13.0, 37.0));
    if let Some(i) = &s.cached { r = r.child(el().size(37.0, 37.0).pixels(i.pixels())); }
    r
}
fn main() {
    let mut spec = AppSpec::new("mips", (87.0, 37.0), init, view);
    spec.setup = Some(setup);
    App::new(spec).run();
}
```

The streamed square shows a beat pattern of near-black and white blocks (grey levels 7 to 248,
standard deviation 37 of 255, inside its edge); the cached one is an even grey (188, no
deviation). A 256 → 32 draw (an exact eighth) hides it.

**Ask.** A mipmapped `set` (levels built off the main thread), or
`ImageCache::rgba_into(&Pixels, …)` that fills an existing handle.

Related: #163 (atlas pages keep a CPU copy), #148 (reading `Pixels` back).
