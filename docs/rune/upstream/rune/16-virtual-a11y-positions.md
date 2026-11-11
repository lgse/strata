# 16. Accessibility of virtual lists and grids: positions, set sizes, groups, row and column

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C1f-2, U-C1f-3, U-C3-4, R-C4-3, U-C11b-1 |
| Affects | C1e/C1f (grouped lists), C3 (icon grid), C4 (list table), C11b (sheet grid), C20d (icon picker) |
| Retires | Group headers that read as items; grids whose rows read "row 2 of 40" |
| Checked | Re-checked at `2a44156`: `doc_16`'s output in `run-2a44156.txt` matches the Output block. By source, a virtual row takes its list index as its position (`a11y/tree.rs:397`), an item's `position_in_set` size goes to its container (`:340`, applied at `:398`), and no `El` builder sets `column_index`, `row_index`, counts or `member_of` (`el/a11y.rs`; its only change past `04b38ae` is `El::unselectable`). Through the gallery, the accessibility goldens (their tests pass in `c1::` and `c3::`) show it: `c1-groups` reads the headings "Folders" and "Images" at positions 1 and 10, and `c3-default`'s rows of tiles read positions 1, 2, 3, … with set size 40 (the tile count). Earlier: the repro ran at `5d421fd` and `04b38ae` with the same output. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Virtual lists and grids: a heading row counts as an item, an item's set size replaces its row's, and no row or column index

Context: Strata (lgse/strata), a file manager on Rune: its list, icon and table views are virtual
rows (`virtual_rows`) with section headings, tiles packed several to a row, and table cells.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
1. Every mounted virtual row gets its list index as its position (`a11y/tree.rs:397`), so a
   heading among options reads as an item ("12 of 20").
2. Nothing relates items to the heading above them (AccessKit's `member_of`, ARIA
   `aria-owns`), so a screen reader never hears the group it enters.
3. `El::position_in_set(i, n)` on an item sets `n` as the size of set of the element holding it
   (`a11y/tree.rs:340`, applied at `:398`), overriding the row's own place among the rows.
4. AccessKit has `row_count`, `column_count`, `row_index` and `column_index`; no `El` builder
   sets them, so "row 50,000 of 100,000, column C" can't be said.

**Repro** (headless; ran at `5d421fd` and `2a44156`):

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, col, row, text};
fn grouped(_: &(), _: &mut Cx<()>) -> El {
    col().size(300.0, 600.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().key("list").role("listbox").label("Files").growth(GrowthSpec::default())
            .virtual_rows(20, 24.0, |_: &(), i| format!("r{i}").into(), |_: &(), i| {
                if i == 0 || i == 11 { text(format!("Group {i}")).h(24.0).role("heading") }
                else { text(format!("Item {i}")).h(24.0).role("option") }
            }))
}
fn grid(_: &(), _: &mut Cx<()>) -> El {
    col().size(600.0, 300.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().key("grid").role("grid").label("Icons").growth(GrowthSpec::default())
            .virtual_rows(5, 30.0, |_: &(), i| format!("line{i}").into(), |_: &(), i| {
                row().h(30.0).role("row").children((0..8).map(move |c| text(format!("tile {}", i * 8 + c)).role("gridcell").position_in_set(i * 8 + c + 1, 40)))
            }))
}
fn show(name: &str, view: fn(&(), &mut Cx<()>) -> El, roles: &[&str]) {
    let mut rt = Runtime::new((), view, (600.0, 600.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(0.016);
    println!("{name}:");
    let mut stack = vec![rt.accessibility_json()];
    while let Some(mut n) = stack.pop() {
        if let Some(kids) = n.get_mut("children").map(serde_json::Value::take) {
            stack.extend(kids.as_array().into_iter().flatten().rev().cloned());
        }
        if n.get("role").and_then(|r| r.as_str()).is_some_and(|r| roles.contains(&r)) {
            println!("  {} {} position {} setSize {}", n["role"], n["name"], n["position"], n["setSize"]);
        }
    }
}
fn main() {
    show("headings among options", grouped, &["heading"]);
    show("grid of rows of cells", grid, &["grid", "row"]);
}
```

Output:

```
headings among options:
  "heading" "Group 0" position 1 setSize null
  "heading" "Group 11" position 12 setSize null
grid of rows of cells:
  "grid" "Icons" position null setSize 5
  "row" "tile 0 tile 1 tile 2 tile 3 tile 4 tile 5 tile 6 tile 7" position 1 setSize 40
  "row" "tile 8 tile 9 tile 10 tile 11 tile 12 tile 13 tile 14 tile 15" position 2 setSize 40
  "row" "tile 16 tile 17 tile 18 tile 19 tile 20 tile 21 tile 22 tile 23" position 3 setSize 40
  "row" "tile 24 tile 25 tile 26 tile 27 tile 28 tile 29 tile 30 tile 31" position 4 setSize 40
  "row" "tile 32 tile 33 tile 34 tile 35 tile 36 tile 37 tile 38 tile 39" position 5 setSize 40
```

**Ask.**
1. A way for a virtual row to take no position and not count (a heading).
2. An `El` builder naming the element (by key) a row belongs to.
3. Keep a container's size of set to its own set; give the items' size to the container as
   a count of its children where its role has one.
4. `El::column_index(i)`, `El::row_index(i)`, and `El::grid_size(rows, cols)` (or counts on
   `grid` and `table`), mapped to AccessKit's.

Related: #111 (WCAG audit on desktop).
