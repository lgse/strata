//! Context menus (C21): where Strata's menus open, how the keyboard moves through them, and
//! the text-field menu, on rune-kit's `Menu`.
//!
//! Work package: C21, `docs/work/phase-1-leaves/C21-context-menus.md`.
//!
//! **Rune dev 5d421fd: partly.** rune-kit's `Menu` gives pointer menus, placement inside the
//! window (flip, slide, shrink and scroll), Up, Down, Home, End and type-select, Escape and a
//! press outside, check items, submenus and an open menu holding the keyboard. It leaves out a
//! field menu on Linux, a way to give rune-kit's text inputs one, submenus from the keyboard,
//! a menu's name, and popup surfaces. C21 fills what it can (below) and drafts the rest.
//!
//! # What the host does
//!
//! Which items a menu shows, and what they do, is Strata's policy: the host builds a
//! [`MenuModel`] for the target and runs [`MenuIntent::Activate`]. C21 holds no I/O.
//!
//! ```ignore
//! // The collection: C1b's right-click and C21's keys both arrive as Intent::ContextMenu.
//! coll.controllers.keyboard = Some(Box::new(MenuKeys::new(Keyboard::default(), AnchorEdge::Center)));
//! Collection::new(..).on_intent(|s: &mut App, intent, _| {
//!     if let Intent::ContextMenu { target, anchor } = intent {
//!         let target = target.map_or(MenuTarget::Background, MenuTarget::Entry);
//!         let model = s.policy.menu_for(&target);
//!         s.menu.open(target, Point::new(anchor.x, anchor.y), model);
//!     }
//! });
//! // The view: the menu's element anywhere, its layer at the root.
//! let menu = ContextMenu::new("menu", |s: &App| &s.menu, |s: &mut App| &mut s.menu)
//!     .name(s.labels.menu.clone())
//!     .on_intent(|s: &mut App, intent, tx| s.run(intent, tx))
//!     .build(s, &s.look, &mut layer);
//! ```
//!
//! - **Pointer.** C1b selects an unselected entry alone (or keeps the selection it lands in)
//!   and sends [`Intent::ContextMenu`](crate::collection::element::Intent::ContextMenu) at the
//!   press. A sidebar row (C8's `SidebarIntent::ContextMenu`) or a crumb opens the same way,
//!   with [`MenuTarget::Other`].
//! - **Keyboard.** [`MenuKeys`] wraps the collection's keyboard controller (C1c's). Menu and
//!   Shift+F10 open the cursor's entry's menu when the cursor is selected, else the
//!   background menu, at [`keyboard_anchor`]: the entry's right edge (Columns, Icons) or
//!   centre (List), vertically centred; the background menu at the viewport's centre. A cursor
//!   scrolled out of view is revealed first. Rune's own Menu-key handling would open the
//!   collection's `on_secondary_press` at the collection's bottom-left, outside its viewport;
//!   the collection's keys come first, so `MenuKeys` takes the keys there, the chords that
//!   open no menu included (Rune's rule would let Shift+Menu through to that press).
//! - **Text fields.** [`FieldMenu::hook`] is the field's `on_secondary_press`. Without one, at
//!   Rune 5d421fd, a right-click, Menu or Shift+F10 on a field asks for Rune's native field
//!   menu, which shows nothing on Linux. Rune hands the field's Menu key and Shift+F10 to the
//!   hook too, at the field's bottom-left. C5's inline editor sends `RenameIntent::FieldMenu {
//!   id, at }` instead: the host calls [`FieldMenu::open`] and `RenameState::menu_opened`, and
//!   `menu_closed` from [`FieldMenu::on_close`]. The host's poll calls [`FieldMenu::check`]:
//!   a menu whose field went (the entry deleted mid-rename) closes and tells `on_close`. A
//!   field's chords are Rune's: Menu plain or with Shift, never with Ctrl, Alt or Super.
//!   rune-kit's `TextInput`, `TextArea` and
//!   `TextField` take no `on_secondary_press` for their inner field, so they get no menu until
//!   Rune adds one (U-C21-2): C6's entry, and C17's, C18's and C20's fields, stay as they are.
//! - **The chooser's minimum size.** [`chooser_minimum`] measures the chooser's menus as drawn:
//!   a menu's width plus twice its widest submenu's, plus 16 pt, and the tallest submenu plus
//!   16 pt, required while menus stay in the window (S1, Q5; `3 × W + 16` when all are `W`).
//!
//! # Placement
//!
//! Menus open inside the window (U14). A menu opens below its point and flips above it, slides
//! sideways to stay in, keeps 8 pt from each edge, and with room neither above nor below
//! shrinks and scrolls (rune-kit's `anchored_fit`). A submenu opens beside its row, flips
//! sideways and slides up or down, but never shrinks or scrolls: [`chooser_minimum`] covers
//! it. A menu grows to show its widest row whole, up to 360 pt; rune-kit's rows would shrink
//! their labels instead, and it draws submenus 160 pt wide.
//!
//! # Keys
//!
//! An open menu holds the keyboard, its first enabled row focused. Up and Down skip separators
//! and insensitive rows and wrap; Home, End and type-select are Rune's. Enter and Space run the
//! focused row and close the menu, and the keyboard goes back to what had it. Escape or a press
//! outside closes it. C21 adds:
//!
//! - Right, Enter or Space on a submenu's row opens it with its first row focused; Left closes
//!   it and refocuses the row; Escape in it closes the whole menu. The keys open it as a click
//!   does, then send a Tab, which moves the keyboard into it (the next focus group in the
//!   menu's modal); its `focus_return` brings the keyboard back to its row as it closes. So
//!   the parent's rows stay clickable, and a press outside both closes the whole menu. (A
//!   submenu of its own modal would take the keyboard as well, but swallows a press outside
//!   it: a click on the parent's Copy would run nothing.) A submenu with no sensitive row
//!   opens too, so its greyed rows show, but the keyboard stays on its row: it has nowhere to
//!   go there, and the Tab would leave the menu (Rune's `tab_order` takes the next group's
//!   selected or first member, and a group of insensitive rows has none).
//! - Tab and Shift+Tab move as Down and Up, as GTK's menus do (Rune's Tab leaves a focus group).
//! - Ctrl+A stays in the menu, so the window's Select All doesn't run under it.
//!
//! After a pointer input, the focused first row shows its highlight on the first arrow, which
//! then doesn't move (rune-kit's NSMenu model; a deviation, below); Enter runs it at once.
//! After a key, the highlight shows at once.
//!
//! # Look
//!
//! rune-kit's compact menu, with Strata's corners (9 pt, at the look's roundness) and, with the
//! element glow on, the look's glow. The 1 pt edge that shows is rune-kit's floating
//! elevation's ring, in `border_base`; C21's own 1 pt `border_base` border lies under it and
//! adds nothing visible. The elevation's shadow stays under the glow: an El's shadows can't be
//! cleared. Field menus are the same
//! surface. Rows hover and highlight in `kit.c.hover`, as `Kit::interactive` paints.
//!
//! # Coverage
//!
//! Scripts are `tests/scripts/c21/<name>.json`; "test" is in
//! `crates/strata-gallery/tests/c21.rs`; "unit" is in `tests.rs`; "golden" is a scene's a11y
//! or PNG golden. `browser/context-menu`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | Right-clicking an unselected entry makes it the only selected entry… | script `pointer-open` (C1b selects; C21 opens) |
//! | Right-clicking an entry inside a multi-selection keeps the whole selection | script `pointer-multi` |
//! | Menu alone or Shift+F10 opens the menu of the focused selected entry… | script `menu-key`; unit `menu_and_shift_f10_…` |
//! | Ctrl+Shift+F10, Shift+Menu, and Ctrl+Menu open no menu | script `menu-chords`; unit `menu_and_shift_f10_…`, `the_collections_menu_chords_…`. In a text field the chords are Rune's: Shift+Menu opens the field menu (unit `chords_in_a_field_…`) |
//! | Menu or Shift+F10 with Hyper or Meta held opens the menu… | script `menu-key-super` (`"logo": true` on the keys); unit `menu_and_shift_f10_…`. Rune reports Super, Hyper and Meta as one `logo`. In a text field Super+Menu opens nothing (Rune's rule; unit `chords_in_a_field_…`) |
//! | With several entries selected, Menu or Shift+F10 opens the multi-selection menu… | script `menu-key-multi` |
//! | With nothing selected, Menu or Shift+F10 opens the active pane's background menu | script `background`; unit `the_keyboard_anchor_…` |
//! | A keyboard-opened item menu anchors to the entry… right edge… center in List | unit `the_keyboard_anchor_…`, `a_cursor_out_of_view_…`; test `the_menu_key_reveals_…`; golden `c21-keyboard-anchor` |
//! | While a text input has focus, Menu and Shift+F10 open the input's text-editing menu… | script `field-keys`; unit `menu_and_shift_f10_in_a_focused_field_…`, `chords_in_a_field_…` (Rune's chords: Shift+Menu opens it, Super+Menu doesn't). Partial: core text fields only (U-C21-2) |
//! | With a menu open, a secondary click on another entry… opens that entry's menu in one click | **Not met**: the click does nothing; the menu stays open on its old target (unit `a_right_click_outside_an_open_menu_…`). The menu's scrim takes no secondary press and nothing under it hears one (U-C21-6); reopening at the entry would also need a C1e seam (`secondary_press_at`, a follow-up) |
//! | A button release that reaches a menu… before any press inside it is ignored | script `release-without-press` (on the menu and on a submenu's row); unit `a_release_without_a_press_…` |
//! | Pressing the secondary button, moving onto a menu row, and releasing does not run the row… | the same: the secondary press lands outside the open menu (where it does nothing, as above), moves onto a row and is released there; the row doesn't run |
//! | After that, a full click on a menu row runs it | the same |
//! | A menu opens with keyboard focus on its first visible action… | script `first-focus`; unit `the_menu_opens_with_the_keyboard_…`; test `a_right_click_menu_has_the_keyboard_…`. **Partial**: after a pointer input the first arrow shows the highlight before moving (deviation); focus stays while Open With… is replaced in place (unit `a_model_update_that_replaces_…`), but a row added above the focused one moves it (U-C21-5) |
//! | Up and Down move between enabled actions, skipping separators and wrapping…; Home and End… | script `menu-keys`; unit `up_and_down_skip_…` |
//! | Enter or Space activates the focused action and closes the menu | scripts `menu-keys`, `space-activates`; unit `enter_and_space_…` |
//! | Escape closes the menu, keeps the selection, and returns focus… | script `escape`; unit `escape_closes_the_menu_…` |
//! | Ctrl+A while a menu is open leaves the menu open | script `ctrl-a`; unit `ctrl_a_leaves_the_menu_open` (a window Ctrl+A binding under the menu doesn't run) |
//! | On a submenu row…, Right, Space, or Enter opens the submenu with its first action focused | script `submenu-keys`; unit `right_opens_a_submenu_…`, `with_a_keyboard_opened_submenu_…` (the parent's rows still click; a press outside both closes the menu), `right_on_a_nested_submenu_…`, `right_on_a_submenu_with_no_sensitive_row_…`, `tab_inside_a_keyboard_opened_submenu_…` |
//! | Left in a submenu closes only the submenu and refocuses its row… | script `submenu-left`; unit `right_opens_a_submenu_…` |
//! | In the file chooser's menus, Up, Down, Tab, and Shift+Tab move… and wrap | script `chooser-tab` (every menu moves so) |
//! | Actions hidden for the current target leave no empty group or doubled separator | unit `an_empty_section_draws_no_separator_or_label` |
//! | An action with a shortcut shows it at the row's right | golden `c21-item` (PNG); the hint's text is Strata's |
//! | While the selection's types are looked up… Open With… insensitive, described… | mechanism: [`Item::enabled`] and [`Item::description`]; golden `c21-item` (a11y: disabled, the description). When is Strata's |
//! | Permanently delete shows CircleX with a red label and icon… | mechanism: [`Item::danger`]; golden `c21-item`. The icon is Strata's |
//! | Screen readers get each row's label as its name and its shortcut as its description | test `a_rows_shortcut_is_its_keyboard_shortcut_property`. **Partial**: the label is the name; the shortcut is the item's keyboard-shortcut property, not its description (owner's ruling) |
//! | Right-clicking an entry anywhere in a long, scrolled list shows its menu at the click | script `scrolled-list` (the target); test `a_right_click_in_a_scrolled_list_…` (the point) |
//! | In a small window, right-clicking an entry near the bottom edge opens the menu at the click | script `edge`; test `a_right_click_near_the_bottom_right_…` (the point); golden `c21-edge` |
//! | A menu opens below the click, and flips above it when the screen has no room below | golden `c21-edge`. **Partial**: the window's room, not the screen's (U14) |
//! | A menu with no room… above or below the click shrinks and scrolls… | golden `c21-scrolling` (in the window) |
//! | A menu opened near the screen's left or right edge slides horizontally… | golden `c21-edge` (the window's edge) |
//! | A menu's height is capped at the monitor height less 16 px… | **Partial**: the window's height less 16 pt (U14) |
//! | That menu extends past the window when needed and stays fully on screen | **Not met**: menus stay in the window (U14) |
//! | A keyboard-opened background menu is not held inside the window… | **Not met** (U14) |
//! | Menu rows are frameless on the theme surface, framed by a 1 px border, 9 px corners, and an accent glow… | goldens (the 1 px edge is rune-kit's elevation ring; C21's border under it changes nothing visible). **Partial**: rune-kit's elevation shadow stays under the glow |
//! | Right-click menus in text inputs use the same surface… as file menus… | script `field-edit`; goldens `c21-field`, `c21-field-empty`; unit `a_right_click_on_a_field_…` |
//! | Text-input menu styling covers the filter and search fields, location entry, inline rename… | script `field-rename` and tests `c5s_field_menu_holds_the_edit_open_…`, `an_entry_deleted_under_its_rename_field_menu_…` (C5's real editor on the gallery page). **Partial**: rune-kit's text inputs get none until U-C21-2 (C6's entry, C17, C18, C20); text previews are selectable text, whose menu is Rune's (macOS only) |
//!
//! Strata's, not C21's: which items each menu has and in what order (the creation rows, the
//! opening, clipboard, custom action, compress, customize, removal and Trash groups; Open
//! With… and Flatpak; Recent; `admin://`; `can-trash` and `can-delete`; Restore; Remove from
//! Recent; Open file location; the multi-selection header's wording; the background menu's
//! Paste sensitivity), every hint's text (10xer, German, rebinding), and each action's icon.
//! Elsewhere: a Ctrl+F result's menu is C17's target, a sidebar row's C8's (both open through
//! this module); the Columns highlight under an open menu is C1e's and C2's. Not here: the
//! niri and GTK 4.14 pointer and Open With… bullets (GTK and compositor behaviour; Rune opens a
//! menu on the secondary button's release), and the Hyprland, wlroots and X11 one.
//!
//! The field menu has Cut, Copy, Paste, Delete and Select All, GTK's text menu, which Strata
//! styles (#1087). Undo and Redo show with [`FieldMenu::undo`] (off by default: GTK's menu has
//! neither). Insert Emoji is left out: there is no emoji chooser.
//!
//! # Deviations
//!
//! - Menus stay inside the window (U14), so the screen-edge bullets above are partial or not
//!   met.
//! - The field menu moves neither the caret nor the selection when it opens, as GTK's doesn't
//!   (Rune's own moves the caret to the press).
//! - Submenus open on a click or the keys, not on hover: rune-kit's `MenuItem` has no hover
//!   hook (U-C21-4).
//! - A shortcut is the item's keyboard-shortcut property, not its description.
//! - Rows are keyed by their place in the menu (rune-kit's `{key}-item{n}`, counting the
//!   header, labels and separators), so a model update that adds or removes an entry above the
//!   focused row hands its key to another entry. The keyboard then lands on another action, or
//!   on the first row when that key is now a separator or a label. Replacing a row in place
//!   keeps it ([`ContextMenuState::update_model`]). Strata's multi-selection Open, added above
//!   Open With… once the lookup ends, moves it: reserve its row (insensitive) until then.
//! - A text field's menu chords are Rune's (`context_menu_key`), not the collection's: Menu
//!   opens the field menu plain or with Shift, and not with Super (Hyper, Meta), where a
//!   collection's Menu opens with Super and not with Shift.
//! - A submenu with no sensitive row opens from the keys with the keyboard left on its row.
//! - After a pointer input, the menu's first row has the keyboard but no visible highlight:
//!   the first arrow shows it there, and only the next moves (rune-kit's NSMenu model; Rune
//!   decides it from whether the last input was a key). GTK moves on the first arrow.
//!
//! # Rune gaps
//!
//! Drafts, not filed (the owner files). Probes are in `strata-spikes/c21-probes`.
//!
//! - **U-C21-1 (draft, not filed)** A text field's menu shows nothing where
//!   `popup_menu_supported()` is false (Linux): `field_menu.rs` queues a native menu that
//!   answers None. Ask: let the app draw it there, such as an `AppSpec::on_field_menu(|s,
//!   request, tx|)` hook handed the field, the point and Rune's own items. Here: each field's
//!   own `on_secondary_press` ([`FieldMenu::hook`]).
//! - **U-C21-2 (draft, not filed)** rune-kit's `TextInput`, `TextArea` and `TextField` build
//!   their inner field themselves and take no `on_secondary_press` for it, and a field's own
//!   menu wins over every ancestor's (`secondary_target`), so an app can't give them a menu.
//!   Ask: `.on_secondary_press(..)` on the three, passed to the inner field. Here: none.
//! - **U-C21-3 (draft, not filed)** `TextEdit` has no `can_undo` or `can_redo`. Ask: both.
//!   Here: undoing a clone when the menu opens ([`FieldMenu`]'s `history`). (A field's line
//!   mode needs nothing: Rune gives every mounted field its line mode whenever focus moves,
//!   so `paste` keeps a single-line field on one line once the menu has the keyboard.)
//! - **U-C21-4 (draft, not filed)** rune-kit's submenus open only when the app sets them
//!   open, and nothing in them moves the keyboard: no Right or Left, and the keyboard can't
//!   enter one, and no hover opens one. Ask: Right, Left, Enter and Space on a submenu row, focus
//!   into it, and hover-open with a delay. `Tx::focus` and `Tx::by_keys` (Rune 22aa8cde,
//!   3847cf0a) cover the focus part. Here: C21's own keys, and a sent Tab that moves the
//!   keyboard into a submenu, whose rows are keyed by position (U-C21-5).
//! - **U-C21-5 (draft, not filed)** `MenuItem` is keyed by position, so a row added above the
//!   focused one moves the keyboard. Ask: `MenuItem::key`. Its menu-name half is U-C6-8 (extend
//!   it; C21 names the surface it finds in the layer).
//! - **U-C21-6 (draft, not filed)** A secondary press on an open menu's outside scrim reaches
//!   nothing: `secondary_target` asks only the topmost hit and its ancestors, and the scrim
//!   has no `on_secondary_press`. So it doesn't even dismiss the menu, which stays open on its
//!   old target, and the entry under the scrim can't open its menu in the same click. Ask: let
//!   the scrim pass a secondary press through, or dismiss and replay it.
//! - **U14** (S5, not filed): popup surfaces for menus. Cited for the placement bullets.
//! - rune-kit's layer order. C21 finds the surfaces it adjusts by the order `Menu::build` pushes
//!   them (`[scrim, point, surface, submenus…]`), since an El's key can't be read once built;
//!   unit `layer_order_is_runekits` fails if a Rune bump moves them.

mod anchor;
mod field;
mod menu;
mod model;
mod state;
mod view;

#[cfg(test)]
mod tests;

pub use anchor::{AnchorEdge, KeyboardAnchor, MenuKeys, keyboard_anchor, menu_chord};
pub use field::{Clipboard, FieldCommand, FieldLabels, FieldMenu, FieldMenuState};
pub use menu::ContextMenu;
pub use model::{ActionId, ChooserMinimum, Item, MenuModel, Section, chooser_minimum};
pub use state::{ContextMenuState, MenuIntent, MenuTarget};
