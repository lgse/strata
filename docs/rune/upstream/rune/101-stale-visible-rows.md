# 101. A virtual list's `on_visible_rows` reports no rows, and never again, though rows show

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A3 of the parity plan (`docs/rune/parity/packages/A3.md`), with A5's findings; finding [lgse/strata#2290](https://github.com/lgse/strata/issues/2290) |
| Affects | Every collection of strata-components (C1e): the visible range Columns, Icons and List ask metadata fills and thumbnails by, and the rename editor's scroll check |
| Retires | `catch_up_visible` in `crates/strata-components/src/collection/element/route.rs`, which takes the range from the list's own layout when the range last reported is empty, with `CollectionState::lines_in_view` (U-C1f-7) |
| Checked | At `2a44156` (2026-11-13), in `strata-app`'s hosted tests only: `columns::tests::fills` fail every time without the workaround once the sidebar (A3) or the location bar (A5) is mounted, and pass with it. Not cut down to a Rune-only repro: a window of the same shape built from Rune's `virtual_rows` and strata-components' `PlacesSidebar` reports correctly |
| Status | Drafted, not filed. Cut a Rune-only repro before filing: the conditions below are what the hosted tests show, and the cause is not found. Workaround in force: the collection's catch-up at layout |

Paste from here.

---

**Title:** `on_visible_rows` reports an empty range for a newly mounted list, then never reports again

Context: Strata (lgse/strata), a file manager on Rune, asks for each row's metadata and
thumbnail by the rows a list reports in view through `on_visible_rows`.

**What happens.** A list of five index-built rows (`growth` and `virtual_rows`, in a
vertical scroll viewport) is mounted while the window draws its first frames, with
animations on. In one frame its handler hears `0..5` and then `0..0`. The rows stay drawn,
in view, and the handler hears nothing more: Rune's last report stands at `0..0` until the
rows in view change, which needs a scroll.

**When**, from the app's tests:

- It needs a part of the window drawn *before* the list in the tree that lays out text in
  those first frames: three or more rows with a text label in a sidebar to the list's left,
  or a breadcrumb bar whose layout handler calls `tx.scroll_to_x(viewport, f32::MAX)`.
- It does not happen with two such rows, ten rows without text, the same rows placed
  *after* the list in the tree, animations off, or one frame drawn before the list's rows
  arrive.
- A second list mounted later in the same window can do it again.
- A commit asked from the handler (returning `true` on the empty report) does not bring a
  new report.

**Expected.** The last report matches the rows in the viewport, or a later frame corrects
it.

**What would help.** A report that follows the list's viewport as last laid out; or a way
for an app to ask a list to report its range again.
