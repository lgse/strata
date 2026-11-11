//! InlineRenameEditor (C5): the in-place rename field that C1e's rename slot hosts, the same
//! in List (C4), Icons (C3) and Columns (C2).
//!
//! The editor holds the interaction: the first selection, the keys that end an edit, the blur,
//! the row leaving the viewport or the listing, the validator's message and what assistive
//! technology hears. Renaming, moving, conflicts and the rules for a name are the host's
//! (Strata's): the editor asks for them with [`RenameIntent`]s and never touches an entry.
//!
//! # Hosting
//!
//! The host keeps one [`RenameState`] per window and gives it to a [`RenameEditor`] by
//! accessor:
//!
//! ```ignore
//! let editor = RenameEditor::new(|s: &App| &s.rename, |s: &mut App| &mut s.rename)
//!     .validate(|s: &App, id, name| s.name_error(id, name)) // Strata's rules and messages
//!     .on_intent(|s: &mut App, intent, tx| s.renamed(intent, tx));
//! let list = Collection::new(KEY, ..)
//!     .rename(|s: &App| s.rename.renaming_in(KEY), editor.slot());
//! layer.extend(editor.message(s, look)); // the window's overlay layer (`rune_kit::root`)
//! // each poll:
//! let polled = s.rename.tick(KEY, &s.listing, &s.coll);
//! ```
//!
//! - **Opening.** On C1b's `Intent::BeginRename`, or its own F2 or Ctrl+R, the host calls
//!   [`RenameState::begin`] with the collection's key, the entry and its name, then reveals
//!   the row (`CollectionState::reveal`, which reaches unmounted rows too). A new item's
//!   default name opens with [`Initial::All`].
//! - **Every poll** it calls [`RenameState::tick`] with the key, listing and collection of the
//!   edit's scope ([`RenameState::scope`]), builds the view again when the result changed,
//!   and acts on its intents.
//! - **Intents.** [`RenameIntent::Rename`] and [`RenameIntent::Move`] (a typed path) are the
//!   host's to carry out; [`RenameIntent::Keep`] renames nothing. An ending [`By::Next`] or
//!   [`By::Previous`] (Tab, Shift+Tab) asks the host to move the selection to the neighbour
//!   and open the editor there, unless the entry was at that end.
//! - **Keys.** While [`RenameState::has_keyboard`], the host leaves alone the keys the field
//!   passes on: its view shortcuts (Ctrl+1–3) and F2 and Ctrl+R, as Strata's do. A host that
//!   changes the listing from a press (navigating to a sidebar place) calls
//!   [`RenameState::commit`] first: the blur reaches `tick` a frame later.
//! - **Per view.** C4: on the base collection, so the editor takes the primary column's
//!   label. C3: `.align(Align::Center)` and the caption's font. C2: through
//!   `ColumnsView::collection`, so only the column whose key the edit names answers.
//!
//! # Behaviour
//!
//! - **The first selection** ([`initial_selection`]): a file's stem, up to its last `.`
//!   (`archive.tar` of `archive.tar.gz`); the whole name of a folder, of a name whose only `.`
//!   leads it (`.bashrc`) and of a name without one.
//! - **Ending an edit.** Enter (plain or with Shift), the field losing the keyboard, Tab,
//!   Shift+Tab and [`RenameState::commit`] all end it by one rule: an empty field, the old
//!   name (compared exactly, spaces kept) or a name the validator refused keeps the old name
//!   ([`Kept`]); a name holding a `/` is a [`RenameIntent::Move`]; anything else a
//!   [`RenameIntent::Rename`]. Escape, [`RenameState::cancel`], the entry leaving the listing
//!   or being renamed elsewhere, and its row scrolling out keep the old name. Ctrl, Alt and
//!   Super with Enter or Escape pass on to the host; Shift changes neither. A held Enter or
//!   Escape ends one edit; a held Tab walks on.
//! - **Page keys stay in the field.** Page Up and Page Down (with or without Shift) do nothing
//!   in it, as Up, Down, Home and End stay its own: passed on, Rune would page the listing, the
//!   row would leave the viewport and the edit would end, dropping the typed name.
//! - **The blur.** Rune has no focus-change hook (upstream #106), so `tick` notices the field
//!   lost the keyboard, as C6's entry does. Another window taking activation leaves the field
//!   focused (C6 probe P2), so the edit stays open, as Strata's does. C24's autoscroll shield
//!   takes the keyboard when autoscroll starts, so a middle press in the listing commits the
//!   edit (probe P10), as Strata's capture-phase click does for any button.
//! - **The row leaving the viewport.** Rune keeps a focused virtual row mounted out of view,
//!   so the field would go on taking keys unseen. The edit ends ([`By::Scrolled`], keeping the
//!   name) once the row has left [`CollectionState::visible_range`] after it had shown there,
//!   as Strata's ends when its row unmaps. The rule waits for the row to show, so a reveal
//!   still gliding to it doesn't end it.
//! - **Focus afterwards.** The editor carries `El::focus_return` to its collection, so when
//!   Enter, Escape or a last Tab removes the field, the keyboard goes back to the listing
//!   (probe P2). It acts on every way the field goes, and only while the field has the
//!   keyboard, which `Tx::focus` from each path wouldn't.
//! - **Input methods.** Rune gives Enter and Escape to a composition, and the editor passes
//!   every key while one is open. A blur mid-composition keeps only the committed text.
//! - **Paste.** The field is single-line, so a pasted line break or tab is dropped (Rune).
//! - **Validation.** The validator runs as the text changes; an empty field is never an error.
//!   A refused name turns the field's outline `danger`, marks it invalid, gives it the message
//!   as its description, and shows the message ([`RenameEditor::message`]): a polite live
//!   region (`status`) anchored under the field, in the host's overlay layer. Inside the row it
//!   was clipped to the row (probe P4), whose height it mustn't change. Rune flips it over the
//!   field only at the window's edge, not the listing's: under the listing's last visible row
//!   it hangs past the listing, over whatever lies below. It takes no presses: a press on it
//!   reaches what lies under it (the row below) and ends the edit, as any press outside the
//!   field does in Strata.
//! - **The field's menu.** The field has its own `on_secondary_press`, which hears a
//!   right-click, the Menu key and Shift+F10, so Rune's native field menu (which never shows on
//!   Linux at the pinned rev) never opens. It asks for [`RenameIntent::FieldMenu`]; the menu is
//!   C21's. A host that opens one, from the intent's handler, calls
//!   [`RenameState::menu_opened`], so the field losing the keyboard to it ends nothing, and
//!   [`RenameState::menu_closed`] after. The hold ends on its own too, on the first `tick` that
//!   finds the field with the keyboard: the menu gave it back (a host that forgot
//!   `menu_closed`), or never took it (no menu opened, or one that doesn't take the keyboard).
//!   A menu that takes the keyboard has it by the first `tick` after: it opens and takes it in
//!   the commit of the press or key that asked for it, which comes before the next poll
//!   (probed with rune-kit's context menu, as C21's is built, from a right-click and the Menu
//!   key).
//! - **The box.** The field's box (its fill and outline, a row tall) is a `textbox`, as
//!   rune-kit's text inputs are: Rune routes a press anywhere in it to the field, so a press in
//!   its margin, off the text line, moves the caret rather than ending the edit. A right-click
//!   there is the field's menu press too. That routing is for primary presses: the box also
//!   takes the middle button, or a middle press in it would walk up to the collection's root
//!   and open the row being renamed in a new tab. Back and Forward stay the host's.
//! - **Spellcheck** is off: a file name isn't prose. Rune has no checker on Linux anyway.
//! - **Upstream #180.** Rune syncs focus into every field of the last view, the one just
//!   removed too, so [`RenameState::edit`] is always present: a spare while no edit is open.
//!
//! # Accessibility
//!
//! The field is a text box named "Rename" (Strata's; [`RenameEditor::label`] replaces it), its
//! value the name being typed, focused, and invalid with the message as its description while
//! the validator refuses the name. The message is a polite live region (`status`), so a screen
//! reader says each new one. The row stays an option, selected, named by the old name.
//!
//! # Tokens
//!
//! Kit only: `c.bg_base` behind the field, a 1 pt `c.border_focus` outline (`c.danger.fg` while
//! refused), `c.text_base` and `k.selection()` for the text, caret and selection, `radius.sm`;
//! the message in `ty.small` and `c.danger` on `c.bg_base`. No strata-kit slot.
//!
//! # Deviations
//!
//! - **D1: Rune's `text_input`, not rune-kit's `TextInput`** (owner ruling). The field's menu
//!   press must sit on the input node itself (Rune gives a field's own menu precedence over its
//!   ancestors' handlers), and rune-kit's `TextInput` takes no handler for its inner field
//!   (Rune draft U-C5-1). Its look is rebuilt from Kit tokens.
//! - **D2: a visible message** (owner ruling; the work package and plan ask for it). Strata
//!   shows only the error style and the accessible description ("no tooltip shows").
//! - **D3: the edit ends when its row leaves the viewport** (owner ruling), not at GTK's unmap,
//!   which comes a few rows later.
//! - **D4: a press that leaves the field the keyboard doesn't commit.** Strata commits on a
//!   press with any button anywhere outside the field (a capture-phase click); here only a
//!   press that takes the keyboard does. At the pinned rev these leave the field focused
//!   (`runtime/input/pointer.rs`, `pointer_down`; `gesture.rs`, `button_down`; `secondary.rs`):
//!   - a primary press on an unfocusable element with `preserve_focus_on_press`, or inside
//!     one (C24's bar, toolbar buttons built that way);
//!   - a primary press on a link in text;
//!   - a primary press on an element with `keeps_text_selection`, or inside one;
//!   - a primary press on an unfocusable element inside another `textbox` (rune-kit
//!     `TextInput`'s copy and clear buttons);
//!   - a primary press outside an open modal that dismisses on an outside press: it closes
//!     the modal and goes no further;
//!   - any press with another button: secondary, middle, Back, Forward (probed: a right-click
//!     or middle click on another row leaves the field focused).
//!
//!   Where the host commits for each:
//!   - **Presses on the listing.** C1e's `Collection::on_press` hears every primary, middle
//!     and secondary press on the listing outside the editor, so the host commits there.
//!     Back and Forward never reach it (C1e takes only the middle button besides the primary
//!     and secondary); the host commits from its own Back and Forward handling.
//!   - **Presses elsewhere.** Preserve-focus controls, links, `keeps_text_selection`
//!     elements and other text inputs sit outside the listing (toolbars, headers, the
//!     sidebar, the preview), so the host commits from those controls' own handlers.
//!   - **A dismissed modal** is one the host opened, and closing it is its own handler's.
//!
//!   Rune draft U-C5-2.
//! - **D5: no field menu on Linux until C21** (owner ruling). Strata's GTK entry shows Cut,
//!   Copy, Paste, Delete, Select All and Insert Emoji.
//! - **D6: a pasted tab or line break is dropped** (Rune's single-line field).
//! - **D7 was a fallback, not needed.** It would have covered focus landing on nothing after
//!   the editor closed; probe P2 passed, and the keyboard goes back to the collection.
//! - **D8: a name holding a line break draws on two lines** in the single-line field, clipped
//!   to the row (probe P7). Names with line breaks are legal on Linux, and rare.
//! - **No editor rect.** The work package has the editor report its rect for
//!   `keep_on_screen`; Rune's `keep_on_screen` pins headers, and C1e's reveal by index reaches
//!   unmounted rows, which a rect can't.
//!
//! # Bullets
//!
//! Scripts are in `tests/scripts/c5/`; "unit" is `rename/tests.rs`.
//!
//! | Bullet (`operations/rename`) | Checked by |
//! | --- | --- |
//! | A file's editor opens with the name before the last `.` selected; a folder's… | `stem-selection`; unit `a_files_stem_…` |
//! | A name whose only `.` is its first character… opens fully selected | `dotfile-selection`; unit `a_name_whose_only_dot_…` |
//! | Ctrl+A in the editor selects only the field's text… | `ctrl-a-in-editor` |
//! | Ctrl+1, Ctrl+2, and Ctrl+3 reach the editor… | **partial**: `ctrl-digits-stay-in-editor`; the host's shortcuts check [`RenameState::has_keyboard`] |
//! | For an item outside a local folder… `/`… error style… | `slash-outside-local` (the message is the host's validator's) |
//! | In a local folder, a name containing `/` is a path… | `typed-path-reports-move`; unit `a_name_with_a_slash_…` |
//! | For `.` or `..` the description is… | `reserved-names`, `blank-name` |
//! | An empty field is not styled as an error… | `empty-is-not-an-error`; unit `an_empty_field_…` |
//! | Enter or a click outside the field… commits a valid changed name | `commit-enter-and-click-away` |
//! | Tab and Shift+Tab commit too… | `tab-commits` |
//! | Keyboard focus moving to another widget… commits too | `focus-move-commits`; unit `a_press_on_another_field_…` |
//! | When another window… takes activation, the editor stays open… | `window-deactivate-keeps-editing` |
//! | Returning to the window keeps focus in the field, and Enter then commits | `window-return-enter-commits` |
//! | Clicking inside the field moves the caret and keeps editing | `click-inside-keeps-editing` |
//! | Escape closes the editor and keeps the original name | `escape-cancels` |
//! | Committing an empty, invalid, or unchanged name closes the editor… | `empty-invalid-unchanged-keep`; unit `an_empty_an_unchanged_…` |
//! | A click-away onto a sidebar place or another folder commits… and still navigates | **partial**: `click-away-still-navigates`; navigation is the host's |
//! | With the editor open on an item… Tab renames it… next item, stem selected | `tab-renames-and-moves-on` (the host moves the selection) |
//! | Shift+Tab does the same toward the previous item | `shift-tab-moves-back` |
//! | Tab or Shift+Tab with the name unchanged… moves the editor… | `tab-unchanged-moves-on` |
//! | Tab with an invalid name keeps the original name and still moves… | `tab-invalid-moves-on` |
//! | Tab on the last item, or Shift+Tab on the first… keyboard focus stays in the listing | `tab-at-edge-closes`; unit `closing_from_the_field_…` |
//! | The committed row does not pull the cursor back… | **partial**: `tab-renames-and-moves-on`; the order of the rename landing is the host's |
//! | Tab is taken only while keyboard focus is in the rename field… | `tab-outside-field` |
//! | Tab and Shift+Tab move the rename this way in Columns, List, and Icons… | **partial**: one editor in every host; `c5-list` scenes (C4). C2 and C3 adopt it |
//! | With a long name in a narrow Columns view… caret and extension visible | **partial**: Rune's field scrolls; review scene `c5-long` (no geometry tests) |
//! | Editor text is centered in Icons and left-aligned in List and Columns | **partial**: [`RenameEditor::align`]; review scene `c5-centered`. C3 sets it |
//! | Opening the editor on a row that is already fully visible does not scroll | **partial**: C1e's reveal moves only as far as needed (scroll offsets aren't tested) |
//!
//! | Bullet (`operations/rename/typed-paths`) | Checked by |
//! | --- | --- |
//! | F2 on an existing file and typing `docs/` styles the field as an error… | **partial**: `typed-path-folder-refused`, `typed-path-folder-commit-keeps`; the rule and message are Strata's |
//! | `a//b`, `/abs`, or `a/ /b` reads "Enter a name"… | **partial**: `typed-path-blank-segment`, `typed-path-reserved`; the rules are Strata's |
//!
//! Not C5's:
//!
//! - **Strata:** starting a rename (F2, Ctrl+R and their modifiers, the menus, Properties, the
//!   chooser); the committed name showing at once; the "Unable to rename item" dialog and focus
//!   after it (with C15); every filesystem and name-rule bullet (vfat, SMB, case renames,
//!   reserved and overlong names); child columns staying open; renaming to a dot name with
//!   hidden files off; the reveals after a commit; Tab onto a taken name; every placement, undo
//!   and after-the-move bullet of `typed-paths`.
//! - **C1b:** the slow second click and what cancels it.
//! - **C2:** the size badge hidden while editing (`RowState::renaming` says so).
//!
//! In process, without bullets (`rename/tests.rs`): a row scrolled out, the entry leaving the
//! listing or renamed elsewhere, the field menu and its hold with a real rune-kit menu (opened
//! by a right-click and by the Menu key), the hold ending without `menu_closed` and when no
//! menu takes the keyboard, a press in the box's margin, a middle press in the field and its
//! margin, a press on the message, page keys in the field, Shift+Escape, `menu_closed`'s
//! refocus, a `tick` with another collection's scope, #180, a composition ending in a blur,
//! and paste.
//!
//! # Rune
//!
//! **Rune 2a44156: partly.** `text_input` covers the editing, `set_selection`, undo, IME
//! (Enter and Escape to the composition), single-line paste, Ctrl+A and Ctrl+Z in the field, the
//! keyboard kept across window deactivation, focused virtual rows kept mounted,
//! `El::focus_return`, `invalid`, `description`, `live_region`, `on_input_change` and a field's
//! own `on_secondary_press`. Not covered: a focus-change hook (#106), a field menu on Linux,
//! an accessor safe after its field is removed (#180). `El::dir` and Left and Right across
//! bidi text in a field (cded95f) are in the pin, for right-to-left names.
//!
//! ## Rune drafts (not filed)
//!
//! - **U-C5-1: rune-kit's `TextInput` takes no handlers for its inner field.** Its builder
//!   forwards `on_edit`, `on_submit`, `on_key` and `on_change` to the inner `text_input`, but not
//!   `on_secondary_press`, and the `El` it builds is the box around the field. Rune gives a
//!   field's own menu precedence over its ancestors' `on_secondary_press`
//!   (`runtime/input/secondary.rs`, `secondary_target`), so a handler on the box never runs.
//!   Repro (headless, `strata-spikes/c5-probes/tests/u_c5_1.rs`):
//!   `TextInput::new(&edit).build(k).on_secondary_press(f)`, right-click the field: `f` never
//!   runs. Ask: forward `on_secondary_press` (or expose the inner field's `El`).
//! - **U-C5-2: no "press outside" for a field.** A press that leaves a field focused (D4's
//!   list: a `preserve_focus_on_press` control, a link, a `keeps_text_selection` element, a
//!   part of another `textbox`, a press dismissing a modal, any press with another button)
//!   tells it nothing, so an inline editor can't end on a press
//!   elsewhere, as a GTK capture-phase click or a web `pointerdown` listener on the document
//!   can. Repro: a focused `text_input` beside a `preserve_focus_on_press(true)` button; press
//!   the button, or right-click beside the field: no handler of the field's runs, and it stays
//!   focused. Ask: an `El::on_press_outside` for an element and its subtree, any button.
//! - Reused: #106 (focus change; C6's U-C6-3), #180, #225 (a focused field
//!   redraws every frame on Wayland), U-C6-7 (scripts have no input-method event), S5's popup
//!   menus on Linux.
//!
//! Work package: C5, `docs/work/phase-1-leaves/C5-inline-rename.md`.

#![warn(missing_docs)]

mod editor;
mod intent;
pub mod keys;
mod state;
mod stem;

#[cfg(test)]
mod tests;

pub use editor::RenameEditor;
pub use intent::{By, Kept, Polled, RenameIntent};
pub use state::RenameState;
pub use stem::{Initial, initial_selection};

#[cfg(doc)]
use crate::collection::element::CollectionState;
