# Minimal Mode (Yazi / Ranger home-row browsing)

| Field | Value |
| --- | --- |
| **Author** | Strata maintainers |
| **Date** | 2026-09-18 |
| **Status** | Draft (experimental feature, under active development) |
| **Design ID** | ab442a2a |
| **Related docs** | `docs/minimal-mode.md`, `docs/keyboard-navigation.md`, `docs/preferences.md`, `docs/architecture.md`, `docs/themes.md` |

## Overview

Strata already has a partial home-row language: with **Type to search** off, `h`/`j`/`k`/`l` remap to arrows via `ui::focus_navigation::navigation_key`, `y` copies a path, and `p` pins. That is opt-in only because the default `type_to_search = true` consumes every printable key as a pane filter (`TypeToSearch::show` in `src/ui/window.rs`). There is no dedicated browsing mode, pane chrome stays dense (Close / filter / refresh / sort on every Miller header; Search / Close on the window header), and file verbs stay on Ctrl chords (`Ctrl+C`/`X`/`V`, `Delete`, `F2`).

**Minimal mode** is an opt-in, application-wide browsing mode that (1) hides the named chrome, (2) disables the normal keymap including type-to-search, and (3) installs a Yazi/Ranger/lf/Vifm-style verb language on the existing `Browser` / `BrowserView` operations. It is not a Vim clone: no insert mode, no `:w`, no registers, no operator-pending grammar (`d3j`). The user stays in a browse state; keys are verbs and motions for files. Typed input (`/`, `f`, `s`, `a`, `r`, `g` then Space, …) uses the bottom bar as the prompt, not the pane-header filter or the `SearchDialog` overlay.

The mode is an **experimental feature, under active development**. Settings → General → Browsing shows **Minimal mode** with the note `(experimental feature, under active development)` immediately after the title. The same qualifier appears on the Settings → Keybindings “Minimal mode” heading, the F1 reference section of that name, and the footer `MIN` tag tooltip. Default is **off**. Existing users keep today’s chrome and keybinds.

## Background & Motivation

### Current keyboard architecture

Window-level keys are owned by a capture-phase `gtk::EventControllerKey` installed in `ui::window::keyboard::install` (`src/ui/window/keyboard.rs`). `Dispatcher::handle_key` runs a fixed pipeline:

```text
text size
 → input_owner (modals, F1 footer, Escape autoscroll)
 → window_commands (Ctrl+1/2/3, Ctrl+K / Ctrl+Shift+K)
 → inline_editing (F2 / Ctrl+R, Escape)
 → filter_and_location (Ctrl+F, Ctrl+L, Space-on-search-result)
 → video_controls
 → sidebar_commands (Ctrl+B, Ctrl+Shift+B)
 → context_menu (Menu / Shift+F10)
 → search-result clipboard special case
 → text_input (type-to-search, undo, location Escape)
 → file_commands (clipboard, hidden, terminal, refresh, Delete)
 → focus_navigation (header / sidebar / top bar)
 → dismissal (Escape, Backspace-on-empty-filter)
 → item_navigation (hjkl, Home/End, Page, y/p, Space preview)
```

A handful of shortcuts are also GTK application accelerators (`DEFAULT_ACCELS` in `src/ui/window.rs`): `win.search` (`<Control>k`), `win.jump-folder` (`<Control><Shift>k`), `win.open-terminal` (`<Primary>t`), `win.refresh` (`F5`), `win.toggle-arrow-scope` (`<Primary>backslash`). Those accels are process-wide (`application.set_accels_for_action` in `composition::install_browser_actions`).

`h`/`j`/`k`/`l` only become arrows when `type_to_search` is false. With the default on, they type into the pane filter. `y`/`p` (copy path / pin) only fire in `item_commands` after type-to-search declines the key.

### Current chrome

| Surface | Buttons to hide | Owner |
| --- | --- | --- |
| Window header | Search, Close window | `layout::Header` in `src/ui/window/composition/layout.rs` |
| Miller pane header | Close pane (`depth > 0`), filter toggle, refresh, sort field, sort direction | `ViewState` column assembly in `src/ui/browser/columns.rs` (~lines 643–677). Close calls `close_column`; Backspace already uses `navigate_up`. |
| Icons header | Filter toggle, refresh, sort field, sort direction | `icons_controls` in `src/ui/browser_modes.rs` |
| List header | Filter toggle, refresh; sort **buttons** only on Recent / Camera Photos | `build_list_pane` ~2626–2633. Ordinary List sort is clickable headings, not these buttons. |

Kept: sidebar toggle, location / breadcrumb bar, Settings, Appearance menu, sidebar, Miller/Icons/List body, preview drawer, footer, context menus. Strata has no tabs; extra windows are the multi-workspace model.

### Current operations (reuse, do not fork)

| Verb | Existing entry point |
| --- | --- |
| Copy / cut / paste | `BrowserView::copy_selection` / `cut_selection` / `paste` → `ViewState::copy_entries` / `cut_entries` / `paste_into` |
| Duplicate | `BrowserView::duplicate_selection` (no Yazi letter; lost in this mode unless a follow-up) |
| Trash / permanent delete | `BrowserView::confirm_trash()` (Move to Trash dialog), `confirm_delete(false\|true)`, `confirm_delete_preferring_cancel()` → dialogs in `ui/browser/trash.rs` |
| Rename | `BrowserView::begin_rename` + `inline_edit.rs` + `validate_basename` |
| New folder / file | `ViewState::begin_new_entry` → `Browser::create_new_folder` / `create_new_file` (`unique_name` retries in `adapters/local_operations/create_entry.rs`) |
| Filter | `BrowserView::show_filter` / `show_filter_with_query` + `index_filter` (always activates the header funnel) |
| Pane recursive listing | `InlineSearch` + `index_filter(root, …)` on **one** directory (`inline_search.rs`) |
| Global name search | `SearchDialog` + `index_trees(global_search_roots())` (`services/search.rs`, result cap 100) — **not** InlineSearch |
| Folder jump | `win.jump-folder` → `SearchDialog::show_history` on `NavigationHistory` |
| First / last | `BrowserView::jump_selection` (today `Ctrl+Up`/`Down`) |
| Page | `BrowserView::page_selection` (today `PageUp`/`PageDown`) |
| Parent / close pane / enter | `navigate_up` (Backspace, `close_column` when depth > 0), `navigate_left` (`focus_parent`, does **not** close), `activate_focused` (`l`/`Enter`), `enter_focused_directory` (Columns `Right`) |
| History | `Browser::back` / `forward` |
| Select all / clear | `BrowserView::select_all` / `Browser::clear_active_selection` |
| Range extend | `Browser::extend_selection` / `extend_visual_selection` |

There is **no** invert-selection API, **no** single-item toggle helper (docs mention `Ctrl+Space`; GTK ListView may handle it natively, Strata’s dispatcher does not), **no** content (ripgrep) search, **no** fzf/zoxide binary, **no** preview-scroll command (`PreviewDrawer::handle_video_key` is Ctrl+Alt media only).

## Goals & Non-Goals

### Goals

- Opt-in **minimal mode** that hides Close, Search, filter, refresh, and sort chrome.
- Disable the colliding default keymap (including type-to-search and `DEFAULT_ACCELS`) while the mode is on. Non-conflicting GUI conventions stay bound.
- Install a Yazi-family home-row language on top of existing `Browser` / `BrowserView` operations.
- Use the footer as the typed-command surface (`/`, `?`, `f`, `s`, `a`, `r`, `g` then Space). `S` is unbound until a content index exists.
- Show `g`-prefix chord hints as keycap overlays on sidebar destinations, including `1`–`9` for pins.
- Persist the mode as a `ThemeManager` preference; live-update every open window; default off.
- Work in Columns, Icons, and List. Pointer, marquee, drag/drop, and context menus keep working. Context-menu accelerator labels follow the active keymap (`x`/`y`/`p`/`d`/`D`/`r`/`i`, kept `Ctrl+A` / Enter); colliding default-map hints (`Y` for copy path, `Space` for preview, `Ctrl+R` for rename) are hidden. Copy path is `c c` (and the menu); Properties is `Alt+Enter`. Chooser menus keep the default map.

### Non-Goals

- Full Vim (insert mode, text objects, counts, operator-pending, `:commands`, named registers).
- Shipping `fd`, `ripgrep`, `fzf`, or `zoxide` as required binaries. `s` is current-location `index_filter`; `z`/`Z` are Strata `NavigationHistory`; `S` is unbound.
- Changing the portal file chooser (`ui/chooser.rs`). Chooser keeps its own dispatcher **and** its pane-header chrome. Chrome-visibility bindings are gated on `ViewState.interactive` (same pattern as folder peeking).
- Per-window mode that diverges from other windows in the same process (GTK accels and `ThemeManager` are process-wide).
- Layout tests, pixel assertions, or CSS-string tests for the new chrome.
- Redesigning Miller columns, preview sandbox, or the operation provider.
- Customizable keymaps in v1 (the Settings keybindings page stays a reference, not an editor).

## Proposed Design

### Preference and lifecycle

Add `minimal_mode: bool` to `ui::theme::Preferences` (`src/ui/theme.rs`), default `false`, serde `#[serde(default)]` so old `settings.toml` files load unchanged.

```rust
#[serde(default)]
minimal_mode: bool,
```

Follow `docs/preferences.md` exactly:

1. Getter `ThemeManager::minimal_mode()` and setter `set_minimal_mode(bool)` going through `save_preferences`.
2. Consumers bind with `ThemeManager::bind_preference` at widget construction (chrome visibility, keymap, footer prompt, sidebar overlay host). Settings only edits the boolean.
3. Extend the exhaustive fixture in `src/ui/theme/tests/preferences.rs` (`non_default_preferences` has no `..Default`) and the setter list in `all_preference_setters_publish_and_persist_without_duplicate_notifications`.
4. Behavioral coverage: saved `minimal_mode = true` applies before Settings opens; toggling in window A updates window B **including teardown of window-local state** (see `apply_minimal_mode` below); lazy view rebuilds (mode switch Columns ↔ Icons ↔ List) reconstruct pane headers already hidden.

**Two-window live update:** both windows follow. `ThemeManager` is process-wide (`SHARED_MANAGER` in `theme.rs`); `composition_initializes_live_preferences_before_settings_in_two_windows` is the existing pattern. A per-window mode would fight `application.set_accels_for_action`, which is also process-wide.

**`apply_minimal_mode(enabled)` (once per window, plus accels once per application):** each interactive browser window binds the preference on a long-lived widget (the window or footer root) and runs a single apply function:

When `enabled` becomes **true**:

- Cancel any in-flight pane filter: `dismiss_hidden_filter()` (clears `filter_entry` / query **without** requiring the funnel to have focus; `dismiss_focused_filter` is the default-map helper and is the wrong API here).
- Hide listed chrome (interactive browsers only).
- Swap F1 content to the minimal table; add `minimal-mode` CSS class.
- Do **not** start a prompt/chord/visual.

When `enabled` becomes **false** (including window B toggling while A is in Prompt/Chord/VisualSelect):

- Cancel `MinimalState`: cancel a pending `g` chord, remove sidebar `minimal-chord-hint` labels, leave visual mode **keeping** the filled selection, dismiss the footer prompt (clear the `gtk::Entry` so URI credentials do not linger), restore the footer stack to status, unfocus the prompt, restore previous file-view focus.
- Show chrome again.
- Remove `minimal-mode` CSS class; restore default F1 table.

**Accels:** clear/restore `DEFAULT_ACCELS` **once at the `gtk::Application`**, not from each window’s destroy path. Bind accels from a process-level listener (or the first window, re-bound on the application object) that reads `ThemeManager::minimal_mode()`. A window’s `connect_destroy` must **not** restore accels while the preference is still true and another window exists. Restoring accels happens only when the preference is false.

Test: prompt open in A, toggle from B (or Settings), A is Browse with chrome, default keys, no keycaps, empty footer entry.

**Entering / leaving:**

| Path | Effect |
| --- | --- |
| Settings → General → Browsing → **Minimal mode** `(experimental feature, under active development)` | Writes the preference. |
| `Ctrl+Shift+M` | Toggles the preference. Bound in **both** maps so it is always reachable. |
| `q` (while in the mode, not in a prompt/chord) | Leaves the mode (`set_minimal_mode(false)`) and flashes `Left minimal mode — Ctrl+Shift+M returns`. Does not close the window. |
| `Q` | Closes the current window (`gtk::Window::close`), same as today’s header Close. |

Default off: first launch and existing installs never enter the mode until the user opts in.

**Chooser / portal:** `BrowserView::new_chooser(..., interactive: false)` and the `--portal` process still load `ThemeManager::shared()` from the same `settings.toml`. **Keys** are already a separate dispatcher in `ui/chooser.rs`. **Chrome is not:** pane Close/filter/refresh/sort are built in `columns.rs` and `browser_modes.rs` for both the main window and the chooser. Every chrome-visibility binding **must** gate on `ViewState.interactive` (same as folder peeking: `interactive && manager.folder_peeking()` in `src/ui/browser/preferences.rs`). Chooser headers stay fully visible even when `minimal_mode = true`. Test: saved `minimal_mode = true` does not hide chooser header actions and does not install `MinimalDispatcher` on the chooser window.

### Chrome: what hides vs stays

Bind visibility from the preference **and** `interactive`. Do not destroy widgets: hiding preserves construction, accelerators on Settings, and lazy search-dialog setup.

**Hidden in interactive browsers (requirement):**

- Window header **Search** (`Header.search`) and **Close** (store a `close: gtk::Button` on `Header`; today it is a local in `Header::new`). `Q` replaces window Close.
- Miller pane **Close** (`depth > 0`, `browser.close_column`). **Replacement verb:** `h` calls `BrowserView::navigate_up`, which already does `close_column(depth)` when `depth > 0` and `parent()` at depth 0 (`browser.rs` 1114–1137). `h` is **not** `navigate_left` / `focus_parent` (those leave child panes open). Hide Close only because `h` is that button.
- **Filter toggle** and **filter revealer** on every pane. Footer `f` writes the query without `filter_button.set_active(true)` (new `BrowserView` helper). An active hidden filter is shown as a footer status mark (`filter: *.png`); see Footer.
- **Refresh** (`pane_refresh_button` hidden; **F5** stays bound). Auto-refresh preference still runs.
- **Sort menu** and **sort direction** on Miller headers, Icons headers, and List’s **Recent / Camera Photos** header buttons (`build_list_pane` ~2626–2633). **Ordinary List clickable headings stay** — they are the List sort UI, not those header buttons. Camera Photos Device-order rules in `docs/preferences.md` still apply via remaining headings / menus outside this hide list. Sort is also `, a` / `, m` / `, s` / `, e` (shift reverses) plus Appearance.

**Stays:**

| Chrome | Why |
| --- | --- |
| Sidebar + toggle | `g` chords land on it; mouse still useful |
| Location / breadcrumbs | Path context; `g` then Space types a path into the footer, not this bar |
| Settings + Appearance | Escape hatch; view mode still changeable from Appearance |
| Miller / Icons / List body | The product |
| Preview drawer | `J`/`K` scroll it; pointer still opens it |
| Footer | Becomes the command field |
| Context menus | Pointer and `Shift+F10` / Menu key remain |
| Empty Trash (Trash locations) | Destructive, not in the hide list |
| Icons thumbnail-size menu | Not in the hide list |
| List sort headings | Ordinary List sort; only Recent/Camera header sort buttons hide |

Add CSS class `minimal-mode` on the window root for any styling of remaining chrome (tighter header, footer prompt). Colors stay `@theme_*` only.

```mermaid
flowchart LR
  subgraph header [Header]
    SB[Sidebar toggle]
    LOC[Location bar]
    APP[Appearance]
    SET[Settings]
    SRCH[Search - hidden]
    X[Close - hidden]
  end
  subgraph panes [Pane headers]
    NAME[Folder name]
    CLOSE[Pane Close - hidden]
    FILT[Filter - hidden]
    REF[Refresh - hidden]
    SORT[Sort - hidden]
  end
  subgraph body [Body]
    SIDE[Sidebar + chord overlays]
    MILLER[Columns / Icons / List]
    PREV[Preview]
  end
  FOOT[Footer: status or prompt]
```

### Keymap installation and disabling the normal map

Minimal mode does **not** add a second `EventControllerKey`. It inserts a stage in the existing capture controller so modal layers, F1 help, an **already started** inline rename, and native text editing keep precedence.

**`navigation_key` remap must not run in this mode.** Today `Dispatcher::handle_key` remaps `h`/`j`/`k`/`l` to arrows **before** every stage whenever `type_to_search` is false (`focus_navigation.rs` 14–31). Key Decision 3 leaves `type_to_search` unchanged, so a user can have `type_to_search = false` **and** `minimal_mode = true`. If the remap still runs, `g` then `h` becomes `g` then `Left` and Home never fires (`gg` would still work because `g` is not remapped). Compute `navigation_key` **only** in the default pipeline, or pass the original GDK key into `MinimalDispatcher`. Test: `type_to_search = false`, `minimal_mode = true`, `g` then `h` navigates Home, not parent.

```text
handle_key:
  text size                          # always (Ctrl++ / Ctrl+- / Ctrl+0)
  input_owner (modals, F1/~ footer, Escape autoscroll)
  if focused widget is gtk::Text/Entry/TextView
       and not the minimal footer prompt:
       Proceed                       # native editing, location bar, Settings, dialogs
  if rename_is_active() || new_entry_is_active():
       existing inline_editing Escape / Ctrl+A-in-field only
       # do NOT call begin_rename() — F2/Ctrl+R must not start the row editor
  if selected_search_results() is Some and not text_has_focus
       and minimal_mode:
       search-result branch (see Search): map letters here; do not
       move_selection on the hidden directory column
  if minimal_mode:
       MinimalDispatcher             # keep/stop list below; never type-to-search
  else:
       navigation_key remap, then existing pipeline
       (search-result clipboard/arrow special case stays here)
```

**Keep vs stop (capture controller).** `Ctrl+,` is **not** in `DEFAULT_ACCELS`. Settings is a separate window `EventControllerKey` in `composition/settings.rs` on the default **Bubble** phase. Capture `Stop` on unmatched chords makes that controller dead. Clearing `DEFAULT_ACCELS` is still required for `win.search` / `win.refresh` / `win.open-terminal` / `win.jump-folder` / `win.toggle-arrow-scope`, but it is not sufficient for Settings.

| Capture action | Keys |
| --- | --- |
| **Handle in capture, Stop** | `Ctrl+Shift+M` (toggle), `Ctrl+,` (open Settings — handle inside `MinimalDispatcher`, do not rely on bubble), `F1` (help; **global**, stays in `ShortcutFooter::handle_key` / `input_owner`; toggles closed), `~` **opens in `MinimalDispatcher` only** (do **not** handle closed-state `~` in `ShortcutFooter::handle_key` — that runs before the mode split and would steal default-map type-to-search). Open-state `~` closes in `handle_key` so `input_owner` does not swallow it. Text size, `Menu` / `Shift+F10`, `Ctrl+Shift+B` (focus sidebar; does **not** show a hidden sidebar — header toggle still does), `Ctrl+Z` (undo), all keys in the minimal tables below, unmatched **printable** / function keys that would otherwise type-to-search (`Backspace`, `Space`, leftover letters, …) |
| **Proceed** | Unmatched **Ctrl/Alt/Super** chords so bubble Settings is redundant-safe, GTK widgets and dialogs keep native bindings we did not claim |

**Still bound in the mode (user-facing keep list):**

| Key | Action |
| --- | --- |
| `Ctrl+Shift+M` | Toggle minimal mode |
| `q` | Leave mode (footer flash `Left minimal mode — Ctrl+Shift+M returns`) |
| `Q` | Close window |
| `F1` | Shortcut reference (global, both maps) |
| `~` | Shortcut reference **in this mode only** (opens in `MinimalDispatcher`; a second `~` closes via `handle_key`. Default-map `~` stays type-to-search) |
| `Ctrl+,` | Settings |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` | Text size |
| `Menu` / `Shift+F10` | Context menu |
| `Ctrl+Shift+B` | Focus sidebar if visible. There, `j`/`k` move places, `l`/Enter/Space activate, `h` returns to the list. |
| `Ctrl+Z` | `undo_last_operation` |
| `Ctrl+a` | Select all (Yazi) |
| `Ctrl+r` | Invert selection (Yazi) |
| `Ctrl+c` / `Ctrl+x` / `Ctrl+v` | Copy / cut / paste (GUI; `Ctrl+v` uses the default-map paste, Replace on conflicts) |
| `Ctrl+u` / `Ctrl+d` | Half page |
| `Ctrl+b` / `Ctrl+f` | Full page (sidebar toggle is the header button; `Ctrl+B` is **not** toggle) |
| `PageUp` / `PageDown` | Same as `Ctrl+b` / `Ctrl+f` |
| `Home` / `End` | Same as `gg` / `G` |
| `Alt+Left` / `Alt+Right` | Same as `H` / `L` (history) |
| `Alt+Up` | Same as `h` (`navigate_up`) |
| `Alt+Enter` | Properties |
| `Delete` / `Shift+Delete` | Same as `d` / `D` |
| `F2` | Footer rename (same as `r`; does not start the row editor) |
| `F5` | Refresh |
| `Ctrl+H` / `Ctrl+.` | Toggle hidden files (same as `.`) |
| `Ctrl+L` | Edit the location bar |
| `Ctrl+K` | Global search (`win.search`) |
| `Ctrl+1` / `2` / `3` | Columns / Icons / List |
| `Ctrl+Shift+N` | New folder (existing create path) |
| Arrows / `Enter` / `Backspace` | Left / Up / Down same as `h`/`k`/`j`; Right and `l` open a directory or preview a file; Enter opens; Backspace same as `h` |
| `i` | Toggle preview drawer (replaces Space) |

**Intentionally unbound** (conflicts with Yazi, or Appearance / pointer / context menu only):

| Default-map key | Today | In minimal mode |
| --- | --- | --- |
| `Ctrl+Shift+K` / `win.jump-folder` | History jump | Unbound (`z` / `Z`) |
| `Ctrl+T` / `win.open-terminal` | Terminal | Unbound (context menu) |
| `Ctrl+\` / `win.toggle-arrow-scope` | Arrow scope | Unbound (arrows never leave the list; `h` is parent not sidebar) |
| `Ctrl+D` | Duplicate | Unbound (duplicate dropped; `Ctrl+d` is half-page) |
| `Ctrl+F` | Filter | Unbound (`f` footer). **After PR 2 there is no keyboard filter** (no menu equivalent). |
| `Ctrl+B` | Sidebar toggle | Unbound as toggle (`Ctrl+b` = page up; header button toggles) |
| `Ctrl+R` | Rename | Unbound as row editor (`r` / `F2` footer; `Ctrl+r` = invert) |
| `y` / `p` (default map) | Copy path / pin | Unbound (`y` yank, `p` paste; path copy is `c c`; pin via `g`+digit) |
| `Space` | Preview / open folder column | Toggle selection (`i` previews; `l` / `o` enter) |
| Type-to-search printables | Filter | Minimal verbs. After PR 2, `f`/`r`/`a`/`s`/`z`/`/`/`?` **Stop** so they do not type-to-search, but they are no-ops until PR 3. |

Conflict with type-to-search: **forced off for dispatch only.** Tests: `type_to_search = true` and `minimal_mode = true`, `j` moves the cursor and does not open a filter; `Ctrl+,` still opens Settings from a focused file list.

### Mode state machine

New module `src/ui/minimal_mode.rs` (window-local `Rc<RefCell<MinimalState>>` hung off `Dispatcher` / footer, **not** a `Browser` field). Filesystem state stays in `app::Browser`.

```text
Browse ──v──► VisualSelect ──Esc / second v──► Browse (keep fill)
        └──V──► VisualUnset ──Esc / second V──► Browse (keep fill)
Browse ──g──► Chord(g) ──Esc/invalid──► Browse
Browse ──/ ? f s a r──► Prompt(kind) ──Enter/Esc──► Browse
Chord(g)──Space──► Prompt(Goto)
Chord(g)──g/h/d/c/t/1-9──► Browse (act)
Visual* ──y x p P d D──► act, leave visual, keep fill
Visual* ──r──► Prompt(rename), leave visual
```

No insert mode. Prompt is a short-lived footer `gtk::Entry`, not Vim insert. `S` is unbound and does not enter Prompt.

```mermaid
stateDiagram-v2
  [*] --> Browse
  Browse --> VisualSelect: v
  Browse --> VisualUnset: V
  Browse --> Chord: g
  Browse --> Prompt: / ? f s a r
  VisualSelect --> VisualUnset: V
  VisualUnset --> VisualSelect: v
  VisualSelect --> Browse: Esc / v / y x p P d D
  VisualUnset --> Browse: Esc / V / y x p P d D
  VisualSelect --> Prompt: r
  Chord --> Browse: Esc / invalid / g h d c t 1-9
  Chord --> Prompt: Space
  Prompt --> Browse: Enter / Esc
```

### Navigation

All motions call existing `Browser` / `BrowserView` APIs and `keyboard_navigation()` so paste destination ownership stays keyboard-scoped (`docs/keyboard-navigation.md`).

| Key | Action | Existing API |
| --- | --- | --- |
| `h` | **Parent, closing the Miller child.** Same as today’s Backspace: `BrowserView::navigate_up`. Columns `depth > 0`: `close_column(depth)` (the hidden Close button). Columns `depth == 0`: navigate the first column to its parent location. Icons/List: `parent()`. **Never** focuses the sidebar (`navigate_left` / `single_pane_arrow_action` are not used). Sidebar is `g` or `Ctrl+Shift+B`. | `navigate_up` |
| `l` | Same as Right: open a directory, or preview a file when `preview_target` is Some. Does **not** open a file. | `activate_focused` or `preview.toggle` |
| `j` / `k` | Next / previous **visible listing order**. **Browse Columns, cursor-only fill:** `move_selection(±1)`. **Browse Icons/List, cursor-only fill:** `page_along(±1, 1, Some(&visual_order))`. **Browse with an explicit fill** (Space / Ctrl+A / Ctrl+R / leftover visual): `next_visible_index` then `focus_keeping_fill` so the fill is not rewritten. **Visual:** never `move_selection` / `page_along` / `jump_selection` / `page_by`. | Columns Browse cursor-only: `move_selection`; Icons/List Browse cursor-only: `page_along(±1, 1, order)`; explicit Browse: `focus_keeping_fill`; visual: `next_visible_index` then `extend_*` / `subtract_visual_selection` |
| `gg` / `Home` | First visible item. Chord `g` then `g`, or `Home`. **Browse, cursor-only fill:** `jump_selection(-1)` / `page_along(-1, usize::MAX, order)` (`focus_only`). **Browse, explicit fill:** `next_visible_index` then `focus_keeping_fill`. **Visual:** `next_visible_index` to first, then `extend_*` / `subtract_visual_selection` — **do not** call `jump_selection`. | see Browse vs visual split below |
| `G` / `End` | Last visible item. Same split as `gg`. |  |
| `Ctrl+u` / `Ctrl+d` | Half page. Duplicate (`Ctrl+D`) is dropped. **Browse, cursor-only fill:** `page_by(±1, page.items / 2)` (`page_along` + `focus_only`). **Browse, explicit fill:** `focus_keeping_fill`. **Visual:** `next_visible_index` by `page.items/2`, then extend/subtract. |  |
| `Ctrl+b` / `Ctrl+f` / `PageUp` / `PageDown` | Full page. Sidebar toggle is the header button, not `Ctrl+B`. Same Browse vs visual split. |  |
| `H` / `L` | History back / forward. | `Browser::back` / `forward` |
| `i` | **Toggle the preview drawer** on the focused file (today’s Space / `PreviewDrawer::toggle`). Pointer and single-click-previews still work. Empty folder: flash `Nothing to preview` (closing an open drawer still works). | `preview.toggle(preview_target(focused), depth)` |
| `J` / `K` | Scroll the **open** preview drawer, not the list. No-op if closed, or for video/audio (no document scroller). | `PreviewDrawer::scroll_by`: the currently **mapped** preview `ScrolledWindow` (`text_scroll` / PDF / generic `scroll` in `preview.rs`), `vadjustment += direction * page_size/2`. Do **not** change PDF zoom. |
| `Enter` | Enter directory or **open file** (`activate_focused`). | `activate_focused` |
| Left / Up / Down | Same as `h`/`k`/`j` (arrows never leave the file list in this mode). | Same APIs |
| Right / keypad Right | Open a directory (`activate_focused` / search-hit activate). On a file, toggle preview only when `preview_target` is Some; otherwise flash `Nothing to preview` and do not open. | `activate_focused` or `preview.toggle` |
| `Backspace` | Same as `h`. | `navigate_up` |

`gg` shares the `g` chord prefix with `gh`/`gd`/pins. Second `g` is “first item”; it does not navigate Home. Home is `gh`.

Paging must keep using `scrolling::focused_collection` so only the focused pane moves, matching `page_selection`’s comment.

**Shared “next index” helper (required):** extract the visible-order walk from `page_along` (the `visible` / `steps` / clamp logic in `navigation.rs` 1090–1117) as `next_visible_index(direction, steps, order) -> Option<usize>` that **does not** call `focus_only`. Cursor-only Browse wrappers (`page_along`, `page_by`, `jump_selection`, Icons/List `j`/`k`) then `focus_only` at that index. Explicit Browse wrappers use `focus_keeping_fill` at that index. Visual wrappers pass the index to `extend_visual_selection` / `extend_selection` / `subtract_visual_selection`. Test: `v`, `Ctrl+f`, fill is a **range**, not a single row.

In Icons, Browse `j`/`k` via `page_along(..., 1, visual_order)` is a behavior change vs type-to-search-off `j` (GTK spatial Down). That Yazi-like listing order is required so visual and Browse `j`/`k` agree.

### Selection (visual mode, loosely)

Strata’s model (`app::navigation`) already distinguishes:

- **Keyboard cursor:** `column.selected`
- **Filled selection:** `column.selected_locations`
- **Range anchor:** `column.selection_anchor`
- **Visual order** for Icons: `extend_visual_selection(depth, focused, order)`

Copy/cut already use the filled selection of the focused column, never a hovered row (`docs/keyboard-navigation.md`). Minimal mode follows that: **Space toggles the keyboard cursor item, not pointer hover.** Pointer hover remains a peek/pointer-ownership signal only.

`move_selection` **and** `page_along` end in `focus_only`, which **replaces** the fill with the cursor item (`navigation.rs` 1118 and 1251–1256). `page_selection`, `jump_selection`, and `page_by` all sit on `page_along`. That is correct for a **cursor-only** fill (exactly the focused item, not yet marked explicit). After Space / Ctrl+A / Ctrl+R / leaving visual, Browse motions must **not** call those wrappers: compute the target with `next_visible_index` then `focus_keeping_fill` so `column.selected` moves and `selected_locations` stays. Visual `j`/`k`/`gg`/`G`/Home/End/paging **must** compute the target with `next_visible_index` (same walk, pass `ModeViews::visual_order` in Icons/List) then `extend_selection` / `extend_visual_selection` / `subtract_visual_selection`. Never call `move_selection`, `page_along`, `jump_selection`, or `page_by` while visual is on.

| Key | Behavior |
| --- | --- |
| `Space` | Toggle membership of the keyboard cursor in `selected_locations` **without** `focus_only`, then advance one row keeping the fill (`focus_keeping_fill`). A cursor-only fill is treated as hover, not an explicit selection: Space **adds** that item (does not clear it) and moves down, so `Space`, `j`, `Space` builds a non-contiguous fill. Once the fill is explicit, Space toggles the hovered item then moves down. `y`/`x`/`d` then use the cursor fallback below. If the item becomes the only filled item, set `selection_anchor` to it. **Does not** toggle preview (`i` does). Docs/Settings `Ctrl+Space` is a native-GTK claim the dispatcher does not implement; this mode does not add `Ctrl+Space`. Empty folder: flash `Nothing to select`. |
| `v` | Enter **visual select**. Set `selection_anchor` to the cursor **without changing fill**. First motion **replaces** the fill with the range from that anchor to the new cursor (`extend_*`). `v` on an existing multi-fill therefore starts a **new range from the cursor**, it does not keep the old fill after you move. Second `v`: leave visual-select, keep fill. `v` while in `V`: switch to visual-select, anchor at cursor. |
| `V` | Enter **visual unset**. Anchor = cursor; fill is unchanged until motion. Each motion (including `gg` / `G` / page, **no wrap**, clamp at ends): take the span from **previous cursor** to **new cursor** in `visual_order` (inclusive) and **remove those locations from `selected_locations`**. Do not add the new cursor. Update `column.selected` to the new position without `focus_only`. A jump subtracts the whole span, not a no-op. Icons use `visual_order`. `V` while in `v`: switch to visual-unset. Second `V`: leave, keep remaining fill. |
| Space while visual | Same toggle; stay in visual mode; do not advance. |
| `Ctrl+a` | Select all in the focused pane. | `BrowserView::select_all` |
| `Ctrl+r` | Invert the filled selection in the focused pane (visible entries only, honoring hidden-files). New `Browser::invert_selection(depth)`. **Not** `begin_rename`. |
| `Esc` | Prompt or chord first. Else if a **hidden pane filter is active** (`ModeViews::filter_active` / non-empty `filter_entry` text, **no focus required**), `dismiss_hidden_filter()` / `set_filter_query_without_revealer("")` so `bind_filter_query` restores the listing — **before** selection-clear. Do **not** call `dismiss_focused_filter` (it only succeeds when the header funnel has focus, `browser.rs` 1624–1641; after Enter, focus is the list). Else if visual: leave visual, **keep** fill. Else: `clear_active_selection` (and close preview if open). |

Leaving visual on `y`/`x`/`d`/`D`/`p`/`P`: perform the verb, return to Browse, **keep** fill until `Esc`. `r` leaves visual and opens the rename prompt. `Esc` after a yank still clears the filled selection, matching Strata today.

`Shift+j`/`k` are not needed; `v` then `j` is the range language.

### File verbs

Operate on the filled selection if non-empty, else the keyboard cursor (same as copy/cut today when a cursor exists without a fill — `copy_path` already falls back to `focused_entry`; copy/cut currently require a non-empty `selected_entries`. **Decision:** `y`/`x`/`d` on an empty fill yank/cut/delete the **cursor** item, then fill-select it. That matches Yazi “hover then `y`” and avoids a silent no-op. Implement by selecting the cursor if `selected_locations` is empty before calling the existing method.

| Key | Vim analog | Strata mapping |
| --- | --- | --- |
| `y` | yank | `copy_selection` (GIO file clipboard + clear cut marks). **Replaces** today’s `y` = copy path. Path copy is `c c` (and the context menu). Empty folder: flash `Nothing to yank`. |
| `x` | cut | `cut_selection` (clipboard + `set_shared_cut` + cut styling). Empty folder: flash `Nothing to cut`. |
| `p` | put | `paste()` into the keyboard destination directory (`destination_depth`, not a hovered file). A cursor-only fill is hover: ignore the single-selected-folder rule. An explicit fill (Space / `v` / Ctrl+A / Ctrl+R) still pastes into a single selected folder. **Replaces** today’s `p` = pin. Pin via sidebar / `g`+digit. Existing conflict dialog (`FailIfExists` then Replace / Keep both / Skip). Focus **Keep Both** when that button is offered (Cancel on a move, where Keep Both is hidden). Default-map `Ctrl+V` still focuses Replace. Empty clipboard: flash `Nothing to paste` (same for `P`). |
| `P` | put overwrite | Same `paste_into` / `confirm_replace_conflict` path. Preferred primary is **Replace** for this invocation only. Keep both / Skip still offered. **Never** change `confirm_replace_conflict` defaults globally. Never skip the dialog. Test: default-map Ctrl+V still uses the current primary. |
| `Y` / `X` | unyank | Always `clear_cut` / `clear_shared_cut` (process-local, safe). Clear the GDK clipboard with `set_content(None)` **only if** `Clipboard::is_local()` is true (this process set it). There is no `is_local` helper in the crate today; use GDK’s. **Do not** wipe another app’s text/files clipboard. Footer pill already follows `connect_clipboard`. Test: unyank does not clear a foreign text clipboard. |
| `c` then `c` | copy path | `copy_path` (text paths on the clipboard). `c` then `n` copies display names. |
| `o` | open | Same as `l` / Enter (`activate_focused`). |
| `O` | open with | Open-with picker for the selection (or the focused item). |
| `.` | hidden | `toggle_hidden` (same as `Ctrl+H`). |
| `,` chords | sort | `, a` / `, m` / `, s` / `, e` sort by name / modified / size / type; shift (`, A` / `, M` / `, S` / `, E`) reverses. |
| `d` | delete | `confirm_trash()` — Move to Trash **confirmation** (file list, Enter confirms, Esc/Cancel aborts), then the existing fly-to-trash path. Default-map Delete / context menu still use `confirm_delete(false)` with no dialog. In Trash locations the existing code already promotes to permanent (`permanent \|\| in_trash`). Empty folder: flash `Nothing to delete`. |
| `D` | delete | `confirm_delete_preferring_cancel()` — permanent dialog, **Cancel** focused so unmodified `D` then Enter does not destroy. `l` / Right still moves to Permanently delete. Default-map Shift+Delete still focuses the destructive button. Never skip confirmation. |
| `r` | rename | Footer prompt only. Do **not** call `begin_rename()` from the capture path. Pre-fill the focused display name (stem-selected for files, full name for folders — reuse inline-edit selection rules). Enter runs `validate_basename` and the existing rename operation. Escape cancels. `F2` opens this prompt; `Ctrl+r` is invert. Empty folder: flash `Nothing to rename`. |
| `a` | append-ish create | Footer prompt, empty, placeholder `new file (append / for a folder)`. On Enter: if the name ends with `/`, **strip the trailing slash first**, then `validate_basename`. Reject empty after strip (`/` only). Then `create_directory` / `create_file_named` with `unique_name: false` so a conflict errors instead of `new folder (1)`. No trailing `/` → file. Do **not** pre-create `new file` then rename. Do not call `validate_basename` on the unstripped `foo/` (`operations.rs` rejects `/`). |

Typical flow: cursor or `v`+move → `y`/`x` → `h`/`l`/`gh`/`g1` to destination → `p`. Nothing is copied until paste; yank only writes the clipboard (existing GIO behavior).

Undo is on the keep list (`Ctrl+Z` in capture → `undo_last_operation`). Document it in the minimal F1 reference.

### Search, filter, find, jumps

| Key | Yazi meaning | Strata mapping |
| --- | --- | --- |
| `/` | Find next name in the current listing | Footer prompt. Incremental, **does not hide rows**. On each change, move the cursor to the next visible entry whose display name contains the query (case-insensitive, `fold_for_search`), then `grab_focus_without_selecting()` so the prompt keeps the keyboard. Wrap. **Live substring highlight:** every visible listing name whose folded form contains the query also highlights that match (`@theme_accent` on `@theme_surface`). Empty query: no highlights. Highlights clear when the prompt closes (Enter or Escape), even if `n`/`N` can repeat the stored query. View rebuilds while the prompt is open re-apply highlights. Closing then `/` again shows the prompt. Empty query + Enter is no-op. |
| `?` | Find previous | Same, reverse direction. |
| `n` / `N` | Repeat find | Repeat last submitted `/`/`?` query and direction (`N` flips), including multi-character queries. Stored on `MinimalState`, not a preference. |
| `f` | Filter current listing | Footer prompt writes the pane filter through `set_filter_query_without_revealer` (does **not** call `filter_button.set_active(true)` / `show_filter`; those always reveal the funnel and focus the header entry, `browser.rs` 1441–1452). Drive `notify_filter_query` / `filter_entry` text with the revealer collapsed. Wildcards and include-subfolders unchanged. **While a hidden filter is active**, the footer status page shows `filter: <query>` (include this widget in the footer visibility union). Enter keeps the filter and returns focus to the list. Escape in the prompt **or** Browse-mode Esc **before** selection-clear calls `dismiss_hidden_filter()` / `set_filter_query_without_revealer("")` — **not** `dismiss_focused_filter`. Test: footer `f` + query, Enter, Esc from the **list** restores all rows. |
| `s` | Recursive name search (`fd`) | **Not** `SearchDialog` and **not** a drop-in of today’s `InlineSearch` + `index_trees`. `InlineSearch` is wired to `index_filter(root, …)` on **one** directory (`inline_search.rs` 528). v1: extend `InlineSearch` to accept a recursive `index_filter` (or `index_trees` of **the current location only**). Honest `fd` subset: current folder tree, result cap 100, same `MAX_INDEX_ENTRIES` budget. Do **not** search `global_search_roots()` and do not “fill more hits” from other devices. Do not open `SearchDialog`. While results are showing, the footer count is the hit count, not the hidden directory’s fill. **Live full-name highlight:** every current hit’s entire display name uses `@theme_accent` on `@theme_surface` (not the matching substring). Empty query: no hits and no highlights. **Search-result branch** (when `selected_search_results()` is Some): because this mode skips `navigation_key`, a raw `Key::j` `Proceed` does **not** move the `InlineSearch` list. Map explicitly: `j`/`k` → Down/Up via `activate_native_arrow` (or `child_focus`); `l`/Right → open a directory hit or preview a file hit; Enter/`o` → activate result; `i` → preview; `v`/`V`/Space → visual/toggle on hits; `h` → dismiss search to the directory listing (clear query / `show_directory_listing`) in **one** step; `y`/`x`/`d` → existing clipboard/delete on search rows. **First Escape** from the prompt (non-empty query) or focus-loss **keeps** the hits and returns focus to them; **second Escape** (Browse, after Enter or that first close) dismisses back to the directory and restores a remembered `f`. Empty query + Escape cancels with no results. **Never** `move_selection` / `page_along` on the hidden directory column. |
| `S` | Content search (`ripgrep`) | **Unbound in v1.** No ripgrep backend; do not silently run `s`. The key is not listed in F1 or Keybindings and does not flash. Follow-up: sandboxed content index. Do not shell out to `rg`. |
| `z` / `Z` | fzf / zoxide | No fzf/zoxide binaries and **no zoxide database**. History is Strata visits only (`NavigationHistory::shared()`, recorded on successful loads). `z`: footer prompt plus a compact candidate popover (name + path rows, same layout as `SearchDialog` history). `NavigationHistory::search(query)` (fuzzy + **frecency**; empty query is frecency, not recency). `Z`: same visits sorted by `last_accessed` descending (needs a recency listing next to `search`; empty-query `search("")` is **not** that), filtered by the typed query. Cap 100. Do **not** open `SearchDialog`. Up/Down move the popover selection; Enter navigates to the selected row. Empty `z`/`Z` still shows the list so the destination is visible before Enter. A miss shows `No matching folders` in the popover and flashes the same on Enter. |
| `g` chords | Places | See next section. |
| `~` / `F1` | Help | `ShortcutFooter` popover with a `set_keymap(Minimal)` table bound on the same preference as chrome. Second `~` / `F1` (or Escape) closes, like Yazi help. `F1` stays in `ShortcutFooter::handle_key` / `input_owner` (both maps). **Closed-state `~` is handled only in `MinimalDispatcher` when `minimal_mode` is true** so default-map type-to-search is not stolen (`type_to_search_query` accepts printable unicode, including `~`). While the popover is already open, `handle_key` closes `~` in this mode — otherwise the read-only shield in `input_owner` swallows it before the dispatcher. |
| `q` / `Q` | Quit variants | Leave mode (footer flash `Left minimal mode — Ctrl+Shift+M returns`) / close window, as above. No confirmation. No application-quit chord in v1. |

Find (`/`) vs filter (`f`) is the important distinction: `/` is a cursor jump; `f` hides non-matches through the existing filter model.

### `g` chords, pins, overlays

**Prefix:** `g` enters `Chord(g)` and stays armed while the footer `g-` mark and keycaps are showing. `c` and `,` use the same no-timeout prefix (footer `c-` / `,-`, no sidebar keycaps). Every armed chord also shows a short popover of valid second keys. A second key completes, cancels, or starts a prompt; Escape or an unknown key cancels.

| Second key | Destination |
| --- | --- |
| `g` | First item (`jump_selection(-1)`), not Home |
| `h` | Home (`Location::local(home_directory())`) |
| `d` | Downloads (`glib::user_special_dir(UserDirectory::Downloads)`). Missing: flash `No Downloads folder`. |
| `c` | Config (`glib::user_config_dir()`, typically `~/.config`) even if it is not a sidebar row |
| `t` | Trash (bonus, maps to the existing Trash place if visible) |
| `n` | Network (`network:///`) |
| `r` | Recent (`recent:///`) |
| `k` | Documents (`UserDirectory::Documents`). Missing: flash `No Documents folder`. |
| `p` | Pictures (`UserDirectory::Pictures`). Missing: flash `No Pictures folder`. |
| `v` | Videos (`UserDirectory::Videos`). Missing: flash `No Videos folder`. |
| `1`…`9` | **Visible PINNED rows** after the same filters as `append_pinned_places` (drop standard-place locations, drop remotes when `local_only`). Number **1-based in display order**, not the raw bookmark index (`pinned_places.iter().enumerate()` keeps the bookmark index today and skips Home/Downloads pins). Missing index: cancel with footer `No pin N`. |
| `Space` | Prompt `go ›` in the footer; Tab / Shift+Tab cycle matching folders; Enter calls existing `navigate_input` / `submit_location` (mount dialogs still win via `input_owner`). Clear the entry on submit **and** Esc so `smb://user:pass@host` does not linger in the widget. |
| `Esc` | Cancel, no navigation |
| Any other key | Cancel. Do **not** interpret as type-to-search. Footer flash `Unknown chord`. Original `h` (not remapped Left) is required — see keymap. |

**Overlays:** when `Chord(g)` starts, show keycaps on matching **static / standard / pinned** rows whose child is the two-widget `icon+label` box from `sidebar_button` (`window.rs` 3040–3058). Do **not** append a third child onto device rows (eject/lock extra children).

- Home → `h`, Downloads → `d`, Trash → `t`, Network → `n`, Recent → `r`, Documents → `k`, Pictures → `p`, Videos → `v` (location targets; the chords work even if that place is hidden and has no overlay).
- Visible PINNED rows → `1`…`9` in the order they appear under the PINNED heading.
- Config (`gc`) has no default sidebar row; no overlay unless that path is a visible pin (then it has a digit; `gc` still works).

Implementation: insert a `gtk::Label` with class `minimal-chord-hint` into the `icon+label` box on chord start; remove it on end. On `queue_rebuild()` / `append_pinned_places`, cancel the chord.

Visual: keycap styling matching Settings (`settings-keycap`): `@theme_surface` background, `@theme_text`, `@theme_border`, `@theme_accent` when the row is the active place. No hex colors. Reduced-motion: show instantly, no pulse.

```mermaid
sequenceDiagram
  participant User
  participant Dispatch as MinimalDispatcher
  participant Footer
  participant Sidebar
  participant Browser
  User->>Dispatch: g
  Dispatch->>Footer: show "g-"
  Dispatch->>Sidebar: show keycaps (h d t 1 2 …)
  alt second key h
    Dispatch->>Sidebar: clear keycaps
    Dispatch->>Browser: navigate Home
  else Esc / invalid
    Dispatch->>Sidebar: clear keycaps
    Dispatch->>Footer: restore status (or flash Unknown)
  else Space
    Dispatch->>Sidebar: clear keycaps
    Dispatch->>Footer: prompt go ›
  end
```

### Footer command field

`ShortcutFooter` (`src/ui/shortcut_footer.rs`) already owns the bottom bar: F1 button, clipboard pill, item count. In minimal mode it gains a `gtk::Stack`:

- **status** page: existing F1 (optional via `show_keybinding_hints`), clipboard pill, count, plus a compact mode tag `MIN` using `@theme_accent`. Recursive `s` results replace the count with the hit total so it is not the hidden directory’s fill.
- **prompt** page: a prefix label (`/` `?` `filter` `search` `find` `new` `rename` `go`) + `gtk::Entry` + optional spinner for `s`/`z`.

`/` `?` `f` `s` `a` `r` `g` Space swap the stack to prompt and `grab_focus` the entry (`S` does not). `show_prompt` reveals `prompt_box` **before** `stack.set_visible_child_name("prompt")` — GtkStack ignores `set_visible_child` for a hidden page, so a second `/` after Enter or Escape must still show the prompt. Incremental `/`/`?` re-grab the entry with `grab_focus_without_selecting()` after each jump so typed characters stay in the query; listing `focus_active` must not steal the keyboard. While the prompt is focused, `Dispatcher` treats it as `text_has_focus` **except** for Enter (submit), Escape (cancel), and Up/Down (move listing / search results, matching today’s Ctrl+F Down-to-results in `docs/keyboard-navigation.md`).

Do not open `SearchDialog` (`search-backdrop` modal) or the pane filter revealer.

Prompt submit mapping:

| Prefix | Submit |
| --- | --- |
| `/` `?` | Store query; jump; leave prompt; listing unchanged |
| `filter` | Keep filter active; show `filter: <query>` on the status page; focus returns to the list |
| `search` | Start/replace `InlineSearch` stream (current-location recursive `index_filter`); focus first result |
| `new` | strip `/` if needed; `create_*` with typed name |
| `rename` | Existing rename op |
| `go` | `submit_location`; **clear the entry** |

Escape: cancel prompt, **clear the entry**, restore status page, restore previous focus (same pattern as F1 `focus_before`).

**Visibility:** `ShortcutFooter` hides the root unless F1 / clipboard pill / count is visible (`status_widgets` in `shortcut_footer.rs` 190–207). Add the prompt page, `MIN` tag, chord `g-` label, and active `filter:` mark to that union so a hints-off footer still appears. `set_mode` today only swaps Columns/Icons/List **sections**; add `set_keymap(Minimal)` and bind it on the same preference as chrome. `F1` stays in `ShortcutFooter::handle_key`. Closed-state `~` is **not** added there; open-state `~` closes in `handle_key` so the read-only shield does not swallow the toggle.

### Theming and icons

- Every new visual state (prompt, chord keycap, `MIN` tag, unknown-chord flash) uses `@theme_bg`, `@theme_surface`, `@theme_text`, `@theme_dim_text`, `@theme_accent`, `@theme_border`, `@theme_danger`, `@theme_glow`. No static hex.
- No new Lucide assets required for keycaps (text labels). If Settings needs a row icon, reuse `assets::icons::KEYBOARD` (`strata-keyboard`) via `assets::primary_icon`.
- Do not add a custom “vim” icon.

### Chooser, dialogs, Settings

- `ui/chooser.rs` keeps its dispatcher. Do **not** install `MinimalDispatcher` on chooser windows.
- Pane Close/filter/refresh/sort widgets are **shared construction**. Gate `set_visible` on `ViewState.interactive && minimal_mode`. Folder peeking is the precedent (`interactive && manager.folder_peeking()`).
- `--portal` loads the same `settings.toml`; the gate is still required.
- Visible modal layers (`visible_modal_layer`) still win in `input_owner` — delete confirmation, conflict dialogs, Properties, Settings overlay. `h`/`l` in those dialogs keep today’s `vim_focus_direction` (already used in `ui/browser/trash.rs`).
- Settings page is a switch only. It does not call `keyboard::install` or hide chrome itself. The switch ships in the keymap slice, not the plumbing-only slice (PR Plan).

## API / Interface Changes

### Preference

```rust
// src/ui/theme.rs, Preferences
#[serde(default)]
minimal_mode: bool, // default false

impl ThemeManager {
    pub fn minimal_mode(&self) -> bool { ... }
    pub fn set_minimal_mode(&self, enabled: bool) { ... }
}
```

Settings: `append_browsing_options` in `src/ui/settings/general.rs` adds a switch **after** “Keep arrows in file list”:

- Title: `Minimal mode`
- Experimental note (always visible, immediately after the title, before the description): `(experimental feature, under active development)`
- Description: `Hide pane chrome and use Yazi-style keys. Toggle with Ctrl+Shift+M.`

The same experimental note is part of every other existing “Minimal mode” label: Settings search still matches the `Minimal mode` row (aliases include `experimental`); **Settings → Keybindings** uses the heading `Minimal mode (experimental feature, under active development)` for that category; the F1 popover uses that heading for the leave/toggle section; the footer `MIN` tag tooltip is `Minimal mode (experimental feature, under active development): Yazi-style keys. Press q to leave.` Prompt accessible names stay `Minimal mode command` / `Minimal mode {prefix}` so the entry is still findable as a command field.

Register search in `src/ui/settings/search.rs` (`id: "minimal-mode"`, aliases `yazi vim ranger lf vifm hjkl keyboard experimental`).

Keybindings page: when the preference is on, the shortcut reference table should list the minimal map (or a second section), including the kept Ctrl/F-key conventions. The F1 popover must match. **Type to search** and **Keep arrows in file list** stay saved and editable; while the mode is on those rows show the subtitle `Not used in minimal mode.`

### Browser helpers (small, next to existing selection APIs)

```rust
// app::navigation.rs — extract from page_along without focus_only
pub fn next_visible_index(
    &self,
    direction: i32,
    steps: usize,
    order: Option<&[usize]>,
) -> Option<usize>;
pub fn toggle_focused_selection(&self) -> bool { ... } // no focus_only
pub fn focus_keeping_fill(&mut self, position: usize) -> Option<(usize, usize)>;
pub fn invert_selection(&self, depth: usize) { ... }
pub fn subtract_visual_selection(&mut self, depth, from, to, order) -> Option<Vec<usize>>;
// page_along keeps calling next_visible_index then focus_only (cursor-only Browse)
```

```rust
// BrowserView
pub fn page_by(&self, direction: i32, items: usize) -> bool;
// Browse only: page_along + focus_only. Visual must not call this.
pub fn set_filter_query_without_revealer(&self, query: &str) -> bool;
// writes filter_entry / notify_filter_query; does not set_active(true)
pub fn dismiss_hidden_filter(&self) -> bool;
// set_filter_query_without_revealer(""); no funnel-focus requirement
// keep dismiss_focused_filter for the default map
```

```rust
// PreviewDrawer
pub fn scroll_by(&self, direction: i32) -> bool;
// mapped scroller vadjustment ± page_size/2; false if closed, video, or audio
```

```rust
// Browser create path — expose named create without unique suffix
// already private: create_file_with_naming(parent, name, unique_name)
pub fn create_file_named(&self, parent: Location, name: String) { self.create_file_with_naming(parent, name, false); }
```

`ViewState::begin_new_entry` stays the allocation+rename path for the **context menu**. Minimal `a` should call `create_file_named` / `create_directory` with the typed name, then select the new row (existing `EntryCreated` handling) **without** starting inline rename.

### Dispatcher

```rust
// src/ui/minimal_mode.rs
pub struct MinimalController { /* prompt, pending chord, visual kind, last-find */ }

impl Dispatcher {
    fn minimal_commands(&self, event: &KeyEvent) -> KeyResult { ... }
}
```

`keyboard::Bindings` grows a `minimal: MinimalController` built in `WindowContent::bind`. `handle_key` must **not** call `focus_navigation::navigation_key` before `minimal_commands` when the preference is on. Pass the original GDK key.

Chooser tests construct `BrowserView::new_chooser` and assert header action widgets remain visible with `minimal_mode = true`.

### Header

```rust
pub(super) struct Header {
    widget: gtk::HeaderBar,
    pub(super) content: gtk::Box,
    pub(super) sidebar_toggle: gtk::ToggleButton,
    pub(super) search: gtk::Button,
    pub(super) close: gtk::Button, // new: needed to hide
    pub(super) settings: gtk::Button,
}
```

## Data Model Changes

**Saved:** one boolean in `$XDG_CONFIG_HOME/strata/settings.toml`:

```toml
minimal_mode = false
```

No migration beyond serde default. Invalid TOML still uses the existing “disable saving” path.

**Not saved (window-local / process-local):**

- Visual mode, chord prefix, prompt text, last `/` query — `MinimalState`
- Filter query — already window-local on each pane
- Navigation history — already `NavigationHistory::shared()`
- Pins — already `~/.config/gtk-3.0/bookmarks`

No schema change to `Location`, `FileEntry`, clipboard, or operations.

## Alternatives Considered

### 1. Per-window mode, not a preference

**Pros:** one “normal” window and one Yazi window. **Cons:** `ThemeManager` and `DEFAULT_ACCELS` are process-wide; Settings would have to lie about “current window”; two-window tests in this repo assume shared prefs. Rejected for v1. Revisit only if accels move entirely into the capture controller (a larger refactor).

### 2. Reuse type-to-search-off + hide chrome, keep Ctrl verbs

**Pros:** tiny change (`y` already copies path, `hjkl` already move). **Cons:** does not deliver yank/cut/paste on `y`/`x`/`p`, visual `v`, footer prompts, or `g` chords; `y`/`p` **conflict** with the Yazi verbs; type-to-search default still surprises. Rejected as the product.

### 3. Embed a real Yazi TUI in a VTE

**Pros:** perfect keymap. **Cons:** second filesystem stack, no Miller GTK preview/sandbox, no Strata operations/undo/trash dialogs, violates “reuse existing operations.” Rejected.

### 4. Footer prompt vs inline rename / SearchDialog

Inline rename is excellent spatially; SearchDialog is the polished palette. The requirement is explicit that `/` `f` `s` `a` `r` `g` Space use the bottom bar and **not** the toolbar search/filter UI. Rename therefore collects in the footer and commits through the existing op. SearchDialog remains for users not in the mode (`Ctrl+K`).

### 5. CSS-only chrome hiding vs widget `set_visible`

CSS `display:none` on new classes is tempting but fights GTK focus (hidden buttons can still be in the focus chain) and the existing header Left/Right walk (`TopBarNavigation::move_focus`). `set_visible(false)` is the same mechanism as `show_keybinding_hints` and Empty Trash. Prefer `set_visible` via `bind_preference`.

## Security & Privacy Considerations

| Risk | Severity | Mitigation |
| --- | --- | --- |
| `a` / `r` / `g` Space accept path-like strings | Medium | Strip a single trailing `/` on `a` **before** `validate_basename`; reject empty after strip. `g` Space uses `submit_location` (no shell) and **clears** the footer entry on submit/Esc so credentials do not linger. |
| `D` permanent delete is a single key | High | Keep the confirmation dialog. Default to Cancel in this mode. Never skip confirmation. |
| `P` overwrite | High | Keep `confirm_replace_conflict`. No silent replace. |
| `s` recursive search | Low | Current-location `index_filter` only; same `MAX_INDEX_ENTRIES` / 10s budget / generated-tree globs. Do not raise caps or fan out to `global_search_roots()`. |
| Content search follow-up | Medium | Must stay in-process or sandboxed; no `rg` on untrusted paths without the preview sandbox policy. |
| Chord overlays | Low | Labels only; no user-controlled markup. |
| Preference file | Low | Same TOML as other settings; no secrets. |

Auth is unchanged (mount password dialogs still own the keyboard via `input_owner`).

## Observability

- `tracing::debug` when the mode is entered/left (preference value, window pointer).
- `tracing::debug` on unknown chords and prompt submit kind (not the typed path, to avoid logging secrets in URIs).
- Existing operation/search tracing stays the source of truth for failures.
- No new metrics required for v1. If we later count mode usage, a single counter on `set_minimal_mode(true)` is enough.
- Failed preference writes already log and keep the in-memory value (`docs/preferences.md`).

## Rollout Plan

1. **Plumbing only** (PR 1): serialize `minimal_mode`, getter/setter, exhaustive fixture. No Settings switch, no production chrome hide, no keymap. Hand-editing `settings.toml` does nothing visible yet.
2. **Keymap + chrome + Settings** (PR 2): dispatcher cut, full motion table including `gg`, visual mode, `y`/`x`/`p`/`d`/`D`/`i`/`Space`, `q` / `Ctrl+Shift+M`, F1 table, `apply_minimal_mode`, interactive-gated chrome hide. Users who flip the switch can leave with `q`.
3. **Footer + chords + docs + e2e** (PR 3): prompt, `r`/`a`/`f`/`s`/`z`, `g` overlays, docs. `r` is not `begin_rename`.
4. No extra feature flag. Rollback = `minimal_mode = false` or delete the key.
5. Portal / packaging unchanged; chooser chrome stays up because of the `interactive` gate.

Repo default remains **one conversation, one PR**. The three slices are the mergeable cut if this is split; they must not land a Settings switch without `q`.

## Open Questions

1. **Application quit.** `q` leaves the mode, `Q` closes the window. There is no `Ctrl+Q` today. A third “quit GtkApplication” gesture is **out of v1**; use the desktop close-all. Revisit a `ZQ`-style chord only if users ask.
2. **Counts and operator-pending** (`3j`, `d3j`). Out of v1 (Vifm, not Yazi). Digits after `g` are pin chords.

Decided items that were previously listed here: `l` opens files (`docs/keyboard-navigation.md`); `i` toggles preview; `S` is unbound; duplicate is dropped (`Ctrl+d` is half-page); Settings/Appearance stay; `h` is `navigate_up`, never sidebar.

## Follow-up adjustments (not yet implemented)

The shipped v1 map above stays in force until a later coding pass lands each item. Do **not** treat this section as current behavior. Colors stay `@theme_*`; no hex.

### 2. Find (`/`) highlights matches live

Landed: `/` and `?` highlight matching substrings live while the prompt is open. Highlights clear on Enter/Escape/focus-loss; `n`/`N` after close does not restore them. Filter (`f`) and search (`s`) are unchanged.

### 3. Current filter stays visible until dismissed

Landed: After `f` + query + Enter, the applied filter remains the listing until dismissed. The footer `filter: <query>` mark survives listing motion, `Ctrl+1`/`2`/`3` rebuilds, opening and closing other prompts (`/` `s` `a` `r` `z` `g` Space), chords, visual mode, and two-window rebuilds. Hints-off still keeps the footer up while the mark is set. Escape from Browse dismisses the hidden filter **before** clearing selection. Opening `f` again pre-fills the current query. An empty Enter on `f` dismisses. The pane-header funnel stays hidden.

### 4. Search typing highlights the entire search

Landed: While typing in `s`, every current hit highlights its entire display name (`@theme_accent` on `@theme_surface`), not the matching substring. Highlights update as the query and hit list change. Empty query: no hits and no highlights. Distinct from `/`, which still highlights matching substrings in the unfiltered listing. Cap and current-location recursion are unchanged.

### 5. Search results persist after the first Escape

Landed: First Escape from `s` (or focus-loss) with a non-empty query keeps the hit list; the prompt closes, focus returns to the results, and `v`/`V`/Space/`i`/`y`/`x`/`d` work on those hits. Footer count stays the hit count. Second Escape dismisses InlineSearch back to the directory listing and restores a remembered `f`. `h` still leaves search in one step. Empty query + Escape cancels with no results. Enter already closes the prompt onto the hits, so the next Escape is the dismiss.

### 6. `l` and Right open directories and preview files

Landed: `l` / Right / keypad Right open a focused directory (listing: `activate_focused`; search-hit directory: activate). On a file they toggle the preview drawer only when `preview_target` is Some; otherwise they flash `Nothing to preview` and do not open the file or a placeholder drawer. `i` still toggles preview regardless. Empty folder: flash `Nothing to preview`. Enter / `o` still open files. F1 / Keybindings list `l` with Right as open-directory / preview-file. Default-map arrows are unchanged.

### 7. Chord hint lists

Landed: Every armed chord shows a short popover of valid second keys, not only a footer `g-` / `c-` / `,-` mark. `g` keeps sidebar keycaps **and** shows the list. `,` lists `a` name, `m` modified, `s` size, `e` type, with “shift reverses”. `c` lists `c` path and `n` name. `g` lists `g` first item, `h` Home, `d` Downloads, `c` Config, `t` Trash, `1`–`9` pins, Space path. The popover uses `@theme_surface` / `@theme_text` / `@theme_border` / `@theme_accent`; reduced-motion shows it instantly. It closes when the chord completes, cancels, or the preference turns off.

### 8. Sort chord is exclusive while armed

Landed: While `,` is armed, the second key is **only** a sort completion. Competing Browse verbs do not run: `s`/`S` are size (not search), `a`/`A` are name (not create), `m`/`M` are modified, `e`/`E` are type. `n`/`N`/`t`/`T` are not sort keys (natural sort and type aliases are unimplemented) and cancel with `Unknown chord`. Unknown second keys **do not** dispatch as Browse. The same exclusivity applies to `g` and `c`: search-result `j`/`k`/`h`/`l` and chrome/sidebar commands do not steal the second key. Capture **Stop**. Shifted letters still reverse sort. Escape cancels without a flash.

### 9. Copy clipboard icon (not only minimal mode)

Landed: Cut items overlay the Lucide scissors (`assets::icons::SCISSORS`) on listing thumbnails. Yanked / copied items overlay `assets::icons::COPY` the same way: Columns, Icons, List, and search-result rows; interactive browsers and the chooser; default map `Ctrl+C` and minimal `y`. Theme-colored via `assets::primary_icon` / `set_primary_icon`. Cut wins if an item is both cut and copied (copy clears cut marks). Clearing the clipboard or unyank (`Y`/`X`) removes the copy overlay. No new icon set; ISC Lucide attribution unchanged.

### 10. Tab-complete `g` Space paths

Landed: In the `g` then Space `go ›` prompt, Tab cycles directories (not files)
whose names match the current prefix in the directory being completed. Bare
`doc` completes against the current listing’s folders. After a slash, complete
in that parent (`~/dow` → `~/Downloads`). `~` expands to home for matching.
Wrap; Shift+Tab reverses. No match is a no-op (typed text stays, prompt stays).
Tab does not move GTK focus out of the footer entry. Enter still runs
`submit_location` on the completed text; Escape still clears the entry.

## Key Decisions

1. **Application-wide preference, default false, live two-window update** via `apply_minimal_mode` per window and accels once on the `gtk::Application`. Matches `ThemeManager`; does not surprise existing users.
2. **One capture controller, new dispatcher stage** — no second keymap widget, no forked `Browser`. Skip `navigation_key` remap in this mode so `gh` sees `h`, not Left.
3. **Do not mutate `type_to_search`.** Dispatch skips that path so leaving restores prior behavior.
4. **Reuse operation APIs.** `y`/`x`/`p`/`d`/`D`/`r`/`a`/`f`/`s`/`z` call `copy_selection`, `cut_selection`, `paste`, `confirm_trash` / `confirm_delete`, rename/create, current-location `index_filter`, `NavigationHistory::search`. No parallel clipboard or trash stack.
5. **Footer is the only typed-command surface in the mode.** Pane filter revealer and `SearchDialog` stay for the default map. Hidden filters keep a footer `filter:` mark; Esc dismisses them before clearing selection.
6. **Space toggles the keyboard cursor (without `focus_only`), then advances one row.** A cursor-only fill is hover, not an explicit selection: Space adds that item and moves down. Motions never rewrite an explicit fill. `i` toggles the preview drawer (today’s Space). `J`/`K` scroll the mapped preview scroller.
7. **`q` leaves the mode; `Q` closes the window.** `q` flashes `Left minimal mode — Ctrl+Shift+M returns` so the persisted, app-wide flip is not silent. No confirmation dialog. No application-quit chord. Settings and Appearance buttons stay so the user cannot get stuck.
8. **`h` is `navigate_up` (closes the Miller child), never `focus_parent` and never sidebar.** That is the hidden pane-Close verb. `l` is `activate_focused` (opens files), matching `docs/keyboard-navigation.md`.
9. **`g` chord stays armed while the overlay is showing; number visible PINNED rows 1–9 after filtering; `gg` is first-item; `gh` is Home.** Esc/invalid cancel. Overlays only on `sidebar_button` icon+label rows.
10. **`S` is unbound** (no footer hint, no silent name-search). `z` is fuzzy+frecency over **Strata** folder history; `Z` is `last_accessed`. No zoxide/fzf/rg binaries.
11. **Chooser/portal excluded for keys and chrome.** Gate visibility on `ViewState.interactive`.
12. **`y`/`p` default-map (copy path / pin) are not available.** Pin via `g`+digit; path copy via `c c` / `c n` and the context menu. Non-conflicting GUI conventions stay bound (`Ctrl+C`/`X`/`V`, `Delete`, `F2`, `F5`, `Ctrl+H`, `Ctrl+L`, `Ctrl+K`, `Ctrl+1`/`2`/`3`, `Ctrl+Shift+N`, `Alt+Enter`).
13. **Capture keep/stop list:** handle `Ctrl+,` in capture; Stop unmatched printables; Proceed unmatched Ctrl/Alt. `Ctrl+Z` stays. `Ctrl+B` is full-page, not sidebar toggle. Duplicate stays dropped (`Ctrl+d` is half-page). Terminal (`Ctrl+T`) and jump-folder (`Ctrl+Shift+K`) stay unbound.
14. **Visual motions never call `page_along` / `jump_selection` / `move_selection` / `page_by`.** They use `next_visible_index` then `extend_*` / `subtract_visual_selection`. Browse Icons/List `j`/`k` are `page_along(±1, 1, visual_order)` only while the fill is cursor-only; an explicit fill uses `focus_keeping_fill`. Columns Browse `j`/`k` stay `move_selection` for cursor-only fills. `V` subtracts the walked span. Space adds a cursor-only item, toggles an explicit item, and always advances one row.
15. **Do not ship a Settings switch or hide chrome until the keymap, `q`, and `Ctrl+Shift+M` exist.** Plumbing PR is field+tests only.

## Risks

| Risk | Severity | Mitigation |
| --- | --- | --- |
| Accidental `D` / `P` | High | Existing confirmation dialogs; never skip |
| Users enable the mode and cannot leave | High | `q`, `Ctrl+Shift+M`, Settings switch, header Settings button all leave or open Settings |
| Type-to-search eats the new map | High | Dispatcher cut-out; tests with `type_to_search = true` |
| GTK accels fire `Ctrl+K`/`F5` anyway | Medium | Clear/restore `DEFAULT_ACCELS` with the preference |
| Sidebar rebuild during `g` chord | Medium | Cancel chord on `queue_rebuild` |
| Footer prompt vs AT-SPI / e2e | Medium | Accessible name on the prompt; e2e uses the same private bus as other keyboard tests |
| Icons `h`/`l` vs spatial arrows | Medium | `h` = `navigate_up`; `j`/`k` = `visual_order`; never `single_pane_arrow_action` |
| Filter model driven from footer without revealer | Medium | `set_filter_query_without_revealer` / `dismiss_hidden_filter` (no funnel focus); footer `filter:` mark; Esc from the list restores rows |
| Visual page/jump wipes fill | High | `next_visible_index` then `extend_*`; never `page_along` in visual |
| `~` steals default type-to-search | High | Closed-state `~` only in `MinimalDispatcher`; open-state close in `handle_key`; F1 stays global |
| `g` then `h` remapped to Left | High | Skip `navigation_key` when `minimal_mode`; test with `type_to_search = false` |
| Capture Stop kills `Ctrl+,` | High | Handle Settings in capture; Proceed unmatched Ctrl/Alt |
| Chooser chrome follows the pref | High | Gate on `interactive`; chooser GTK test |
| Unyank wipes foreign clipboard | Medium | `Clipboard::is_local()` before `set_content(None)` |
| Two-window prompt teardown | Medium | `apply_minimal_mode(false)` cancels Prompt/Chord/Visual |
| Two-window accel flicker | Low | Accels on the application from the preference, not window destroy |

## Testing strategy

GTK tests on the private Xvfb/D-Bus (`docs/e2e-testing.md`). Never the user display. No layout tests.

**Must extend:**

- `ui/theme/tests/preferences.rs` exhaustive fixture + setter persistence
- Two-window live update (pattern: `composition_initializes_live_preferences_before_settings_in_two_windows`), including prompt-open in A then toggle from B
- Startup with saved `minimal_mode = true` before Settings opens (chrome hidden, `j` moves) — after the keymap slice, not the plumbing-only slice
- `ui/window/tests/keyboard_dispatch.rs` / `keyboard_policy.rs`: `j` does not type-to-search; `y` copies files not path; `Space` toggles selection not preview; `i` toggles preview; `q` leaves and flashes `Left minimal mode — Ctrl+Shift+M returns`; `Ctrl+,` opens Settings; `g` then `h` with `type_to_search = false` goes Home; non-conflicting GUI keys (`Ctrl+C`, `F2`, `F5`, `Ctrl+K`, …) stay bound; `.` toggles hidden; `,` sort and `c c` copy path
- Selection: Space adds a cursor-only item and advances; `Space`, `j`, `Space` is a non-contiguous fill; `Ctrl+A` / `Ctrl+R` then `j`/`k` keep the fill; cursor-only `j` still follows the cursor; `invert_selection`; visual `v`+`j` uses `extend_*` not `move_selection`; visual `v` then `Ctrl+f` keeps a **range** (not `page_along`/`focus_only`); `V` span subtract; `Esc` `dismiss_hidden_filter` then visual then clear
- Footer: `/` jumps without filtering; typed `/` characters keep prompt focus (a matching `d` does not trash); second `/` after hide is visible (`prompt_box` + stack page `prompt`); `f` filters without the funnel; Enter then Esc **from the list** restores all rows via `dismiss_hidden_filter`; `S` does not search; default-map `~` still type-to-search
- Footer `z` / `Z`: candidate popover lists history; Up/Down pick; Enter navigates to the selected row (empty query still shows the list)
- Search results: `j`/`k` move the InlineSearch list via `activate_native_arrow`, not `move_selection`
- `g` then `h`/`1`/invalid/Escape (no idle timeout; `g` … 1 s … `h` still goes Home); overlays only on icon+label rows; pin digits match **visible** PINNED order; missing Downloads flashes `No Downloads folder`
- `a` strips `/` before `validate_basename`; empty after strip rejected; conflict does not unique-rename
- Unyank does not clear a foreign text clipboard; `p` conflict focuses Keep Both, `P` focuses Replace; `P` does not change Ctrl+V dialog defaults; empty clipboard `p` flashes `Nothing to paste`; empty-folder `y`/`x`/`d`/`r`/`i`/`Space` flash matching no-op hints
- `d`/`D` still open confirmation
- Chooser: saved `minimal_mode = true` does not hide chooser headers or install `MinimalDispatcher`
- View rebuild Columns → Icons keeps chrome hidden (interactive only)
- F1 / `~` table swaps to the minimal reference; second `~` / F1 closes it. Settings, Keybindings, F1, and the `MIN` tooltip show `(experimental feature, under active development)` next to Minimal mode
- Context menu accelerator labels follow the active keymap (hide unbound default-map hints)

**E2E (canonical `./scripts/e2e.sh`, after unit coverage):** a focused scenario `tests/e2e/scenarios/test_minimal_mode.py` for enter/leave, `hjkl`, yank-paste, footer `/`. Not a full-suite excuse — only this file plus existing keyboard tests that would break if accels are cleared globally (they must see the default-off path).

**Pre-push (when implementing):** `./scripts/quality.sh fmt`, `./scripts/quality.sh clippy`, `./scripts/test-headless.py` with a filter that actually matches the new tests (e.g. `minimal_mode` plus `theme::tests::preferences`).

## References

- `docs/minimal-mode.md` — user-facing keymap
- `docs/keyboard-navigation.md` — cursor vs selection vs open path; paste ownership; type-to-search; footer
- `docs/preferences.md` — bind/save/fixture/two-window rules
- `docs/architecture.md` — UI vs `app::Browser` vs capabilities; presentations share one controller
- `src/ui/window/keyboard.rs` — capture dispatcher
- `src/ui/window.rs` — `TypeToSearch`, `DEFAULT_ACCELS`, `y`/`p` when type-to-search is off
- `src/ui/focus_navigation.rs` — `hjkl` → arrows
- `src/ui/browser/columns.rs` / `pane_header.rs` / `browser_modes.rs` — chrome to hide
- `src/ui/shortcut_footer.rs` — F1 + clipboard pill + count
- `src/ui/window/composition/search.rs` — `SearchDialog` / jump-folder
- `src/services/search.rs` — `index_filter` / `index_trees`
- `src/services/navigation_history.rs` — Strata folder history frecency (`search`), not zoxide
- `src/app/navigation.rs` — `extend_selection`, `clear_active_selection`, `move_selection`
- `src/ui/browser/clipboard.rs` / `transfer.rs` / `trash.rs` / `inline_edit.rs`
- Yazi keymap (home-row verbs, not Vim)

---

## PR Plan

Repo policy is **one conversation, one PR**. If this lands as a single PR, that PR must include keymap + `q` + chrome together — never a Settings switch that hides Close with no replacement.

If split, use **three** mergeable slices. Each leaves `main` usable (default off).

### PR 1 — `feat(minimal): add minimal_mode preference plumbing`

- **Title:** `feat(minimal): add minimal_mode preference plumbing`
- **Files:** `src/ui/theme.rs`, `src/ui/theme/tests/preferences.rs`, `docs/preferences.md`
- **Depends on:** none
- **Changes:** `minimal_mode` field (`#[serde(default)]`, default false), getter/setter through `save_preferences`, exhaustive fixture, setter persistence. **No** Settings row, **no** production chrome `set_visible`, **no** keymap. Tests may set the pref programmatically; the running UI must not change.

### PR 2 — `feat(minimal): Yazi keymap, chrome hide, and leave-mode`

- **Title:** `feat(minimal): install Yazi keymap and hide listed chrome`
- **Files:** `src/ui/minimal_mode.rs`, `src/ui/window/keyboard.rs`, `src/ui/window/keyboard/commands.rs`, `src/ui/window/keyboard/items.rs`, `src/ui/window.rs` (accels once on the application), `src/ui/window/composition/layout.rs`, `src/ui/window/composition/settings.rs` (keep `Ctrl+,` working), `src/ui/browser.rs` (`navigate_up`, `page_by`, `toggle_focused_selection` wiring), `src/ui/browser/columns.rs`, `src/ui/browser_modes.rs`, `src/ui/browser/preferences.rs` (interactive gate), `src/ui/preview.rs` (`scroll_by`, `i` uses existing `toggle`), `src/app/navigation.rs`, `src/app/browser.rs`, `src/ui/browser/clipboard.rs`, `src/ui/browser/transfer.rs`, `src/ui/settings/general.rs`, `src/ui/settings/search.rs`, `src/ui/shortcut_footer.rs` (minimal F1 table + `~`), `src/ui/window/tests/keyboard_dispatch.rs`, `src/ui/window/tests/keyboard_policy.rs`, `src/ui/window/tests/type_to_search.rs`, `src/app/navigation/tests.rs`, `src/ui/chooser` tests, `src/ui/window/composition/tests.rs`
- **Depends on:** PR 1
- **Changes:** Full motion table including the `g` prefix **actions** (`gg` first item, `gh` Home, `gd` Downloads, `gc` config, `gt` Trash, `g1`–`g9` visible pins). Unknown second key / timeout / Esc cancel (no overlays yet). Visual select/unset (`next_visible_index`, not `page_along`), Space toggle, `i` preview, `J`/`K` scroll mapped scroller, `y`/`x`/`p`/`P`/`Y`/`X`/`d`/`D` (not `r`/`a`/`f`/`s`/`z`/`/` — those need the footer). After this slice, `f`/`r`/`a`/`s`/`z`/`/`/`?` **Stop** as unmatched printables (no type-to-search) but are **no-ops**; filter has **no** keyboard or menu equivalent until PR 3 (create/rename still have the context menu). `q` / `Q` / `Ctrl+Shift+M`, keep/stop list (`~` only here, not in `ShortcutFooter::handle_key`), skip `navigation_key` remap, `apply_minimal_mode`, interactive-gated chrome hide, Settings switch, F1 minimal table. **Do not** call `begin_rename`. Tests listed under Testing strategy for keymap/chrome/chooser/two-window teardown.

### PR 3 — `feat(minimal): footer prompts, g-chords, docs, and e2e`

- **Title:** `feat(minimal): footer command prompt and g-chord overlays`
- **Files:** `src/ui/shortcut_footer.rs`, `src/ui/minimal_mode/prompt.rs`, `src/ui/minimal_mode/chords.rs`, `src/ui/browser.rs` (filter helper without revealer), `src/ui/inline_search.rs` (current-location recursive feed), `src/ui/window.rs` (visible PINNED numbering, chord hints on `sidebar_button` only), `src/style.css`, `src/app/browser.rs` (`create_file_named`), `docs/keyboard-navigation.md`, `docs/preferences.md`, `src/ui/settings/keybindings.rs`, `tests/e2e/scenarios/test_minimal_mode.py`
- **Depends on:** PR 2
- **Changes:** Footer stack in the visibility union; `/` `?` `n` `N` `f` `s` `z`/`Z` `a` `r`; `S` hard-disable hint; **overlays** on `g` chords already shipped in PR 2; `g` Space `submit_location` with entry clear; docs; e2e enter/leave, `hjkl`, yank-paste, footer `/`. `r` only here (footer, not `begin_rename`). After PR 2, `gh` already navigates; this slice only adds keycaps and the typed-path prompt.
