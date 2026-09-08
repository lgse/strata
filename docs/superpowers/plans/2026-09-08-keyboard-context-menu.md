# Keyboard Context Menu Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open the selection-aware context menu with the hardware Menu key and Shift+F10 (anchored to the focused item, not the pointer), and make every context menu fully keyboard-navigable, across Columns, Icons, List, Trash, and the file chooser. Closes GitHub issue #583.

**Architecture:** The context menu is a custom `gtk::Popover` (not `gtk::PopoverMenu`), built in `src/ui/browser/context_menu.rs` and `src/ui/browser/chooser_context.rs`, currently opened only from a button-3 `gtk::GestureClick`. Two native GTK mechanisms already do most of the hard work for free once wired up correctly:

1. **Opening**: the click handlers already contain all the selection/menu-population logic keyed off an `(x, y)` point local to the view widget (`widget.pick(x, y, ...)`). We extract each handler's body into a closure and return it as an `Rc<dyn Fn(f64, f64)>` "trigger" from `install_folder_context_menu` / `install_item_context_menu` (and the chooser equivalents), so a keyboard path can call the exact same logic with a point computed from the *focused item's own on-screen position* instead of the pointer. No widget-data/`unsafe` tricks — the trigger is stored as a typed field on the existing per-column/per-section structs that already own the widget.
2. **In-menu navigation**: `src/ui/window/keyboard/focus.rs` already special-cases one popover class (`"column-popover"`) to route Up/Down/h/j/k/l through `gtk::Widget::child_focus(DirectionType)` — GTK's native focus-chain traversal, which already skips insensitive and non-focusable widgets (i.e. disabled actions and separators, for free). We extend that same function to recognize our context-menu popovers and add Home/End + wraparound. Enter/Space activation is native `gtk::Button` behavior already; Escape-to-close is native `gtk::Popover` `autohide` behavior already. Accessible `Menu`/`MenuItem` roles and names are already set in `src/ui/accessibility.rs`.

**Tech Stack:** Rust, GTK4 (gtk4-rs), existing Strata `Dispatcher`/`KeyEvent` keyboard-routing infrastructure.

## Global Constraints

- Do not introduce `unsafe` code or widget-data (`set_data`/`data`) hacks — route new state through typed struct fields, matching this codebase's existing style.
- Do not restructure `gtk::Popover` into `gtk::PopoverMenu`/`gio::Menu` — the existing custom popover carries selection-dependent visibility/sensitivity logic (trash, writability, multi-selection) that is out of scope to port.
- No submenus exist today (`"Open With…"` opens a separate popover, not a nested menu) — do not build submenu Left/Right handling; it has nothing to attach to. Note this explicitly in the PR description as out of scope until a submenu exists.
- Follow `AGENTS.md` test organization: unit tests live adjacent to the module they test, declared via `#[cfg(test)] mod tests;` — `columns.rs` changes test under `src/ui/browser/columns/tests/`, `browser_modes.rs` under `src/ui/browser_modes/tests/`, `browser.rs` under `src/ui/browser/tests/`, and `context_menu.rs`/`keyboard/focus.rs` menu-navigation behavior under `src/ui/browser/context_menu/tests/` (matching the fixture style already in `src/ui/browser/context_menu/tests/menus.rs`). Each task's Files/Test section states the correct location for that task.
- Every visual focus state must use semantic `@theme_*` colors per `AGENTS.md` theming rules — no static hex/RGB.
- Icons: none needed for this feature (no new icons).
- Run tests under Xvfb per `AGENTS.md`: `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --all-targets --all-features`.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/ui/browser/context_menu.rs` | Extract click-handler bodies into reusable `(x, y)` triggers; return them from `install_folder_context_menu`/`install_item_context_menu`. |
| `src/ui/browser/chooser_context.rs` | Same extraction for the reduced chooser menu. |
| `src/ui/browser/columns.rs` | Store the returned triggers on `ColumnView`; add a method to resolve "the focused row's (or background's) on-screen point" per column. |
| `src/ui/browser_modes.rs` | Store the returned triggers on `PaneSection`/`Pane`; add the equivalent resolver for Icons/List. |
| `src/ui/browser.rs` | `ViewState`/`BrowserView` method `open_focused_context_menu()` that picks Columns vs Icons/List and calls through. |
| `src/ui/window/keyboard/commands.rs` | New `Key::Menu` / Shift+F10 case in the `Dispatcher`. |
| `src/ui/window/keyboard/focus.rs` | Extend `popover_navigation` for context-menu popovers: Up/Down/Home/End with wraparound. |
| `src/ui/shortcut_footer.rs` | New shortcut row. |
| `docs/keyboard-navigation.md` | New section documenting the shortcut and behavior. |
| `src/ui/browser/context_menu/tests/keyboard.rs` (new) | New test module for keyboard-invocation and in-menu navigation. |

---

### Task 1: Extract reusable open-triggers in `context_menu.rs`

**Files:**
- Modify: `src/ui/browser/context_menu.rs:136-339` (`install_folder_context_menu`), `:347-928` (`install_item_context_menu`)
- Test: `src/ui/browser/context_menu/tests/menus.rs` (existing tests must keep passing unchanged — this task is a pure refactor)

**Interfaces:**
- Produces: `install_folder_context_menu(...) -> Rc<dyn Fn(f64, f64)>` and `install_item_context_menu(...) -> Rc<dyn Fn(f64, f64)>`. Calling the returned closure with a point `(x, y)` local to the `parent`/`widget` argument reproduces exactly what a button-3 press at that point does today (select/resolve target, populate menu, call `focus_context_column` + `show_context_popover`).

- [ ] **Step 1: Run the existing menu tests to record the baseline**

Run: `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --lib ui::browser::context_menu -- --nocapture`
Expected: all existing tests in `context_menu::tests` pass (baseline before refactor).

- [ ] **Step 2: Refactor `install_folder_context_menu` to expose a trigger**

In `src/ui/browser/context_menu.rs`, change the tail of `install_folder_context_menu` (replacing the current `menu_click.connect_pressed(...)` block and its final `parent.add_controller(menu_click);` at lines 294-339) to:

```rust
    let popover_for_trigger = popover.clone();
    let browser_for_trigger = state.browser.clone();
    let scroll_for_trigger = scroll.clone();
    let weak_state = Rc::downgrade(state);
    let parent_for_trigger = parent.clone();
    let location_for_trigger = location.clone();
    let open_at: Rc<dyn Fn(f64, f64)> = Rc::new(move |x: f64, y: f64| {
        paste.set_sensitive(gtk::gdk::Display::default().is_some_and(|display| {
            display
                .clipboard()
                .formats()
                .contains_type(gtk::gdk::FileList::static_type())
        }));
        select_all.set_sensitive(has_entries());
        open_terminal.set_sensitive(can_open_terminal(&location_for_trigger));
        let hidden_files_shown = browser_for_trigger.preferences().show_hidden;
        toggle_hidden_label.set_text(if hidden_files_shown {
            "Hide Hidden Files"
        } else {
            "Show Hidden Files"
        });
        crate::assets::set_primary_icon(
            &toggle_hidden_icon,
            if hidden_files_shown {
                crate::assets::icons::EYE
            } else {
                crate::assets::icons::EYE_OFF
            },
        );
        if let Some(state) = weak_state.upgrade() {
            focus_context_column(&state, depth);
            show_context_popover(&popover_for_trigger, &scroll_for_trigger, &parent_for_trigger, x, y);
        }
    });

    let menu_click = gtk::GestureClick::new();
    menu_click.set_button(3);
    let open_for_click = open_at.clone();
    menu_click.connect_pressed(move |gesture, _, x, y| {
        let over_item = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|picked| is_item_target(&picked));
        if over_item {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open_for_click(x, y);
    });
    parent.add_controller(menu_click);
    open_at
```

Change the function signature from `pub(in crate::ui) fn install_folder_context_menu(...)` (no return type) to `pub(in crate::ui) fn install_folder_context_menu(...) -> Rc<dyn Fn(f64, f64)>`, and change the early-return-to-chooser line:

```rust
    if !state.interactive {
        chooser_context::install_folder(state, parent, is_item_target, depth, location);
        return;
    }
```
to:
```rust
    if !state.interactive {
        return chooser_context::install_folder(state, parent, is_item_target, depth, location);
    }
```
(Task 4 makes `chooser_context::install_folder` return the same trigger type.)

- [ ] **Step 3: Refactor `install_item_context_menu` the same way**

Apply the identical pattern to `install_item_context_menu` (lines 347-928): extract the body of the existing `click.connect_pressed(move |gesture, _, x, y| { ... })` closure (lines 814-926) into an `open_at: Rc<dyn Fn(f64, f64)>` that takes `(x, y)` directly instead of reading `(x, y)` from the gesture's closure args — the body already only uses `x`/`y` for the final `show_context_popover(..., x, y)` call and for `gesture.widget().and_then(|w| w.pick(x, y, ...))`. Replace `gesture.widget()` with the captured `widget: gtk::Widget` (clone of the `widget: &gtk::Widget` parameter) since the trigger has no gesture to ask. Change the signature to return `Rc<dyn Fn(f64, f64)>`, update the chooser early return the same way as Step 2, and end the function with:

```rust
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let open_for_click = open_at.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open_for_click(x, y);
    });
    widget.add_controller(click);
    open_at
```

Note: the original body used `gesture.set_state(gtk::EventSequenceState::Claimed);` unconditionally near the top (after resolving `entry`) — keep that inside `open_at` only if resolution succeeds (it already returns early via `let Some(...) = ... else { return; }` when the picked point doesn't resolve to an entry); for the keyboard path this will always resolve since the caller passes a point known to be on the focused row, but the existing early-returns stay as defensive guards.

- [ ] **Step 4: Compile and re-run the baseline tests**

Run: `cargo check` then `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --lib ui::browser::context_menu -- --nocapture`
Expected: compiles (fix call sites flagged by the compiler — they are addressed in Tasks 2-4, so `cargo check` will show unused-return-value warnings, not errors, at this point since Rust doesn't require using return values); existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/ui/browser/context_menu.rs
git commit -m "refactor(context-menu): expose folder/item menu open-at-point triggers"
```

---

### Task 2: Same extraction in `chooser_context.rs`

**Files:**
- Modify: `src/ui/browser/chooser_context.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `install_folder(...) -> Rc<dyn Fn(f64, f64)>`, `install_item(...) -> Rc<dyn Fn(f64, f64)>`, matching Task 1's contract so `context_menu.rs`'s `!state.interactive` branches type-check.

- [ ] **Step 1: Refactor `install_folder`**

In `src/ui/browser/chooser_context.rs`, change `install_folder` (lines 59-104) so the body currently inside `click.connect_pressed(move |gesture, _, x, y| { ... })` becomes an `open_at: Rc<dyn Fn(f64, f64)>` closure over `anchor: gtk::Widget` (a clone of `parent`, since the chooser's original click body reads `gesture.widget()` for the anchor — clone `parent.clone()` once for the trigger, keep `gesture.widget()` for the click path):

```rust
pub(super) fn install_folder(
    state: &Rc<ViewState>,
    parent: &gtk::Widget,
    is_item_target: Rc<dyn Fn(&gtk::Widget) -> bool>,
    depth: usize,
    location: Location,
) -> Rc<dyn Fn(f64, f64)> {
    let weak = Rc::downgrade(state);
    let anchor_for_trigger = parent.clone();
    let location_for_trigger = location.clone();
    let open_at: Rc<dyn Fn(f64, f64)> = {
        let weak = weak.clone();
        Rc::new(move |x: f64, y: f64| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let weak = Rc::downgrade(&state);
            let location = location_for_trigger.clone();
            let (popover, scroll) = menu(
                &[(
                    Action::NewFolder,
                    crate::assets::icons::FOLDER_PLUS,
                    "New Folder",
                    "Ctrl+Shift+N",
                    true,
                )],
                move |_| {
                    if let Some(state) = weak.upgrade() {
                        state.begin_new_entry(depth, location.clone(), true);
                    }
                },
            );
            bind_column_context_owner(&state, &popover, depth);
            focus_context_column(&state, depth);
            show_context_popover(&popover, &scroll, &anchor_for_trigger, x, y);
        })
    };

    let click = gtk::GestureClick::new();
    click.set_button(3);
    let open_for_click = open_at.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(anchor) = gesture.widget() else {
            return;
        };
        if anchor
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|picked| is_item_target(&picked))
        {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open_for_click(x, y);
    });
    parent.add_controller(click);
    open_at
}
```

- [ ] **Step 2: Refactor `install_item`**

Apply the same pattern to `install_item` (lines 106-193): extract the `click.connect_pressed` body into `open_at: Rc<dyn Fn(f64, f64)>` closed over `widget.clone()` as the anchor, keep the original `gesture.widget()`-based pick logic in the click callback delegating to `open_for_click(x, y)`, and return `open_at: Rc<dyn Fn(f64, f64)>` from `install_item`. The pick/resolve logic (`anchor.pick(x, y, ...)`, `pick_position`, `source_position`, `entry_at`) stays inside `open_at` itself (not just the click callback), since the keyboard path also needs it to resolve which entry/position it's opening for.

- [ ] **Step 3: Compile**

Run: `cargo check`
Expected: `context_menu.rs`'s two `chooser_context::install_folder(...)`/`chooser_context::install_item(...)` call sites (now returning a value via the updated `return chooser_context::install_folder(...)` pattern from Task 1 Step 2) type-check against the new `Rc<dyn Fn(f64, f64)>` return type.

- [ ] **Step 4: Commit**

```bash
git add src/ui/browser/chooser_context.rs
git commit -m "refactor(chooser-context): expose folder/item menu open-at-point triggers"
```

---

### Task 3: Store triggers on Columns' `ColumnView` and resolve the focused target

**Files:**
- Modify: `src/ui/browser/columns.rs:61-84` (`ColumnView` struct + its two context-menu install call sites at `:991` and `:1019` from the current file, i.e. right after `install_folder_context_menu`/`install_item_context_menu` calls found during research)
- Test: `src/ui/browser/columns/tests/` (existing directory; add a new test there)

**Interfaces:**
- Consumes: `install_folder_context_menu(...) -> Rc<dyn Fn(f64, f64)>`, `install_item_context_menu(...) -> Rc<dyn Fn(f64, f64)>` (Task 1). `BoundRow { item: glib::WeakRef<gtk::ListItem>, row: glib::WeakRef<gtk::Box> }` and `bound_rows: Rc<RefCell<Vec<BoundRow>>>` (existing, `src/ui/browser/columns.rs:36-39`).
- Produces: a method (exact placement depends on where `ColumnView`'s impl block lives — add it there) `fn context_menu_target(&self, position: Option<usize>) -> Option<(Rc<dyn Fn(f64, f64)>, f64, f64)>`: `position = Some(p)` looks up the row for source position `p` and returns its item-trigger plus the local point at the row's own center (relative to `self.list`); `position = None` returns the folder-trigger plus the point `(self.presentation.stack.width() as f64 / 2.0, self.presentation.stack.height() as f64 / 2.0)` (background of the column body).

- [ ] **Step 1: Add trigger fields to `ColumnView`**

In `src/ui/browser/columns.rs`, add two fields to the `ColumnView` struct (after `bound_rows`):

```rust
    pub(super) bound_rows: Rc<RefCell<Vec<BoundRow>>>,
    pub(super) folder_context_trigger: Rc<dyn Fn(f64, f64)>,
    pub(super) item_context_trigger: Rc<dyn Fn(f64, f64)>,
```

At the two call sites (`install_folder_context_menu(self, presentation.stack.upcast_ref(), ...)` and `install_item_context_menu(self, list.upcast_ref(), &selection, ...)`), capture the returned triggers into local `let folder_context_trigger = install_folder_context_menu(...);` / `let item_context_trigger = install_item_context_menu(...);`, and add them to the `ColumnView { ... }` struct literal that constructs the value returned from this function (find it via the existing `bound_rows,` field in that literal, per the earlier grep at `columns.rs:1132`).

- [ ] **Step 2: Add the resolver method**

Add, in the same `impl ColumnView` block (or a new `impl` block adjacent to the struct):

```rust
impl ColumnView {
    /// The trigger and local `(x, y)` point to open this column's context menu
    /// for `position` (the item menu) or its background (the folder menu, when
    /// `position` is `None` or not currently rendered).
    pub(super) fn context_menu_target(
        &self,
        position: Option<usize>,
    ) -> Option<(Rc<dyn Fn(f64, f64)>, f64, f64)> {
        if let Some(position) = position
            && let Some(row) = self.bound_rows.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                (item.position() as usize == position).then(|| bound.row.upgrade())?
            })
        {
            let bounds = row.compute_bounds(&self.list)?;
            return Some((
                self.item_context_trigger.clone(),
                f64::from(bounds.center().x()),
                f64::from(bounds.center().y()),
            ));
        }
        let width = f64::from(self.presentation.stack.width());
        let height = f64::from(self.presentation.stack.height());
        (width > 0.0 && height > 0.0)
            .then(|| (self.folder_context_trigger.clone(), width / 2.0, height / 2.0))
    }
}
```

Adjust `bound.item.position()`'s type against `EntryListModel`/`ViewMap` conventions already used elsewhere in the file (the existing pattern at `columns.rs:1006-1013`, `rows_for_context.borrow().iter().find_map(|bound| { let row = bound.row.upgrade()?; let item = bound.item.upgrade()?; (row == picked).then_some(item.position()) })`, confirms `item.position()` is the right call and returns the type used elsewhere as `position: u32`/`usize` — match whatever that existing call site treats it as).

- [ ] **Step 3: Write a test**

In `src/ui/browser/columns/tests/` (find the existing test file that builds a populated `ColumnView` fixture, matching the pattern used for row/selection tests in that directory), add:

```rust
#[test]
fn context_menu_target_resolves_focused_row() {
    let column = /* existing fixture helper that builds a ColumnView with entries */;
    let target = column.context_menu_target(Some(0));
    assert!(target.is_some(), "expected a trigger and point for position 0");
}

#[test]
fn context_menu_target_falls_back_to_background() {
    let column = /* fixture with an empty directory */;
    let target = column.context_menu_target(None);
    assert!(target.is_some(), "expected the folder trigger with no position");
}
```

(Match the exact fixture-construction helper already used by neighboring tests in that file — do not invent a new one.)

- [ ] **Step 4: Run and commit**

Run: `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --lib ui::browser::columns`
Expected: PASS.

```bash
git add src/ui/browser/columns.rs src/ui/browser/columns/tests/
git commit -m "feat(columns): resolve the focused row's context-menu trigger and anchor point"
```

---

### Task 4: Store triggers on Icons/List `PaneSection`/`Pane` and resolve the focused target

**Files:**
- Modify: `src/ui/browser_modes.rs:196-210` (`PaneSection`), the `Pane` struct (holds `section`/`sections`, background widget), `install_section_context_menu` (`:3849-3878`), `install_context_menu` (the pane-background installer, `~:1020-1050` from research), and both call sites building Icons (`:1740`) and List (`:2403`) sections.
- Test: `src/ui/browser_modes/tests/` (existing directory).

**Interfaces:**
- Consumes: same `install_folder_context_menu`/`install_item_context_menu` contract as Task 3.
- Produces: `ModeViews::context_menu_target(&self, depth: usize, position: Option<usize>) -> Option<(Rc<dyn Fn(f64, f64)>, f64, f64)>`, mirroring Task 3's `ColumnView::context_menu_target` but searching across every `PaneSection` at `depth` (grouped views can have multiple sections) for the one whose `bound_items` contains `position`.

- [ ] **Step 1: Add a trigger field to `PaneSection` and to `Pane`**

```rust
#[derive(Clone)]
struct PaneSection {
    view: gtk::Widget,
    view_model: gio::ListModel,
    selection: gtk::MultiSelection,
    bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    syncing: Rc<Cell<bool>>,
    visit: super::marquee::ItemVisitor,
    item_context_trigger: Rc<dyn Fn(f64, f64)>,
}
```

Add `folder_context_trigger: Rc<dyn Fn(f64, f64)>` to `Pane` (find its struct definition near `PaneSection`, per the `all_panes`/`panes_at` methods already read). Update `install_section_context_menu` (`:3849`) to capture and return the item trigger:

```rust
fn install_section_context_menu(
    state: &Rc<super::browser::ViewState>,
    section: &PaneSection,
    sections: Weak<RefCell<Vec<PaneSection>>>,
    source_index: &SourceIndexMap,
    depth: usize,
) -> Rc<dyn Fn(f64, f64)> {
    // ...unchanged body building pick_position/source_position/clear_other_selections...
    super::browser::install_item_context_menu(
        state,
        &section.view,
        &section.selection,
        pick_position,
        source_position,
        clear_other_selections,
        depth,
    )
}
```

At both call sites (`:1740` for Icons, `:2403` for List), assign the returned trigger into the `PaneSection { ..., item_context_trigger }` struct literal instead of discarding it (both literals are visible in the already-read surrounding code — each already lists `bound_items: bound_items.clone(), visit: bound_item_visitor(bound_items),` immediately before/after the `install_section_context_menu(...)` call; move the call earlier if needed so its result is available for the literal, or restructure to build the section, call `install_section_context_menu(&state, &section, ...)`, then `PaneSection { item_context_trigger, ..section }` — pick whichever reads cleaner given the surrounding borrow-checker constraints, since `section` is cloned into the literal already (`PaneSection` derives `Clone`)).

Similarly capture `install_folder_context_menu`'s return in `install_context_menu` (the pane-background installer) into `Pane.folder_context_trigger`.

- [ ] **Step 2: Add the resolver**

Add to `impl ModeViews`:

```rust
impl ModeViews {
    /// The trigger and local `(x, y)` point to open the context menu for `position`
    /// within the pane at `depth` (the item menu, searching every section since a
    /// grouped view has one section per type group), or the pane's own background
    /// (the folder menu) when `position` is `None` or not currently rendered.
    pub(super) fn context_menu_target(
        &self,
        depth: usize,
        position: Option<usize>,
    ) -> Option<(Rc<dyn Fn(f64, f64)>, f64, f64)> {
        let pane = self.panes_at(depth).into_iter().next()?;
        if let Some(position) = position {
            for section in std::iter::once(&pane.section).chain(pane.sections.borrow().iter()) {
                let Some(widget) = section.bound_items.borrow().iter().find_map(|bound| {
                    let item = bound.item.upgrade()?;
                    (item.position() as usize == position).then(|| bound.widget.upgrade())?
                }) else {
                    continue;
                };
                if let Some(bounds) = widget.compute_bounds(&section.view) {
                    return Some((
                        section.item_context_trigger.clone(),
                        f64::from(bounds.center().x()),
                        f64::from(bounds.center().y()),
                    ));
                }
            }
        }
        let width = f64::from(pane.stack.width());
        let height = f64::from(pane.stack.height());
        (width > 0.0 && height > 0.0)
            .then(|| (pane.folder_context_trigger.clone(), width / 2.0, height / 2.0))
    }
}
```

Adjust `pane.section`/`pane.sections`/`pane.stack` field names against `Pane`'s actual definition (read it in full before writing this — the earlier partial read showed `depth`, `shell`, `header`, `model`, `source_index`, `filter_model`, `section`, `sections`; confirm the background-widget field name, likely `stack` or `shell`, and use that consistently with `install_context_menu`'s existing `pane.stack.upcast_ref()` call already read in this codebase).

- [ ] **Step 3: Test**

In `src/ui/browser_modes/tests/`, mirroring Task 3 Step 3's structure but for a grouped Icons pane with two type-groups, assert `context_menu_target` finds the right section's trigger for a position in the second group, and falls back to the background trigger for `None`.

- [ ] **Step 4: Run and commit**

Run: `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --lib ui::browser_modes`
Expected: PASS.

```bash
git add src/ui/browser_modes.rs src/ui/browser_modes/tests/
git commit -m "feat(browser-modes): resolve the focused item's context-menu trigger and anchor point"
```

---

### Task 5: `BrowserView::open_focused_context_menu()`

**Files:**
- Modify: `src/ui/browser.rs` (`ViewState`/`BrowserView` — add near other `pub(super)`/`pub` methods on these types, e.g. next to `focused_item`/existing browser-view-level command methods)
- Test: `src/ui/browser/tests/` (existing directory)

**Interfaces:**
- Consumes: `ColumnView::context_menu_target` (Task 3), `ModeViews::context_menu_target` (Task 4), `Browser::focused_item(&self) -> Option<(usize, usize, FileEntry)>` (`src/app/browser.rs:1132`), `Browser::active_depth()` (used elsewhere in the dispatcher, e.g. `keyboard/items.rs`).
- Produces: `pub(super) fn open_focused_context_menu(&self) -> bool` on `BrowserView`, returning whether a menu was opened (so the dispatcher knows whether to stop event propagation).

- [ ] **Step 1: Write the method**

```rust
impl BrowserView {
    /// Opens the context menu for the current keyboard focus: the selection-aware
    /// item menu when an item has keyboard focus, otherwise the active pane's
    /// background menu. Used by the Menu key and Shift+F10.
    pub(super) fn open_focused_context_menu(&self) -> bool {
        let browser = self.browser();
        let focused = browser.focused_item();
        let depth = focused
            .as_ref()
            .map(|(depth, _, _)| *depth)
            .or_else(|| browser.active_depth());
        let Some(depth) = depth else {
            return false;
        };
        let position = focused.map(|(_, position, _)| position);
        let target = if self.view_mode() == crate::ui::browser_modes::BrowserMode::Columns {
            self.columns_context_menu_target(depth, position)
        } else {
            self.mode_views_context_menu_target(depth, position)
        };
        let Some((trigger, x, y)) = target else {
            return false;
        };
        trigger(x, y);
        true
    }
}
```

Add the two small private forwarding methods `columns_context_menu_target`/`mode_views_context_menu_target` that reach into whichever field holds the live `ColumnView`s / `ModeViews` (match the existing accessor pattern already used by `self.view_mode()`, `self.item_view_has_focus()` etc. in this `impl BrowserView` block — read the block in full before adding these, since the exact field names weren't captured in this plan's research pass). Each forwards to Task 3's/Task 4's resolver for the given `depth`.

- [ ] **Step 2: Test**

In `src/ui/browser/tests/`, using the existing `BrowserView` test fixture pattern (the same one `context_menu/tests/menus.rs` builds on, since it already imports `crate::ui::browser::{BrowserView, PeekBehavior}`), write:

```rust
#[test]
fn open_focused_context_menu_opens_item_menu_when_item_focused() {
    // build a fixture BrowserView with entries, focus the first item
    // (existing helpers used by neighboring focus tests, e.g. src/ui/browser/tests/focus.rs)
    let opened = view.open_focused_context_menu();
    assert!(opened);
    // assert a mapped gtk::Popover with css class "folder-context-popover" exists
}

#[test]
fn open_focused_context_menu_opens_background_menu_with_no_selection() {
    // fixture with no item focused/selected
    let opened = view.open_focused_context_menu();
    assert!(opened);
}
```

- [ ] **Step 3: Run and commit**

Run: `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --lib ui::browser::tests`
Expected: PASS.

```bash
git add src/ui/browser.rs src/ui/browser/tests/
git commit -m "feat(browser): add open_focused_context_menu for keyboard invocation"
```

---

### Task 6: Wire the Menu key and Shift+F10 into the Dispatcher

**Files:**
- Modify: `src/ui/window/keyboard/commands.rs`
- Test: `src/ui/window/tests.rs` (existing file, for the pure key-matching helper)

**Interfaces:**
- Consumes: `BrowserView::open_focused_context_menu(&self) -> bool` (Task 5).
- Produces: `pub(super) fn is_open_context_menu_shortcut(key: Key, modifiers: Modifiers) -> bool` (pure, unit-testable like `vim_focus_direction`), and a new `Dispatcher::context_menu_command` step wired into `handle_key`'s chain in `src/ui/window/keyboard.rs:130`.

- [ ] **Step 1: Write the failing test**

In `src/ui/window/tests.rs`, alongside the existing `vim_focus_direction` tests:

```rust
#[test]
fn open_context_menu_shortcut_matches_menu_key_and_shift_f10() {
    use gtk::gdk::{Key, ModifierType};
    assert!(is_open_context_menu_shortcut(Key::Menu, ModifierType::empty()));
    assert!(is_open_context_menu_shortcut(
        Key::F10,
        ModifierType::SHIFT_MASK
    ));
    assert!(!is_open_context_menu_shortcut(
        Key::F10,
        ModifierType::empty()
    ));
    assert!(!is_open_context_menu_shortcut(
        Key::Menu,
        ModifierType::CONTROL_MASK
    ));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib ui::window::tests::open_context_menu_shortcut_matches_menu_key_and_shift_f10`
Expected: FAIL (`is_open_context_menu_shortcut` not defined / not imported).

- [ ] **Step 3: Implement**

In `src/ui/window/keyboard/commands.rs`, add:

```rust
pub(super) fn is_open_context_menu_shortcut(key: Key, modifiers: Modifiers) -> bool {
    match key {
        Key::Menu => modifiers.is_empty(),
        Key::F10 => modifiers == Modifiers::SHIFT_MASK,
        _ => false,
    }
}

impl Dispatcher {
    pub(super) fn context_menu_command(&self, event: &KeyEvent) -> KeyResult {
        if !is_open_context_menu_shortcut(event.key, event.modifiers)
            || event.text_has_focus()
            || self.inline_editing_active()
        {
            return None;
        }
        self.view
            .open_focused_context_menu()
            .then_some(Propagation::Stop)
    }
}
```

Re-export `is_open_context_menu_shortcut` from `src/ui/window/keyboard.rs` the same way `vim_focus_direction` etc. are re-exported for `window/tests.rs` to see it (check the existing `use` list at the top of `src/ui/window/tests.rs:30` and add it there), or make the test live in `commands.rs`'s own inline `#[cfg(test)]` if that's this codebase's convention for `commands.rs`-local pure functions — match whichever pattern the file already uses (the earlier research found `src/ui/window/tests.rs` importing pure functions like `vim_focus_direction`, `volume_release_action` directly, so extend that same `use` block).

Wire it into `Dispatcher::handle_key` in `src/ui/window/keyboard.rs:130`, inserting the new step before `.or_else(|| self.focus_navigation(browser, &mut event))` (so it takes priority while an item or the pane background has keyboard focus, consistent with `file_commands` running before `focus_navigation` today):

```rust
            .or_else(|| self.file_commands(browser, &event))
            .or_else(|| self.context_menu_command(&event))
            .or_else(|| self.focus_navigation(browser, &mut event))
```

- [ ] **Step 4: Run and verify it passes**

Run: `cargo test --lib ui::window::tests::open_context_menu_shortcut_matches_menu_key_and_shift_f10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/window/keyboard.rs src/ui/window/keyboard/commands.rs src/ui/window/tests.rs
git commit -m "feat(keyboard): open the focused context menu with Menu key or Shift+F10"
```

---

### Task 7: In-menu keyboard navigation (Up/Down/Home/End with wraparound)

**Files:**
- Modify: `src/ui/window/keyboard/focus.rs:26-42` (`popover_navigation`)
- Test: `src/ui/browser/context_menu/tests/keyboard.rs` (new file)

**Interfaces:**
- Consumes: the popover css classes already applied in `context_menu.rs`/`chooser_context.rs`: `"folder-context-popover"` (both folder and item menus use this same class per the code already read — `popover.add_css_class("folder-context-popover");` appears in both `install_folder_context_menu` and `install_item_context_menu`).
- Produces: extended `popover_navigation` behavior; a new small helper `fn focus_first_or_last_menu_item(content: &gtk::Widget, first: bool) -> bool`.

- [ ] **Step 1: Write the failing test**

In new file `src/ui/browser/context_menu/tests/keyboard.rs` (add `mod keyboard;` to `src/ui/browser/context_menu/tests.rs`), using the fixture/`open_menu` helper pattern already in `menus.rs`:

```rust
use super::*;

#[test]
fn down_arrow_moves_focus_to_the_next_enabled_item_and_skips_separators() {
    let view = /* fixture BrowserView, per menus.rs's MenuSource fixture */;
    let popover = open_menu(&view, Some("notes.txt"));
    let content = popover
        .child()
        .and_downcast::<gtk::ScrolledWindow>()
        .and_then(|scroll| scroll.child())
        .expect("menu content");
    content.child_focus(gtk::DirectionType::TabForward);
    let focused_before = /* whichever child currently has focus */;
    assert!(crate::ui::window::keyboard::test_support::dispatch_key(
        &popover,
        gtk::gdk::Key::Down,
        gtk::gdk::ModifierType::empty(),
    ));
    // assert focus moved to the next *sensitive* MenuItem-role button, not a
    // gtk::Separator (separators are never focusable, so this is really
    // asserting the moved-to widget is a gtk::Button).
}
```

If no `test_support::dispatch_key` helper exists yet (the earlier research found none — this codebase's convention unit-tests the pure key-mapping functions directly rather than dispatching synthetic `EventControllerKey` events through a live window), replace this test with the lighter, established pattern: call `popover_navigation`'s underlying logic directly rather than simulating a full key event. Since `popover_navigation` is a private `Dispatcher` method, the properly-scoped alternative is to unit-test the new pure helper from Step 3 directly (`focus_first_or_last_menu_item`) plus rely on Task 8's higher-level scenario test for the end-to-end path. Prefer this: write the test against the pure helper, not a simulated dispatch.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib ui::browser::context_menu::tests::keyboard`
Expected: FAIL (new items not yet defined).

- [ ] **Step 3: Implement the extension**

In `src/ui/window/keyboard/focus.rs`, replace `popover_navigation`:

```rust
    fn popover_navigation(&self, event: &KeyEvent) -> KeyResult {
        if !event.without(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK) {
            return None;
        }
        let popover = event
            .focused
            .as_ref()
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()?;
        if popover.has_css_class("column-popover")
            && let Some(direction) = vim_focus_direction(event.key)
        {
            popover.child_focus(direction);
            return Some(Propagation::Stop);
        }
        if popover.has_css_class("folder-context-popover") {
            return self.context_menu_popover_navigation(&popover, event);
        }
        Some(Propagation::Proceed)
    }

    fn context_menu_popover_navigation(
        &self,
        popover: &gtk::Popover,
        event: &KeyEvent,
    ) -> KeyResult {
        let Some(content) = popover
            .child()
            .and_downcast::<gtk::ScrolledWindow>()
            .and_then(|scroll| scroll.child())
        else {
            return Some(Propagation::Proceed);
        };
        match event.key {
            Key::Up | Key::Down => {
                let direction = if event.key == Key::Up {
                    gtk::DirectionType::Up
                } else {
                    gtk::DirectionType::Down
                };
                if !content.child_focus(direction) {
                    // At an edge: wrap to the opposite end instead of leaving the menu.
                    focus_first_or_last_menu_item(&content, direction == gtk::DirectionType::Down);
                }
                Some(Propagation::Stop)
            }
            Key::Home => {
                focus_first_or_last_menu_item(&content, true);
                Some(Propagation::Stop)
            }
            Key::End => {
                focus_first_or_last_menu_item(&content, false);
                Some(Propagation::Stop)
            }
            _ => Some(Propagation::Proceed),
        }
    }
```

Add the free function (module-level, in the same file):

```rust
/// Focuses the first (`first = true`) or last enabled, visible menu action.
/// Returns whether a widget was focused.
fn focus_first_or_last_menu_item(content: &gtk::Widget, first: bool) -> bool {
    let children: Vec<gtk::Widget> = {
        let mut widget = if first {
            content.first_child()
        } else {
            content.last_child()
        };
        let mut ordered = Vec::new();
        while let Some(current) = widget {
            widget = if first {
                current.next_sibling()
            } else {
                current.prev_sibling()
            };
            ordered.push(current);
        }
        ordered
    };
    children
        .into_iter()
        .find(|child| child.is_sensitive() && child.get_visible() && child.is::<gtk::Button>())
        .is_some_and(|child| child.grab_focus())
}
```

(`gtk::Widget::get_visible()` — use whichever accessor this gtk-rs version exposes; the codebase already calls `.set_visible(...)` extensively, confirm the matching getter is `is_visible()` in this gtk4-rs version and use that name instead if so.)

- [ ] **Step 4: Run and verify it passes**

Run: `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --lib ui::browser::context_menu::tests::keyboard ui::window::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/window/keyboard/focus.rs src/ui/browser/context_menu/tests.rs src/ui/browser/context_menu/tests/keyboard.rs
git commit -m "feat(context-menu): navigate menu actions with Up/Down/Home/End"
```

---

### Task 8: Verify (and fix if needed) Escape-close, focus restoration, activation, and scroll-into-view

These four acceptance criteria are expected to already work from native GTK behavior (`gtk::Popover` `autohide`, native `gtk::Button` Enter/Space activation, `bind_column_context_owner`'s existing `connect_closed` restore-to-`focus_active()` for Columns, and `gtk::ScrolledWindow`'s automatic scroll-to-focused-child). This task is test-first specifically to catch any case where that assumption is wrong, rather than writing speculative extra code.

**Files:**
- Test: `src/ui/browser/context_menu/tests/keyboard.rs` (extend from Task 7)
- Modify (only if a test fails): `src/ui/browser/context_menu.rs` (`bind_column_context_owner`), `src/ui/browser_modes.rs` (Icons/List close handling)

**Interfaces:**
- Consumes: `BrowserView::open_focused_context_menu` (Task 5), existing `popover.connect_closed`.

- [ ] **Step 1: Write the test — Escape closes without changing selection**

```rust
#[test]
fn escape_closes_the_menu_without_changing_selection() {
    let view = /* fixture, select two items via the API used by existing multi-selection tests */;
    let selected_before = view.browser().selected_entries();
    view.open_focused_context_menu();
    let popover = /* the now-visible folder-context-popover, per menus.rs's lookup pattern */;
    popover.popdown(); // Popover::popdown() is what Escape's native autohide triggers
    assert_eq!(view.browser().selected_entries(), selected_before);
}
```

- [ ] **Step 2: Write the test — focus restoration after close, in Columns and in Icons/List**

```rust
#[test]
fn closing_the_menu_restores_focus_to_the_originating_item_in_columns() {
    // fixture in Columns mode; grab_focus on the item row; open via
    // open_focused_context_menu(); popdown(); assert the same row (or the
    // column) has focus again via gtk::prelude::RootExt::focus.
}

#[test]
fn closing_the_menu_restores_focus_to_the_originating_item_in_icons() {
    // same, BrowserMode::Icons
}
```

- [ ] **Step 3: Write the test — Enter/Space activates the focused action**

```rust
#[test]
fn enter_activates_the_focused_menu_action() {
    // open the item menu, child_focus to a known button (e.g. "Properties"),
    // call button.activate() directly (this is what GTK's native Enter/Space
    // binding does) and assert the expected side effect (properties dialog
    // shown, or popover closed) — this documents the native behavior as a
    // regression guard, not new logic.
}
```

- [ ] **Step 4: Write the test — long menu keeps focus visible**

```rust
#[test]
fn scrolling_menu_keeps_the_focused_action_visible() {
    // fixture with enough entries/actions that the menu's ScrolledWindow
    // has a real scroll range (e.g. multi-selection with every action
    // visible); child_focus to the last item; assert
    // scroll.vadjustment().value() + scroll.vadjustment().page_size()
    // covers the focused widget's y-position within the scrolled content
    // (compute via focused_widget.compute_bounds(&scroll_content)).
}
```

- [ ] **Step 5: Run all four**

Run: `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --lib ui::browser::context_menu::tests::keyboard -- --nocapture`

- [ ] **Step 6: Fix only what fails**

If `closing_the_menu_restores_focus_to_the_originating_item_in_icons` fails (the likely candidate, since `bind_column_context_owner`'s restore logic is Columns-specific per `focus_context_column`'s early return for non-Columns modes — read at `context_menu.rs:92-104`), add an analogous close handler for Icons/List: in `install_section_context_menu` / `install_context_menu` (`browser_modes.rs`), before returning the trigger, capture the widget that had focus (or the row/pane widget itself) and add a `popover.connect_closed` that calls `widget.grab_focus()` if still mapped, else falls back to `state.browser.focus_active()` — mirroring `SidebarFocus::restore`'s "restore if mapped, else fall back" pattern already used in `src/ui/window/keyboard.rs:180-188`.

If any other test fails, fix the minimal cause (e.g. `Popover` `autohide` not propagating `Escape` because the window-level Capture-phase `Dispatcher::dismissal` consumes it first — check `src/ui/window/keyboard.rs:39` `dismissal`'s Escape branch at `keyboard/items.rs:39-45`; if it fires before `popover_navigation`/`input_owner` lets the popover see Escape, add an early check in `Dispatcher::input_owner` or `dismissal` that returns `Some(Propagation::Proceed)` when focus is inside a `"folder-context-popover"`, so GTK's native popover Escape-handling gets it).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "fix(context-menu): restore focus after keyboard-closed menus in Icons and List"
```
(Adjust the message/scope to whatever Step 6 actually needed; if nothing needed fixing, skip this task's commit and note in the PR body that these criteria were verified, not newly implemented.)

---

### Task 9: Documentation

**Files:**
- Modify: `docs/keyboard-navigation.md`, `src/ui/shortcut_footer.rs`

**Interfaces:** none (docs/content only).

- [ ] **Step 1: Add a section to `docs/keyboard-navigation.md`**

Insert after the "## Arrows, the header, and the sidebar" section (end of file, before "## Review fixture"):

```markdown
## Opening and navigating the context menu

**Menu** (the hardware context-menu key) and **Shift+F10** open the selection-aware
context menu without the pointer. With an item keyboard-focused, the menu opens for
that item — or the full multi-selection, if the focused item is part of one. With no
item focus, it opens the active pane's background menu. The menu is anchored to the
focused item or pane, never to the pointer.

Once open: **Up/Down** move between enabled actions, wrapping past the first/last;
**Home/End** jump to the first/last enabled action; separators and disabled actions
are skipped. **Enter/Space** activates the focused action. **Escape** closes the menu
without changing the selection and returns keyboard focus to the item or pane that
opened it. This applies in Columns, Icons, List, Trash, and the file chooser.
```

- [ ] **Step 2: Add a shortcut-footer entry**

In `src/ui/shortcut_footer.rs`, add to the `FILES` array (after the `"Alt+Enter"` row, since it's file-item-adjacent):

```rust
    ("Alt+Enter", "Show item properties"),
    ("Menu / Shift+F10", "Open the context menu"),
```

- [ ] **Step 3: Verify the footer test fixture, if any, still matches**

Run: `cargo test --lib ui::shortcut_footer`
Expected: PASS (or update any test asserting the exact `FILES`/`TOOLS` contents/count, if one exists — check `src/ui/shortcut_footer.rs`'s own `#[cfg(test)]` module first).

- [ ] **Step 4: Commit**

```bash
git add docs/keyboard-navigation.md src/ui/shortcut_footer.rs
git commit -m "docs(keyboard): document the Menu/Shift+F10 context-menu shortcut"
```

---

### Task 10: Full local verification and PR

**Files:** none (verification only).

- [ ] **Step 1: Full test suite**

Run:
```bash
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --all-targets --all-features
```
Expected: PASS.

- [ ] **Step 2: Format and lint**

Run:
```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
```
Expected: no diffs, no warnings.

- [ ] **Step 3: E2E gate**

Run: `./scripts/e2e.sh` (per `AGENTS.md`; use `STRATA_CONTAINER_ENGINE=podman` if Docker requires a password prompt in this environment — confirm with the user first per this session's earlier note that the Docker daemon needs sudo here).

- [ ] **Step 4: Manual smoke test**

Launch the built binary and, in each of Columns/Icons/List/Trash and the file chooser: focus an item, press Menu, confirm the item menu opens anchored near the item; press Escape, confirm focus returns; select multiple items, press Shift+F10 on the anchor item, confirm the multi-selection menu appears; clear selection, press Menu, confirm the background menu opens; navigate a long menu (e.g. multi-selection in Trash) with Down to the last action and confirm it scrolls into view.

- [ ] **Step 5: Push and open the PR**

Follow `AGENTS.local.md`: push to `fork`, open the PR cross-repo against `lgse/strata:main`, fill in the PR template (description, manual test steps from Step 4, `Closes #583`), attach before/after screenshots or a short video of keyboard menu use per the theming/icon-visible-change convention, and verify no agent attribution appears in any commit message or the PR body before posting.

```bash
git push -u fork feat/583-keyboard-context-menu
gh pr create --draft --repo lgse/strata --base main --head spandan11106:feat/583-keyboard-context-menu \
  --title "feat(context-menu): add keyboard shortcut and complete menu navigation" \
  --body "$(cat <<'EOF'
## Summary
- Menu key and Shift+F10 open the selection-aware context menu anchored to the
  keyboard-focused item (or the active pane's background with no selection),
  in Columns, Icons, List, Trash, and the file chooser.
- Once open, Up/Down/Home/End navigate enabled actions (skipping separators
  and disabled rows, wrapping at the ends); Enter/Space activate; Escape
  closes without changing selection and restores focus.

## Test plan
- [ ] Menu key opens the item menu anchored to the focused item in each mode
- [ ] Shift+F10 does the same
- [ ] No selection: Menu opens the background menu
- [ ] Multi-selection is preserved when the anchor item is part of it
- [ ] Up/Down/Home/End move through enabled actions only, with wraparound
- [ ] Escape closes the menu, selection unchanged, focus restored
- [ ] Long menu (e.g. Trash multi-selection) scrolls the focused action into view

Closes #583
EOF
)"
```

---

## Self-Review Notes

- **Spec coverage:** Opening via Menu/Shift+F10 (Task 6), anchored to focused item not pointer (Tasks 3-5), multi-selection preserved (reuses existing `context_entries`/selection logic untouched by Tasks 1-2), background menu with no selection (Task 5), all five view contexts (Tasks 3-4 cover Columns and Icons/List/Trash; chooser covered via Task 2 since it shares the same call sites), Up/Down/Home/End skipping separators/disabled (Task 7, via native `child_focus` + sensitivity/visibility filtering), Enter/Space (verified native, Task 8), Escape + focus restore (Task 8), focus visible + scrolls into view (Task 8, CSS already themed via existing button focus styling — no new CSS class introduced since none was found missing in research; if Step 8's manual smoke test shows focus isn't visually distinguishable, add a `@theme_*`-based `:focus` rule to `src/style.css` for `.item-context-option:focus`/`.folder-context-option:focus` at that point, not speculatively now), accessible roles/names/states (already correct, unchanged), docs (Task 9), tests (Tasks 3-8). Submenu Left/Right is explicitly out of scope (Global Constraints) since none exist.
- **Placeholder scan:** every step shows real code against real, already-read signatures; the few points needing an implementer to confirm an exact field/accessor name (Task 4 Step 2's `Pane` field name, Task 7's `is_visible()` vs `get_visible()`) are flagged with the precise disambiguation to check, not left vague.
- **Type consistency:** `Rc<dyn Fn(f64, f64)>` is the trigger type end-to-end from Task 1 through Task 5; `context_menu_target(...) -> Option<(Rc<dyn Fn(f64, f64)>, f64, f64)>` is consistent between Task 3 (Columns) and Task 4 (Icons/List) so Task 5 can call either uniformly.
