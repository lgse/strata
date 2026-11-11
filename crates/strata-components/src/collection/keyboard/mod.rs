//! Keyboard navigation over the collection (C1c).
//!
//! [`KeyMachine`] turns keys on a focused collection into selection operations on its
//! [`Selection`] and [`Outcome`]s: scroll the cursor into view, open an entry, the cursor
//! reached the collection's edge, nothing to select. Every move goes through
//! [`CollectionGeometry::neighbor`] and [`CollectionGeometry::page_step`], so the list, the icons
//! grid and each Columns column behave the same. It is plain data with no Rune elements, driven
//! through a [`Surface`]; [`Keyboard`] puts it on the collection element's keyboard seam (C1e).
//! It follows Strata at `upstream/main` 35ee677ab: `src/app/navigation/cursor.rs` and
//! `visual.rs`, `src/ui/browser/view_selection.rs`, `src/ui/window/key_routing.rs`,
//! `src/ui/window/keyboard/items.rs` and `items/tenxer_cursor.rs`, and the tables in
//! `src/ui/keys/bindings/`.
//!
//! # The two key maps
//!
//! [`KeyMap::default_map`] and [`KeyMap::tenxer`]; a host rebinds through [`KeyMap::bind`].
//!
//! | Keys | Default map | 10xer map |
//! | --- | --- | --- |
//! | Arrows | [`Command::Select`] a step | [`Command::Cursor`] a step |
//! | `h` `j` `k` `l` | The arrows, with [`Letters::Arrows`] | The arrows' moves |
//! | Shift+arrows | [`Command::Extend`] a step (Up, Down: also Ctrl, Alt, Super) | Swallow |
//! | Ctrl+Left, Ctrl+Right | [`Command::Nudge`]: the cursor a step, silent at an edge | |
//! | Ctrl+Up, Ctrl+Down | Select the first, the last | Cursor the first, the last |
//! | Home, End | Select the first, the last | Cursor the first, the last; also Shift+G |
//! | Shift+Home, Shift+End | Extend to the first, the last | |
//! | Page Up, Page Down | Select a page (Super ignored) | Cursor a page; Ctrl+B, Ctrl+F too |
//! | Shift+Page Up, Shift+Page Down | Extend a page (Super ignored) | |
//! | Ctrl+U, Ctrl+D | | Cursor half a page |
//! | Space | | [`Command::ToggleAdvance`] |
//! | Ctrl+Space | [`Command::Toggle`] | Toggle |
//! | `v`, Shift+V | | [`Command::Range`] |
//! | Ctrl+A | [`Command::SelectAll`] | Select all |
//! | Ctrl+R | | [`Command::Invert`] |
//! | Escape, Shift+Escape | [`Command::Clear`] | The host's Escape ladder |
//! | Enter | [`Command::Activate`] | Activate; also `o` |
//! | Shift+Enter | Not taken: the host's (Open in new window) | Not taken |
//!
//! 10xer's Shift+arrows are [`Command::Swallow`]: `v` and `V` are its range keys. Its full
//! pages also take Ctrl+Page Up and Ctrl+Page Down.
//!
//! Keys match by Rune's chords, following the layout: see [Layouts](self#layouts). Modifiers
//! match exactly except where Strata's matchers ignore some (`src/ui/window/keyboard/items.rs`):
//! the extra modifiers in the table. So Ctrl+Up with Alt or Super added is left alone, and
//! with Shift added it extends, as Shift+Up with Ctrl held does; it never jumps. A binding of
//! the exact chord wins over a tolerated match ([`KeyMap`]).
//!
//! # The rules
//!
//! - **Select** moves the cursor and selects only its entry, and the anchor follows: plain
//!   arrows, Home, End and the page keys all re-anchor (lgse/strata#2026).
//! - **Extend** selects from the anchor to the target. With nothing selected, the anchor first
//!   moves to the cursor; a step then selects the cursor's entry alone without moving, so
//!   Shift+Down with nothing selected selects one entry and the next selects two
//!   (`extend_selection`), while a page or a jump extends from the cursor at once
//!   (`extend_page_selection`). After Escape the range so starts at the cursor. With entries
//!   selected but no anchor or cursor (a marquee's), the anchor goes to the cursor, or to the
//!   end the move starts from, and the step moves. A sideways step with no neighbour isn't
//!   taken: Columns' Shift+Right and Shift+Left are C2's, and List does nothing.
//! - **Cursor** moves the cursor and keeps the selection, and the anchor stays. During a 10xer
//!   range it walks the range instead.
//! - **With no cursor**, Down, Right, Page Down and Shift+Down start at the first entry; Up,
//!   Left, Page Up and Shift+Up at the last.
//! - **Steps** go to [`CollectionGeometry::neighbor`]. Where it has none, Left and Right cross
//!   a row end with [`KeySettings::wrap_rows`] (the icons grid, lgse/strata#2044); otherwise the
//!   machine emits [`Outcome::Edge`] and changes nothing. What an edge does (the sidebar, the
//!   header, the next column, the parent folder, nothing) is the host's and the view's, as is
//!   Keep arrows in file list. An empty collection reports an edge for every step, so Up from
//!   an empty folder can still reach the header. A held key reports an edge once, when it
//!   arrives there; a new press reports it again. Ctrl+Left and Ctrl+Right report none: Strata
//!   sends only a plain Left to the sidebar.
//! - **Pages** move by [`CollectionGeometry::page_step`] entries, half a page by half that
//!   (at least one), clamped to the first and last entry. The page size is the geometry's: the
//!   rows that fit minus one, times the tiles per row in Icons.
//! - **Escape** clears the selection and keeps the cursor. With nothing selected it isn't
//!   taken, so the view can use it (Columns closes its deepest column).
//! - **Ctrl+A and Escape** turn keyboard mode on ([`Outcome::Keyboard`]), as Strata's
//!   `keyboard_navigation` does; a move turns it on through the element.
//! - Every move emits [`Outcome::Reveal`] for the cursor's entry.
//!
//! # 10xer
//!
//! - **Ranges.** `v` starts a range at the cursor (with no cursor, there is nothing to
//!   select) that adds to the selection as it was; `V` one that removes from it. Moves and
//!   pages walk it; the same key again, or [`KeyHandle::end_range`] (the host's Escape
//!   ladder, leaving the mode), ends it and keeps what it selected. Any other selection
//!   command ends it too. A primary press ends it at once ([`Keyboard`]'s `pointer_seen`); a
//!   secondary press (a context menu) doesn't. Any other change to the selection the machine
//!   didn't make (a sync that dropped entries) ends it at its next key, by the selection's
//!   revision. [`KeyHandle::range`] says whether one is
//!   active, for the `VISUAL`/`UNSET` footer (C18) and Columns' mirroring (C2).
//! - **Space** adds the cursor's entry (even over a load's automatic pick), or removes it,
//!   then moves the cursor to the next entry in display order. Inside a range, it flips the
//!   cursor's entry against the range and doesn't move, and later steps of the range keep the
//!   flip ([`Selection::toggle_in_run`], Strata's `toggle_visual_cursor`).
//! - In an empty folder, Space, `v`, `V`, Ctrl+A and Ctrl+R emit [`Outcome::NothingToSelect`].
//! - Commands from outside the collection (a `g g` chord step from C18, the Escape ladder, a
//!   window-level Ctrl+A) go through [`KeyHandle::run`] and run at the machine's next
//!   [`KeyMachine::tick`].
//!
//! # Type-ahead
//!
//! With [`Letters::TypeAhead`], a plain printable key the map doesn't bind finds the next entry
//! whose name starts with what was typed, case-insensitively, wrapping past the end, and
//! selects it. A character within [`KeySettings::type_ahead_timeout`] of the last extends the
//! prefix; the same letter again steps to the next entry starting with it. Space joins an open
//! prefix only. Time is the key's own clock (`KeyInput::now`), so tests run on a virtual
//! clock and no timer is needed.
//!
//! **Strata has no counterpart.** Its plain letters start type-to-search, which opens the
//! filter (`type_to_search_query` in `src/ui/window/key_routing.rs`; C18), or with Type to
//! search off act as `h j k l`. So type-ahead is opt-in and off by default, as the work package
//! asks.
//!
//! # Layouts
//!
//! Keys match with C1e's `KeyInput::matches`, which is Rune's `Chord::matches_tx_on` over the
//! key's own transaction: a letter follows the key's label under the active layout (Dvorak's
//! `j` is the key labelled J), and a key that types a non-Latin character (Cyrillic, Greek)
//! falls back to the character its key types unmodified, else the US key in its place. So
//! `h j k l` and Ctrl+A follow Strata's layout rules (`docs/keyboard-shortcuts.md`).
//!
//! **Deviation from Strata:** Strata first tries the Latin character the same key types in
//! another configured layout, and uses US-QWERTY only when no configured layout has one. Rune
//! can't list the configured layouts, so the US fallback applies at once.
//!
//! Unit tests build a `KeyInput` with no transaction, where Rune reads the event alone, so the
//! layout fallback is checked by a headless script (`tests/scripts/c1/keyboard-layout.json`)
//! rather than by a copy of Rune's matcher here. It checks the Ctrl+A part of its bullet only:
//! Ctrl+C and Ctrl+K on a Russian layout are the host's commands, not the collection's.
//!
//! Rune draft U-C1c-1 asks for a way to do it in process: a public
//! `Chord::matches_with(event, base, physical, platform)` (private at the pinned rev,
//! `crates/rune/src/input/chord.rs`), or a public `Runtime::physical_key_input` (`pub(crate)`,
//! `crates/rune/src/runtime/input/physical.rs`). Without either, a layout case
//! the gallery page can't reach (10xer's `о` and `л` as `j` and `k`) has no test.
//!
//! # Decisions
//!
//! - **Return with no cursor** selects the first entry and opens it (Strata's grouped-List
//!   rule: Down, Return and Shift+Down start at the first row shown).
//! - **10xer Ctrl+Up and Ctrl+Down** move the cursor only, as the 10xer map's other jumps do.
//! - **Ctrl+Home and Ctrl+End** stay unbound.
//! - **A repeated key** (key repeat) counts as a press. Rune's scroll glide keeps its speed
//!   across requests, so a held arrow scrolls as one motion.
//! - **Plain printable bindings** apply whatever [`Letters`] says; only aliases and type-ahead
//!   follow it. The default map binds no plain printable key.
//! - **Ctrl+A in an empty collection** emits [`Outcome::NothingToSelect`] in both maps; the
//!   default map's host ignores it.
//!
//! # Deviations from Strata
//!
//! - **Shift+Enter** isn't taken. Strata's activation accepts Shift on Enter
//!   (`ENTER_ANY_SHIFT`), but its file list binds Shift+Return to Open in new window, so here
//!   it goes on to the host, which runs that command.
//! - **Ctrl+Shift+Up and Ctrl+Shift+Down** in Icons and List replace the selection with the
//!   range from the anchor, as Shift+Up does. Strata leaves them to GTK, which adds the range to
//!   the selection.
//! - **The non-Latin layout fallback** goes to the US key at once (see [Layouts](self#layouts)).
//!
//! # Left to the host
//!
//! - **Super+Up and Ctrl+Super+Up on the first row** aren't taken. Strata focuses the pane
//!   header with them (`selection_navigation`, Up ignoring Ctrl and Super); a host that wants
//!   that binds them, as it handles [`Outcome::Edge`] for a plain Up.
//!
//! # Settings and the shared handle
//!
//! [`KeySettings`] reach the machine the one way C1e gives controllers settings: the host
//! passes them with `Collection::settings`, current each time it builds the view, and the
//! machine reads them with `Ctx::settings` (the defaults when the host passed none). What the
//! host must reach between builds, the 10xer range and commands that start outside the
//! collection, lives in a [`KeyHandle`] the machine and the host share, because the element
//! keeps its controllers boxed where no accessor reaches. That handle is a recorded deviation
//! from the accessor rule; C1e's `KeyController::tick` names it as the way to queue commands.
//!
//! Work package: C1c, `docs/work/phase-2-composites/C1c-keyboard-navigation.md`.

#![warn(missing_docs)]

mod adapter;
mod command;
mod keymap;
mod machine;
mod settings;
mod type_ahead;

#[cfg(test)]
mod tests;

pub use adapter::{Keyboard, NothingToSelect};
pub use command::{Command, RangeKind, Target, Vertical};
pub use keymap::{Binding, KeyMap};
pub use machine::{KeyMachine, Outcome, Surface};
pub use settings::{KeyHandle, KeySettings, Letters};

#[cfg(doc)]
use crate::{collection::selection::Selection, contract::CollectionGeometry};
