//! IconsGrid (C3): the Icons view, a grid of thumbnail tiles on the collection element (C1e).
//!
//! Work package: C3, `docs/work/phase-3-views/C3-icons-grid.md`.
//!
//! Rune virtualizes vertical rows only (Rune draft U15, S5), so the grid packs N tiles into each
//! of C1e's virtual lines ([`GridLines`]), N from the viewport width and the tile size, and lays
//! them out by index ([`IconsGeometry`]) so selection, keyboard and marquee (C1b, C1c, C1d) work
//! over every tile, mounted or not. Each tile is a thumbnail frame, a two-line name cut in the
//! middle, and a details line (Strata's `icons_cell.rs`).
//!
//! ```ignore
//! let grid = IconsGrid::new("files", base_collection, |s: &App| IconsParts {
//!     source: &s.listing, collection: &s.coll, grid: &s.icons, size: s.icon_size(),
//!     listing: s.version, look: &s.look, thumbs: Some((&s.thumbs, s.viewer)),
//! }, |s: &mut App| &mut s.icons)
//! .thumbs_mut(|s: &mut App| Some((&mut s.thumbs, s.viewer, &s.listing as &dyn EntrySource)))
//! .details(|s, rcx| s.meta_version(rcx.entry.id), |s, rcx| s.details(rcx.entry.id))
//! .on_size(|s, request, _| s.set_icon_size(request));      // the host keeps the size
//!
//! let el = grid.scale(cx.env().scale).build(s);              // the view
//! grid.collection(s).tick(s, tx); grid.poll(s, tx);          // the poll
//! // window keys:  if let Some(c) = keys.window_command(ev, tx) { grid.apply_size(s, c, tx) }
//! // on_intent:    if let Some(c) = size_command(&intent) { grid.apply_size(s, c, tx) }
//! ```
//!
//! # What the grid adds to the collection
//!
//! [`IconsGrid::collection`] sets the base collection's roles ([`Roles::GRID_CELLS`]), bare
//! items (C1e draws no row chrome; the tile lights its own frame and name), the row height
//! (a tile's), the [`GridLines`] packing with the grid's side padding, the icon, label and
//! content slots, the hit regions, `on_visible` (the thumbnail sync, then the host's
//! [`IconsGrid::on_visible`]) and `on_wheel` (Ctrl+Shift+wheel). The host sets none of those on
//! its own builder; everything else is the host's as for any view: the clock, controllers and
//! their settings, intents, drops, rename, marks and the description.
//!
//! # Layout
//!
//! [`TileMetrics`] works a tile out from the look and the tile size, after Strata's
//! `icons_cell.rs` and `.icons-card*` rules: the slot (the size, held to the look's ladder
//! `Look::tiles`, 32 to 256), a frame 6 pt around it, a gap, the name's reserved lines and the
//! details line, in a padded card with a margin; the spacing follows Content density
//! (`slots::icons`, K2's `icons_spacing`). The thumbnail size doesn't follow the text size; the
//! caption does, and widens the narrowest cell (12 average characters, as Strata's caption).
//! A row holds as many cells as fit, at most 20 (16 from Airy); they share the row's width, as
//! GTK's GridView shares it. Everything the host streams in (details, thumbnails) lands in
//! reserved space, so nothing moves.
//!
//! A width that changes the count a row reflows (C1e), keeping the cursor tile in view; a new
//! size reflows through [`IconsGrid::apply_size`] or [`IconsGrid::size_changed`] in the same
//! frame, and a cursor tile that showed stays in view. A new width that keeps the count builds
//! once to cut names to the new cell width.
//!
//! # Names
//!
//! A name wraps over at most two lines, breaking inside a word too long for a line (Strata's
//! `WordChar`), centred and top-aligned under the frame. A longer one is cut in the middle,
//! keeping both ends (`2026-10-07-r…tail.tar.gz`), so the part that tells names apart stays.
//! Rune can't cut two lines in the middle (U-C3-1), so the grid measures the cut itself and
//! caches it, bounded ([`Cut`]). A cut name carries the whole name as a tooltip (`El::help`)
//! over the frame, the name's text box and the details text, none while the tile renames.
//!
//! # Size
//!
//! The size is the host's, per remembered folder or the default (Strata's policy,
//! `browser/view-modes/folder-views`). The grid steps it along the ladder and hands the host a
//! [`SizeRequest`]: from Ctrl+Alt+plus, Ctrl+Alt+=, Ctrl+Alt+minus and Ctrl+Alt+0 through
//! [`SizeKeys::window_command`] (the host's window keys) or the focused grid's
//! [`SizeKeyed`] controller (10xer's plain +, = and − too); and from Ctrl+Shift+wheel over the
//! grid ([`WheelSteps`], Strata's `WheelZoom`: a notch a step, 10 pt of precise scrolling a
//! step). A reset asks the host, which knows what the folder follows. The size notice, the
//! popover's slider (C20a) and its Make default and Reset actions are the host's.
//!
//! # Thumbnails (C25)
//!
//! The answers to C25's "For C3's review" list:
//!
//! - **The part.** The frame is keyed `ICON` (so C23's flights find it; they fly the frame,
//!   its selection fill included) and holds the placeholder icon and the image over the slot,
//!   drawn as C25's `picture` draws them (fitted, letterboxed, the icon faded through the
//!   handle's opacity) without its two wrappers; the name sits under the slot, not the image.
//! - **The step.** `Step::for_tile(look, size, scale)`, the scale from the view
//!   ([`IconsGrid::scale`]) and recorded for handlers. A size change calls
//!   `ThumbnailStore::set_step` from its handler; [`IconsGrid::poll`] catches a size or scale
//!   change made elsewhere within a frame. A display scale change alone still waits for a
//!   build (C14a's draft).
//! - **Zoom.** Landed images stay drawn (their icon hidden) until the new step's replace them:
//!   gallery test `a_size_step_keeps_landed_thumbnails_drawn…`.
//! - **Protection after a scroll, re-step order:** C25's, unchanged.
//! - **Tiles over 128 pt at scale 2** draw up from the look's largest class, 256 px: a
//!   deviation (owner ruling: no 512 px class for now).
//! - **Sync.** `on_visible` syncs with `Viewport::per_line` from the grid's own count and the
//!   host's listing version; the host calls `resync` after a listing change.
//! - **Caption sizes** are the host's details ([`IconsGrid::details`]).
//! - **B1** holds at the grid: runtime test `a_4k_grid_at_scale_2_settles…`.
//! - **The version** is the store's `version(key)` and the build's step, never the key's
//!   state: runtime test `thumbnails_landing_run_no_view…`.
//! - **The worker** in the host's poll, returning false: the gallery page's.
//! - **Placeholder icons** stay mounted under a held image; the perf run's zoom covers their
//!   rasterizing at every size.
//! - **Memory at 256 px**: a held image is 256 KiB, as C25 says; a 256 pt grid shows few.
//! - **Recycled rows:** Rune unmounts rows rather than recycling them; a re-keying reflow
//!   rebuilds the tiles on screen, and their handles persist, so nothing reloads.
//!
//! # What assistive technology hears
//!
//! The collection is a grid named by the host, of rows (the lines), of grid cells named by
//! their file names, each with its place in the listing and its selected state; the cursor's
//! cell is the grid's active descendant, and the status line counts a selection (C1e). The
//! frame, name and details are hidden: a cell's description is the host's `Describe`
//! (Strata's entry kind, as the gallery shows). No column index or count (Rune draft
//! R-C4-3).
//!
//! A line row reads "row N of <entry count>" (`line-9` in `c3-default` is position 2 of 40,
//! though its group holds 5): each cell's `position_in_set(index, len)` gives its container
//! the cells' set size (Rune `a11y/tree.rs:340`), and that overrides the row's own size of
//! set (`:398`, after the virtual list's at `:395`). No fix here short of dropping the cells'
//! positions, which say more (draft U-C3-4).
//!
//! # Decisions and deviations
//!
//! - **Down and Up with no tile under or over** (a short last row) move as GTK's GridView does
//!   (`gtk_grid_view_move_focus_along`): Down into a shorter last row lands on its last tile;
//!   from the last row there is nothing, an edge. GTK's own quirk, Down on the last row of a
//!   grid whose rows are all full moving to the last tile (its `n_items / n_columns` test), is
//!   not followed: there it is an edge. Left and Right cross row ends (C1c's `wrap_rows`); Left
//!   on the first tile and Up from the first row are edges the host hears (`Intent::Edge`).
//! - **Tooltips are Rune's `El::help`** (owner ruling): the delay is the window's, set by the
//!   host with `Cx::help_look` (the gallery sets Strata's 900 ms); the grid sets none. A press
//!   while it waits cancels it until the pointer leaves the element; a press on an open one
//!   keeps it (Rune 0cde17ec), where Strata hides it. Moving inside an element doesn't restart
//!   the wait (Strata restarts after 4 px). Sweeping across tiles shows
//!   none, as each new element starts the wait again. Draft U-C3-2.
//! - **Ctrl+Shift+wheel right after a plain wheel scrolls** (C3 review round 1): Rune keeps a
//!   wheel gesture a scroll until it has been still 0.1 s (a trackpad gesture until it ends),
//!   and never offers its later deltas to `on_wheel` (draft U-C3-3). Strata, whose zoom
//!   controller runs in GTK's capture phase, steps at once. No clean fix in the grid: claiming
//!   plain wheels to scroll by hand would lose Rune's scrolling.
//! - **Hover lights the frame and the name each on its own**, where Strata lights both from
//!   either: Rune's hover is per element, and a hover group over both would cover the card's
//!   blank space. No hover after keyboard navigation until the pointer moves (C1e).
//! - **No top and bottom grid padding** (owner ruling): Strata pads the grid 5 pt at Airy (0 at
//!   the default Compact); the sides keep it (`Lines::inset`), the top and bottom have none,
//!   since C1e's virtual lines start at the content's top.
//! - **Cut styling and descriptions apply as a tile mounts**, not 25 ms after scrolling stops:
//!   Rune builds a mounting tile whole, and the settle was GTK's cost.
//! - **The size keys are data** ([`SizeKeys`]): Strata's chords; rebinding and routing window
//!   keys are the host's.
//! - **Tiles over 128 pt at scale 2** draw a 256 px image up (above).
//! - **A view switch mid-drag on the scroll bar** (C24b review round 1): Rune tells an element
//!   removed under a drag nothing (Rune draft R-C4-2, in C4's docs), so a thumb drag a view
//!   switch cuts off leaves [`IconsGridState`]'s bar held: if the host keeps the state, the
//!   overlay bar shows when the grid comes back, until a later press on it ends. The grid has
//!   no reset hook for it yet, unlike C4's `ListTableState::reset`; C24's `Bar::release` is
//!   what one would call.
//!
//! # Left to other work packages and hosts
//!
//! - The size's policy: per-folder sizes, the default, Reset's result, saving (`folder-views`),
//!   the size notice (C16), the Thumbnail size popover and its slider (C20a).
//! - The details' text and streaming, and which entries get thumbnails (Strata, C25).
//! - Window-level key routing, 10xer mode, the portal chooser, the sidebar and pane header an
//!   edge moves to, Ctrl+F results (C18) shown in a grid.
//! - The rename editor (C5): the grid places it in the name's reserved lines.
//! - Drag and drop wiring (X2): folder tiles take C1e's drop zones; [`IconsGrid::tile_path`]
//!   finds a tile's path at the current count.
//! - Position restore's focus rules (C1e's Decisions: Rune can't move focus).
//! - Autoscroll on the grid: the host installs C24's `AutoscrollPointer` with
//!   [`IconsGridState::feed`] and passes [`autoscroll_background`], which counts a tile's blank
//!   space as the background. The grid draws C24's listing bar itself (C24b): the pointer over
//!   the grid shows it, and [`IconsGridState::set_bar_mode`] shows it always.
//!
//! # Tests
//!
//! Scripts are in `tests/scripts/c3/` on the gallery page (`scenes/c3.rs`, whose host stands in
//! for Strata's size policy, metadata and thumbnail workers, and rename); "gallery" tests are in
//! `crates/strata-gallery/tests/c3.rs`, "runtime" tests in `tests/runtime.rs`, unit tests in
//! `tests.rs`, `geometry/tests.rs` and `size/tests.rs`. *Partial* covers only the half named.
//!
//! | Bullet (node) | Check |
//! | --- | --- |
//! | icons: Icons shows one grid with no file-type groups | `one-grid` (groups set, no header lines) |
//! | icons: The number of tile columns follows the viewport width and reflows… | `reflow-width` (a window resize; the cursor tile stays mounted); gallery `a_tiles_path_finds_it_at_any_width` |
//! | icons: Each tile has a fixed-size thumbnail slot, a two-line filename, and one details line | PNG goldens (every scene); unit `a_tile_is_as_tall_with_any_name…` |
//! | icons: Filenames start directly below the thumbnail, top-aligned… | PNG goldens `c3-long-names`, `c3-default` |
//! | icons: Hover feedback appears only over a tile's thumbnail and caption… | gallery `hover_lights_the_frame_or_the_name…` |
//! | icons: Folders show "No items" or their item count, audio and video their duration… | *partial*: PNG goldens show the host's text; the text is the host's |
//! | icons: Details stream into visible tiles…; revisiting… shows them from cache | *partial*: runtime `a_details_line_landing_rebuilds_its_tile_alone`; streaming and caching are the host's |
//! | icons: Details arriving do not change tile size or move the grid | runtime `a_details_line_landing…` (the metrics take no details text); PNG goldens `c3-loading` against `c3-default` |
//! | icons: After a fast scroll stops, visible tiles fill their thumbnails and details… | `fling-fill` |
//! | icons: Moving the slider scales existing thumbnails in place without flashing generic icons | gallery `a_size_step_keeps_landed_thumbnails_drawn…` |
//! | icons: With Remember… off, the slider sets the default size, and other windows' Icons panes follow it | *partial*: runtime `two_grids_over_one_size_both_follow_a_step`; the default is the host's |
//! | icons: Resting the pointer about a second (900 ms) on a card whose name is middle-ellipsized… | *partial*: gallery `a_cut_name_rests_into_a_tooltip…`, `the_tooltip_waits_for_the_pages_delay` (the host's window-wide delay) |
//! | icons: A name that fits in its two caption lines shows no tooltip | gallery `a_cut_name_rests_into_a_tooltip…` |
//! | icons: The tooltip appears only over the card's thumbnail or caption, not its padding | gallery `a_cut_name_rests_into_a_tooltip…` |
//! | icons: Over the caption, only the name's text box and the details text count… | gallery `a_cut_name_rests_into_a_tooltip…`, `a_rest_beside_a_cut_names_box_or_its_details_shows_no_tooltip` |
//! | icons: Moving more than 4 px or leaving the card restarts the wait. Sweeping… shows no tooltips | *partial*: Rune restarts on leaving an element (deviation above) |
//! | icons: Pressing a mouse button cancels the wait until the pointer moves again… | *partial*: Rune's press hides it until the pointer leaves (deviation above) |
//! | icons: After a rename commits, the tooltip shows the new name; while the rename field is open, none appears | gallery `a_tile_being_renamed_has_no_tooltip…` |
//! | icons: Keyboard focus on a card shows no name tooltip | gallery `a_key_moving_onto_a_cut_tile_shows_no_tooltip` |
//! | icons: Tiles bound mid-scroll set their name and request thumbnail and details at once; cut styling and accessible descriptions refresh once scrolling settles | *partial*: all at mount (deviation above); `fling-fill` |
//! | thumbnail-size: The size steps through 32, 36, … 256 px | `size-ladder`; `size-ends` |
//! | thumbnail-size: In Icons, Ctrl+Alt++ or Ctrl+Alt+= moves the size one step larger | `size-ladder` (focused), `size-window-keys` (the window's keys) |
//! | thumbnail-size: In Icons, Ctrl+Alt+− moves the size one step smaller… | `size-ladder`, `size-window-keys` |
//! | thumbnail-size: From a size between two steps, such as 100 px… | `size-between-up`, `size-between-down` |
//! | thumbnail-size: A step up at 256 px stays at 256, and a step down at 32 px stays at 32 | `size-top`, `size-ends` |
//! | thumbnail-size: In 10xer mode, with the Icons listing focused, plain +, =, or − steps the size… | `size-tenxer` |
//! | thumbnail-size: The Ctrl+Alt step keys come from `[keymap.default]` in both modes… | *partial*: unit `a_rebound_default_chord_applies_in_tenxer_too`; the keymap file is the host's |
//! | thumbnail-size: With Remember sort and icon size per folder off, Ctrl+Alt+0 returns the size… to 64 px | *partial*: `size-reset` (the grid sends Reset; the gallery host's policy lands on 64) |
//! | thumbnail-size, by Ctrl+Shift+wheel (lgse/strata#1721, no bullet of its own) | `size-wheel` (named for the ladder bullet); unit `a_notch_is_one_step…`; C1e hooks `the_viewport_hears_wheels_first…` (a taken wheel doesn't scroll) |
//! | keyboard-navigation: In Icons, arrows move one tile in that direction, using rows as laid out | `keyboard-2d`; unit `up_and_down_move_by_rows…` |
//! | keyboard-navigation: In Icons and List, Up and Down move focus and selection… | `keyboard-2d`, *partial*: Icons |
//! | keyboard-navigation: In Icons, Left on a later row's first tile moves to the previous row's last tile… | `keyboard-wrap` |
//! | keyboard-navigation: In Icons, Left on the first tile focuses the visible sidebar… | *partial*: `keyboard-edge-left` (the edge reaches the host; focusing the sidebar is the host's) |
//! | keyboard-navigation: Up from the first Icons row… focuses the pane header… | *partial*: `keyboard-edges` (the edge reaches the host) |
//! | selection: plain Home, End, Page Up, and Page Down move the range anchor…; Shift+Home after End selects every entry | `keyboard-pages`, `keyboard-home-end`, *partial*: Icons |
//! | scrolling: A page is the number of rows that fit… times the tiles per row | `keyboard-page-down`, `keyboard-pages`; unit `a_page_is_the_rows_that_fit…` |
//! | scrolling: In Icons the tiles per row follow the current width, so… changing thumbnail size changes the next page at once | `page-after-size` |
//! | view-modes: …Shift+Right in Icons then selects only the third and fourth… | `shift-right` (Ctrl held through the click), *partial*: Icons |
//! | view-modes: Icons shows at most 20 columns at Content Compact and at most 16 at Airy | `column-cap-compact`, `column-cap-airy`; unit `rows_hold_more_tiles…` |
//! | view-modes: Long filenames are middle-ellipsized in… Icons captions… | PNG golden `c3-long-names`; unit `a_name_that_fits_is_whole…`, *partial*: Icons |
//! | position-restore: The exact scroll offset returns only… for Icons only at the same grid width… | runtime `a_scroll_offset_comes_back_only_at_its_grids_width` |
//! | marquee: A plain marquee replaces the selection with the entries under the band | `marquee-2d` (two columns by four rows, the short last row's one tile included) |
//! | marquee: A Ctrl-marquee toggles…; A Shift-marquee adds… | `marquee-shift-ctrl` (the modifiers held: Shift over one column by three rows, Ctrl over two columns by three rows) |
//! | pointer-intent: In Icons, the gutter beside the thumbnail inside a card is blank space for marquee | gallery `a_drag_from_a_cards_blank_space_draws_a_band` |
//! | pointer-intent: On an unselected entry, a drag from blank row space beside the filename starts a marquee | gallery `beside_a_short_name_is_blank_space_and_its_drawn_box_is_the_name` |
//! | pointer-intent: On an unselected entry, a drag from the icon or rendered filename starts a file drag | `thumbnail-drag`, `name-drag`; gallery `beside_a_short_name…` (the name box's edge) |
//! | rename: Clicking the name text of the only selected item again, after the double-click interval, opens the editor | `caption-hits` (on the name's box) |
//! | scrolling: Middle-clicking the empty background or gutter of a scrollable… Icons… listing starts autoscroll | `autoscroll-gutter` (a tile's blank space) |
//! | scrolling: Middle-clicking an entry row starts no autoscroll; the row's open-in-new-tab… runs instead | `autoscroll-tile` (a folder's thumbnail) |
//! | scrolling: Scrollbars fade in on hover or scrolling and stay hidden at rest… | gallery `the_grids_bar_shows_as_it_scrolls_and_its_thumb_drags_it` |
//! | scrolling: The vertical scrollbar of an Icons or List listing is 9 px wide, with a 7 px fully rounded thumb on a fainter trough | PNG golden `c3-bar` |
//! | marquee: A press on a… scrollbar… keeps that control's own behavior and starts no marquee | `bar-press` (a trough press jumps, no band) |
//!
//! The thumbnail-size bullets on the popover, the notice, saving and the chooser, and every
//! `folder-views` bullet but the one above, are Strata's policy (N/A here). Perf:
//! `tests/perf.rs`.
//!
//! # Rune drafts (not filed)
//!
//! Issues C3 found in Rune at the pinned rev, written up for the owner to file; each repro runs
//! headless (`rune::Runtime`). Upstream searched (GitHub issues: ellipsis, `max_lines`, wrap,
//! grid, tile, virtual): none of these is asked.
//!
//! - **U-C3-1: two-line middle truncation.** `max_lines(n)` ends its last line with "…", and
//!   `truncate_middle` makes text one line, whichever comes last winning, so a file name over
//!   two lines can't keep its end. Repro (probed at 78d22ed and 5d421fd): in a 120 pt column,
//!   `text(long).max_lines(2)` (two lines, 32.5 pt at 5d421fd), `.max_lines(2).truncate_middle()`
//!   (16.25 pt: one line) and `.truncate_middle().max_lines(2)` (two lines, cut at the end). Ask:
//!   an elision mode that
//!   combines with `max_lines` (`.max_lines(2).elide_middle()`), cutting between graphemes.
//! - **U-C3-2: per-element tooltip timing.** `El::help`'s delay is the window's
//!   (`HelpLook::delay`), a press keeps an open tooltip (one still waiting is cancelled until the
//!   pointer leaves), and a move inside the element never restarts the wait. Strata's Icons names
//!   wait 900 ms where its buttons wait the toolkit's, restart after 4 px of movement, and show
//!   again after a press once the pointer moves. Repro: two elements with `.help`, one wanting
//!   0.9 s; there is no builder for it, and `Runtime::help_tooltip` shows both at the window's
//!   delay. Ask: `El::help_delay(secs)`, and a restart distance.
//! - **U-C3-3: modified wheels over a scroll view's content.** `on_wheel` is offered from the
//!   element under the pointer up to the nearest scroll view, so an ancestor of a scroll view
//!   never hears a wheel over its content (C1e's `Collection::on_wheel` sits on the viewport
//!   for it). Repro (probed at 78d22ed and 5d421fd): `col().on_wheel(..)` around a
//!   `.scroll(..)` viewport; a Ctrl+Shift wheel over the content reaches the viewport's handler
//!   (2 of 2) and the outer one 0 of 2. Also: a wheel gesture that began as a scroll stays one
//!   (`offer_wheel`: a gesture with an axis and no handler isn't offered again), and a wheel
//!   continues the gesture until it has been still 0.1 s (`WHEEL_IDLE`; a trackpad's until it
//!   ends, or 1 s), so Ctrl+Shift+wheel within 0.1 s of a plain wheel scrolls instead of
//!   reaching the handler. Repro: five plain notches over the grid, then at once a Ctrl+Shift
//!   notch: the view scrolls and the size stays; 1.5 s later the same notch steps it (C3 review
//!   round 1). Ask: a capture-phase `on_wheel`, or let ancestors claim a modified wheel, as
//!   GTK's capture phase does; and offer a delta whose modifiers changed as a new gesture.
//!
//! - **U-C3-4: an item's set size becomes its container's.** `El::position_in_set(i, n)` on
//!   an item sets `n` as the size of set of the element that holds it (`a11y/tree.rs:340`,
//!   applied at `:398`), overriding the container's own: its place among its siblings (the
//!   virtual list's `:395`, or the app's). Repro (probed at 5d421fd): in a `.scroll(..)`
//!   viewport, `col().growth(..).virtual_rows(5, ..)` of `row().role("row")`s, each holding 8
//!   `text(..).role("gridcell").position_in_set(n + 1, 40)`; `accessibility_json()` gives the
//!   list `setSize: 5`, every row `position: k, setSize: 40`, and the cells no size, so a
//!   screen reader says "row 2 of 40". Searched upstream for `position_in_set`, `size_of_set`, `set_size` and
//!   "position in set": nothing filed. Ask: keep a container's size of set to its own set, and
//!   give the items' size to the container as a count of its children (AccessKit's row or
//!   column count) where its role has one.
//!
//! A fourth probe found nothing to file: a memo whose line is re-keyed by a reflow is reused,
//! not rebuilt; tiles a reflow builds again are the ones the mounted window newly takes in.
//! Already drafted and met here: U15 (grid virtualization), R-C4-3, U-C1f-5, U-C25-1 to 3.
//!

#![warn(missing_docs)]

mod caption;
mod geometry;
mod metrics;
mod regions;
mod size;
mod state;
mod tile;

#[cfg(test)]
mod tests;

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use rune::{El, Tx, WheelEvent, col};
use strata_kit::look::Look;

pub use caption::Cut;
pub use geometry::{GridLines, IconsGeometry};
pub use metrics::{COLUMN_SPACING, MIN_CARD, TileMetrics};
pub use size::{
    PRECISE_STEP, SizeCommand, SizeKeyed, SizeKeys, SizeRequest, WheelSteps, size_command,
};
pub use state::IconsGridState;
pub use tile::DETAILS;

use crate::collection::element::{
    Collection, CollectionState, Ctx, PointerInput, Region, Roles, RowCx, RowState, entry_path,
    viewport_path,
};
use crate::contract::{EntryId, EntrySource, Point};
use crate::scroll::{AutoscrollSettings, BarLook};
use crate::thumbnails::{Step, ThumbnailStore, ViewerId, Viewport};
use tile::Built;

/// What the grid reads from the host's state, in one accessor.
///
/// Used by: hosts.
pub struct IconsParts<'a> {
    /// The listing, as the collection shows it.
    pub source: &'a dyn EntrySource,
    /// The collection's state.
    pub collection: &'a CollectionState,
    /// The grid's own state.
    pub grid: &'a IconsGridState,
    /// The tile size in points: the host's, per folder or the default (Strata's policy). It
    /// is held to the look's ladder (`Look::tiles`).
    pub size: f32,
    /// The listing version the collection is built with (C1e's), for the thumbnail store.
    pub listing: u64,
    /// The look.
    pub look: &'a Look,
    /// The thumbnail store and this grid's viewer in it, when thumbnails show (C25).
    pub thumbs: Option<(&'a ThumbnailStore, ViewerId)>,
}

type PartsFn<S> = Rc<dyn for<'a> Fn(&'a S) -> IconsParts<'a>>;
type GridMutFn<S> = Rc<dyn Fn(&mut S) -> &mut IconsGridState>;
type ThumbsMutFn<S> = Rc<
    dyn for<'a> Fn(&'a mut S) -> Option<(&'a mut ThumbnailStore, ViewerId, &'a dyn EntrySource)>,
>;
type BaseFn<S> = Rc<dyn Fn(&S) -> Collection<S>>;
type VersionFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> u64>;
type DetailsFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> Option<Arc<str>>>;
type SizeFn<S> = Rc<dyn Fn(&mut S, SizeRequest, &mut Tx)>;
type VisibleFn<S> = Rc<dyn Fn(&mut S, Range<usize>) -> bool>;

/// What the grid was built with, shared by its parts' handlers.
pub(crate) struct Shared<S> {
    key: Rc<str>,
    base: BaseFn<S>,
    parts: PartsFn<S>,
    grid_mut: GridMutFn<S>,
    thumbs_mut: Option<ThumbsMutFn<S>>,
    details: Option<(VersionFn<S>, DetailsFn<S>)>,
    lines: usize,
    on_size: Option<SizeFn<S>>,
    on_visible: Option<VisibleFn<S>>,
    scale: Option<f32>,
}

impl<S> Clone for Shared<S> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            base: self.base.clone(),
            parts: self.parts.clone(),
            grid_mut: self.grid_mut.clone(),
            thumbs_mut: self.thumbs_mut.clone(),
            details: self.details.clone(),
            lines: self.lines,
            on_size: self.on_size.clone(),
            on_visible: self.on_visible.clone(),
            scale: self.scale,
        }
    }
}

impl<S: 'static> Shared<S> {
    fn metrics(&self, s: &S) -> TileMetrics {
        let p = (self.parts)(s);
        TileMetrics::new(p.look, p.size, self.lines, self.details.is_some())
    }

    fn scale(&self, s: &S) -> f32 {
        self.scale.unwrap_or_else(|| (self.parts)(s).grid.scale())
    }

    fn step(&self, s: &S, m: &TileMetrics) -> Step {
        Step::for_tile((self.parts)(s).look, m.slot, self.scale(s))
    }

    fn built(&self, s: &S) -> Built {
        let m = self.metrics(s);
        let w = (self.parts)(s).collection.viewport().w;
        let per = m.per_line(w);
        let cell_w = m.cell_width(w, per);
        Built {
            m,
            cell_w,
            caption_w: m.caption_width(cell_w),
            step: self.step(s, &m),
        }
    }
}

/// The Icons view's grid: see the [module docs](self).
///
/// Used by: hosts (Strata's Icons pane and the chooser's), C20d.
pub struct IconsGrid<S> {
    shared: Shared<S>,
}

impl<S: 'static> IconsGrid<S> {
    /// A grid over the collection `base` builds (keyed `key`, as `base` keys it), reading the
    /// host's state through `parts` and its own through `grid_mut`.
    ///
    /// `base` is the host's collection as for any view: its accessors, clock, controllers'
    /// settings, intents, drops, rename, marks and description. The grid sets the rest (see
    /// [`IconsGrid::collection`]).
    pub fn new(
        key: &str,
        base: impl Fn(&S) -> Collection<S> + 'static,
        parts: impl for<'a> Fn(&'a S) -> IconsParts<'a> + 'static,
        grid_mut: impl Fn(&mut S) -> &mut IconsGridState + 'static,
    ) -> Self {
        Self {
            shared: Shared {
                key: key.into(),
                base: Rc::new(base),
                parts: Rc::new(parts),
                grid_mut: Rc::new(grid_mut),
                thumbs_mut: None,
                details: None,
                lines: 2,
                on_size: None,
                on_visible: None,
                scale: None,
            },
        }
    }

    /// The thumbnail store and the grid's viewer in it, with the listing, for the grid's own
    /// calls: [`ThumbnailStore::sync`] as rows show, [`ThumbnailStore::set_step`] on a new
    /// size. Without it the grid asks for no thumbnails (the host may still draw them through
    /// [`IconsParts::thumbs`] and sync them itself).
    pub fn thumbs_mut(
        mut self,
        f: impl for<'a> Fn(&'a mut S) -> Option<(&'a mut ThumbnailStore, ViewerId, &'a dyn EntrySource)>
        + 'static,
    ) -> Self {
        self.shared.thumbs_mut = Some(Rc::new(f));
        self
    }

    /// The details line under each name ("12 items", "320×180", "4.2 MB"): the host's metadata,
    /// streamed in as it arrives. `version` changes with what `text` reads for the tile, so a
    /// tile builds again only when its own details land. Without it a tile has no details
    /// line.
    pub fn details(
        mut self,
        version: impl Fn(&S, &RowCx<'_>) -> u64 + 'static,
        text: impl Fn(&S, &RowCx<'_>) -> Option<Arc<str>> + 'static,
    ) -> Self {
        self.shared.details = Some((Rc::new(version), Rc::new(text)));
        self
    }

    /// How many lines a name takes at most: Strata's 2 by default; 0 shows no name.
    pub fn caption_lines(mut self, lines: usize) -> Self {
        self.shared.lines = lines;
        self
    }

    /// Hears a size step or reset (keys, Ctrl+Shift+wheel, [`IconsGrid::apply_size`]): the
    /// host sets the size it keeps (per folder or the default), and may show its notice. The
    /// grid then lays out at the new size.
    pub fn on_size(mut self, f: impl Fn(&mut S, SizeRequest, &mut Tx) + 'static) -> Self {
        self.shared.on_size = Some(Rc::new(f));
        self
    }

    /// Hears which entries the viewport shows, after the grid synced its thumbnails (a folder
    /// recount, say). Return true only if the view reads what it changed.
    pub fn on_visible(mut self, f: impl Fn(&mut S, Range<usize>) -> bool + 'static) -> Self {
        self.shared.on_visible = Some(Rc::new(f));
        self
    }

    /// The display scale the view builds at (`cx.env().scale`), which picks the thumbnail
    /// step. Handlers built without it use the scale the last view recorded.
    pub fn scale(mut self, scale: f32) -> Self {
        self.shared.scale = (scale.is_finite() && scale > 0.0).then_some(scale);
        self
    }

    /// The tile sizes for `s` as it stands.
    pub fn metrics(&self, s: &S) -> TileMetrics {
        self.shared.metrics(s)
    }

    /// The layout key a scroll offset is remembered under in Icons: the tiles a row and their
    /// height, for a viewport `width` points wide at tile `size`. Strata brings an Icons offset
    /// back only at the same grid width; set it on the collection's state
    /// (`CollectionState::set_layout_key`) before `enter`, from the pane's last width.
    pub fn layout_key(width: f32, size: f32, look: &Look) -> u64 {
        let m = TileMetrics::new(look, size, 2, true);
        let mut h = std::hash::DefaultHasher::new();
        "icons".hash(&mut h);
        m.per_line(width).hash(&mut h);
        m.height.to_bits().hash(&mut h);
        h.finish()
    }

    /// The full key path of entry `id`'s tile as the grid lays it out now (it changes when the
    /// grid reflows to another count): where X2 finds a folder tile's drop zone, and what a
    /// motion effect flies from (`{path}/icon`).
    pub fn tile_path(&self, s: &S, id: EntryId) -> Option<String> {
        let p = (self.shared.parts)(s);
        let index = p.source.index_of(id)?;
        Some(entry_path(
            &self.shared.key,
            p.source,
            index,
            p.collection.per_line(),
        ))
    }

    /// `base` with the grid's parts: its roles ([`Roles::GRID_CELLS`]), bare items, row height,
    /// the [`GridLines`] packing, the tile's slots (icon, label and content), its hit regions,
    /// its thumbnail sync (`on_visible`) and its size wheel (`on_wheel`). The host sets none of
    /// those on its own builder. Use it for the collection's `tick` and `geometry` too.
    pub fn collection(&self, s: &S) -> Collection<S> {
        collection_with(&Rc::new(self.shared.clone()), s)
    }

    /// The grid, filling the height its parent gives it.
    pub fn build(&self, s: &S) -> El {
        let shared = Rc::new(self.shared.clone());
        let parts = (shared.parts)(s);
        if let Some(scale) = shared.scale {
            parts.grid.scale.set(scale);
        }
        let list = collection_with(&shared, s).build(s);
        let grid_mut = shared.grid_mut.clone();
        let bar = (shared.parts)(s).grid.bar.clone();
        let grid = col()
            .w_full()
            .h_full()
            .min_h(0.0)
            // A new width cuts captions to it: the collection builds again only when the
            // count a row changes, and a name's cut follows the cell's width.
            .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
                let grid = grid_mut(s);
                if grid.width.get() == r[2] {
                    tx.live_only();
                } else {
                    grid.width.set(r[2]);
                }
            })
            .child(list);
        // The pointer over the grid shows its scroll bar.
        bar.area(grid)
    }

    /// Steps or resets the size: the host hears the request ([`IconsGrid::on_size`]), sets
    /// its size, and the grid lays out at it at once, keeping the cursor tile in view, and
    /// asks the thumbnail store for the new step. Call it from the host's window keys
    /// ([`SizeKeys::window_command`]) and its `on_intent` ([`size_command`]).
    pub fn apply_size(&self, s: &mut S, command: SizeCommand, tx: &mut Tx) {
        apply_size(&Rc::new(self.shared.clone()), s, command, tx);
    }

    /// Lays out at a size the host changed itself (a slider, another window, a folder with its
    /// own size): the cursor tile stays in view and thumbnails follow the step. True when the
    /// count a row changed.
    pub fn size_changed(&self, s: &mut S, tx: &mut Tx) -> bool {
        size_changed(&Rc::new(self.shared.clone()), s, tx)
    }

    /// From the host's poll: catches a size or display scale the host changed without
    /// [`IconsGrid::size_changed`], so the thumbnail step and the cursor reveal follow within
    /// a frame. Returns false: nothing the view reads changed.
    pub fn poll(&self, s: &mut S, tx: &mut Tx) -> bool {
        let shared = Rc::new(self.shared.clone());
        let (size, last) = {
            let p = (shared.parts)(s);
            (p.size, p.grid.size.get())
        };
        if last != Some(size) {
            size_changed(&shared, s, tx);
        } else {
            sync_step(&shared, s);
        }
        false
    }
}

/// Where a middle press starts autoscroll in a grid (C24's
/// `AutoscrollPointer::background`): on no tile, or on a tile's blank space (the gutter C1b
/// hears from the grid's regions), as Strata's Icons view starts it from "the empty background
/// or gutter". A press on a thumbnail, a name or a details line is a tile's (a middle click on
/// a folder opens it in a new tab).
///
/// Used by: hosts.
pub fn autoscroll_background(
    input: &PointerInput,
    _cx: &Ctx<'_>,
    _settings: &AutoscrollSettings,
) -> bool {
    input.hit.index.is_none() || input.hit.region == Region::Gutter
}

fn collection_with<S: 'static>(shared: &Rc<Shared<S>>, s: &S) -> Collection<S> {
    let b = shared.built(s);
    let m = b.m;
    let region_shared = shared.clone();
    let visible_shared = shared.clone();
    let wheel_shared = shared.clone();
    let base = (shared.base)(s);
    let grid = (shared.parts)(s).grid;
    let (bar, feed) = (grid.bar.clone(), grid.feed.clone());
    let edge_bar = bar.clone();
    let (bar_key, target) = (format!("{}-bar", shared.key), viewport_path(base.scope()));
    let edge_shared = shared.clone();
    base.on_scroll(move |p| {
        feed.set(p);
        bar.follow(p);
    })
    .edge(move |s: &S| {
        let look = BarLook::listing((edge_shared.parts)(s).look);
        Some(edge_bar.build::<S>(&bar_key, &target, look))
    })
    .roles(Roles::GRID_CELLS)
    .bare_items()
    .row_height(move |_: &S, _: &Look| m.height)
    .lines(GridLines::new(m))
    .icon(tile::icon_slot(shared, b))
    .label(tile::label_slot(shared, b))
    .content(tile::content_slot(shared, b))
    .regions(move |s: &S, index, local: Point| {
        let p = (region_shared.parts)(s);
        if index >= p.source.len() {
            return Region::Gutter;
        }
        let entry = p.source.entry(index);
        let cut = b.cut(&region_shared, s, entry.name);
        let details_w = region_shared
            .details
            .as_ref()
            .and_then(|(_, f)| {
                let rcx = row_cx(p.look, entry, index, &region_shared.key);
                f(s, &rcx)
            })
            .filter(|d| !d.is_empty())
            .map_or(0.0, |d| {
                rune::text::measure(&d, &[], m.details_font, None).0 + 2.0 * m.caption_pad
            });
        regions::region(&m, b.cell_w, &cut, details_w, local)
    })
    .on_visible(move |s: &mut S, range| on_visible(&visible_shared, s, range))
    .on_wheel(move |s: &mut S, e: WheelEvent, tx: &mut Tx| on_wheel(&wheel_shared, s, e, tx))
}

/// A [`RowCx`] for reading the host's details outside a build (the regions).
fn row_cx<'a>(
    look: &'a Look,
    entry: crate::contract::Entry<'a>,
    index: usize,
    scope: &'a str,
) -> RowCx<'a> {
    RowCx {
        index,
        entry,
        state: RowState::default(),
        look,
        scope,
        hover_live: false,
    }
}

fn on_visible<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, range: Range<usize>) -> bool {
    let synced = sync_thumbs(shared, s, Some(range.clone()));
    let host = shared.on_visible.as_ref().is_some_and(|f| f(s, range));
    synced || host
}

/// Tells the store what the grid shows at the step it draws at: `range` (as Rune reports it)
/// or, with none, the viewport it last synced at the new step.
fn sync_thumbs<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, range: Option<Range<usize>>) -> bool {
    let Some(thumbs_mut) = shared.thumbs_mut.clone() else {
        return false;
    };
    let m = shared.metrics(s);
    let step = shared.step(s, &m);
    let (per, listing) = {
        let p = (shared.parts)(s);
        (p.collection.per_line().max(1), p.listing)
    };
    (shared.parts)(s).grid.synced.set(Some(step));
    let Some((store, viewer, src)) = thumbs_mut(s) else {
        return false;
    };
    match range {
        Some(r) => store.sync(
            viewer,
            src,
            Viewport::new(r, step).per_line(per).listing(listing),
        ),
        None => store.set_step(viewer, src, step),
    }
}

/// Asks the store for the step the grid draws at now, if it isn't the one it last asked for.
fn sync_step<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S) -> bool {
    if shared.thumbs_mut.is_none() {
        return false;
    }
    let m = shared.metrics(s);
    let step = shared.step(s, &m);
    if (shared.parts)(s).grid.synced.get() == Some(step) {
        return false;
    }
    sync_thumbs(shared, s, None)
}

fn size_changed<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, tx: &mut Tx) -> bool {
    let (size, shown) = {
        let p = (shared.parts)(s);
        let cursor = p.collection.selection.cursor_index(p.source);
        let shown = cursor.filter(|i| p.collection.visible_range().contains(i));
        (p.size, shown)
    };
    let old = (shared.grid_mut)(s).size.replace(Some(size));
    let reflowed = collection_with(shared, s).reflow(s, tx);
    // The same count a row at another tile height moves every row: a cursor tile that
    // showed stays in view, as a reflow keeps it (which `reflow` did if the count changed).
    if !reflowed
        && old.is_some_and(|old| old != size)
        && let Some(index) = shown
    {
        let p = (shared.parts)(s);
        p.collection.reveal(&shared.key, p.source, index, tx);
    }
    sync_step(shared, s);
    reflowed
}

fn apply_size<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, command: SizeCommand, tx: &mut Tx) {
    let request = {
        let p = (shared.parts)(s);
        match command {
            SizeCommand::Step(n) => SizeRequest::Step {
                from: p.size,
                to: p.look.tiles.step(p.size, n),
            },
            SizeCommand::Reset => SizeRequest::Reset,
        }
    };
    if let Some(f) = &shared.on_size {
        f(s, request, tx);
    }
    size_changed(shared, s, tx);
}

fn on_wheel<S: 'static>(shared: &Rc<Shared<S>>, s: &mut S, e: WheelEvent, tx: &mut Tx) -> bool {
    let grid = (shared.grid_mut)(s);
    let mut wheel = grid.wheel.get();
    if !WheelSteps::claims(&e) {
        wheel.reset();
        grid.wheel.set(wheel);
        return false;
    }
    let steps = wheel.steps(&e);
    grid.wheel.set(wheel);
    if steps != 0 {
        apply_size(shared, s, SizeCommand::Step(steps), tx);
    } else {
        tx.live_only();
    }
    true
}
