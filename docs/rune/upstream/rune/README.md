# Rune filing packet

Issue and comment drafts for [Rune](https://github.com/anomalyco/rune), ready for the owner to
review and file. Nothing here has been filed or posted. Rune takes issues with headless repros
and no pull requests; each entry below is one issue or one comment.

| | |
| --- | --- |
| Pin | `2a44156b21d63b0342a366b26d9222a6edd4622c` (plan §0), `dev` head on 2026-11-10 |
| Written against | `5d421fd` (the earlier pin) and `dev` `04b38ae` (2026-11-10), in a read-only clone |
| Re-verified | Every entry at `2a44156`, 2026-11-10 (see [Re-verification](#re-verification-at-2a44156)) |
| Upstream issues | All 222 issues (open and closed), with bodies and comments, read-only with `gh issue list` on 2026-11-10, plus targeted searches that day; none past #507. Re-check before filing: upstream moves daily. |
| Entries | 96 files: 80 new issues and 16 comments on open issues. 12 drafts dropped, and 3 drafts dropped in part. At `2a44156`, 94 are ready to file (6 of them trimmed) and 2 are dropped (65, 80), leaving 79 new issues and 15 comments. |
| Sources | 174 drafts: 161 with ids (S5's U1–U17, C8's U18–U23, the work packages' `U-*` and `R-*`, C11's R1–R7) and 13 without (comment drafts for #102, #149 and #173, C15's five gaps, C14a's two, C9's, C20a's NumberField proposal and wheel follow-up). The [mapping](#draft-to-entry-mapping) places every one. |

## How it was built

1. **Collected** every draft: `git grep` for `U-…`, `R-…` and "Rune draft" in `crates` and
   `docs`, the "Rune drafts" and "Rune gaps" sections of each module's docs,
   [`S5-upstream.md`](../../decisions/S5-upstream.md) (U1–U17 and the later appendix), the S1–S3
   records it cites, `docs/testing.md` and `docs/work/STATUS.md`.
2. **Checked each gap twice**: at the pin, in cargo's checkout of Rune, and at `04b38ae`, in a
   worktree of the upstream clone. Each entry's `Checked` row says how: a repro run, the source
   read (with `file:line`), or an earlier probe by the work package that found it.
3. **Ran the headless repros** where cheap: two scratch crates in
   `strata-spikes/rune-packet/` (`repros` pins Rune at `5d421fd`, `repros-head` paths to the
   `04b38ae` worktree), built one at a time with `-j 4`, run with `DISPLAY` and
   `WAYLAND_DISPLAY` cleared. Each runnable entry's Rust block was extracted from its markdown
   into both crates as is (`extract_docs.py`, `run_docs.sh`), so the pasted program is the one
   that ran. 24 entries ran, every one with the same output at the pin and at head; 4 windowed
   programs were compiled at both revs but not run (they need a compositor; S1–S3 measured them
   under headless sway at `78d22ed`).
4. **De-duplicated**: drafts asking for the same thing were merged into one entry; drafts that
   match an issue filed upstream became a comment on it (several were filed upstream after S5
   wrote its drafts: #309, #310, #313, #354, #357, #364, #387, #397, #424, #505); drafts fixed past
   the pin, or that no longer apply, are dropped with the reason.
5. **Ranked** by value: how many components carry a workaround for the gap, and how costly the
   workaround is. Entries 01 and 05–08 retire whole `strata-platform` shims; the next tier
   retires workarounds in many components; rune-kit asks and small gaps come last.

## Re-verification at `2a44156`

On 2026-11-10 the pin moved to `2a44156` (Rune `dev` head, 2 commits past `04b38ae`) and every
entry was checked again before filing:

- **Duplicates**: every issue's body and comments (222, fetched that day) grepped for the
  entry's terms, then targeted `gh` searches; for comments, the target thread read live. Nothing
  was filed after #507.
- **The gap**: the Rune source each body cites re-read at `2a44156`, with its `file:line`
  updated; the 22 headless `doc_NN` repros re-run (all printed their Output blocks); the 12
  entries checked only at the old pin, and several checked by source only, probed headless at
  `2a44156` (`rv_NN` programs in `strata-spikes/rune-packet/repros-head/src/bin/`; 58 under a
  private Xvfb). Windowed halves that need a compositor (01, 05, 06, 07, 11, 12, 23, 24) were
  not re-run; their programs compile at `2a44156`.
- **Against the gallery**: each cited workaround, deviation or known gap confirmed still in the
  code, and the gallery and component tests that cover it run filtered at `2a44156` (all
  passed; P3, P4 and P5's compositor tests ran in private headless sway).

Each entry's `Checked` row leads with how it was re-checked, and its new `Status` row says
whether it is ready. Results: 88 ready as written (01 reworded around `App::turn_scope`), 6 ready trimmed (05, 13, 22, 31, 54 and 68
drop parts Rune or the target thread already has), 2 dropped (65, 80). No gap was fixed
upstream between `5d421fd` and `2a44156`.

Follow-ups in this repository, found on the way (not filings):

- C2 can use `El::drawn_rect_into` for its drop targets during a width spring (entry 65).
- C11's module docs (R3) and `strata-platform`'s `dnd/mod.rs` say Rune lacks things it has:
  inline images in read-only text (`El::inline_icon`, entry 80) and `drag_out`'s copy, move
  and link (entry 05).
- C3's module docs say a press hides a tooltip; since Rune `0cde17ec` a press on the element
  keeps an open one.
- Entry 89 asks against Rune's stated design (`frame/clock.rs`: "a late frame never loses
  time"); the owner may keep only its "make it an option" half.

## Filing

- Each file has notes for the owner (a table), then a line "Paste from here.", then the title and
  the body. Paste everything below the rule.
- File **01 first**: 05, 06, 07 and 23 link it. "(link to NN)" in a body is a placeholder for
  entry NN's issue URL once filed; replace each, or drop it when that entry isn't filed.
- Repro programs depend on `rune` at the pin with features `accessibility`, `dbus`,
  `embedded-fonts` and `svg`, as S5's drafts did; those that use them also need `rune-kit` at the
  same rev and `serde_json`. Run them headless (`DISPLAY` and `WAYLAND_DISPLAY` cleared) unless
  the entry says a compositor is needed.
- The bodies name Strata as "Strata (lgse/strata), a file manager on Rune" (owner decision,
  2026-11-10).
- Each entry's last notes row, `Status`, says whether it is ready. Before filing, if Rune's `dev`
  has moved past `2a44156`, re-check the gap there and search for a duplicate filed since
  2026-11-10. After filing, record the link in
  [`S5-upstream.md`](../../decisions/S5-upstream.md)'s index and in the module docs that cite the
  draft.

## Entries, highest value first

| # | Entry | Kind | Affects | Retires | Status at `2a44156` |
| --- | --- | --- | --- | --- | --- |
| 01 | [Display handle, its lifetime, and an every-turn hook](01-display-handle.md) | New issue | P0, P1, P2, P3 (X11), P4 | P0's `dlsym("wl_proxy_get_display")` recovery and its reader thread, the second X11 connection | Ready |
| 02 | [A held drag hears no `Up` when its element leaves or a modal mounts](02-held-drag-no-up.md) | New issue | C2, C4, C10b, C11b, C24, P1 | `stop()`/`reset()` calls hosts must make on a view switch, C10b's "bar wasn't laid out" check, C24's `Bar::release` | Ready |
| 03 | [rune-kit text fields drop handlers meant for their inner field](03-kit-text-field-handlers.md) | New issue | C5, C6, C14b, C17, C18, C19, C20a, C20b, C21, C26 | Each component's own frame around core `text_input`, in place of rune-kit `TextInput` | Ready |
| 04 | [Linux pop-up menus: field menus draw nothing, and in-window submenus run off the window](04-linux-popup-menus.md) | New issue | C5, C6, C10b, C18, C21, C22, the chooser's minimum size | C21's in-window `FieldMenu` attached to each field; the chooser's minimum-size floor | Ready |
| 05 | [Linux `drag_out`, with several MIME types, an "ask" action and an element snapshot](05-drag-out-linux.md) | New issue | P1, P0, X2 | P1's `wl_data_device` and XDND drag source, its second-`Runtime` drag icon | Ready, trimmed (see its Status) |
| 06 | [Comment on #424: Wayland drops, and drops with any MIME type, the action and a position](06-comment-424-typed-drops.md) | Comment on #424 | P1, X2 | P1's `wl_data_device` drop target and its X11 `XdndProxy` (which turns off Rune's own X11 `on_drop_files`) | Ready |
| 07 | [Linux clipboard: custom MIME types, several forms per copy, typed reads, ownership](07-clipboard.md) | New issue | P2, X3 | P2's data-control and `wl_data_device` clipboard, its X11 selection owner and `SAVE_TARGETS` hand-off | Ready |
| 08 | [No public way to end a pointer capture](08-end-pointer-capture.md) | New issue | P1 | P1's synthetic Escape through `Tx::send_key`, its proof checks, and the seven capture paths in `dnd/mod.rs` | Ready |
| 09 | [Comment on #106: focus change, focus within, and a field's blur](09-comment-106-focus-change.md) | Comment on #106 | C1e, C5, C6, C9, C16, C18, C20a, C20b | Each host's per-frame poll comparing `TextEdit::focused` (C6, C18, C20a, C20b, a frame late), C16's countdown that focus can't pause, C9's whole-pane ring | Ready |
| 10 | [Wheel routing: modified wheels, gesture latching, overlays, and sideways strips](10-wheel-routing.md) | New issue | C1e, C2, C3, C6, C7, C20a, C24b | C1e's `Collection::on_wheel` on the viewport, C6's and C7's vertical-to-sideways mapping (C7's `wheel.rs`, "fragile"), C24b's `OutsideWheel` panel guard | Ready |
| 11 | [Comment on #309: edge resize from a borderless window, on Wayland](11-comment-309-edge-resize.md) | Comment on #309 | P3, X1 | P3's "no edge resize on Wayland" known limit (owner's P3 decision), and on X11 the `_NET_WM_MOVERESIZE` sent from a second connection | Ready |
| 12 | [Comment on #310: minimise a borderless window, and its maximised state, on Linux](12-comment-310-minimise-maximised.md) | Comment on #310 | P3, X1 (header) | X1's hidden Minimise button on Wayland, the header's guessed maximise/restore icon | Ready |
| 13 | [Comment on #313: the window menu on Wayland too, and `wm_capabilities`](13-comment-313-window-menu.md) | Comment on #313 | P3, X1 | X1's right-click on the header doing nothing on Linux; buttons shown that the compositor can't honour | Ready, trimmed (see its Status) |
| 14 | [File-drop hooks: acceptance with app state, the whole drag, enter and leave, the action](14-file-drop-hooks.md) | New issue | W1's `Dnd::rune` binder, C1e, C2, C6, C7, C8, X2 | C1e's forwarded drag-over hook (`Collection::on_drag_over`), C8's folder check that waits for the drop, C7's tabs that open only after the drop | Ready |
| 15 | [`element_at` for app-side drop routing: passive layers, rects, and what a hit carries](15-element-at-drop-routing.md) | New issue | P1, W1e, X2, C2, C6, C24 | W1e's keys and scopes passed to every drop zone, P1's `drawn_rect_into` takeover, C24's zero-size `scroll::anchor` for bars over targets | Ready |
| 16 | [Accessibility of virtual lists and grids: positions, set sizes, groups, row and column](16-virtual-a11y-positions.md) | New issue | C1e/C1f (grouped lists), C3 (icon grid), C4 (list table), C11b (sheet grid), C20d (icon picker) | Group headers that read as items; grids whose rows read "row 2 of 40" | Ready |
| 17 | [Virtualization is vertical only: no grid, no columns, and sideways content is cut](17-virtualize-horizontal.md) | New issue | C3 (icon grid), C11b (sheet grid), C10b and C11 (wrap-off documents) | C3's tiles packed into rows and re-keyed on every reflow, C11b's own column windowing from `on_layout_tx`, C10b's min width from an estimate of the widest rows | Ready |
| 18 | [Exact or per-index heights for `virtual_rows`](18-virtual-row-heights.md) | New issue | C1e/C1f (group headers forced to the row height), C12 (PDF pages of mixed sizes) | C1f's "header lines are the row height" deviation; C12's zoom that lands up to 0.4 pages off for a frame | Ready |
| 19 | [`live_text` turns off the virtual-window fast path](19-live-text-fast-path.md) | New issue | C1e/C1f (stuck group header), C13, C14b (route labels), C25 (gallery status lines) | Built text where a live label was wanted, and the view build per group crossed | Ready |
| 20 | [Mipmaps for a streamed `Pixels`](20-pixels-mipmaps.md) | New issue | C25 (thumbnail store), W2d (thumbnail size classes), C12 (zoomed-out pages), C14a (images shrunk to fit) | W2d's 32/64/128/256 px thumbnail classes held at most twice their drawn size; hosts' use of `ImageCache` for stills only | Ready |
| 21 | [A text input can't take another role: combobox, spin button](21-text-input-roles.md) | New issue | C6 (location bar), C17 (search palette), C18 (filter bar), C20a (number field) | The `combobox` group wrapped around C6's, C17's and C18's inputs; C20a's spin-button wrapper with `focusable(false)` and its own Page keys | Ready |
| 22 | [Comment on #21: what freedesktop icon lookup needs for a file manager](22-comment-21-icon-theme.md) | Comment on #21 | P6 (icon theme lookup), C21 (Open With), file-type icons everywhere | P6's GTK-4-style lookup (search path, `Inherits`, `index.theme` sizes, its worker thread and LRU) | Ready, trimmed (see its Status) |
| 23 | [A window transient for another app's window (or one of the app's own), applied before it maps](23-transient-for.md) | New issue | P4 (foreign parent), the portal file chooser | P4's `zxdg_importer_v2` import on winit's display and its own-connection X11 hints; the "opened hidden, one window per request" rule | Ready |
| 24 | [Settings portal keys beyond `color-scheme`, with change signals](24-settings-portal.md) | New issue | P5 (desktop settings), K2 (text scale), C1b (double-click time), C23 (animations) | P5's zbus reader of `org.freedesktop.portal.Settings` | Ready |
| 25 | [The double-click interval is fixed at 0.5 s on Linux](25-double-click-interval.md) | New issue | P5, C1b (click runs), C4 (fit-to-content double-click), X1 (title-bar double-click) | C1b's own click-run counting with the host's double-click time; C4's deviation "fit at Rune's 0.5 s" | Ready |
| 26 | [Linux: play app-decoded PCM with a playback clock, without `Player`](26-pcm-output.md) | New issue | P7 (media audio), C13 (media player) | P7's GStreamer `appsrc` sink and its clock | Ready |
| 27 | [A scroll request past the mounted rows runs the view; the wheel doesn't](27-scroll-request-view-runs.md) | New issue | C24 (scroll bars), rune-kit's own `ScrollBar` | C24's allowance for Rune's "stepping its own glide" warning on a thumb drag | Ready |
| 28 | [A scoped reveal can't be kept to its own viewport's axis](28-scoped-reveal-axis.md) | New issue | C2 (Miller columns), C1e (`set_own_viewport_reveals`) | C1e's own-viewport reveals by an offset computed from geometry, which give up Rune's following of a row layout moves | Ready |
| 29 | [`virtual_rows`: `reveal_selected` and `scroll_key_to` miss rows that aren't laid out](29-virtual-rows-reveal.md) | New issue | C17 (search palette), C12 (PDF view) | C17's `Tx::scroll_into_view_in` on each arrow; C12's re-request of a zoom's centre row a frame later | Ready |
| 30 | [Comment on #265: hover and hover delays reach elements under an overlay too](30-comment-265-hover-under-overlay.md) | Comment on #265 | C22 (folder peek), C1e (row hover) | C22's `set_covered` host duty and its "no peek while covered" rule | Ready |
| 31 | [Live values for more than rects, opacity and text](31-live-bindings.md) | New issue | C13 (spectrum, waveform), C16 (progress bars), C4 (column resize) | C13's 96 `live_rect` bars in place of a shape; C16's track width read back through `on_layout_tx` and its four-a-second accessibility commits; C4's view commit on every resize move | Ready, trimmed (see its Status) |
| 32 | [`El::modal`: say why it was dismissed, and a capture layer that leaves focus where it is](32-modal-dismiss-and-focus.md) | New issue | C20b (colour picker), C24b (autoscroll shield), C5 (rename commits on blur), C17 | C20b's Escape taken in an ancestor's `on_key` first, and its outside-press report a frame late; C5's "autoscroll commits the rename" note | Ready |
| 33 | [Text shaping: colour changes and single keystrokes reshape the whole text](33-text-reshape-cost.md) | New issue | C10b (code view and script editor), C10a (highlighting) | C10b's 128-line colour window and its D9 deviation (a 25–27 ms keystroke at 5,000 lines) | Ready |
| 34 | [A caret and keyboard selection in read-only text](34-read-only-caret.md) | New issue | C10b (code view), C11 (document view), C12 (PDF text, which ports its own caret) | C10b's D1 and C11's D3 deviations (no caret or keyboard selection in read-only views) | Ready |
| 35 | [Selection across several texts: `select_text`, triple-click, an empty end, links](35-selection-across-texts.md) | New issue | C11 (document view), C10b (code view) | C11's D4 (a find match across units selects its first part) and D9; C10b's D11 and D12 | Ready |
| 36 | [Copying a selection across unbuilt virtual rows stops at a cap, silently](36-selection-copy-cap.md) | New issue | C11 (document view), C10b (code view) | C11's and C10b's Ctrl+A copy from their own model (and the frame in which Ctrl+C copies Rune's cut report) | Ready |
| 37 | [rune-kit `Table`: reusable column sizing, reorder, visibility, sideways overflow, fit](37-kit-table.md) | New issue | C4 (list table, built on its own collection instead of `Table`), C11b (sheet grid) | C4's and C11b's own tables styled as rune-kit's | Ready |
| 38 | [rune-kit menus and pop-up lists: submenu keys and hover, item keys, names, rich labels](38-kit-menu.md) | New issue | C21 (context menus), C6 (location bar's hierarchy menu and completion list), C17, C18 | C21's own Right/Left/Enter handling and the sent Tab into a submenu, its "rows keyed by position" deviation, C21's reliance on `Menu::build`'s layer order; C6's own completion list instead of `PopupList` | Ready |
| 39 | [rune-kit `TabStrip`: labels, look hooks, gaps, keys and edge margin](39-kit-tab-strip.md) | New issue | C7 (browser tabs), C11b (sheet tabs) | C7's ids prefixed with the strip key, the underline and tint set over each tab, its one-jump reveal clear of the edge fades (`probe.rs`) | Ready |
| 40 | [rune-kit `Sidebar` and `Reorder`: a rail form, focus without pressing, sections, keyboard moves](40-kit-sidebar.md) | New issue | C8 (places sidebar) | C8's own list, headings, section and icon-only rail on `SidebarRow`, its parked reorder move | Ready |
| 41 | [Route an assistive client's action on an option to its container](41-a11y-option-action.md) | New issue | C1e (every collection: list, icons, columns, table), C20d (icon picker, D10) | "A screen-reader user selects with the keyboard only" in C1e; C20d's D10 | Ready |
| 42 | [Accessibility: `aria-current`, a scroll bar role, busy, and a two-axis control](42-a11y-roles-states.md) | New issue | C6 (breadcrumbs), C10b and C24 (scroll bars), C14b (3D model view), C20b (colour plane) | C6's current crumb as a label last in the list, C10b's position bar read as a slider, C14b's spinner label standing for busy | Ready |
| 43 | [`Tx::scroll_by`, a viewport's offset, and arrow keys that scroll a focused viewport](43-scroll-by.md) | New issue | C19 (shortcut reference), C7 (tab strip wheel) | C19's row-step reveals for arrow scrolling; C7's offset read back from where its content drew | Ready |
| 44 | [Headless tooling: what scripts and `--tree` can't reach](44-headless-tooling.md) | New issue | C1c, C1e, C5, C6, C7, C17, C18, C25, X1 (tests that run in process instead of as scripts) | In-process tests for IME, titlebar actions, layout-dependent chords and image state; the gallery's 1.1 s warm-up for the commit-budget line | Ready |
| 45 | [Section headers that stick across virtual rows](45-sticky-sections.md) | New issue | C1e/C1f (grouped collections), C4 (grouped list table) | C1f's stuck-header copy drawn over the viewport's top line, pushed by a live value | Ready |
| 46 | [`on_press` also fires after the element's own drag](46-press-after-drag.md) | New issue | C4 (header sort click after a resize or reorder drag), C2 | C4's held-back sort click cleared in `ListTable::build` | Ready |
| 47 | [Alt with a letter types the letter into a text field on Linux](47-alt-letter.md) | New issue | C18 (footer prompt), C19 (shortcut reference search) | The "Alt+letter types into a prompt" known gap; the prompt's Alt chords | Ready |
| 48 | [No "press outside" for a field](48-press-outside.md) | New issue | C5 (inline rename), C6 (path bar: a press outside cancels a frame later) | C5's D4 list of six presses that keep the field focused, each turned into a host commit | Ready |
| 49 | [A secondary press on a menu's outside scrim reaches nothing](49-secondary-press-scrim.md) | New issue | C21 (context menus), C1e | C21's "a right-click outside an open menu does nothing" (bullet 184 not met) | Ready |
| 50 | [A second `on_scroll_offset` replaces the first; chaining is hidden](50-scroll-offset-chain.md) | New issue | C4 (sideways scroll followers), C22 (folder peek), C24b | C4's single handler that calls the host's followers (`ListTable::follow_sideways`) | Ready |
| 51 | [rune-kit scroll bars over viewports keyed alike share state and drag the first](51-kit-scroll-bar-keys.md) | New issue | C24 (its own overlay bar, a deviation from the plan) | One reason C24 keeps its own scroll bar instead of rune-kit's | Ready |
| 52 | [Comment on #365: a scroll bar with a trough, apart from its viewport](52-comment-365-scroll-bar.md) | Comment on #365 | C24 (its own overlay bar) | C24's own bar (with 51) | Ready |
| 53 | [`on_step`'s Page Up and Page Down are always 10 steps](53-range-page.md) | New issue | C13 (media timeline: 5 s steps, 30 s pages), C20a (slider: Strata pages 4 steps) | C13's and C20a's Page keys taken in `on_key` before `on_step` | Ready |
| 54 | [Comment on #149: what a kit Slider needs, from a file manager's sliders](54-comment-149-slider.md) | Comment on #149 | C20a (slider), C13 (volume and timeline) | C20a's own Slider (with 53) | Ready, trimmed (see its Status) |
| 55 | [Linux: middle-click paste of the primary selection in text fields](55-middle-click-paste.md) | New issue | C24b, C5 (rename field), C6, every text field; P2 | C24b's "no middle-click paste" deviation | Ready |
| 56 | [rune-kit `Keybind` upper-cases single characters](56-keybind-case.md) | New issue | C18 (chord panel), C8 (sidebar keycaps), C19 | C18's own keycap chip | Ready |
| 57 | [rune-kit: export the Combobox fuzzy matcher, with match positions](57-fuzzy-matcher.md) | New issue | W1's `contract::fuzzy`, C17, C19, C20c, C20d | The port of `score` that also records matched ranges | Ready |
| 58 | [X11: `AppHandle::activate(token)` on an open window ignores the startup id](58-x11-activation-token.md) | New issue | P8 (single instance) | P8's note that a forwarded launch may not raise the window on mutter or KWin | Ready |
| 59 | [Comment on #364: at offset 0, rows inserted above scroll out of view](59-comment-364-anchor-at-top.md) | Comment on #364 | C17 (search palette streaming better results first) | C17's rows keyed by place so the anchored key keeps its place | Ready |
| 60 | [Two-line text with a middle cut](60-two-line-middle-elision.md) | New issue | C3 (icon captions) | C3's own two-line middle cut on grapheme clusters (and its `unicode-segmentation` dependency) | Ready |
| 61 | [Per-element tooltip timing](61-tooltip-timing.md) | New issue | C3 (icon names wait 900 ms; buttons the toolkit's) | C3's deviation "tooltips via Rune's `.help` at the host's delay" | Ready |
| 62 | [`on_visible_rows` a frame late after a rekeying commit](62-visible-rows-late.md) | New issue | C1f, C3 (grid reflow), C25 (thumbnail requests) | A frame of thumbnail requests for the old range after every reflow | Ready |
| 63 | [`on_drag` on a `.virtualize` stack reports y from the mounted rows](63-virtualize-drag-y.md) | New issue | C1e (all pointer input on the container) | C1e's content coordinates rebuilt from the window point, the viewport's recorded origin and the scroll offset | Ready |
| 64 | [Stop a glide in place](64-stop-glide.md) | New issue | C1e (keyboard and marquee scrolls), C2 (column strip glides) | C1e's stop by a jump to a computed target, or a glide on a stiff spring | Ready |
| 65 | [`on_layout` reports a layout spring's end, not where the frame draws](65-on-layout-drawn.md) | New issue | C2 (column hit tests during a width spring) | C2's drop targets ahead of what is drawn while a column's width springs | Drop: already in Rune (`El::drawn_rect_into`) |
| 66 | [rune-kit `HoverCard`: placements, gap, width, hover on the surface, a role and name](66-kit-hover-card.md) | New issue | C22 (folder peek) | C22's computed anchor and its overhang deviation | Ready |
| 67 | [Anchored overlays are bounded by the window only](67-anchor-boundary.md) | New issue | C22 (folder peek beside a drawer) | C22's `FolderPeek::bounds` clip computed by the host | Ready |
| 68 | [rune-kit `ToastRegion` and `Toast` are fixed](68-kit-toast-region.md) | New issue | C16 (progress dock, built on Rune primitives instead) | C16's own dock | Ready, trimmed (see its Status) |
| 69 | [rune-kit `Popover` has no Escape and no initial focus](69-kit-popover-modal.md) | New issue | C20b (colour button) | C20b's `El::modal` on the popover's body | Ready |
| 70 | [Comment on #397: a tab mode for `SegmentedControl`](70-comment-397-segmented-tabs.md) | Comment on #397 | C20c (settings tabs), C19, C20d (category tabs) | C20c's, C19's and C20d's own tablists | Ready |
| 71 | [An offstage subtree: kept mounted, out of layout, input and the tree](71-offstage.md) | New issue | C20c (tabbed form's kept pages) | C20c's kept pages as opacity 0, `inert` and `a11y_hidden` in a shared grid cell, still laid out every frame | Ready |
| 72 | [A scroll offset survives an instant removal, but not a faded one](72-instant-removal-offset.md) | New issue | C20c (rebuilt pages) | C20c's per-visit key for rebuilt pages | Ready |
| 73 | [A `Sequence` has no reduced pose](73-sequence-reduced.md) | New issue | C20c (settings search's found-row highlight) | C20c's static fill removed by the host's tick when animations are off | Ready |
| 74 | [Comment on #505: matched flights that start late or arc, and rotation in a beat](74-comment-505-matched-flights.md) | Comment on #505 | C23 (trash flight, shudder) | C23's deviations 1 (a straight path), 2 (stagger by arrival) and 4 (a scale-only shake) | Ready |
| 75 | [Capture an element's drawn pixels](75-capture-pixels.md) | New issue | C23 (dissolve), P1 (drag icons rendered in a second `Runtime`) | C23's dissolve from a host-built ghost instead of the real pixels | Ready |
| 76 | [A handler can't read a layout](76-tx-layout.md) | New issue | C23 (effects started from a handler) | C23's frame spent recording where an effect's sources are | Ready |
| 77 | [Cursors: all-scroll, wait and progress](77-cursors.md) | New issue | C24 (middle-click autoscroll), C14b (model view rendering) | C24's `Move` cursor for autoscroll; C14b's default arrow while rendering | Ready |
| 78 | [Inside a draggable parent, a child's cursor never shows](78-cursor-innermost.md) | New issue | C12 (PDF text over draggable pages) | C12's text band showing the page's grab cursor | Ready |
| 79 | [Selectable text laid over glyph boxes the app places](79-positioned-text.md) | New issue | C12 (PDF text layer) | C12's port of a PDF text selection (caret, word and line, copy) and its silent accessible text | Ready |
| 80 | [Comment on #350: inline images in read-only rich text](80-comment-350-inline-images.md) | Comment on #350 | C11 (document view: inline images in Markdown and documents) | C11's images as blocks of their own | Drop: duplicate of #350 |
| 81 | [A multi-line text field that doesn't wrap](81-field-no-wrap.md) | New issue | C10b (script editor with wrap off) | C10b's field made wider than its widest line inside a sideways viewport | Ready |
| 82 | [Ask whether a grapheme draws](82-glyph-coverage.md) | New issue | C20d (emoji picker) | C20d's D3 (an emoji version cap instead of hiding what can't draw) | Ready |
| 83 | [Comment on #102: the outline route isn't DejaVu's alone](83-comment-102-emoji-outlines.md) | Comment on #102 | C20d (`drawn_text` adds U+FE0F) | C20d's `drawn_text` workaround | Ready |
| 84 | [Comment on #114: list the installed families](84-comment-114-font-list.md) | Comment on #114 | K2 (font picker) | K2's font list passed in as data | Ready |
| 85 | [A thread's measurer can keep the wrong fonts under the new fonts' generation](85-measurer-race.md) | New issue | K6 (font tests), strata-kit and gallery test layout | The rule that a test needing the measure path's fonts lives alone in its own test binary | Ready |
| 86 | [Off macOS, Ctrl+wheel over `on_magnify` becomes an inverted pinch with no fallback](86-ctrl-wheel-pinch.md) | New issue | C14a (image viewport), C12 (PDF zoom) | C14a's `on_magnify` on macOS only, with Ctrl+wheel handled in `on_wheel` elsewhere | Ready |
| 87 | [No notification on a scale-only change](87-scale-change.md) | New issue | C14a, C14b (image and model viewports), C12 | C14a's one-off rescale child mounted to resync after a scale change | Ready |
| 88 | [Closing a modal returns focus to an opener that became disabled](88-modal-restore-disabled.md) | New issue | C15 (dialogs) | C15's "Rune gap … or Strata policy with `return_focus`" bullet | Ready |
| 89 | [Presence motion takes a long frame's whole step](89-presence-long-frame.md) | New issue | C15 (dialog open and close) | C15's known gap for the bullet "A frame longer than 33 ms…" | Ready |
| 90 | [Arrow keys between a dialog's controls by position](90-arrow-focus-by-position.md) | New issue | C15 (dialogs) | C15's partial bullet "With focus outside a text field, an unmodified arrow key…" | Ready |
| 91 | [`on_window_focus` is heard only on the root](91-window-focus-root.md) | New issue | C6 (location bar), X1 (header's active state), gallery pages | Hosts forwarding window activation to each component | Ready |
| 92 | [`Pixels::set` drops the image it replaces](92-pixels-replace.md) | New issue | P7 (frame pacer) | A 3.7 MB allocation and free per 720p frame | Ready |
| 93 | [Draw a run-time image in the element's colour (symbolic icons)](93-pixels-tint.md) | New issue | P6 (icon theme), the sidebar and toolbar icons loaded from the desktop's theme | P6's deviation "symbolic icons drawn as authored" | Ready |
| 94 | [The single-colour icon rasterizer ignores dashes, miter limits and clip paths](94-icon-rasterizer-attributes.md) | New issue | K3 (icon catalogs drawn under Theme colors) | The three Catppuccin icons listed as drawn differently under Theme colors | Ready |
| 95 | [Whether a `Pixels` is drawn](95-pixels-last-drawn.md) | New issue | C25 (thumbnail store's recency) | C25's recency approximated from builds and visible ranges | Ready |
| 96 | [Comment on #354: a text-field menu's Undo and Redo](96-comment-354-undo-state.md) | Comment on #354 | C21 (field menu) | C21's undo of a cloned `TextEdit` when the menu opens, to learn whether Undo would do anything | Ready |

## Owner decisions (2026-11-10)

- **Hold off filing.** The owner will make a second pass over the entries to avoid flooding the project; until then the packet is documentation only.
- **Re-verify at a new pin first.** The owner will bump the Rune pin and have each entry re-verified before anything is filed. Done at `2a44156` (see [Re-verification](#re-verification-at-2a44156)). That covers #173 (`Tx::focus`, fixed upstream in `22aa8cde` but still open) and the 12 entries checked only at `5d421fd`.
- **Name Strata** in the bodies (done: the context lines say "Strata (lgse/strata), a file manager on Rune").
- **Entry 24:** attach the fake-portal script as a gist instead of pasting it inline.
- The scratch in `strata-spikes/rune-packet/` (repro crates and a worktree of Rune head) stays for the re-verification.
- The comment-vs-new-issue question (U2, U6–U8) waits for the owner's second pass.

## Owner questions

Each has a recommendation; the packet is written to it.

1. **Comments in place of S5's new issues.** S5 drafted U2 and U6–U8 as new issues; since then
   #424 (Wayland drops), #309 (edge resize), #310 (minimise and maximised state) and #313
   (window menu) were filed upstream. Recommendation: comments (06, 11–13), as written here.
   U1 stays a new issue linking #171 (S5 question 5).
2. **How much to file, and when.** 80 new issues is a lot for one pre-release project.
   Recommendation: file 01–31 first (the platform shims and the cross-component gaps), then the
   rest in batches, and hold the rune-kit look and configuration asks (37–40, 66, 68–70) if
   the maintainers prefer fewer, larger kit issues.
3. **Naming Strata** in the bodies (S5 question 7). They say "a file manager on Rune".
4. **#173 is fixed but open.** `Tx::focus` landed in `22aa8cde`. Recommendation: leave #173
   to its author; adopt `Tx::focus` at the next bump, which retires the `focus_return` hops and sent Tabs
   in C9, C12, C14b, C15, C17, C18, C19 and C20c, and lets C1e restore focus after Back and Up.
5. **Entries checked only at the pin** (12 entries, marked "at the pin" above): re-run their
   probes at head before filing, or file with the pin's evidence and say so (the bodies do).
6. **Entry 24's fake portal** is pasted in full in a `<details>` block (about 100 lines).
   Alternatively, attach it as a gist.
7. **Scratch crates.** `strata-spikes/rune-packet/` (`repros`, `repros-head`, the upstream
   worktree `rune-head`) stays for re-runs; delete it once the packet is filed.

## Dropped

| Draft | Why |
| --- | --- |
| U4 | Withdrawn by S5: headless file drops do carry held modifiers. |
| U-C1b-1 | Fixed in `4805095` (#240), before the pin: scripts hold modifiers with `{"hold": "ctrl"}`. |
| U-C1e-6 | Fixed in `c650fe0c` (#371), past the pin: `Tx::clock` and `Tx::wake_at`. |
| #173 comment (C20c); C15's `Tx::focus(key, visible)`; U-C1e-4's and U-C21-4's programmatic-focus halves | Fixed in `22aa8cde` (#384), past the pin: `Tx::focus(key, ring)` focuses the first shown element with the key, ring or not, and `Tx::return_focus` hands focus back as a menu closes. #173 is still open upstream. U-C21-4's first-arrow case also gets `Tx::by_keys` (`3847cf0a`). |
| U-C17-2, U-C24-6, and U-C20b-2's `Tx` half | Fixed in `22aa8cde`: `El::modal_tx` gives the dismissal the change's `Tx`. The dismissal's reason (U-C20b-2's other half) is entry [32](32-modal-dismiss-and-focus.md). |
| U23 | Fixed in `3847cf0a`, past the pin: `Tx::by_keys` says a key brought the change, including Return or Space on a control. |
| U-C13-4 | Duplicate of [#387](https://github.com/anomalyco/rune/issues/387) (`aspect_fit` with `max_w` overflows: `fit_room` turns `max_w` into the room width), filed upstream 2026-11-09 with its own repro. |
| C15: keyboard focus mode can't be cleared | Likely covered by `Tx::focus(key, ring: false)` (`22aa8cde`; `el/tx.rs:361` at `2a44156`): a chained dialog can take focus without the ring. Confirm when C15 adopts `Tx::focus`; file nothing until then. |
| Entry 65 (U-C2-3) | Dropped at `2a44156`: already in Rune at the pin. `El::drawn_rect_into` writes the drawn rect on every frame of a layout spring; C2 can adopt it. |
| Entry 80 (R3, #350 comment) | Dropped at `2a44156`: duplicate of [#350](https://github.com/anomalyco/rune/issues/350), whose thread already asks for it; read-only text already has `El::inline_icon` and `El::pill`. |
| U-X1-1 | Doesn't apply: scripts can deactivate a window (`{"t": .., "focus": false}` reaches the root's `on_window_focus`, C6's probe P2b). Its real gap is entry [91](91-window-focus-root.md). |
| U-C17-1 | Never a draft: C17 reserved the id, then found the hidden live region works. |

Adopting the fixes above (and dropping the workarounds they retire) waits for the next Rune
bump, which is its own change.

## Draft-to-entry mapping

Every draft found, where it was written, and its packet entry. Module paths are under
`crates/strata-components/src/` unless they name another crate.

| Draft | Written in | Packet |
| --- | --- | --- |
| U1 (S2 R1) | `docs/decisions/S5-upstream.md` | [05] |
| U2 (S2 R2) | `docs/decisions/S5-upstream.md` | [06] |
| U3 (W1) | `docs/decisions/S5-upstream.md` | [14] |
| U4 | `docs/decisions/S5-upstream.md` | drop: withdrawn |
| U5 (S2 R3, P2's amendment) | `docs/decisions/S5-upstream.md` | [07] |
| U6 (S1 R1) | `docs/decisions/S5-upstream.md` | [11], [12] |
| U7 (S1 R2) | `docs/decisions/S5-upstream.md` | [12] |
| U8 (S1 R3) | `docs/decisions/S5-upstream.md` | [13] |
| U9 (S1 R7, S2 R4, S3) | `docs/decisions/S5-upstream.md` | [01] |
| U10 (S2 R5) | `docs/decisions/S5-upstream.md` | [08] |
| U11 (S3) and P4's amendments | `docs/decisions/S5-upstream.md` | [23] |
| U12 (S1 R5) | `docs/decisions/S5-upstream.md` | [24] |
| U13 (S1 R4) | `docs/decisions/S5-upstream.md` | [25] |
| U14 (S1 R6) | `docs/decisions/S5-upstream.md` | [04] |
| U15 | `docs/decisions/S5-upstream.md` | [17] |
| U16 | `docs/decisions/S5-upstream.md` | [84] |
| U17 (W1) | `docs/decisions/S5-upstream.md` | [57] |
| U-P1-1, U-P1-2 | `strata-platform/src/dnd/mod.rs`; S5 | [15] |
| U-X2-1 | `docs/decisions/S5-upstream.md` | [15] |
| U-P6-1 | `strata-platform/src/icon_theme`; S5 | [22] |
| U-P6-2 | `strata-platform/src/icon_theme`; S5 | [93] |
| U-P7-1 | `strata-platform/src/media`; S5 | [26] |
| U-P7-2 | `docs/decisions/S5-upstream.md` | [92] |
| U-P8-1 | `strata-platform/src/single_instance`; S5 | [58] |
| U18, U19, U20, U21, U22 | `sidebar/mod.rs` | [40] |
| U23 | `sidebar/mod.rs` | drop: fixed in `3847cf0a` |
| U-C1b-1 | `location_bar/mod.rs`; S5 | drop: fixed in `4805095` |
| U-C1c-1 | `collection/keyboard/mod.rs` | [44] |
| U-C1e-1 | `collection/element/route.rs` | [63] |
| U-C1e-4, focus-change half | `collection/element/mod.rs` | [09] |
| U-C1e-4, programmatic-focus half | `collection/element/mod.rs` | drop: fixed in `22aa8cde` |
| U-C1e-5 | gallery `scenes/c1.rs` | [44] |
| U-C1e-6 | `collection/element/mod.rs` | drop: fixed in `c650fe0c` |
| U-C1e-7 | `collection/element/seam.rs` | [64] |
| U-C1e-8 | `collection/element/mod.rs` | [41] |
| U-C1f-1 | `collection/element/mod.rs` | [18] |
| U-C1f-2, U-C1f-3 | `collection/element/mod.rs` | [16] |
| U-C1f-4 | `collection/element/mod.rs` | [45] |
| U-C1f-5 | `collection/element/mod.rs` | [62] |
| U-C1f-6 | `collection/element/mod.rs` | [19] |
| U-C2-1 | `columns/mod.rs` | [14] |
| U-C2-3 | `columns/mod.rs` | [65] |
| U-C2-4 | `columns/mod.rs` | [28] |
| U-C3-1 | `icons_grid/mod.rs` | [60] |
| U-C3-2 | `icons_grid/mod.rs` | [61] |
| U-C3-3 | `icons_grid/mod.rs` | [10] |
| U-C3-4 | `icons_grid/mod.rs` | [16] |
| U-C5-1 | `rename/mod.rs` | [03] |
| U-C5-2 | `rename/mod.rs` | [48] |
| U-C6-1 | `location_bar/mod.rs` | [21] |
| U-C6-2 | `location_bar/mod.rs` | [42] |
| U-C6-3 | `location_bar/mod.rs` | [09] |
| U-C6-4 | `location_bar/mod.rs` | [38] |
| U-C6-5 | `location_bar/mod.rs` | [10] |
| U-C6-6 | `location_bar/mod.rs` | [91] |
| U-C6-7 | `location_bar/mod.rs` | [44] |
| U-C6-8 | `location_bar/mod.rs` | [38] |
| U-C7-1, U-C7-3, U-C7-4, U-C7-5, U-C7-7 | `tabs/mod.rs` | [39] |
| U-C7-2 | `tabs/mod.rs` | [10], [43] |
| U-C7-6 | `tabs/mod.rs` | [44] |
| U-C10b-1, U-C10b-5 | `code_view/mod.rs` | [33] |
| U-C10b-2 | `code_view/mod.rs` | [34] |
| U-C10b-3 | `code_view/mod.rs` | [42] |
| U-C10b-4 | `code_view/mod.rs` | [81] |
| U-C10b-6, U-C10b-7 | `code_view/mod.rs` | [35] |
| U-C10b-8 | `code_view/mod.rs` | [02] |
| R1, R5, R6 | `document_view/mod.rs` | [35] |
| R2 | `document_view/mod.rs` | [34] |
| R3 | `document_view/mod.rs` | [80] |
| R4 | `document_view/mod.rs` | [36] |
| R7 | `document_view/mod.rs` | [17] |
| U-C11b-1 | `sheet_grid/mod.rs` | [16] |
| U-C11b-2 | `sheet_grid/mod.rs` | [37] |
| U-C11b-3 | `sheet_grid/mod.rs` | [17] |
| U-C12-1 | `pdf_view/mod.rs` | [79] |
| U-C12-2 | `pdf_view/mod.rs` | [18] |
| U-C12-3 | `pdf_view/mod.rs` | [29] |
| U-C12-4 | `pdf_view/mod.rs` | [78] |
| U-C13-1, U-C13-3 | `media_player/mod.rs` | [31] |
| U-C13-2 | `media_player/mod.rs` | [53] |
| U-C13-4 | `media_player/mod.rs` | drop: duplicate of #387 |
| U-C16-1 | `progress_dock/mod.rs` | [68] |
| U-C16-2, U-C16-3 | `progress_dock/mod.rs` | [31] |
| U-C16-4 | `progress_dock/mod.rs` | [09] |
| U-C17-1 | `search_palette/mod.rs` | drop: never used |
| U-C17-2 | `search_palette/mod.rs` | drop: fixed in `22aa8cde` |
| U-C17-3 | `search_palette/mod.rs` | [59] |
| U-C17-4 | `search_palette/mod.rs` | [29] |
| U-C18-1 | `filter_bar/mod.rs` | [56] |
| U-C19-1 | `footer/mod.rs` | [43] |
| U-C19-2 | `footer/mod.rs` | [47] |
| U-C20a-1, rune-kit NumberField proposal | `controls/number_field/mod.rs` | [21] |
| #149 comment | `controls/slider/mod.rs` | [54] |
| U-C20a follow-up: a declined notch is dropped | `docs/work/STATUS.md` (C20a) | [10] |
| U-C20b-1 | `controls/color_picker/mod.rs` | [69] |
| U-C20b-2, reason half | `controls/color_picker/mod.rs` | [32] |
| U-C20b-2, `Tx` half | `controls/color_picker/mod.rs` | drop: fixed in `22aa8cde` |
| U-C20b-3 | `controls/color_picker/mod.rs` | [42] |
| U-C20c-1 | `controls/tabbed_form/mod.rs` | [70] |
| U-C20c-2 | `controls/tabbed_form/mod.rs` | [71] |
| U-C20c-3 | `controls/settings_search/mod.rs` | [73] |
| U-C20c-4 | `controls/tabbed_form/mod.rs` | [72] |
| #173 comment (C20c) | `controls/tabbed_form/mod.rs` | drop: fixed in `22aa8cde` |
| U-C20d-1 | `controls/icon_picker/mod.rs` | [82] |
| #102 comment (C20d) | `controls/icon_picker/mod.rs` | [83] |
| U-C21-1 | `context_menu/mod.rs` | [04] |
| U-C21-2 | `context_menu/mod.rs` | [03] |
| U-C21-3 | `context_menu/mod.rs` | [96] |
| U-C21-4, keyboard and hover half | `context_menu/mod.rs` | [38] |
| U-C21-4, #173 half (first arrow) | `context_menu/mod.rs` | drop: fixed in `22aa8cde`, `3847cf0a` |
| U-C21-5 | `context_menu/mod.rs` | [38] |
| U-C21-6 | `context_menu/mod.rs` | [49] |
| U-C22-1 | `folder_peek/mod.rs` | [30] |
| U-C22-2 | `folder_peek/mod.rs` | [66] |
| U-C22-3 | `folder_peek/mod.rs` | [67] |
| U-C23-1, U-C23-2 | `motion/mod.rs` | [74] |
| U-C23-3 | `motion/mod.rs` | [75] |
| U-C23-4 | `motion/mod.rs` | [76] |
| U-C24-3, U-C24-7 | `scroll/mod.rs` | [10] |
| U-C24-4 | `scroll/mod.rs` | [02] |
| U-C24-5 | `scroll/mod.rs` | [77] |
| U-C24-6 | `scroll/mod.rs` | drop: fixed in `22aa8cde` |
| U-C24-8 | `scroll/mod.rs` | [27] |
| U-C24b-1 | `scroll/mod.rs` | [50] |
| U-C24b-2 | `scroll/mod.rs` | [10] |
| U-C24b-3 | `scroll/mod.rs` | [55] |
| U-C24b-4 | `scroll/mod.rs` | [32] |
| R-C24-1 | `scroll/mod.rs` | [51] |
| R-C24-2 (#365 comment) | `scroll/mod.rs` | [52] |
| U-C25-1 | `thumbnails/mod.rs` | [20] |
| U-C25-2 | `thumbnails/mod.rs` | [95] |
| U-C25-3 | `thumbnails/mod.rs` | [44] |
| R-K1, R-K2, R-K3, R-K5, R-K6, R-K7 | `list_table/mod.rs` | [37] |
| R-K4 | `list_table/mod.rs` | [37], [45] |
| R-C4-1 | `list_table/mod.rs` | [46] |
| R-C4-2 | `list_table/mod.rs` | [02] |
| R-C4-3 | `list_table/mod.rs` | [16] |
| R-C4-4 | `list_table/mod.rs` | [31] |
| R-C14b-1 | `viewers/mod.rs` | [42] |
| R-C14b-2 | `viewers/mod.rs` | [77] |
| R-C14b-3 | `viewers/mod.rs` | [03] |
| R-K6-1 | `docs/testing.md` | [85] |
| U-K3-3 | `strata-kit/icons/README.md` | [94] |
| U-X1-1 | `header/mod.rs` | drop: doesn't apply |
| U-X1-2 | `header/mod.rs` | [44] |
| C14a: Ctrl+wheel turned into a pinch zooms the wrong way | `viewers/viewport/mod.rs` | [86] |
| C14a: no notification on a scale-only change | `viewers/viewport/mod.rs` | [87] |
| C15: `Tx::focus(key, visible)` | `preview_drawer/mod.rs`, `dialog/mod.rs` | drop: fixed in `22aa8cde` |
| C15: keyboard focus mode can't be cleared | `dialog/mod.rs` | drop: likely covered by `22aa8cde` |
| C15: a long frame's step (33 ms) | `dialog/mod.rs` | [89] |
| C15: arrows between body controls | `dialog/mod.rs` | [90] |
| C15: modal restore to a disabled opener | `dialog/mod.rs` | [88] |
| C9: a styled focus-within | `preview_drawer/mod.rs` | [09] |

[01]: 01-display-handle.md
[02]: 02-held-drag-no-up.md
[03]: 03-kit-text-field-handlers.md
[04]: 04-linux-popup-menus.md
[05]: 05-drag-out-linux.md
[06]: 06-comment-424-typed-drops.md
[07]: 07-clipboard.md
[08]: 08-end-pointer-capture.md
[09]: 09-comment-106-focus-change.md
[10]: 10-wheel-routing.md
[11]: 11-comment-309-edge-resize.md
[12]: 12-comment-310-minimise-maximised.md
[13]: 13-comment-313-window-menu.md
[14]: 14-file-drop-hooks.md
[15]: 15-element-at-drop-routing.md
[16]: 16-virtual-a11y-positions.md
[17]: 17-virtualize-horizontal.md
[18]: 18-virtual-row-heights.md
[19]: 19-live-text-fast-path.md
[20]: 20-pixels-mipmaps.md
[21]: 21-text-input-roles.md
[22]: 22-comment-21-icon-theme.md
[23]: 23-transient-for.md
[24]: 24-settings-portal.md
[25]: 25-double-click-interval.md
[26]: 26-pcm-output.md
[27]: 27-scroll-request-view-runs.md
[28]: 28-scoped-reveal-axis.md
[29]: 29-virtual-rows-reveal.md
[30]: 30-comment-265-hover-under-overlay.md
[31]: 31-live-bindings.md
[32]: 32-modal-dismiss-and-focus.md
[33]: 33-text-reshape-cost.md
[34]: 34-read-only-caret.md
[35]: 35-selection-across-texts.md
[36]: 36-selection-copy-cap.md
[37]: 37-kit-table.md
[38]: 38-kit-menu.md
[39]: 39-kit-tab-strip.md
[40]: 40-kit-sidebar.md
[41]: 41-a11y-option-action.md
[42]: 42-a11y-roles-states.md
[43]: 43-scroll-by.md
[44]: 44-headless-tooling.md
[45]: 45-sticky-sections.md
[46]: 46-press-after-drag.md
[47]: 47-alt-letter.md
[48]: 48-press-outside.md
[49]: 49-secondary-press-scrim.md
[50]: 50-scroll-offset-chain.md
[51]: 51-kit-scroll-bar-keys.md
[52]: 52-comment-365-scroll-bar.md
[53]: 53-range-page.md
[54]: 54-comment-149-slider.md
[55]: 55-middle-click-paste.md
[56]: 56-keybind-case.md
[57]: 57-fuzzy-matcher.md
[58]: 58-x11-activation-token.md
[59]: 59-comment-364-anchor-at-top.md
[60]: 60-two-line-middle-elision.md
[61]: 61-tooltip-timing.md
[62]: 62-visible-rows-late.md
[63]: 63-virtualize-drag-y.md
[64]: 64-stop-glide.md
[65]: 65-on-layout-drawn.md
[66]: 66-kit-hover-card.md
[67]: 67-anchor-boundary.md
[68]: 68-kit-toast-region.md
[69]: 69-kit-popover-modal.md
[70]: 70-comment-397-segmented-tabs.md
[71]: 71-offstage.md
[72]: 72-instant-removal-offset.md
[73]: 73-sequence-reduced.md
[74]: 74-comment-505-matched-flights.md
[75]: 75-capture-pixels.md
[76]: 76-tx-layout.md
[77]: 77-cursors.md
[78]: 78-cursor-innermost.md
[79]: 79-positioned-text.md
[80]: 80-comment-350-inline-images.md
[81]: 81-field-no-wrap.md
[82]: 82-glyph-coverage.md
[83]: 83-comment-102-emoji-outlines.md
[84]: 84-comment-114-font-list.md
[85]: 85-measurer-race.md
[86]: 86-ctrl-wheel-pinch.md
[87]: 87-scale-change.md
[88]: 88-modal-restore-disabled.md
[89]: 89-presence-long-frame.md
[90]: 90-arrow-focus-by-position.md
[91]: 91-window-focus-root.md
[92]: 92-pixels-replace.md
[93]: 93-pixels-tint.md
[94]: 94-icon-rasterizer-attributes.md
[95]: 95-pixels-last-drawn.md
[96]: 96-comment-354-undo-state.md
