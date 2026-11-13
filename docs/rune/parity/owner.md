# Owner decisions

What the parity work waits on the owner for, and what the owner has ruled. A package's agent
adds an open question here in the pull request that meets it, with the default it works to
meanwhile, and moves it to "Ruled" with the date when the owner answers. The plan's own
decisions are its ["Decided" list](README.md#decided) and are not repeated here.

"The orchestrating session" is the session the owner delegated the waves to. A default it
set stands until the owner rules otherwise. Where the only record of an item is that
session's notes, the item says so.

## Open

### Area packages

- **Does a `scope` row that names another package count as pending outside the package?**
  The plan's deliverable 4 says an area package closes with no `scope` run left. But
  `docs/testing.md` ("The ledger", the `pending` statuses) defines `scope` as a run that needs
  a part of the interface no package has ported yet. So an A1 row that stays `scope` with
  another package named as its blocker, such as `right-click-selects-entry/icons-file` waiting
  on B3's opener, would stop A1 from ever closing. Every area package meets this. The two
  readings: such a row counts as "pending on a blocker outside the package", or it needs
  another reason code or another owner. Default in force, the orchestrating session's: it
  counts as outside the package, provided the row names the blocking package and what it
  waits for, and E1 closes whatever is left. Source: the review of A1's
  [#2199](https://github.com/lgse/strata/pull/2199), 2026-11-12.

### E4: exit criterion 6

- **`gdk-pixbuf` in the Rune binary through `strata-fs`.** Exit criterion 6 names
  `gdk-pixbuf`, which `strata-fs` brings into `strata-rune` today by two paths, before any
  thumbnail worker: `crates/strata-fs/Cargo.toml` depends on `gdk-pixbuf` directly (line 35)
  and on `strata-preview` (line 29), neither optional. Whether E4 removes it from
  `strata-fs` as well, or criterion 6 is narrowed, is the owner's to say. Meanwhile:
  criterion 6 stays as written and [E4](packages/E4.md) owns it. Source: A1's layering
  check, 2026-11-12, confirmed from the manifests and with
  `cargo tree -p strata-rune -i gdk-pixbuf`.

  A correction of the premise, not a new ruling (recorded by the orchestrating session,
  2026-11-12): the thumbnail question was put to the owner on the premise that the worker
  would bring `gdk-pixbuf` into the Rune binary, which was wrong. The ruling stands as
  given. But the two answers not chosen, a separate worker binary and a client crate free of
  GDK, would not have kept `gdk-pixbuf` out of `strata-rune` either unless `strata-fs`
  changed too.

### The gallery while its tests are off

- **What is unguarded while the gallery's tests are off, and whether to re-bless at the
  cleanup.** Not run as validation now:
  - the look of every component scene (514 PNG goldens);
  - each scene's accessibility tree (514 accessibility goldens);
  - the 897 scripts, which are the per-bullet behaviour checks of the feature map;
  - the per-page in-process tests (`tests/c*.rs`, `k*`, `w*`, `x1`, `gallery.rs`);
  - `tests/goldens/registry.rs` and `keys.rs`: every page has tests and docs, scene names are
    unique and come in light and dark, and no golden or script folder is orphaned.

  The goldens will go stale while they are not run; when the gallery is cleaned up, decide
  whether to re-bless them from scratch. Two facts beside it:
  - the parity runs reach only components mounted in `strata-app`, so a component no area
    has mounted yet has `strata-components`' unit tests and nothing else;
  - the accessibility goldens, the registry and key tests and the page tests start no GPU
    child (four page targets, `c1`, `c1_marquee`, `c4` and `c24`, each have one call that
    does), so they
    are off because the instruction names the crate, not because they were measured as
    costly.

  So there are three options the owner has not chosen between, for running some of it again:
  installing lavapipe, which the harness uses with no code change
  (`crates/strata-gallery/tests/goldens/support.rs`); a cap across worktrees on how many
  gallery processes run at once; or keeping the in-process tests (accessibility goldens,
  registry, keys, page tests) and leaving only the PNG goldens and scripts off. Meanwhile:
  all of them stay off, and `mise run bless` stays. Source: the coordinator, 2026-11-12.

### The plan's copy

- **The plan's copy and the owner's document.** From the pull request that moved tracking
  here, the copy in [`README.md`](README.md) differs from the owner's document in
  "Coordination" and in the sentences that sent a reader to an issue for a package's state;
  the note at its top says so. Whether the document takes the same change. Meanwhile: the
  copy is what agents follow. Source: that pull request.

### H1: Whole-corpus ledger

Each of these is H1's reading of the plan or a default the orchestrating session set; the
rules as they landed are in `docs/testing.md`, "Who owns a run", and
are in force meanwhile. Source: the archived issue
[#2122](https://github.com/lgse/strata/issues/2122), H1's closing comment.

1. `excluded` landed with H1 and `parallel` with H2: a split made while sequencing wave 0.
2. A run named `tenxer`, by scenario or variant, is [A11](packages/A11.md)'s: 38 runs in 21
   files. So A11, a wave 2 package, cannot close before wave 3. The alternative is that such
   a run stays with its folder's package and names A11 as its blocker.
3. `letter_bookmarks` stays with [A3](packages/A3.md) though its three runs need 10xer mode;
   `preferences/enabled-tenxer-survives-restart` goes to A11.
4. `quick_preview`: the PDF, model and route files and one PDF variant to
   [A16](packages/A16.md) (7 runs), the rest to [A15](packages/A15.md) (50).
5. In [A6](packages/A6.md)'s folders, a run that works in the Settings window is
   [A18](packages/A18.md)'s: 9 runs.
6. `escape-dismisses-transient/columns` to [A17](packages/A17.md) and
   `focus-usable-after-view-change/columns` to [A8](packages/A8.md), by the first part each
   stops at.
7. `preview-blank-click-keeps-selection/columns` to A15: the plan gives the image viewer to
   neither A15 nor A16.
8. The two `input-method-composes-over-selection` runs and
   `tone-mark-escape-before-rename/columns` to [A7](packages/A7.md), with the slice's
   reasons and blockers.
9. `V7`, `V8` and `V9` own the slice's 36 required runs, which the check keeps `required`;
   [E1](packages/E1.md) owns the 9 excluded runs.
10. A new Columns run in the slice's seven folders that needs nothing unmounted is E1's.
11. **How a flaky oracle is carried in the ledger.** A flaky allowance may name neither a
    `required` nor (since H2) a `parallel` run, and no pending reason describes a flaky
    oracle. So `new-folder-with-selection-groups/columns` (finding [#2162](https://github.com/lgse/strata/issues/2162)) can only stay
    `pending` on `scope`, which A7 must not leave when it closes. What the entry becomes if
    the flake is not fixed on `main` first. Also on the archived issues
    [#2134](https://github.com/lgse/strata/issues/2134) and
    [#2162](https://github.com/lgse/strata/issues/2162).

### H2: Exception conventions in the runner

- **A `required` run goes `parallel` only with the owner's agreement**, whatever the
  exception's code: it is the direction that loses parity, and no check tells it apart from
  `pending` to `parallel`. A default of the orchestrating session, written into
  `docs/testing.md`, "Twins", and in force. Source: the archived issue
  [#2123](https://github.com/lgse/strata/issues/2123).
- **The reviewer opens a `deviation` twin's record** and confirms the owner's ruling is
  there before the twin merges, since no check reads it. A default of the orchestrating
  session, in "Twins", in force. Source: [#2123](https://github.com/lgse/strata/issues/2123). The record is the ruling in "Ruled"
  below, which names the twin's reference issue.
- **The ten decisions H2 made beyond the plan's text**: a twin's run has no ledger entry;
  `targets` only on a twin and only `["rune"]`; a Rune mutation patch for every twin; no
  `mutation` or `known_bug` on a twin; an optional `host` on a `parallel` entry; the form of
  a reference; the flaky allowance refusing a `parallel` run; `qa_cost.py` leaving twins
  alone; a twin keeping every `fs` step of its original; `message` and `timeout_ms` being no
  difference. In force as landed. Source:
  [#2171](https://github.com/lgse/strata/pull/2171)'s description.

### B1: Shell seams

- **Who owns which file of `strata-app`**: the table "Who owns what" in the module docs of
  `crates/strata-app/src/slots.rs`, among it that `qa/` is the harness's
  ([H3](packages/H3.md)) and an area adds its own steps in files of its own. A default of
  the orchestrating session, in force. Source: `slots.rs`, and the archived issue
  [#2135](https://github.com/lgse/strata/issues/2135).
- **No CI job builds the Rune side**, so Rune-lane evidence is written on each pull request.
  That is slice decision 13, still open (`docs/rune/slice/slice-report.md`); B1's record
  lists it.
- **When CI comes back for `strata-next`.** The owner turned the `CI` workflow off for
  `strata-next` "for now" on 2026-11-12 (see "Ruled"). When it runs again, and whether with a
  Rune-lane job (decision 13), is the owner's to say. Meanwhile: the agents'
  [checks in place of CI](README.md#checks-in-place-of-ci). Source: that instruction.
- **The trigger change must not reach `main` at the cutover.** `ci.yml` on `strata-next` no
  longer runs for `strata-next`'s pushes and pull requests; if `main` takes `strata-next` by
  merge at [E4](packages/E4.md), that change must not turn CI off for anything `main` needs.
  The review of that pull request finds it harmless there: `push` on `main` and "ignore pull
  requests based on `strata-next`" turn off nothing `main` needs, so this is not urgent.
  Meanwhile: E4 checks it. Source: the pull request that changed the triggers.

### B2: Shared state out of the GTK package

Sources: the archived issue [#2125](https://github.com/lgse/strata/issues/2125), its
comments of 2026-11-12, unless another is named.

- **Several pull requests per store**, against the plan's "one pull request per store": two
  so far for `PreferenceManager` ([#2172](https://github.com/lgse/strata/pull/2172), [#2178](https://github.com/lgse/strata/pull/2178)) and for the per-folder views ([#2168](https://github.com/lgse/strata/pull/2168), [#2188](https://github.com/lgse/strata/pull/2188)),
  each with more to come, and the key catalog in three parts. In force, taken as within the
  ruling's intent, for review safety.
- **The key lookups behind one backend.** The five key lookups and the accelerator reader
  and writer sit behind one backend in `strata_desktop::keys`; GTK installs a GDK-backed one
  and stays an exact oracle; the `xkbcommon` backend is a `strata-desktop` feature that only
  `strata-app` and the GTK lane's tests enable. So CI's guard against `xkbcommon` in a release build stays as it is.
  A ruling of the orchestrating session after an independent read, waiting for the owner's
  confirmation. In force: it orders B2's remaining work.
- **The Rune side writes `settings.toml` as GTK does**, because the plan puts what
  `settings.toml` holds among what must match with no exception. A default of the
  orchestrating session; in force since [#2178](https://github.com/lgse/strata/pull/2178).
- **One settings store per process, or per window, on Rune**, and how wide "live across two
  windows" is: from another process GTK adopts five keys only, the sort and the icon size.
  Meanwhile: one `Saving` for the file in a process, and the Rune side follows nothing
  another process saves.
- **Where `active_omarchy_theme` lives**: the Omarchy variant methods need it, and it is
  neither a saved value nor bookkeeping. Meanwhile: in `src/ui`.
- **A home for the exhaustive saved-preferences fixture** before `src/ui` is deleted; the
  Rune write path has no equivalent coverage. Meanwhile: GTK-only. Also on the archived
  issue [#2150](https://github.com/lgse/strata/issues/2150).
- **The `grid` and `explorer` aliases of the view mode**, which need `browser_modes` and are
  outside the ruling. Meanwhile: each Rune view's `shows` answers to both its name and its
  alias. Also on the archived issue [#2124](https://github.com/lgse/strata/issues/2124).
- **`src/ui/keys.rs` becoming `src/ui/keys/mod.rs`** when the catalog moves without its event
  reader, the shape that keeps merges from `main` landing in the shared file. Meanwhile:
  that shape.
- **A key with no name is written as `0x…` or `U+XXXX` and is not read back**, as GTK does
  today. Whether that is behaviour to keep. Meanwhile: kept exactly.
- **The accelerator reader and writer for the `xkbcommon` backend** are written from GTK's
  documented format and its observed behaviour, by a session that has not read GTK's
  accelerator source. In force, as the rule that follows from the owner's ruling on GTK's
  source.
- **Which bindings the Rune side says never fire.** A key label leaves out a binding that
  can never fire in its map, and which those are is the interface's answer, installed
  with its key backend (`NeverFires`). GTK answers from `src/ui/keys/conflicts.rs`, which
  knows the keys GTK's own stages and widgets take. It is one question with which
  conflicts Rune's keyboard settings report, under A6 and A18 below. Meanwhile: the Rune
  side answers that every binding fires, so its labels show every key of an entry. For the
  shipped keys that is GTK's answer too: its check leaves out none of them in either map,
  with type-to-search on or off. The two differ only for a key a user added, which the
  Rune side does not read yet. Source: [#2195](https://github.com/lgse/strata/pull/2195) and [#2197](https://github.com/lgse/strata/pull/2197).
- **A rebinding's round trip where key libraries differ.** A key the code declares never
  diverges: both interfaces read and write it from Strata's own table, whatever is
  installed, and that is every key a shipped binding uses. The question is a key a user
  binds beyond those. With the `xkbcommon` backend its name follows the user's installed
  libraries, and the backend's rules are exact for the test image's GTK, GLib and
  libxkbcommon. Measured ([#2255](https://github.com/lgse/strata/pull/2255)): libxkbcommon 1.13 reads 359 names that GDK refuses and names 321 keys
  that GDK writes as a number. The backend refuses and numbers all of them: 328 names and
  306 keys by two rules (GDK names no vendor's keys but XFree86's, and none of XFree86's
  keys named after the Linux kernel's key codes), and 31 names and 15 keys one by one. GDK
  reads two names libxkbcommon does not, and writes 16 keys by another name. The image's
  GTK 4.22 and a developer host's GTK 4.24 gave the same rows throughout, so on those
  pairs no name is read on one interface and refused on the other. Where a user's
  libraries differ from the image's, a rebinding of such a key can be lost at restart or be
  unreadable by the other interface or by a 1.x release, in four ways:
  1. *A newer libxkbcommon* reads names it added outside the two ranges: read on Rune,
     refused by GTK.
  2. *An older libxkbcommon* lacks names GDK has: read by GTK, refused on Rune.
  3. *A newer GDK* names keys the rules write as numbers, or reads names the rules refuse:
     read by GTK, refused on Rune, and the reverse for a name GDK dropped.
  4. *A GLib whose Unicode tables differ from the compiler's* gives a letter another case
     on one interface only, so a binding to a recently added cased letter with Shift is
     recorded differently.

  How many keys that is depends on the user's libraries and cannot be counted here. The
  Rune interface logs which libxkbcommon it runs on when it installs the backend, so a
  report can be placed. Meanwhile, by the orchestrating session's ruling: the rules exact
  for the image's libraries, no reading of numbers added to cure it, and every declared
  key read from Strata's own table whatever library is installed.
- **`strata-rune` links libxkbcommon directly.** The owner ruled "link `xkbcommon` at run
  time" for the key tables. The orchestrating session read that as ordinary dynamic
  linking: the system's library, resolved by the loader at start, not tables built in and
  not a library loaded by hand. In force from [#2259](https://github.com/lgse/strata/pull/2259), and the owner's to reverse. The
  consequence: before it, the Rune binary did not link the library (Rune's window stack
  loaded it with `dlopen` when it set up the keyboard), so on a machine without
  libxkbcommon `strata-rune --help` and the sandbox's thumbnail worker worked and the
  window failed at its keyboard set-up; now none of the three starts. Accepted because GTK
  4 links the same library, and because loading it by hand means unsafe calls through raw
  function pointers in a shared crate. `strata-rune`'s packaging owes the dependency when
  it is packaged.
- **Tab-session restore has no QA scenario.** No scenario seeds or reads `tabs.toml` or
  sets `restore_tabs`, so the ledger holds no interface to the rules for using the shared
  store, which live in GTK code that is deleted at parity (`docs/rune/shared-state.md`,
  "What the seam does not carry"). Whether a scenario is owed and whose it is. Meanwhile:
  the rules are listed there, and GTK's unit tests are their only tests. Also on the
  archived issues [#2131](https://github.com/lgse/strata/issues/2131) and [#2137](https://github.com/lgse/strata/issues/2137).

### B3: Commands and key dispatch

Source for all of these: [B3](packages/B3.md), "Design", and the archived issue
[#2126](https://github.com/lgse/strata/issues/2126). What the builder does is in
`crates/strata-app/src/commands/press.rs`.

- **B3 depends on B2's key-catalog move**, which the plan does not record: what B3 builds on
  arrived with it. The move merged on 2026-11-12 and B3 started then; whether the plan's
  table takes the dependency is still the owner's.
- **A non-Latin layout with a non-US Latin layout also configured (case 1 of the seven).**
  A deviation that needs the owner's word, not a default to confirm. GTK reads a key that
  types no ASCII through the other configured layout, and Rune cannot see the layouts that
  are configured and not active: its window host has already named the US key for a chord,
  and the press builder falls back to the US position for a plain key. So for a user with a
  Russian and a German layout, Ctrl with the key that is Z on QWERTZ is Undo on GTK and Redo
  on Rune, and the reverse. No workaround exists in `strata-app`. Rune packet entry
  [99](../upstream/rune/99-configured-layouts.md) asks for what would be needed. Meanwhile:
  the US position, which is GTK's own rule when no other layout has a Latin key.
- **The Menu key with Super held.** Rune reports Super, Hyper and Meta as one flag. GTK
  opens the context menu with Hyper or Meta held and not with Super; strata-components' C21
  opens with the flag set, on purpose, to keep Hyper and Meta. One of the three keys must
  differ from GTK. Which is the owner's. Meanwhile, in force from the pull request that
  binds the Menu key: the flag is read as Super, the common key, so the catalog's
  `context_menu` does not match and no menu opens, as on GTK; Hyper and Meta then differ.
  The same reading holds for every binding: Rune's flag counts as Super, which no shipped
  binding holds.
- **Where the Menu key is read.** On Rune a view's keyboard controller opens the menu, after
  every stage of the chain through `dismissal` has declined the press. GTK's
  `context_menu_command` sits in the middle of that chain. No shipped key shows the
  difference; a user's binding that put the Menu key on an earlier or later command could.
  Meanwhile: the controller's place.
- **The Menu key with the cursor's entry scrolled out of view.** Seen in a run on the GTK
  target on 2026-11-12 (400 files, the cursor on the first and selected, the wheel turned
  until that row was out of view, then the Menu key; Shift+F10 the same): GTK's Columns and
  Icons views open nothing and scroll nothing; its List view opens the menu of another
  entry, the one drawn where the cursor's row was last placed, and makes that entry the
  selection, so Move to Trash and Permanently delete are offered for a file the user did
  not choose (finding [#2235](https://github.com/lgse/strata/issues/2235)). A row partly in view was not part of the run: by
  reading, such a row is drawn, and GTK opens its own menu at it in every view. Two
  questions:
  1. The Rune interface's List view differs from GTK's on purpose: it opens nothing, as
     Columns and Icons do, because GTK's List behaviour is a fault. To confirm; and the bug
     on `main` is the owner's to file and schedule.
  2. Whether the Rune interface should go further and scroll the row into view and open its
     menu there, in every view, which strata-components' C21 can do and GTK does in none.

  In force, by the orchestrating session's ruling of 2026-11-12, from the pull request that
  binds the Menu key: nothing opens and nothing scrolls in any view, and a row partly in
  view gets its menu where it is drawn. Revealing instead is one condition in
  `crates/strata-app/src/commands/menu_key.rs`.
- **The other six cases, each with the default in force** from the registry's pull request:
  2. *The digit row above level 1 (AltGr).* Read as GTK reads it where Rune gives the key's
     base character and Shift is not held: such a character has no position, so it is a
     digit only if it types one, and it never falls back. With Shift held the two levels
     cannot be told apart and the position stands. A digit typed at such a level on a key
     outside the digit row counts as a digit on Rune and not on GTK.
  3. *The keypad's decimal and separator keys.* Read as GTK reads them wherever the key's
     position is known: the keypad key itself, which is not the main block's period or
     comma.
  4. *Every press in the QA target.* It has no position and no base character until
     anomalyco/rune#657 lands (packet entry 97), so the character is read as the US key
     that types it. Exact on a US layout, which every required run uses. A plain letter of
     another alphabet gets no Latin fallback there, which is the ledger's `rune` blocker.
     A workaround outside Rune, the QA backend telling the window the next key's origin, is
     [H3](packages/H3.md)'s to take or leave.
  5. *Keys Rune's host never delivers* (dead keys, Compose, Caps Lock, AltGr, media keys,
     F25 to F35, the keypad's 5 with Num Lock off). Nothing arrives, so nothing is routed.
     What a dead key or Compose goes on to compose does arrive, as text at the last key's
     position: a dead key and then Space types `^` on Rune, where GTK's list hears Space.
  6. *A modifier's own press.* Built as GDK reports it, held without the key itself. Rune
     sends none unless a host asks, and neither host asks yet.
  7. *Character to keysym.* B2's one keysym for a character, `Key::from_char`.
- **An eighth case, found in the review of the registry: a chord whose Shift or AltGr level
  types no ASCII on a key whose base is ASCII.** With Ctrl held, Rune's window host
  replaces a character that is not ASCII by the US key at that position and drops the
  character typed. The press builder undoes that where the key's base character is not
  ASCII either (a Russian or Greek key). Where the base is ASCII it cannot tell that
  anything was replaced, and reads the US unshifted key: on AZERTY, Ctrl+Shift with the `*`
  key (which types `µ`) reads as Ctrl+`\`, the arrow-scope key, where GTK reads Ctrl+`|`;
  on a Polish layout Ctrl+AltGr+A (`ą`) reads as Ctrl+A, where GTK reads no shortcut. No
  command with a registered key is reached by either today. Rune packet entry
  [100](../upstream/rune/100-chord-typed-character.md) asks for the character back.
  Meanwhile: the US key, as Rune names it.

### A6 and A18: keyboard settings

- **Which conflicts Rune's keyboard settings report**: GTK's as they are, or those of Rune's
  own text field and views, which nobody has listed; and what they say of a key Rune's host
  never delivers (dead keys, Compose, Caps Lock, AltGr, media keys, F25 to F35), whose
  binding loads, gets a label and never fires. Meanwhile: `src/ui/keys/conflicts.rs` stays in
  `src/ui`, and the Rune side has no conflict check. Source: the archived issues
  [#2133](https://github.com/lgse/strata/issues/2133) and [#2150](https://github.com/lgse/strata/issues/2150).

### A1: Icons view and thumbnails

Found in the review of [#2192](https://github.com/lgse/strata/pull/2192) and by the
package's agent. Source: the package's agent, 2026-11-12; first written on the archived
issue [#2128](https://github.com/lgse/strata/issues/2128).

1. **Down on the last row when every row is full.** GTK moves to the last tile (GtkGridView's
   `n_items / n_columns` test in `gtk_grid_view_move_focus_along`). Rune's `IconsGrid` treats
   it as an edge, on purpose (`crates/strata-components/src/icons_grid/mod.rs`, "Decisions
   and deviations", first bullet). A deviation for the owner to rule. Meanwhile: the
   component's edge; no run pins it.
2. **Left with no cursor.** GTK sends it to the sidebar when the pane is at its left edge
   and the sidebar shows (`single_pane_arrow_action`, `src/ui/window/key_routing.rs:96`;
   `at_left_edge`, `src/ui/browser_modes/focus.rs:133`). Rune's keyboard controller lands
   on the last tile. Meanwhile: Rune's, until there is a sidebar ([A3](packages/A3.md)) to
   go to; then the Icons part matches GTK.
3. **Up from the first row.** GTK focuses the pane header (`focus_header_from_top`,
   `src/ui/window/keyboard/items.rs:166-180`). Rune ignores `Edge(Up)`. No package owns the
   pane header. Meanwhile: nothing happens.
4. **Focus after Back, Forward and Up.** GTK focuses the new listing on an explicit
   navigation, and leaves focus on a header button or the sidebar for history unless it was
   in the pane (`on_navigation_starting`, `src/ui/browser_modes/events.rs:59-73`). The Rune
   views focus the listing whenever another folder shows. Meanwhile: the Rune views', until
   the header buttons ([B3](packages/B3.md)) and the sidebar exist to keep focus.
5. **"Arrows scoped to content"** (`arrow_navigation_scoped`) keeps Left and Up inside the
   content in GTK (`src/ui/window/keyboard/items.rs:152, 180`). Neither Rune view honours
   it. Meanwhile: not honoured; it goes with items 2 and 3.

The orchestrating session's rulings for A1, in force until the owner reverses them (source:
the orchestrating session, 2026-11-12):

6. **`strata-rune` is built with UnRAR by default,** as the GTK application's default build
   is (the root package's `rar` feature). `strata-app` and `strata-rune` have a default `rar`
   feature forwarding `strata-core`, `strata-fs` and `strata-preview`'s, so `strata-rune`'s
   thumbnail worker decodes a CBR cover as GTK's default worker does. A build without UnRAR is
   `--no-default-features`, as for GTK.
7. **The thumbnail store is one per window, the render pool one per process.** The pool and
   its limits are process-wide state of `crates/strata-app/src/thumbnails/`, as GTK's are in
   `src/ui/thumbnail/jobs.rs`. C25's `ThumbnailStore` is in the shell (`Window`), one per
   window, so every view of a window reads it. GTK's decoded-image cache is one per process
   (`THUMBNAIL_CACHE`), so with several windows an image is decoded and held once per window:
   [A21](packages/A21.md)'s to settle with multi-window. Nothing a user sees differs.
8. **The pane header's Back, Forward and Up buttons, with the history list, are [A5](packages/A5.md)'s**
   (Location bar), with the header's shell, since the breadcrumbs live in the same header. The
   plan placed them with no package; B3 said so when it closed. Placed by the coordinator,
   2026-11-12. `keyboard_navigation/back-history-dropdown` waits on it.
9. **The Icons view's thumbnail size is [A1](packages/A1.md)'s, its header button and slider
   [A5](packages/A5.md)'s.** The coordinator's placing, 2026-11-12, for the owner to confirm. A1
   owns the commands `thumbnails_larger`, `thumbnails_smaller` and `thumbnails_reset` with their
   catalog bindings (read at the dispatcher's `zoom_shortcut` stage), Ctrl+Shift+wheel over the
   grid (the steps of GTK's `WheelZoom`; GTK's wheel reaches the size from anywhere in the
   window, which is item 10), the size each folder keeps through B2's folder-view store, and
   the grid laying out at the size live. A5 owns the pane header's "Thumbnail size" button and
   its slider popover (`ThumbnailSizeMenu`, `src/ui/browser_modes/icons_controls.rs`), which
   call A1's commands. [A11](packages/A11.md) owns the 10xer map's plain +, = and − for the
   size (`for_tenxer_key`), which call A1's commands too. Sources:
   `src/ui/browser_modes/thumbnail_size.rs`, `src/ui/window/keyboard/wheel_zoom.rs`,
   `src/ui/preferences/folder_view_settings.rs`.
10. **Who owns the window-level zoom wheel, and the text size?** GTK's Ctrl+wheel steps the
   interface's text size and Ctrl+Shift+wheel the Icons thumbnail size from anywhere in the
   window except the PDF preview, which zooms itself (`zoom_controller` and `claims_zoom` in
   `src/ui/window/keyboard/wheel_zoom.rs`, a capture-phase controller on the window's root).
   Rune's Icons view takes Ctrl+Shift+wheel only over its grid (C3's `WheelSteps`), and no
   package has the text size (Ctrl+plus, Ctrl+minus, Ctrl+0, `text_size().for_shortcut` at the
   same `zoom_shortcut` stage). The coordinator suggests [A6](packages/A6.md) or a window
   package take both, the wheel calling A1's thumbnail commands. Meanwhile: the wheel works
   over the grid only, and the text size does not change on Rune.

### A2: List view

Found by the package's agent, 2026-11-12, against A1's five items above.

1. **Right with no cursor.** GTK keeps the keyboard where it is: List rows have no
   sideways neighbours (`horizontal_arrow_action`, `src/ui/window/key_routing.rs:120`).
   Rune's keyboard controller puts the cursor on the first row, as a step with no cursor
   does in any collection (`first_step` in
   `crates/strata-components/src/collection/keyboard/machine.rs`). A deviation of the
   component for the owner to rule. Meanwhile: the component's; no run pins it.
2. **A1's items, in List.** Item 1 (Down on the last full row) does not arise: a List row
   holds one entry. Item 2 (Left with no cursor) does not differ: every List row is at the
   pane's left edge (`at_left_edge`, `src/ui/browser_modes/focus.rs:133`), so the List part
   has Left leave the pane with or without a cursor, as GTK does. Items 3, 4 and 5 (Up from
   the first row, focus after Back, Forward and Up, "Arrows scoped to content") differ in
   List as they do in Icons, with the same defaults in force.
3. **Nothing drawn while a folder is read.** GTK shows its loading skeleton once a read
   outlasts its grace period; Rune has no loading skeleton yet, so the Icons and List views
   draw nothing then, though they say "Loading" as GTK does. Meanwhile: nothing drawn; no
   run pins it.
4. **No entry under the keyboard after a rename, an open or a menu, on GTK.** In the List
   fuzz walk, after F2, Return on a folder or the Menu key, GTK's snapshot named no entry
   under the keyboard (`app.focused` null) until a key moved it; Rune names the entry the
   keyboard is on. Beside A1's items 3 and 4 (Up from the first row; focus after Back,
   Forward and Up). Found by the List alphabet (#2258), whose renames start with a Down for
   it; a differential run will compare `app.focused` there. Meanwhile: Rune keeps the entry.
5. **Which package takes the Open With chooser?** Rune has none; GTK opens its own when a file
   has no default application, and no package of the plan covers it. The coordinator suggests
   [A7](packages/A7.md), entry management, or a package of its own after A7. Meanwhile:
   Return on such a file does nothing on Rune.

### A7: Entry management and undo

1. **Who takes the List details cell for a symlink's size?** On GTK the List row of a file
   link shows its target's size (`delete-file-symlink-sizes-link`, 100 kB at line 5, in
   both its runs); on Rune the cell shows nothing. It is the List view's details cell, A2's
   area and the contract's `details.rs` and `fills.rs`, and A2 has closed. Meanwhile: the two
   runs stay `pending`, unplaced. Found by A7's closing survey, 2026-11-13.

### A5: Location bar

1. **The window's own buttons and the keyboard.** Rune draws the client-side window buttons
   (minimise, maximise, close: C5's `window_buttons`, `crates/strata-components/src/header/bar.rs`)
   as Tab stops that Return does not press; Space does (finding
   [#2310](https://github.com/lgse/strata/issues/2310)). GTK's window controls most likely take
   no focus at all. Should Rune's stop being Tab stops, as GTK's, or stay and take Return like
   every other button? Meanwhile: they stay as they are, named on the exceptions list of
   `crates/strata-app/src/buttons/tests.rs`, the check that every other focusable button takes
   Return. Source: the review of A5's [#2311](https://github.com/lgse/strata/pull/2311),
   2026-11-14.

### Packages added later

- **A branch name for a package with no archived issue.** Branch names keep the package's
  archived issue number, as `AGENTS.md` asks for one. A package added to the plan after
  2026-11-12 has none; the owner decides the rule. Source: the pull request that moved
  tracking here.

### H5: Coverage-gap scenarios

- **Who merges H5's pull requests.** H5 writes its scenarios on `main` first, and agents were
  authorized to merge into `strata-next`, not into `main`. Meanwhile: H5 is held. Source:
  recorded by the orchestrating session on 2026-11-12.

## Ruled

Rulings of the owner on the parity work, oldest first.

- **2026-11-11: the slice is approved** once its report is committed and
  `./scripts/qa-parity.sh slice` exits 0. Both held, so `strata-next` is an integration
  branch and migration work lands there by pull request. Source:
  `docs/rune/slice/README.md`, "Approval".
- **2026-11-11: the slice's flaky run is allowed outright** in `slice`'s whole-suite part,
  and `cut.patch`'s `[[stale_patch]]` entry is what "as before" means for the GTK patches
  (slice decision 27). `slice` keeps both allowances. Since the rule on mutation runs below,
  the merge gate has no stale-patch step, and the flaky allowance applies there only by
  reading a failure as the plan's "The merge gate" says. Source:
  `docs/rune/slice/README.md`, decision 27.
- **2026-11-11: package agents merge their own pull requests.** The owner said that a
  package's agent merges its pull requests into `strata-next`, with a merge commit, once a
  session other than the author's has reviewed them, the plan's
  [merge gate](README.md#the-merge-gate) has passed and CI is green (recorded by the
  orchestrating session, 2026-11-11).
- **2026-11-12: link `xkbcommon` at run time for the key tables.** Nothing derived from GTK's
  source is committed: no generated tables and no script that generated them. The shared
  crate carries the rules where GTK behaves differently, found by running GTK. Source: the
  archived issue [#2125](https://github.com/lgse/strata/issues/2125).
- **2026-11-12: one line of `tests/qa/mutations/quick-preview.patch` may change**: its key
  expression, so that it compiles against the shared key type, proved with
  `./scripts/qa-mutation.sh quick-preview`. The rule that a GTK mutation patch is never
  edited stands for every other line and patch. Source: the archived issue [#2125](https://github.com/lgse/strata/issues/2125).
- **2026-11-12: wave 1 packages start as their own dependencies finish.** The owner said
  that a wave 1 package starts when its own dependencies are done, not after the end of wave
  0's checkpoint, which is still reported (recorded by the orchestrating session,
  2026-11-12). The plan's "Checkpoints" says the same of waves.
- **2026-11-12: tracking moves to Markdown in the repository.** After every package and
  finding issue was closed that day, finished or not, the owner said to keep going and to
  move tracking into Markdown checked in under `docs/`, as agents finish and new ones start
  (recorded by the orchestrating session, 2026-11-12). So a package's state, its record and
  its open questions live under `docs/rune/parity/`, and the closed issues are an archive.
- **2026-11-12: mutation runs are not routine verification.** The owner said: "As a general
  rule going forward, mutation testing should only be run if there's a question about whether
  tests are effective; it is far too heavy on wall-clock time to run as a routine
  verification" (recorded by the orchestrating session, 2026-11-12). So the routine checks
  and the merge gate run no mutation set; the plan's
  [When a mutation run is right](README.md#when-a-mutation-run-is-right) says when one is.
  Two parts of that section are the orchestrating session's reading of "as a general rule",
  for the owner to confirm: that the exit checks (E2's soak and exit criterion 3) stay as
  written, and that a patch a package writes is still proven once.
- **2026-11-12: a new finding still gets a GitHub issue, as its reference.** The owner said
  "Keep filing GitHub issues", choosing: "A new finding still gets a GitHub issue purely as
  its reference (it can be closed at once); findings.md holds the state" (recorded by the
  orchestrating session, 2026-11-12). So a new `oracle`, `divergence`, `exception` or
  `rune-gap` finding gets an issue on `lgse/strata`, which the ledger, twins and the fuzz
  driver's files cite as `lgse/strata#<number>` (a `rune` twin still cites its packet
  entry), and [`findings.md`](findings.md) holds its state. Package tracking stays in
  Markdown; no package issue comes back.
- **2026-11-12: the thumbnail worker is in `strata-rune` until E4.** The owner said "In
  strata-rune until E4" (recorded by the orchestrating session, 2026-11-12), choosing among
  three answers: `strata-rune` gets the `--browser-worker` entry point now, as the GTK
  binary has it, which brings `gdk-pixbuf` into the Rune binary through `strata-preview`.
  [E4](packages/E4.md) owns removing it, and exit criterion 6 is knowingly not met until
  then. The other two answers were a separate worker binary and a client crate split from
  `strata-preview`. In fact `gdk-pixbuf`, `poppler-rs`, `cairo-rs` and `pango` are in
  `strata-rune`'s tree already, by two paths that have nothing to do with thumbnails in the
  Rune interface: `strata-app` depends on `strata-fs`, and `crates/strata-fs/Cargo.toml`
  depends on `strata-preview` (line 29) and on `gdk-pixbuf` directly (line 35), neither
  optional (`cargo tree -p strata-rune -i gdk-pixbuf`). The coordinator's reading: so the ruling adds a direct edge
  from `strata-app` to `strata-preview`, not a new library, and criterion 6 was not met
  before it either. E4 owns removing `gdk-pixbuf` from the whole tree: `strata-fs`'s own
  use of it, its use of `strata-preview`, and the thumbnail worker's direct use.
- **2026-11-12: CI does not run for `strata-next`.** The owner said: "Lets disable CI for
  strata-next for now and stop waiting for it, agent's own checks are sufficient" (recorded
  by the orchestrating session, 2026-11-12). Changed: the `on:` block of
  `.github/workflows/ci.yml`, whose `push` trigger no longer names `strata-next` and whose
  `pull_request` trigger ignores pull requests based on it; `workflow_dispatch` stays, so it
  can still be run by hand. Still running for `strata-next`: `pr-policy.yml` (the title, type
  label and metadata checks), `test-image.yml` (publishes the test image base when a manifest
  or `tests/container/` changes on `strata-next`), and `integrations.yml` and
  `packaging.yml` on pull requests that touch their paths. For `strata-next` this replaces
  "CI is green" in the 2026-11-11 merge ruling above with the plan's
  [checks in place of CI](README.md#checks-in-place-of-ci).
- **2026-11-12: the gallery's tests are not run, except its unit tests.** The owner said:
  "I want to stop running tests on the gallery except for the basic unit tests; we'll clean
  up that crate and reduce the overhead later but for now the parity with GTK strata is
  quite enough to assure quality in my opinion" (recorded by the orchestrating session,
  2026-11-12). So `mise run test-rune` runs `strata-gallery`'s unit tests only, its goldens,
  page tests and scripts run on demand with `mise run test-gallery`. That `AGENTS.md`'s
  bullets on a gallery scene, an accessibility golden and a script per component are
  suspended until the cleanup too is the coordinator's reading. `strata-components`' and `strata-app`'s own tests still run. Which test targets
  count as "the basic unit tests" (`--lib --bins --examples`) is the coordinator's reading.
- **2026-11-12: B2's `PreferenceManager` shape is accepted.** Asked "Does the
  bookkeeping-only `PreferenceManager` shape meet your ruling?", the owner said: "Accept it"
  (to the orchestrating session's question, 2026-11-12). So moving `PreferenceManager`'s save
  bookkeeping and leaving its getters and setters in `src/ui` meets the ruling of
  2026-11-11, and a setter's body moves when a Rune package first needs it. The
  coordinator's reading: the three edits to a saved keymap (`set_bindings`,
  `reset_binding`, `reset_map`) move the same way, with A18's keyboard settings. Built in
  [#2172](https://github.com/lgse/strata/pull/2172), [#2178](https://github.com/lgse/strata/pull/2178) and [#2265](https://github.com/lgse/strata/pull/2265).
- **2026-11-12: B2 closes with the folder-view timer rule left in each interface.** Asked
  "Can B2 close with the folder-view timer rule left in each interface?", the owner said:
  "Close with it" (to the orchestrating session's question, 2026-11-12). Built in
  [#2262](https://github.com/lgse/strata/pull/2262).
- **2026-11-12: the per-folder views' policy is shared.** Asked under B2 where the policy that
  spans the settings and the per-folder views lives (`set_remember_folder_views`,
  `set_default_sort_options`, `set_folder_icons_size`, `set_default_icons_size`), the owner
  said: "Move it, shared" (relayed by the coordinator, 2026-11-12). So it moves into the
  shared crate, for both interfaces. It is `strata_desktop::settings::folder_view_policy`, a
  new file beside the moved store, which `src/ui/preferences/folder_view_settings.rs` calls:
  the gate, where a changed size is kept, and what a new default prunes. That the sort's share
  goes there when [A10](packages/A10.md) needs it is the package's reading, not the owner's
  words. The timer rule stays in each interface, by the ruling above; the Rune side's is
  `crates/strata-app/src/folder_views.rs`. Moved by [A1](packages/A1.md)'s pull request
  [#2281](https://github.com/lgse/strata/pull/2281).
- **2026-11-12: shared scenarios written during parity work stay on `strata-next`.** Asked
  whether parity-written shared scenarios stay on `strata-next` only, the coordinator's
  ruling, the owner said: "Yep I agree." So such a scenario is written on `strata-next`, with
  its ledger row, and not on `main` first as the plan's "Integration with `main`" has it for
  GTK-side changes: agents open no pull requests against `main`, and the scenario earns its
  row only with Rune there. So far: `inline_renaming/rename-survives-entry-listed-above`
  ([A7](packages/A7.md), [#2283](https://github.com/lgse/strata/pull/2283)) and
  `thumbnail_worker_settings/icons-thumbnail-size-keys-saved` ([A1](packages/A1.md),
  [#2281](https://github.com/lgse/strata/pull/2281)).

## Housekeeping

- Merged branches still on `lgse/strata`: H1's `feat/2122-h1-whole-corpus-ledger` and
  `fix/2122-h1-runner-test-in-ci-shards`, and B2's five, which go when B2 is done.
