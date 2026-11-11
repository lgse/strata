//! BrowserTabs (C7): the browser window's tab strip (`browser/tabs`), on rune-kit's
//! `TabStrip`, with what a file manager adds: tabs and New tab as drop targets, a drag that
//! rests on a tab selecting it, the closed-tabs stack, and a window title that follows the
//! active tab.
//!
//! Work package: C7, `docs/work/phase-1-leaves/C7-browser-tabs.md`. Behaviour follows the
//! feature map, against lgse/strata `upstream/main` 35ee677ab
//! (`src/ui/window/composition/tabs.rs`, `tabs/strip.rs`, `tabs/strip/drop.rs`, `style.css`).
//!
//! Rune dev 5d421fd: **partly**. rune-kit's `TabStrip` covers reorder (with Escape to put the
//! tab back), tear-out, a gap for a tab coming in from another strip, middle-click close,
//! shrink-then-scroll with edge fades and auto-scroll while dragging, a keyboard-focused tab's
//! close button, and the `tablist` and `tab` roles with the selected state; `Tx::set_title`
//! applies the title. Nothing covers drop targets on tabs, the spring, the closed-tabs stack,
//! the wheel's sideways mapping or Strata's look. Since 5d421fd a tab's title runs in its own
//! direction (cded95f) and `WindowTitlebar::rtl` became `WindowTitlebar::direction`.
//!
//! # Parts
//!
//! - [`BrowserTabs`] builds the strip's band from [`TabsData`] (plain data the host rebuilds
//!   each view: [`Tab`]s in strip order and the active one) and a [`BrowserTabsState`] (held by
//!   the host per window, reached by accessor). Each tab shows its location's icon (the host's
//!   [`IconName`](strata_kit::icons::IconName)) and its name, shortened past [`LABEL_CHARS`];
//!   the New tab button ends the strip.
//! - [`NewTabButton`] is the same button on its own, for a header that holds it while the
//!   strip is hidden (Strata shows the strip only with two or more tabs).
//! - Everything the strip does is a [`TabsIntent`]. The host owns every policy and routes them:
//!
//! ```ignore
//! let tabs: Vec<Tab> = s.tabs.iter().map(|t| Tab::new(t.id, t.name(), t.icon()).accepts(t.accepts())).collect();
//! let data = TabsData::new(&tabs, s.active);
//! let band = BrowserTabs::new(&data, &s.strip, |s| &mut s.strip, |s| &mut s.strip.drag)
//!     .dnd(dnd)                                   // X2's binder (Rune's by default)
//!     .drop_action(|s, id, offer, mods| ..)       // choose_action + the tab folder's volume
//!     .opens_tabs(|s, offer| ..)                  // Strata's own folder-only drags
//!     .content_types(CONTENT_DROP_TYPES)
//!     .badges(s.ctrl_shift_held)                  // App::on_modifiers
//!     .on_intent(|s, intent, tx| s.route(intent, tx))
//!     .build_layered(look, cx, &mut layer);
//! // Each frame, from the host's poll, with its clock:
//! let polled = s.strip.tick(now, &data, tx);      // the spring, the title, revealing the active tab
//! for intent in polled.intents { s.route(intent, tx) }
//! // TabsIntent::Title(t)        => tx.set_title(t)
//! // TabsIntent::Close { id }    => if the close goes ahead: s.closed.record(tab, index); select data.after_close(id)
//! // Ctrl+Shift+T                => if let Some(c) = s.closed.reopen() { insert at c.index_in(len), select it }
//! // Ctrl+Tab / Ctrl+Page Down   => data.step(1); Ctrl+Shift+Page Down => data.moved(active, 1); Ctrl+Shift+1 => data.nth(0)
//! ```
//!
//! # Built on rune-kit
//!
//! The tabs are `TabStrip`'s, with its pill look (Kit tokens through `fill`, `close_icon`,
//! `row_height`, `gap` and the overflow policy, from [`strata_kit::slots::tabs`]), and C7 sets
//! over each built tab, through `TabItem::decorate`: its whole name as its accessible name, the
//! drop tint, Strata's 3 point accent underline on the active tab (shared between tabs with
//! `El::matched`, so it slides; it jumps with animations off), its drop zone, and the wheel's
//! sideways mapping (`wheel.rs`). Tab keys carry the strip's key ([`tab_key`]: `t-<key>-<id>`).
//! Hidden markers report where the strip, tab 0 and the active tab drew (`probe.rs`), so the
//! wheel can read the strip's offset and a newly active tab can be revealed clear of the edge
//! fades, which Rune can't do from the outside (U-C7-2, U-C7-7).
//!
//! # Drops and the spring
//!
//! Every zone attaches through the host's [`Dnd`](crate::contract::dnd::Dnd) binder, on an
//! element keyed for X2's: each tab ([`tab_key`]), New tab ([`new_tab_key`]) and the strip (its
//! key, for the gaps between tabs). See `drops.rs`.
//!
//! - **Files and content.** A tab takes what its [`Accepts`] says: files (`text/uri-list`), and
//!   content of the host's types ([`BrowserTabs::content_types`]). Trash and Recent take
//!   neither, so they build no zone, and a drag passes through them to the strip, which refuses
//!   it: they neither tint nor select. A drop is [`TabsIntent::DropInto`]; the host transfers
//!   or saves, with its action ([`BrowserTabs::drop_action`]), and selects the tab.
//! - **The spring.** A file or content drag over a tab that isn't active tints it and arms a
//!   [`HOVER_DELAY`] (450 ms, Strata's) wait: the hover stamps its start on the handler's clock
//!   (`Tx::clock`) and asks for a wake when it is due, [`BrowserTabsState::tick`] re-asks for the
//!   time left at every tick, and the tick at the due time reports [`TabsIntent::Select`] by
//!   [`Via::Spring`]. The same hover reported again (X2's binder may, on each motion) keeps the
//!   start; a leave, a drop, the tab closing or becoming active ends it. It holds in a window at
//!   rest, which polls only at the wakes it asked for (`tests/c7.rs` steps from wake to wake).
//! - **Drags that open tabs.** A drag the host says opens tabs ([`BrowserTabs::opens_tabs`]:
//!   Strata's own folder-only drags and sidebar rows, never folders from another app) neither
//!   tints nor selects a tab. Over the strip its moves place rune-kit's incoming gap where it
//!   would open; only the zone holding the gap closes it, so moving from a tab to the gap
//!   beside it keeps it open. Its drop is [`TabsIntent::OpenTabs`] at the gap, or at the end on
//!   New tab. Both real binders report a zone's leave before its drop (W1's `Left`: left,
//!   moved, or dropped), so a leave closes the gap at once, as a cancel needs, and keeps its
//!   slot for the drop that may follow. A zone reached next keeps that slot until its own gap
//!   has built (rune-kit finds a gap's slot as the strip builds, so a drop in the same turn has
//!   no other), and two ticks drop it, each asking for the next, so it expires in a window at
//!   rest (reviews 1 and 2). Only a strip whose host set [`BrowserTabs::opens_tabs`] follows
//!   drag moves: following them asks for a frame on each motion. While such a drag runs
//!   anywhere in the window, the host says so with [`BrowserTabsState::set_open_drag`] and New
//!   tab shows its tint.
//!
//! **The host keeps every tab's view mounted while a drag runs.** A spring shows another tab
//! in the middle of a drag, and on Wayland P1 needs the drag's source to stay mounted at its
//! path, covering its press point, with its `on_drag`, until the drag ends (P1's "Rune's capture
//! on Wayland"). So a host that drops the inactive tabs' views from the tree must not do it
//! during a drag: it keeps them where they were, styled hidden (opacity 0, out of the
//! accessibility tree, under the active view), not made inert and not removed. The gallery page
//! keeps every view mounted that way.
//!
//! # Moving tabs between windows
//!
//! [`BrowserTabs::tear_out`] (off by default) lets a tab be dragged 36 points off the strip.
//! While the button is down the host hears [`TabsIntent::TearMove`] and may open the tab's
//! window under the pointer, then call `state.drag.hand_off()`; on release it hears
//! [`TabsIntent::TearOut`]. The torn tab stays in the source window's list until then (its
//! slot holds the gesture). To move it into another window's strip, the host maps the move's
//! `screen_point` into that window and calls its `state.drag.set_incoming(Some(point))`, reads
//! `incoming_slot()` at the release, moves the tab there and calls `set_incoming(None)`.
//! `screen_point` is `None` on Wayland (the protocol hides window positions) and headless, so
//! moving between windows and placing a window under the pointer work on X11 only.
//!
//! # Keyboard and assistive technology
//!
//! The tab list is one Tab stop: the arrows move between tabs, Return or Space selects, and a
//! focused tab shows its close button (pointer only). Strata's tab shortcuts are the host's key
//! map; [`TabsData::step`], [`TabsData::nth`], [`TabsData::moved`] and [`TabsData::after_close`]
//! are its rules, and a tab selected any way scrolls into view at the next tick. Assistive
//! technology hears a tab list ([`TabsLabels::list`], "Tabs"), each tab by its whole name with
//! its place in the set, the active one selected, a "Close tab" button in each, and "New tab".
//!
//! # Coverage
//!
//! Scripts are `tests/scripts/c7/<name>.json`; "test" is in `crates/strata-gallery/tests/c7.rs`;
//! "unit" is in `tests.rs`; "golden" is a scene's a11y or PNG golden. "Strata" marks a bullet
//! that is the host's policy or another component's.
//!
//! `browser/tabs` (110 bullets):
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | Ctrl+T, the New tab (+) button, or 10xer `t` then `n` opens a tab… | **partial**: script `new-tab` (the button and the page's Ctrl+T); 10xer and the location are Strata's |
//! | A new tab is added at the right end of the strip | **partial**: script `new-tab` (the page appends; where is the host's) |
//! | Each tab keeps its own location…; Changing Show hidden files updates… | **Strata** |
//! | Ctrl+W, a tab's X button, or 10xer `t` then `x` closes that tab; closing the last tab closes the window | **partial**: script `close-button` (the X; Ctrl+W, 10xer and the window are Strata's) |
//! | Middle-clicking a tab closes it | script `middle-click-close` |
//! | Closing the active tab selects its right neighbour, or its left neighbour when it was last | script `close-button`; unit `closing_hands_over_to_the_right_then_the_left` ([`TabsData::after_close`]) |
//! | Closing a tab (or the window) whose folder has a file operation running… (2) | **Strata** |
//! | Ctrl+Shift+T, or the window action `win.reopen-tab`, reopens the last closed tab… | script `reopen-closed` ([`ClosedTabs`]; the binding is Strata's) |
//! | Repeated Ctrl+Shift+T reopens closed tabs newest first, up to 20… | script `reopen-closed`; unit `the_stack_keeps_twenty_newest_first` |
//! | Ctrl+W, the X button, a middle-click, 10xer…, and closing an inactive tab each record the closed tab | **partial**: every close path is one [`TabsIntent::Close`]; the host records it |
//! | Closed tabs belong to their window…; are not saved…; A reopened tab starts with fresh selection… | **Strata** (the stack is per-window data and is never saved) |
//! | When fewer tabs are open than the former position, the reopened tab stays at the right end | unit `a_reopened_tab_goes_back_to_its_place_or_the_end` ([`Closed::index_in`]) |
//! | With no closed tab recorded, Ctrl+Shift+T opens nothing… | script `reopen-empty` |
//! | A close blocked by a modal… records nothing; While a modal dialog is open, Ctrl+Shift+T does nothing…; Reopening a folder deleted…; works from text fields… (4) | **Strata** |
//! | Opening folders in a new tab or window (8) | **Strata** (views and key map) |
//! | Ctrl+Tab and Ctrl+Page Down select the next tab…; each wraps | **partial**: script `switch-keys`, unit `step_wraps_at_either_end` ([`TabsData::step`]); the bindings are Strata's |
//! | Ctrl+Shift+1 to Ctrl+Shift+9 select tabs 1 to 9 and Ctrl+Shift+0 selects tab 10… | **partial**: script `number-keys` ([`TabsData::nth`]); the digit row is Strata's key map |
//! | The tab digit is the key's position…; Where Shift+1 types `+`…; 10xer `t`… (3); Escape after `t`…; Typing `t`, `n`, or `x`… | **Strata** (key map, C18) |
//! | Holding Ctrl+Shift without Alt or Super shows number badges… | **partial**: [`BrowserTabs::badges`], golden `c7-badges` (PNG); the page's F9 stands in for the hold, which Strata hears with `App::on_modifiers` |
//! | Badges hide when Ctrl or Shift is released or the window loses focus; Keypad…; Alt or Super…; Hyper or Meta… (4) | **Strata** (key map) |
//! | Selecting a tab refocuses the widget it last focused… (4 bullets) | **Strata** |
//! | While a modal dialog is open, tab shortcuts and the New tab button do not… | **Strata** (and Rune's modal routing) |
//! | Ctrl+Shift+Page Up and Ctrl+Shift+Page Down move the active tab…; Moving stops at either end | **partial**: script `move-keys`, unit `moving_stops_at_the_ends` ([`TabsData::moved`]) |
//! | Moving keeps the tab's location, selection…; Numbered shortcuts… follow the reordered strip; The F1 shortcut reference… | **Strata** (C19) |
//! | Dragging a tab label over the strip shows an accent insertion line… | **deliberate change**: the tab lifts and its neighbours step aside (below) |
//! | Releasing a tab drag moves the tab into the marked gap, and the drag image shows the tab's label | script `reorder-drag` (the lifted tab is its own image) |
//! | Dropping a tab on the New tab button moves it to the right end | script `reorder-to-end` |
//! | During a sidebar row drag or a folder-only drag, a strip of two or more tabs marks an insertion line after the last tab | **partial**: New tab lights while the host says such a drag runs ([`BrowserTabsState::set_open_drag`], golden `c7-new-tab-drop`); the gap opens only over the strip, as opening it at the end would reflow every tab on every folder drag (owner's ruling) |
//! | Folder-only drags from Columns, Icons, and List…; from the preview header and Ctrl+F results… (2) | **Strata**, X2 (drag sources) |
//! | Over the strip, that line moves to the gap nearest the pointer. Releasing opens… at that gap | tests `a_drag_that_opens_tabs_places_a_gap_and_opens_there`, `a_drop_in_the_gap_between_tabs_opens_there_after_its_leave`, `a_leave_from_a_tab_after_the_strip_took_the_drag_keeps_the_gap` (a binder like X2's, leave then drop; Rune's reads paths only at the drop) |
//! | A drag of several folders opens one tab per folder, in drag order… | **partial**: [`TabsIntent::OpenTabs`] carries the drop and its index; opening is the host's |
//! | With one tab the strip stays hidden during these drags; dropping on the header's New tab button opens the tab | script `drop-on-new-tab` ([`NewTabButton`]; hiding the strip is the host's) |
//! | The New tab button under a sidebar, folder, or tab drag shows an accent tint | test `new_tab_lights_and_opens_a_drag_that_opens_tabs_at_the_end`; golden `c7-new-tab-drop` (PNG) |
//! | A folder-only drag released over a tab header opens a new tab and moves or copies nothing… | test `a_drag_that_opens_tabs_places_a_gap_and_opens_there` |
//! | A folder-only drag held over a tab does not select it | test `a_drag_that_opens_tabs_places_a_gap_and_opens_there`; unit `nothing_springs_unarmed_or_for_a_tab_now_active_or_gone` |
//! | Dropping a sidebar row… changes neither the sidebar order nor the pins | **Strata**, C8 |
//! | Folders dragged from another application are not opened as tabs; dropped on a tab header, they transfer… | script `foreign-folder-not-a-tab` (the host's [`BrowserTabs::opens_tabs`] decides; the page's stand-in tells its own drags by a path prefix) |
//! | While a modal dialog is open, drops on the strip or the New tab button open and move no tabs | **Strata** (Rune routes no drop outside a modal) |
//! | Escape during a file drag in Columns, List, or Icons…; also cancels a drag from the preview header… (2) | P1, X2 |
//! | Escape during a tab drag, a sidebar row drag, or a folder drag from a listing cancels it… | script `reorder-escape` (a tab drag); the others are X2's, whose binder reports a leave: test `a_leave_closes_the_gap` |
//! | After an Escape-cancelled drag, the strip shows no insertion line | test `a_leave_closes_the_gap` |
//! | Escape that cancels a drag dismisses nothing else…; After a cancelled drag, the next Escape… (2) | P1, Strata |
//! | The strip appears only with two or more tabs. The New tab and window-close buttons then move into it… | **partial**: the band takes trailing children (the window's Close) and [`NewTabButton`] goes in the header; when to show the strip is the host's |
//! | With Show close button off, the window-close button stays hidden… | **Strata**, X1 |
//! | A 3 px accent underline slides to the selected tab, and jumps without sliding when animations are disabled | test `the_underline_slides_and_jumps_with_animations_off`; golden `c7-several` (PNG) |
//! | When the strip is wider than the window, the wheel scrolls it horizontally and no scrollbar is shown | test `a_vertical_wheel_scrolls_an_overflowing_strip_sideways` (a workaround: `wheel.rs`, U-C7-2); golden `c7-overflow` (PNG) |
//! | Selecting a tab scrolls it into view | tests `a_tab_selected_from_the_keyboard_scrolls_into_view`, `a_revealed_tab_is_clear_of_the_edge_fades` (a fade's width clear of either edge, in one jump: `probe.rs`, U-C7-7), `a_key_selection_reveals_and_retitles_in_a_still_window`, `a_wheel_right_after_a_selection_beats_its_reveal` |
//! | Selecting the last tab also scrolls the New tab button into view | golden `c7-overflow` (PNG): New tab sits outside the scrolling, always shown |
//! | Hovering an inactive tab accents its label; its X turns accent only while the pointer is over the X itself | **deliberate change**: rune-kit's hover wash and ghost close button (U-C7-3) |
//! | Assistive technologies read the strip as a tab list named "Tabs", and each X as "Close tab". Each tab reads as its name, with the active one selected | golden `c7-several` (a11y) |
//! | A tab is named after its active directory…; The Home, Trash… names follow the interface language; SMB…; Photos…; awaits…; keeps its name…; Columns presses… (10) | **Strata** (names are data) |
//! | Names longer than 22 characters are ellipsized | unit `names_past_22_characters_end_in_an_ellipsis`; test `the_narrowest_tab_still_shows_about_ten_characters`; golden `c7-several` (a11y: the whole name; PNG: the label) |
//! | A window whose active tab is at folder `docs` is titled `docs — Strata`, with an em dash | script `title`; unit `the_title_carries_the_whole_name_or_the_app_alone` |
//! | Navigating the active tab, or selecting another tab, retitles the window… | script `title`; test `selecting_a_tab_retitles_the_window`; unit `the_title_is_emitted_once_per_change_of_the_active_tab` |
//! | A window with no folder open and no location awaited is titled "Strata" | unit `the_title_carries_the_whole_name_or_the_app_alone` |
//! | While the active tab awaits a location…; SMB…; translated…; `~` and `/`…; Columns press… (5) | **Strata** (the title carries the tab's name) |
//! | A background tab that navigates leaves the window title unchanged | unit `the_title_is_emitted_once_per_change_of_the_active_tab` |
//! | The title shows a folder name over 22 characters in full, though the tab label ellipsizes it | test `selecting_a_tab_retitles_the_window`; unit `the_title_carries_the_whole_name_or_the_app_alone` |
//! | Dropping files, or a selection mixing files and folders, on another tab transfers them…: a same-volume drop moves, and holding Ctrl copies | scripts `drop-on-tab`, `drop-ctrl-copies` (the action is the host's [`BrowserTabs::drop_action`]) |
//! | Holding a file or browser-content drag over a tab for 450 ms selects it… | script `hover-selects`; test `the_hover_selects_a_tab_in_a_resting_window`; units `a_repeated_hover_on_the_same_tab_keeps_its_start`, `another_tab_restarts_the_wait_and_a_leave_ends_it` |
//! | A tab under a file or browser-content drag accents its label and shows no drop outline | **deliberate change**: a highlight tint, no outline; golden `c7-drop-hover` (PNG) |
//! | Text dragged from another application onto the strip is never taken as a tab reorder… | test `content_goes_into_a_tab_that_takes_it_and_no_other` (reorder is a pointer drag, never a drop) |
//! | Text, an image, or a web address dropped on a tab… goes to that tab's current folder, and selects the tab | **partial**: test `content_goes_into_a_tab_that_takes_it_and_no_other` ([`TabsIntent::DropInto`]); saving and selecting are the host's |
//! | An image dropped on a tab while a modal dialog is open is still saved…; While a modal dialog is open, a text or file drop… saves nothing (2) | **Strata**; Rune delivers no drop outside a modal, so the first can't hold (below) |
//! | A tab showing Trash, Recent, or a remote folder refuses dropped text and images | test `content_goes_into_a_tab_that_takes_it_and_no_other` ([`Accepts`]) |
//! | A Trash or Recent tab also refuses file drags: it neither accents nor selects under the drag | script `trash-refuses`; test `a_drag_on_the_active_tab_or_trash_selects_nothing` |
//!
//! `browser/tabs/session-restore` (23 bullets): **Strata**, all but one. Saving, restoring,
//! what is skipped and the setting are its own; the strip takes its tabs as data. "Adding,
//! selecting, closing, or reordering a tab… saves the window's tab locations" is **partial**:
//! each of those is a [`TabsIntent`] the host saves on.
//!
//! # Deliberate changes
//!
//! - **The look is rune-kit's pill tabs**: the active tab's pill, a hover wash, faint and base
//!   title colours, separators between quiet tabs, and the close button in a slot that shows
//!   on hover or focus. Over it C7 draws Strata's sliding 3 point accent underline. Strata's
//!   tabs are bare labels that turn accent when active, hovered or under a drag, with the X
//!   always shown at half opacity. A tab under a drag shows a highlight tint instead of an
//!   accent label (U-C7-3). Owner item: the coordinator takes this to the owner.
//! - **Reorder lifts the tab** and its neighbours step aside, with no insertion line (as C8).
//! - **A drag that opens tabs opens a gap** where the tab will land, rune-kit's incoming gap,
//!   and the tabs give up a share of their width for it; Strata draws an accent line.
//! - **Widths:** tabs share the strip from 224 points down to room for about 10 characters of
//!   their label beside the icon and the close button (`tabs.tab.min_width`: 136 points at the
//!   default type, growing with the text size), then the strip scrolls with faded edges. Names
//!   longer than that fade at the tab's end. Owner item: the widths. Strata's tabs take
//!   their label's width. The 22-character label is kept as well.
//! - **Each tab shows its location's icon**, which rune-kit's lead slot would otherwise fill
//!   with an empty box; the number badges take its place while they show (Strata's sit after
//!   the label).
//! - **Tear-out** is new and off by default; moving tabs between windows works on X11 only.
//! - **The tab list is one Tab stop** with arrow keys, as rune-kit's lists are; Strata makes
//!   each tab button a Tab stop. The close buttons are pointer-only.
//! - **The close button reads "Close tab"** in English with no tooltip (U-C7-1); Strata
//!   translates it and its tooltip names Ctrl+W.
//! - **An image dropped on a tab behind a modal** isn't delivered: Rune routes no drop outside
//!   a modal.
//!
//! # What stays in Strata
//!
//! Tab names and icons, opening, closing (and its refusals), reopening, selecting and moving
//! tabs in its data, the key map, the badges' modifier tracking, transfers, content saving and
//! the drop action, which drags open tabs, hiding the strip at one tab and where the New tab
//! and window buttons go, focus restore per tab, keeping inactive views mounted during a drag,
//! windows for torn-out tabs, and session saving and restoring.
//!
//! # Rune gaps
//!
//! Drafts for the owner to file, after searching `anomalyco/rune` (none covers TabStrip; #329
//! and #302 are near). Headless repros are in `strata-spikes/c7-probes` (`RESULTS.md`).
//!
//! - **U-C7-1 (draft, not filed)** rune-kit: TabStrip's close button is labelled "Close tab"
//!   in English, with no tooltip and no shortcut hint, and nothing sets them. Ask:
//!   `TabStrip::close_label` and `close_help`.
//! - **U-C7-2 (draft, not filed)** A vertical wheel over a sideways-only viewport scrolls
//!   nothing (U-C6-5), and an app can neither read a viewport's offset nor scroll one by a
//!   delta, so it can't map the wheel itself without reading where its content drew (probe 2).
//!   Ask: TabStrip maps a vertical wheel to its axis, as browsers' tab strips do, or
//!   `Cx::scroll_offset(key)` or `Tx::scroll_by`. Here: `wheel.rs`, fragile.
//! - **U-C7-3 (draft, not filed)** rune-kit: TabStrip's look has no hooks past its fill, close
//!   glyph and sizes: the active tab's fill is `c.pressed`, a title's colour is fixed, and there
//!   is no active indicator and no drop-hover state. Ask: an active-indicator option (an
//!   underline) and a title colour or a hovered/drop-hovered style. Here: the underline and
//!   tint are set over each tab.
//! - **U-C7-4 (draft, not filed)** rune-kit: `TabDragState::set_incoming` takes only a point,
//!   so a strip can't open its gap at a slot (after the last tab, from the keyboard), and the
//!   gap always takes a tab's share of the width; there is no thin insertion mark.
//! - **U-C7-5 (draft, not filed)** rune-kit: TabStrip keys each tab `t-<id>` whatever the
//!   strip's key, so two strips' tabs (or any other `t-` key) collide. Here: ids carry the key.
//! - **U-C7-6 (draft, not filed)** `Runtime` has no public reader for the window title
//!   (`window_title` is `pub(crate)`); tests read the window's accessible name.
//! - **U-C7-7 (draft, not filed)** rune-kit: a horizontal TabStrip's viewport has no scroll
//!   margin (its vertical one has `scroll_margin(fade)`), so a tab revealed at an end, by a
//!   press or `scroll_into_view_in`, lands under the edge fade, half hidden. Ask: the same
//!   margin across. Here: a newly active tab is revealed a fade's width clear of either edge in
//!   one jump (`scroll_to_instant`), from the offset its markers read (`probe.rs`), at the tick
//!   after it drew: a frame after the selection. Rune's reveal is only the fallback for a tab
//!   that never draws, and a tab pressed to start a drag is left to rune-kit's. Any wheel over a
//!   tab before the jump lands cancels it; in the frame the jump goes out, a sideways wheel jumps
//!   too, from the strip as shown, so the user's scroll lands last (review 2), and a gesture the
//!   tab took scrolls to its end (review 3). The active tab's marker, keyed by its id and name,
//!   reports its first layout, which brings the tick a host's selection or rename from a key
//!   handler needs in a still window.
//! - Reused: U3 (a drag's paths only at the drop: under Rune's binder a drag that opens tabs is
//!   told only then, and opens after the tab it dropped on), U-P1-1 (zones found by key), U-C6-5.

mod build;
mod closed;
mod data;
mod drops;
mod intent;
mod new_tab;
mod probe;
mod state;
mod wheel;

#[cfg(test)]
mod tests;

pub use build::BrowserTabs;
pub use closed::{CLOSED_TABS_KEPT, Closed, ClosedTabs};
pub use data::{Accepts, LABEL_CHARS, Tab, TabId, TabsData, TitleFormat, elide_end, window_title};
pub use intent::{DropSite, TabsIntent, Via};
pub use new_tab::{NewTabButton, TabsLabels};
pub use state::{BrowserTabsState, HOVER_DELAY, Polled};

/// The key the strip's elements start with unless the host sets one
/// ([`BrowserTabsState::new`]).
pub const DEFAULT_KEY: &str = "tabs";

/// Tab `id`'s key in the strip keyed `key`: its drop zone's key for X2's binder, and what
/// scripts tap.
pub fn tab_key(key: &str, id: TabId) -> String {
    format!("t-{key}-{id}")
}

/// The New tab button's key in the strip keyed `key`.
pub fn new_tab_key(key: &str) -> String {
    format!("{key}-new")
}

/// The strip's sideways scroll viewport's key (rune-kit's `<key>-scroll`).
pub fn scroll_key(key: &str) -> String {
    format!("{key}-scroll")
}
