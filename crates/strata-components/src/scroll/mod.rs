//! Scroll extras (C24): Strata's overlay scroll bar and middle-click autoscroll, on any Rune
//! scroll viewport, and on C1e's collection through its `edge` and `on_scroll` hooks.
//!
//! Work package: C24, `docs/work/phase-1-leaves/C24-scroll-extras.md`. Behaviour follows the
//! feature map's `browser/scrolling` (and the scroll-bar bullets of
//! `browser/selection/marquee`), against Strata's `src/ui/scrolling.rs` and `src/style.css`
//! at `upstream/main` 35ee677ab.
//!
//! # The pieces
//!
//! - [`ScrollFeed`]: a viewport's latest `ScrollPos`, where handlers without the app's state
//!   read it.
//! - [`Bar`] and [`BarLook`]: the overlay scroll bar. Fed by the viewport's position, drawn
//!   over the viewport, revealed by the pointer over the listing.
//! - [`Autoscroll`]: the window's one autoscroll, its state machine and its per-frame
//!   [`Autoscroll::drive`]; [`AutoscrollPointer`] starts it from a collection's middle press;
//!   [`Shield`] is what the window shows while it runs.
//! - [`OutsideWheel`]: an open panel's outside layer (Sort by, Appearance, Thumbnail size): a
//!   wheel tick there closes the panel and scrolls the listing under the pointer.
//!
//! # On a collection
//!
//! ```ignore
//! // State: listing, coll: CollectionState, feed: ScrollFeed, bar: Bar, auto: Autoscroll.
//! // Once: coll.controllers.pointer = Some(Box::new(AutoscrollPointer::new(
//! //     Some(Box::new(PointerIntent::new())), auto.clone(), SCOPE, feed.clone())));
//! let (feed, bar) = (s.feed.clone(), s.bar.clone());
//! let list = Collection::new(SCOPE, ..)
//!     .settings(autoscroll_settings) // AutoscrollSettings, margin from slots::collection::row_inset
//!     .on_scroll(move |p| { feed.set(p); bar.follow(p); })
//!     .edge(|s: &S| Some(s.bar.build::<S>("files-bar", &viewport_path(SCOPE), BarLook::list(s.look()))))
//!     .build(s);
//! let list = s.bar.area(list);
//! // poll:  s.auto.drive(now, look.animations, tx) | collection(..).tick(s, tx)
//! // root:  the window's root gets Shield::new(|s| &s.auto, |s| &s.look).build(s) last.
//! ```
//!
//! Hosts that hide or close a listing call [`Autoscroll::stop_gone`] with its key; a folder
//! change (`CollectionState::enter`) stops it through the decorator's reset.
//!
//! - **C4** (C24b) owns its bars: the rows' [`BarLook::list`] bar in the collection's `edge`,
//!   kept to the sideways scroll's visible right edge by [`Bar::follow_across`] (its vertical
//!   viewport is as wide as its columns), and an app-wide bar along the sideways scroll's
//!   bottom ([`overlay`]). The sideways scroll's one `on_scroll_offset` feeds both and then every
//!   host follower (`ListTable::follow_sideways`: C22's peek). The host installs
//!   [`AutoscrollPointer`] with `ListTableState::feed` and `ListTable::sideways_target`
//!   ([`AutoscrollPointer::sideways`]), so autoscroll moves both axes.
//! - **C3** (C24b) owns its [`BarLook::listing`] bar; the host installs [`AutoscrollPointer`]
//!   with `IconsGridState::feed` and `icons_grid::autoscroll_background`.
//! - **The archive preview** (C24b) owns its app-wide bar; it has no autoscroll.
//! - **C2** gives each column [`BarLook::column`] and its strip a [`BarLook::strip`] bar
//!   ([`overlay`]), with one [`Autoscroll`] for the window that every column's decorator
//!   shares.
//! - Every bar hangs from a zero-size [`anchor`] (X2), so P1's hit tests find the rows under it.
//! - C1e's `Collection::on_scroll` and `edge` add rather than replace (C24b), so a view's own
//!   bar and a host's follower never drop each other.
//! - **A plain viewport** (a sidebar, a preview): `.on_scroll_offset(bar.watch())`, the bar in
//!   [`overlay`], and for autoscroll its own middle-press handler calling
//!   [`Autoscroll::start`] with a [`Target`].
//!
//! # Decisions
//!
//! - **Autoscroll glides.** Each frame retargets a `Tx::scroll_to_with` glide a lead ahead of
//!   the offset (`collection::marquee::glide_lead`, C1d's probe-checked lead), rather than
//!   stepping instant scrolls; at its speeds (up to 33 pt a frame) a frame stays within the
//!   mounted rows and asks for no commit, so Rune's perf warnings stay silent.
//!   With animations off (`Look::animations`, which hosts also pass to
//!   `Cx::reduced_motion`), Rune lands a glide at once, so each frame jumps one frame's travel
//!   instead. It asks for frames only while a viewport moves: in the dead zone, or at an end, it
//!   sleeps until the pointer's next move out of it.
//! - **The stop is dead** (Strata parity): the viewports stay where the last frame drew them.
//! - **The shield is a modal.** Rune has no capture phase; a modal layer over the window is how
//!   every press anywhere, with any button, and Escape whatever has focus, reach the autoscroll
//!   first and go no further (probe P5). It is labelled "Autoscroll" for assistive technology.
//! - **Background is Strata's.** A middle press starts autoscroll where it lands on no entry, or
//!   in the blank strip along either side of a row ([`AutoscrollSettings::margin`]), which lies
//!   outside the row in Strata's lists; anywhere else the controller inside hears it (C1b's
//!   open-in-new-tab). In a listing that fits, a middle press on that background beside or
//!   within an entry's row does nothing (C24b): Strata's start declines, and the row it isn't
//!   on opens no tab. The decorator swallows it, so C1b and C1e are unchanged.
//! - **A text field inside a listing keeps its middle press** (Strata's `is_text_target`): C5's
//!   rename editor takes the middle button itself, so the press reaches neither autoscroll nor
//!   the row. A raw `text_input` inside a listing doesn't: Rune hands a middle press to the
//!   nearest element that takes the button, the collection.
//! - **A wheel tick outside an open panel** ([`OutsideWheel`]) is taken by the panel's outside
//!   layer, which Rune offers it first as the topmost element: it closes the panel and moves
//!   the host's listing targets under the pointer by the tick's own delta (Shift turns a
//!   vertical tick sideways, as Strata swaps), and none over the sidebar or chrome. The panel's
//!   surface takes the ticks over it that nothing inside scrolls (U-C24b-2).
//! - **Several targets stop one by one** (C24b): a target that reaches its end or the dead zone
//!   is stopped once, while another goes on, and a stop halts only the ones moving.
//! - **A stop without the shield** (a host that mounts none) asks the next `drive` to stop
//!   every target, rather than scrolling the pressed collection to where it stands.
//! - **The bar is revealed by the pointer over its listing** as GTK's overlay indicators are,
//!   held while the pointer is over the bar or holds its thumb, and fades [`bar::HOLD`] after.
//!   A faded bar isn't a hit target (Rune skips elements at no opacity), so a press there
//!   reaches the rows; the pointer has to move there first, which shows it.
//! - **A press on the bar's trough jumps** the thumb there and drags on from it (GTK 4's
//!   primary-button warp), rather than paging.
//!
//! # Deviations
//!
//! - **From plan §4 and the work package: C24's own bar, not rune-kit's `ScrollBar`.** At the
//!   pinned rev rune-kit keys a bar's state and its drag target by the viewport's own key, so
//!   two bars over viewports keyed alike (C1e keys every collection's viewport `viewport`:
//!   Columns, two panes) share one state, and dragging one scrolls the first (probe P2,
//!   R-C24-1). Its wrapper also moves the viewport under a new keyed parent, which breaks C1e's
//!   key paths (`{scope}/viewport`, P4), and it must wrap the viewport it scrolls, which would
//!   put C4's vertical bar at its content's right edge, off screen when the table is wider
//!   than its pane. And it draws no trough, has no end-gap or minimum-thumb setting (R-C24-2).
//!   Coordinator-approved; listed for the owner's plan edit.
//! - **While autoscroll runs the wheel does nothing, and keys other than Escape don't reach
//!   the listing**: the modal makes the window behind it inert (probe P5). Strata's capture
//!   gestures take only presses, so its wheel and keys keep working during an autoscroll.
//!   Page and window shortcuts still run (the app's `on_key` hears them, probe A1). Assistive
//!   technology hears only the dialog, "Autoscroll", while it runs (a modal hides the window
//!   behind it); Strata's autoscroll leaves its tree alone.
//! - **The shield takes the keyboard while it runs** (owner ruling, C24 review 2 SF3): the modal
//!   moves focus into it, and Rune gives it back to the element that had it as it closes, so
//!   keys reach the listing again. Strata's capture gestures leave focus alone: here a field
//!   that ends its edit on a blur (C5's rename) ends it when autoscroll starts (U-C24b-4).
//! - **A middle click on a text field pastes nothing**: Rune has no middle-click paste of the
//!   primary selection (U-C24b-3). It starts no autoscroll, as Strata's doesn't.
//! - **C4's two bars cross at the bottom-right corner** of a table that scrolls both ways; GTK's
//!   overlay indicators aren't confirmed either way.
//! - **The scroll cursor is `Cursor::Move`**: Rune has no all-scroll cursor (U-C24-5).
//! - **The bar's fade hold is 1 s**, not GTK's own indicator timing, which isn't confirmed.
//!
//! # Known gaps
//!
//! - **An Escape and a click in the same frame**: the click hit-tests against the frame's tree,
//!   where the shield still is, so it lands on the shield (and does nothing) rather than on the
//!   row. A frame later it reaches the row.
//! - **A listing that grows at the end autoscroll waits at**: at an end the machine sleeps until
//!   the pointer moves, so rows added below don't resume the scroll by themselves.
//! - **A window resized while autoscroll runs**: the shield keeps the viewport rect it started
//!   with, so the scroll cursor's region and the anchor's clip are stale until it stops.
//!
//! # Left to others
//!
//! - X1 and Strata (host chrome): mounting [`OutsideWheel`] outside the real Sort by,
//!   Appearance and Thumbnail size panels (C24b checks the bullets on the c24 page's stand-in),
//!   and mounting the [`Shield`] at the window's root.
//! - A follow-up (owner ruling): the bar on scroll surfaces that aren't listings (C8's
//!   sidebar, C9's drawer, C14a's pixels viewport, C15's, C16's, C18's and C19's lists, C6's
//!   completion popup, the c1 gallery page).
//! - C18/C19: the armed 10xer chord Escape cancels; C24 tells the host the stop's reason
//!   ([`Stop::Escape`], through [`Shield::on_stop`]).
//!
//! # Rune and rune-kit drafts (not filed)
//!
//! Written up for the owner to file; each repro runs headless (`rune::Runtime`). Probes:
//! `strata-spikes/c24-probes`.
//!
//! - **R-C24-1 (rune-kit): scroll bars over viewports keyed alike share state and drag the
//!   first.** `ScrollBar::scroll_bar_styled` keys its kept state by `"{key}-scroll-area"`
//!   (`scroll_bar.rs`, `kept(area)`) and drags `"{area}/{key}"`, which resolves to the first
//!   match in tree order. Repro: two panes keyed `a` and `b`, each holding a 1,000-row virtual
//!   viewport keyed `viewport` with `.scroll_bar_styled(ScrollBarStyle::scroll_view(&kit))`;
//!   wheel `b` to 2,000, then drag `b`'s thumb down 120 pt: `a` scrolls to 7,956 and `b` stays
//!   at 2,000. Ask: key the state and the target by the viewport's path, or take an explicit
//!   key.
//! - **R-C24-2 (rune-kit, a comment for anomalyco/rune#365): a scroll bar with a trough, apart
//!   from its viewport.** Re-checked in C24b at the pin and at Rune dev (111 commits on, read
//!   only): `ScrollBarStyle::track_offset(top, end)` starts the track below a viewport's top and
//!   ends its strip in from the edge, and the `menu_scroll` preset has a 4 pt inset and a 24 pt
//!   least thumb, but the inset and least thumb are still presets' only (#365 asks for them on
//!   any style), there is still no track fill (or its hover fill), and the bar still wraps the
//!   viewport it scrolls (`attach`, placed `viewport[1 - a] - end` across it), so it can't
//!   sit at the visible edge of a vertical viewport inside a wider sideways one. Dev's only
//!   change there is right to left (`ScrollPos::direction`). Repro (from the API, not run): a
//!   `scroll_x` viewport 600 wide holding a 900-wide `.scroll()` viewport with
//!   `.scroll_bar(&kit)`; `attach` places the strip at the inner viewport's right edge, past
//!   the 600-wide view. Ask: track colours on the style, and a bar fed a `ScrollPos` and a
//!   target that the app places. C24 keeps its own bar.
//! - **U-C24-3: a vertical wheel gesture moves to whichever viewport the pointer reaches.** The
//!   README says a gesture latches "as AppKit latches a gesture to the view it began on", but
//!   only the axis latches: `Runtime::wheel` picks `scroll_at(x, y)` for every delta, and only a
//!   horizontal gesture keeps `wheel_lock.scroller`. Repro (P1c): a `scroll_x` strip of three
//!   400-wide vertical viewports; a phased gesture (`Began`, three `Moved`, `Ended`) of `dy -40`
//!   at x 300, 390, 450, 500: columns 0 and 1 each scroll 80. Ask: keep a vertical gesture's
//!   viewport to its end, as a horizontal one keeps its scroller.
//! - **U-C24-4: a modal mounted mid-drag drops the captured drag without a release.** Repro
//!   (P6): an element with `pointer_buttons([Middle])` whose `on_drag` mounts a full-window
//!   `.modal(..)` on a middle Down; move 120 pt with the button held and release: the element's
//!   `on_drag` hears the Down and nothing more, not even a cancelled Up. Ask: end the capture
//!   with a cancelled `Up` (as Escape does mid-drag). The same holds for a bar's thumb: a
//!   modal (any dialog) mounted mid-drag leaves the bar's drag held, so it stays shown, its
//!   trough strengthened, until the next press on it.
//! - **U-C24-5: no all-scroll cursor.** `Cursor` has `Move` but not CSS `all-scroll` (GTK's
//!   `all-scroll`, the autoscroll cursor on Linux and Windows). Repro: none needed; the enum has
//!   no variant. Ask: `Cursor::AllScroll`.
//! - **U-C24-6: a modal's dismiss has no transaction.** `El::modal(click_outside, dismiss)`
//!   takes `Fn(&mut S)`, so an Escape that dismisses can't make a scroll request (here: stop a
//!   glide) in the same frame; it waits for the next poll. Ask: `Fn(&mut S, &mut Tx)`, as
//!   `on_press_tx` is.
//! - **U-C24-7: headless Shift + wheel doesn't scroll sideways.** The winit host turns a
//!   Shift + mouse-wheel delta horizontal (`scroll/x.rs`, `wheel_delta`); `Runtime::wheel_event`
//!   with `mods.shift` and a script `wheel` with `"shift": true` scroll vertically, so the
//!   Columns bullet "Shift+wheel over a listing scrolls the column strip" can't be tested
//!   headless. Repro (P1g): `wheel_event(WheelEvent::new(0, -120).mods(shift).precise(false))`
//!   over a column: the column scrolls, the strip doesn't. Ask: apply the turn in
//!   `wheel_event`, where scripts reach it.
//! - **U-C24-8: a scroll request that moves past the mounted rows runs the app's view about
//!   3 times a frame, where the wheel moving as far runs it 0 times.** A request (instant or
//!   glide) that takes a virtual list past the rows it has mounted commits about 4 times and
//!   runs the view about 3 times that frame; a wheel delta moving it as far commits once and
//!   runs no view. A thumb drag asks for such a request on every move, so over a long list the
//!   commits trip the "stepping its own glide" warning ("an instant scroll … on 8 frames in a
//!   row, each with a commit"), which rune-kit's own `ScrollBar` trips the same way (probe K1).
//!   Measured at the pinned rev (review round 1's probes, per frame):
//!
//!   | What moves the list | Commits | View runs |
//!   | --- | --- | --- |
//!   | C24's thumb drag | +4 | 3 |
//!   | plain Rune with rune-kit's `ScrollBar`, thumb drag | +4 | 3 |
//!   | wheel jumps of up to 6,000 pt | +1 | 0 |
//!   | `scroll_to_instant` of 400 pt or more | 2–3 | 2–3 |
//!   | `scroll_to_with` past the mounted rows | +4 | 3 |
//!
//!   Repro: 1,000 virtual rows with `.scroll_bar_styled(..)`; press the thumb and move it
//!   25 pt a frame for 12 frames, counting `Runtime::stats.commits` and the view's runs; then
//!   wheel the same distances. Ask: let a scroll request remount virtual rows as the wheel
//!   does, without running the view. C24's tests allow this one warning on a thumb drag.
//! - **U-C24b-1: `El::on_scroll_offset` replaces a handler, and chaining is hidden.** A
//!   second `.on_scroll_offset` on a viewport drops the first; rune-kit chains through
//!   `El::chain_scroll_offset`, which is `#[doc(hidden)]` at the pin and at dev. Repro: a
//!   `.scroll_x()` viewport with `.on_scroll_offset(a).on_scroll_offset(b)`; scroll it: only
//!   `b` runs. C4 keeps its sideways scroll's one handler and calls the host's followers from
//!   it (`ListTable::follow_sideways`). Ask: make chaining public (or `on_scroll_offset`
//!   additive), as rune-kit relies on it.
//! - **U-C24b-2: a wheel over an overlay that doesn't scroll scrolls the viewport under it.**
//!   `scroll_at` picks the innermost scroll viewport containing the point that isn't under a
//!   modal (`runtime/input/pointer.rs`), whatever non-scrolling element is drawn over it, so a
//!   wheel over a popover's plain surface scrolls the list beneath. Repro (run: the c24 page's
//!   Sort by stand-in without [`OutsideWheel::surface`]): a 1,000-row virtual viewport with a
//!   220-wide panel in the root layer over it, no handlers but a no-op `on_press`; one
//!   `wheel_event` of `dy -120` over the panel: the list scrolls. Ask: a wheel over an element
//!   drawn above a viewport, and not inside it, doesn't scroll it.
//! - **U-C24b-3: no middle-click paste.** On Linux a middle click in a text field pastes the
//!   primary selection; Rune's fields take no middle press and have no primary-selection
//!   source. Repro: a `text_input` and a middle `tap` on it: nothing reaches the field (the
//!   press goes to the nearest ancestor with `pointer_buttons([Middle])`). Ask: middle-click
//!   paste of the primary selection in text fields on Linux (and the selection's ownership).
//! - **U-C24b-4: a modal that leaves the keyboard where it was.** `El::modal` moves focus into
//!   itself, so a field that commits on a blur commits when one opens, even one with nothing to
//!   type into. Repro (C24 review 2's probe R2-P3, on the c24 page): focus a `text_input`, then
//!   mount a full-window `.modal(false, ..)` with no focusable child: the field's
//!   `ElementInfo::focused` goes false until the modal closes. Ask: an input-capture layer
//!   (presses, Escape) that doesn't take focus, or a modal option that leaves it.

pub mod autoscroll;
pub mod bar;
mod feed;
mod outside;

pub use autoscroll::{
    Autoscroll, AutoscrollPointer, AutoscrollSettings, Shield, Start, Stop, Target,
};
pub use bar::{Bar, BarLook, BarMode, anchor, overlay};
pub use feed::ScrollFeed;
pub use outside::OutsideWheel;
