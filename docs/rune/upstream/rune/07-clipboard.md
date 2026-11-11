# 07. Linux clipboard: custom MIME types, several forms per copy, typed reads, ownership

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U5 (S5, corrected; amended by P2) |
| Affects | P2, X3 |
| Retires | P2's data-control and `wl_data_device` clipboard, its X11 selection owner and `SAVE_TARGETS` hand-off |
| Checked | Re-checked at `2a44156` by source. `write_one_form` (`clipboard/native.rs:139`) still writes one form: files, else the image, else HTML beside the text, else the text. `ClipboardContent` (`clipboard/content.rs:41–60`) still has only text, HTML, RTF, image and files, with no MIME setter or offered-type list. The README's Linux row and "Copies don't write images yet" (`README.md:1363`, `:1384`) are unchanged, and no commit in `5d421fd..2a44156` touches `crates/rune/src/clipboard*`. The program compiled at `2a44156` and was not run: it needs a compositor. P2's data-control and `wl_data_device` clipboard and its X11 `SAVE_TARGETS` hand-off are still there (`clipboard/`). Earlier: S2/P2 measured under sway at `78d22ed`, and it compiled at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Linux clipboard: custom MIME types, several forms in one copy, typed reads, and a notice when another app takes it

Context: Strata (lgse/strata), a file manager on Rune: its copy and cut offer `text/uri-list` beside
`x-special/gnome-copied-files` and `application/x-kde-cutselection` (which carry copy or cut), plus
text for editors, in one selection. Its paste needs the offered types, and its cut badge needs to
know when another app takes the clipboard.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- A copy holds one form on Linux and Windows: the files, else the image, else HTML beside the
  text, else the text (`write_one_form`, `clipboard/native.rs:139`). A file copy offers
  `text/uri-list` alone.
- `ClipboardContent` has no setter for another MIME type, and a read gives no list of offered
  types.
- Nothing tells the app when another client takes the clipboard, or that a write was dropped
  (wlroots ignores a `wl_data_device` selection whose serial is older than the current one,
  silently).
- On X11 only Rune's own text is handed to the clipboard manager at exit.
- The README's Linux row lists text and HTML only, and says "Copies don't write images yet",
  though `with_image` and `with_files` are written.

**Repro** (needs a compositor with data-control for `wl-paste`; click each tile, then run
`wl-paste --list-types`):

```rust
use rune::{AppSpec, ClipboardContent, Cx, El, col, el};
use std::path::PathBuf;
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full()
        .child(el().key("html").size(140.0, 60.0).fill(rune::oklch(0.6, 0.15, 140.0))
            .on_press_tx(|_: &mut S, tx| tx.copy_rich(ClipboardContent::new("file:///tmp/a.txt")
                .with_html("<a href=\"file:///tmp/a.txt\">a.txt</a>"))))
        .child(el().key("files").size(140.0, 60.0).fill(rune::oklch(0.6, 0.15, 260.0))
            .on_press_tx(|_: &mut S, tx| tx.copy_rich(ClipboardContent::new("/tmp/a.txt")
                .with_files([PathBuf::from("/tmp/a.txt")]))))
    // Not expressible: .with_type("x-special/gnome-copied-files", b"cut\nfile:///tmp/a.txt"),
    // or files and text in one copy.
}
fn main() { rune::run(AppSpec::new("clipboard", (320.0, 200.0), S::default, view)); }
```

The HTML copy lists `STRING UTF8_STRING text/plain text/html TEXT text/plain;charset=utf-8`
(measured under sway at `78d22ed`); the file copy offers `text/uri-list` only (by source).

**Ask.**
- `ClipboardContent::with_type(mime, bytes)`, ideally lazy, and every form of a copy written
  together.
- A typed read with the list of offered types.
- A callback when the app loses the clipboard, and a way to learn a write was dropped.
- On X11, every form of the last copy handed to the clipboard manager at exit (`SAVE_TARGETS`).
- The primary selection, where the platform has it (see (link to 55)).

**Why.** A downstream crate writes the selection over data-control on a connection of its own,
or `wl_data_device` on winit's display (which needs (link to 01)), and owns X11 selections on a
second connection. Rune's own copies and its copies then take the selection from each other.

Related: #483 (Shift+Insert paste on Linux).
