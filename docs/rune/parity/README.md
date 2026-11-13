# Strata Rune migration: parity QA and fuzzing strategy

> **About this copy.** The plan is the owner's Claude Doc of the same title
> (<https://claude.ai/artifact/XSdySkE6Zs1d2jvHWox9JM>), copied at rev 57 of 2026-11-11. The
> document is the owner's, and this copy follows it.
>
> - The text is the document's, word for word, with its bold. Three things differ in form:
>   the byline below is plain text where the document has a date chip and a mention; tables
>   have Markdown header rules; and the "At a glance" drawing is replaced by a table, as
>   that section says.
> - The [issue map](#issue-map) at the end is this copy's own. It gives each package's
>   issue on `lgse/strata`; the tracker is
>   [#2160](https://github.com/lgse/strata/issues/2160).
> - Since 2026-11-12 packages are tracked in this folder, not in GitHub issues, by the
>   owner's instruction of that day. "Coordination" and the sentences that sent a reader to
>   an issue for a package's state or a finding were changed for it, so this copy differs
>   from the document there. [`STATUS.md`](STATUS.md) has each package's state,
>   [`packages/`](packages/) a file for each, [`owner.md`](owner.md) the questions and
>   rulings, [`findings.md`](findings.md) the findings, and [`working.md`](working.md) how
>   a package is worked on.
> - Since the owner's instruction of 2026-11-12 the `CI` workflow does not run for
>   `strata-next`, so "The merge gate" and the "Decided" item on CI differ from the document
>   too, and [Checks in place of CI](#checks-in-place-of-ci) is this copy's own.
> - Since the owner's rule of 2026-11-12 on mutation testing, the routine checks and the
>   merge gate run no mutation patches, so "Before a package's pull request" and "The merge
>   gate" differ from the document too, and [When a mutation run is
>   right](#when-a-mutation-run-is-right) is this copy's own.

2026-11-11 · William

## Summary

Parity is proven run by run: one scenario corpus and one fuzz generator drive both the GTK
binary and the Rune binary, and a ledger accounts for every result. The slice built that
machinery for one view. This plan widens it to the whole application in 39 work packages
across five waves, with up to twelve running at once.

Every run of the corpus ends in one of three states: passing on both targets from one shared
file, covered by a labelled Rune twin under a named exception, or pending on a tracked
blocker. Differential fuzzing covers the behaviour no scenario names.

| Measure | Today (`strata-next` at `94ece01`) | At exit |
| --- | --- | --- |
| Scenario files | 351 in 58 folders | The same corpus, plus labelled twins |
| Runs | 760: 465 Columns, 122 Icons, 173 List | Every run `required`, `parallel` or `excluded` by a ruling |
| Runs in the parity ledger | 75, all Columns, from 7 folders | 760 |
| Runs passing on both targets | 36 | All but the named exceptions |
| Rune mutation patches | 3 | At least one per scenario folder |
| UI fuzzing | None | Differential on every area, soak clean |

The plan assumes three things:

- **The slice is approved.** V10's report is written, `strata-next` is an integration
  branch, and packages land by pull request.
- **The component library is done.** Every W, S, P, K, C and X package in
  `docs/rune/work/STATUS.md` is merged. What remains is the application layer in
  `crates/strata-app`, the harness's reach, and proof.
- **GTK stays the oracle.** `main` keeps releasing 1.x and decides what correct means until
  the exit wave.

## At a glance

The document shows a drawing here, "work packages by wave · 39 packages, 5 tracks", which
this copy cannot carry; this table of the packages, one column for each wave and one row for
each track, stands in for it.

| Track | Wave 0: foundations | Wave 1: first fan-out | Wave 2: second fan-out | Wave 3: bound to a real window | Wave 4: exit |
| --- | --- | --- | --- | --- | --- |
| Harness | H0, H1, H2 | H3, H4, H5 |  |  |  |
| Base | B1, B2, B3 |  |  |  |  |
| Fuzz | F1 | F2, F3 | F4 | F5 |  |
| Area |  | A1, A2, A3, A4, A5, A6, A7 | A8, A9, A10, A11, A12, A13, A14, A15, A16, A17, A18 | A19, A20, A21 |  |
| Exit |  |  |  |  | E1, E2, E3, E4 |

Read it left to right. A package starts when the packages it names have merged, not when its
wave does. H4, the session host, starts in wave 1 because all of wave 3 waits on it.

## What parity means

A behaviour is at parity when the same input produces the same user-visible result on both
binaries, and a deliberate break of the Rune side is caught. Three proofs carry that, and
each area needs all three:

1. **Scenario parity.** One scenario file, one fixture, both targets pass. The ledger marks
   the run `required`.
2. **Fuzz parity.** One seeded action sequence gives the same observation at every settled
   point, and the same files on disk at the end.
3. **The proof can fail.** A Rune mutation patch breaks the behaviour and a required run
   catches it.

### What must match with no exception

- Accessible names and descriptions. They are a porting requirement, as
  `src/ui/accessibility.rs` sets them.
- Selection, focus and cursor results after an input settles.
- The filesystem end state, and what `settings.toml` holds.
- No panic, and no GLib warning or critical.

### When a run differs: the order of resort

A difference is a Rune-side bug until shown otherwise. When it is shown to be a bug in
Strata itself, record the finding and go straight to step 4. Work down this list and stop at
the first step that fits:

1. **Fix the port.** Change the adapter in `strata-app` or the component, as the slice did
   for seven Columns keys.
2. **Absorb it in the backend.** Key names, role maps, settle rules and field offsets belong
   in the Rune QA backend, never in a scenario.
3. **Reword the scenario on `main`.** A GTK-side change with its own reason keeps one file
   for both targets. It reaches `strata-next` by merge.
4. **Write a labelled twin.** Only under an exception below, with a reference.
5. **Mark it pending.** With a reason code and a blocker someone can act on.

### Exceptions to total parity

The first four are yours. The other three are suggested: all three exist in the ledger today
as reason codes, and the plan promotes them to standing exceptions.

| Code | What differs | Absorbed first by | A twin is allowed when | Record |
| --- | --- | --- | --- | --- |
| `timing` | Animation length, timer length, frame pacing | Waiting on observable state; the backend's settle | The scenario asserts a duration, a mid-animation state, or a GTK timer race from the "Waits for the app's own timers" list | Issue link in both headers |
| `a11y` | Tree shape, a role, or an element GTK exposes and Rune's tree lacks | The backend's role map, and boxes it reads from drawn elements (slice decision 20) | The backend could only present the target by guessing | Issue link, and the missing node named |
| `rune` | Rune lacks the capability at the pinned revision | A workaround, always: a thin `strata-platform` shim, or a workaround in the component or `strata-app`, that names the entry | The workaround reaches the same user-visible result by another route. A run stays pending only when no workaround is feasible | A new entry in `docs/rune/upstream/rune/`, and the workaround it will retire |
| `oracle` | Strata itself has a bug, and the GTK run shows it or its scenario encodes it | Nothing: the bug is filed, not chased | Rune behaves correctly, by design or because the port fixed it in passing. The twin asserts the correct behaviour | An `oracle` issue on `lgse/strata`, linked from both headers |
| `toolkit` (suggested) | A GTK-only check: pixel predicates, `gtk_settings`, GDK details, coordinates tied to GTK's row heights and fonts | Targeting by name instead of offset | The pixel or the coordinate is the point of the scenario. Visual baselines are out of scope: no Rune image set is made | Issue link |
| `deviation` (suggested) | The port behaves differently on purpose | Nothing: it is a decision | An owner ruling exists. The twin asserts the new behaviour | The ruling's link |
| `platform` (suggested) | The run needs a real window, compositor, clipboard or drag | Nothing | Never. The same file runs on the session host instead of the offscreen host | Ledger host field |

Three things are not exceptions. `scope`, `harness` and `component` are temporary pending
reasons and must reach zero. And noise that differs between two GTK runs of one seed is
masked by fuzz calibration, not excused. And a Strata bug that both targets share leaves
them in agreement: record the finding and go on.

Suggested limits on twins, so exceptions stay small:

- A twin keeps the original's fixture, intent and end-state checks. Only the steps the
  exception names may differ.
- A `rune` twin expires. When the pin moves past the fix, the twin and its workaround are
  removed and the run returns to the shared file.
- An `oracle` twin needs no fix to exist: documenting the bug is enough to move on. Its
  issue says why GTK's behaviour is the bug. If `main` later fixes it, the twin is deleted.
- A `deviation` twin needs the owner. So does an `oracle` twin whose bug is arguable: that
  is a deviation. `timing`, `a11y` and `toolkit` twins need the reviewer to agree steps 1 to
  3 do not fit.

## Verification method

Two loops feed one ledger. The scenario loop exists and gets wider. The fuzz loop is new and
reuses the same runner, protocol and fixtures.

### Scenario parity

`STRATA_QA_TARGET=gtk|rune` picks the binary, and `./scripts/qa-parity.sh` runs a selection
on both. The protocol stays frozen: steps, targets, predicates, reply keys and AT-SPI role
names keep their meanings.

The plan changes the ledger in three ways (packages H1 and H2):

- **One ledger file per scenario folder**, replacing the single
  `tests/qa/parity/slice.toml`, so parallel branches never edit the same file.
- **Every run listed.** The runner's unit test already fails on a missing or doubled run; it
  widens from 7 folders to 58.
- **Two more statuses**, `parallel` and `excluded`, beside `required` and `pending`.

| Status | Meaning | Carries |
| --- | --- | --- |
| `required` | Passes on both targets from the shared file | The host: offscreen or session |
| `parallel` | The shared file cannot pass on Rune under an exception; its twin does | Exception code, twin name, reference |
| `pending` | Does not pass on Rune yet | Reason code and blocker, as today |
| `excluded` | Left out of the migration by an owner ruling. It runs on GTK only | The ruling |

### Differential fuzzing

The fuzzer replays one generated input sequence on both targets and compares what a user
could observe. It adds a generator, a comparator and a shrinker to the QA runner.

1. **Generate.** A seed picks actions from an area's alphabet: keys, clicks and menu picks
   on accessibility targets, typed text, and runner-side `fs` changes. Actions name targets
   by role and name, never by coordinate, so each toolkit resolves them in its own geometry.
2. **Replay.** The runner starts each target on a fresh copy of one fixture and sends the
   same steps.
3. **Observe at rest.** Both targets settle after each step. The observation is a fixed
   projection of the reply's `app` snapshot, plus a digest of the fixture tree and of
   `settings.toml`.
4. **Compare.** A difference outside the normalisation map is a divergence. So is a step one
   target refuses, a panic, or a critical.
5. **Shrink.** Shorter sequences are replayed until the divergence is minimal, then
   confirmed three times out of three.
6. **Triage.** Each divergence becomes one finding in [`findings.md`](findings.md) with one
   outcome from the table below.

| Outcome | What happens |
| --- | --- |
| Rune-side bug | Fixed in `strata-app` or the component. The shrunk sequence joins the fuzz corpus |
| Behaviour no scenario owns | Promoted to a shared scenario, written on `main` first |
| Strata bug | An `oracle` issue, and work goes on. The seed is parked against the issue, with an `oracle` rule when Rune behaves correctly |
| Exception | A normalisation rule, with its exception code and issue |
| Rune gap | An upstream packet entry, a workaround, and a `rune` rule that expires with it |
| Noise | The calibration mask grows, only when two GTK runs of the seed differ |

A parked seed, a rule and a mask line cite the finding's reference issue; its state is in
[`findings.md`](findings.md).

Four conventions keep the fuzzer honest:

- **Calibrate on GTK first.** The slice's V3 found the `app` snapshot differs between two
  GTK runs of one scenario. The projection is fixed only once two GTK runs of a seed agree
  on it (F1).
- **Compare only at rest.** Nothing is compared mid-animation. Timeouts are budgets, never
  assertions, which is how the `timing` exception applies to fuzzing.
- **One normalisation map.** A single file of rules, each naming a snapshot field, an
  exception code and an issue. Its rule count is printed in every fuzz summary and reviewed
  at each wave gate.
- **Alphabets belong to areas.** Each area package adds its alphabet with its first required
  runs. Columns navigation, selection and rename are the pilot (F2).

Rune-side invariants (F3) cover states with no GTK peer to compare. Examples: the selection
is a subset of the listing, exactly one element has focus, undo then redo restores the tree
digest, and `Browser`'s cursor and selection equal the view's.

| Fuzz tier | When | Budget |
| --- | --- | --- |
| Smoke | Before a package's pull request merges | The area's alphabet, short sequences. Set by F2's pilot |
| Long run | An agent the owner delegates it to, as often as the owner chooses | Every merged alphabet, long sequences |
| Soak | The exit wave | A fixed number of hours with no new divergence. Owner sets it |

### What neither loop sees

Offscreen, input never crosses winit or a compositor. Window focus, real drag and drop, the
system clipboard and decorations need the session host (H4): a real window in headless sway
with a virtual keyboard and pointer. The look needs a person: hand checks on Hyprland,
GNOME, KDE and X11 at exit (E3). Layout is never asserted, as `AGENTS.md` rules.

## Conventions

These apply to every package. The header keys, the ledger fields and the file paths below
are proposals for H1, H2 and F1 to settle; the rules are the point.

### Parallel scenarios

A twin is a new file beside the original:
`tests/qa/scenarios/<folder>/<scenario>-rune.jsonl`. The GTK file keeps its name and its
steps.

```json
{"scenario":"<scenario>","modes":["columns"],"parity":{"twin":"<scenario>-rune","exception":"timing","ref":"lgse/strata#NNN"}}
```

```json
{"scenario":"<scenario>-rune","modes":["columns"],"targets":["rune"],"parity":{"twin":"<scenario>","exception":"timing","ref":"lgse/strata#NNN","differs":"<one line: which steps differ and why>"}}
```

- **Both files are labelled.** Each header names its twin, the exception code and the issue.
  The `parity` label is the only edit made to a GTK scenario for parity's sake.
- **The twin runs on Rune only.** `targets` keeps it out of the plain GTK suite. The
  scenario header rejects unknown keys today, so H2 adds both keys.
- **The ledger agrees.** The original's run is `parallel` and names the twin. A runner unit
  test checks that the pair exists, names each other and shares one code and reference.
- **The run is the unit.** When only some runs differ, the twin holds only those modes or
  variants.
- **A twin is a full scenario.** It gets messages on its waits, a proof that it can fail,
  and `cost` and `quality`.
- **Twins outlive GTK.** At retirement they lose the suffix and become the corpus (E4).

### Ledger entries

```toml
[[run]]
scenario = "<scenario>"
run = "columns"
status = "parallel"
exception = "timing"
twin = "<scenario>-rune/columns"
ref = "lgse/strata#NNN"
```

- A run moves to `required` only after it passes on both targets, three times running, in
  the pinned image.
- H1's first pass lists every unlisted run as `pending`, reason `scope`, with the owning
  package's id as the blocker.
- A package edits only the rows of its own runs.

### Rune framework gaps

A gap goes into the existing filing packet, `docs/rune/upstream/rune/`, to be filed later by
the owner.

1. Search the packet's entries (100 on 2026-11-12: 98 from the component work and those
   under "Added during parity work" in its README) and the `rune-gap` findings first.
2. Write the entry as the packet's files are written: the notes table with `Checked` and
   `Status` rows, the line "Paste from here.", then the title and body, with a headless
   repro where one exists.
3. Take the next free number: the one after the last row of "Added during parity work",
   which is 101 on 2026-11-12. On a collision the later pull request renumbers at rebase.
4. Add its row to a new README section, "Added during parity work", sorted by number. The
   owner ranks it at the filing pass.
5. Cite the entry from the ledger blocker or the twin, and from the module docs of the shim
   or workaround.

**Then work around it.** A documented gap is never where a package stops. The package builds
a workaround in the same pull request series and takes the run to parity, in this order of
preference:

1. A thin shim in `strata-platform`, behind its own feature, when the gap is generic to a
   Rune app on Linux.
2. A workaround in the component or in `strata-app`, when the gap is specific to one part.
3. A Rune twin that reaches the same result by another route, when neither can reproduce
   GTK's steps.

The workaround's module docs name the entry, and the entry's notes name the workaround. That
pairing is how it gets removed when the pin moves past the fix.

A run stays `pending` on `rune` only when no workaround is feasible. The package's file
then records what was tried, and the run does not block the rest of the package.

One workaround is deferred by owner ruling: audio output. A16 takes PDF, model and silent
media runs to parity, and its audio playback runs stay `pending` on `rune` against packet
entry 26 until Rune ships it or the owner picks a home for the shim.

Agents file nothing upstream, and never vendor or patch Rune.

### Fuzz artifacts

- Shrunk sequences live in a corpus folder per area, named by seed, and the smoke tier
  replays them.
- A sequence is written in scenario format, so a divergence replays with the ordinary
  runner.
- A sequence becomes a named scenario only when no scenario owns the behaviour, as the
  test-value rules in `AGENTS.md` ask.

### Rules carried over from the slice

- **Never edit the oracle to pass.** No change to a GTK scenario's steps, a visual baseline
  or a GTK mutation patch.
- **Container runs only.** QA evidence comes from the pinned image. One QA script at a time
  per machine.
- **Stop rather than widen.** A package that needs something outside its scope writes it up
  in its package file and waits.
- **No GTK in the Rune binary.** `cargo tree -p strata-rune -i gtk4` finds nothing.
- **Two lanes.** Never `--workspace`. Never move `src/ui`, `src/assets` or `src/qa`.
- **Strings stay in the catalogs.** Labels and accessible names reach components as data
  from the thirteen locales.
- **Cite evidence.** Each "done" names the command, the container run and the artifact
  directory.

## Work packages

There are 39 packages in five tracks, and every run has exactly one owner. Each package is
sized for one package file, one worktree and one agent, and may land as several pull
requests.

| Track | Prefix | Purpose |
| --- | --- | --- |
| Harness | H | The ledger, the runner's conventions, the Rune backend's reach, the session host |
| Base | B | Seams in `strata-app` that let area packages add modules without editing one file |
| Fuzz | F | The fuzz driver, differential mode, invariants and the triage rotation |
| Area | A | One part of the interface taken to parity, with its scenario folders |
| Exit | E | Closing the ledger, the soak, hand checks and retiring GTK |

The prefixes avoid the ids already used: W, S, P, K, C, X and V for packages, G for gates, M
for milestones.

### What every area package delivers

1. The part mounted in `strata-app` behind its B1 slot, fed by `Browser` and the shared
   services.
2. GTK's accessible names and descriptions on that part.
3. The backend steps and predicates only that area uses.
4. Every run in its ledger files `required`, `parallel` with a twin, or `pending` on a
   blocker outside the package. No `scope`, `harness` or `component` run it owns is left.
5. One Rune mutation patch per scenario folder, caught.
6. Its fuzz alphabet, and a clean smoke tier.
7. Each Rune gap drafted into the packet and worked around, and each deviation ruled by the
   owner.
8. No inert row left in a menu for a command the area owns.

Run counts below are read from the scenario headers at `94ece01` and cover all three view
modes. They sum to 676. The slice's 75 Columns runs and the 9 visual-baseline runs left out
of scope make 760. A package can flip its Columns runs before A1 and A2 merge. Its Icons and
List runs wait for them.

### Wave 0: foundations

Seven packages that everything else stands on. All but H2 and B3 start at once.

| ID | Package | Delivers | Depends on |
| --- | --- | --- | --- |
| H0 | Issue map and labels | The tracker issue, one issue per package, the label set | None |
| H1 | Whole-corpus ledger | All 760 runs listed in per-folder files, each unlisted run assigned to a package; `qa-parity.sh` selects by folder or package. The slice's 39 pending runs go to the package that owns each blocker | None |
| H2 | Exception conventions in the runner | The `parity` and `targets` header keys, the `parallel` status, the pair checks, and their text in `docs/testing.md` and the `qa-scenarios` skill | H1 |
| B1 | Shell seams | Named slots in the shell for the sidebar, tabs, location bar, view, drawer, footer, overlays and dialogs | None |
| B2 | Shared state out of the GTK package | `PreferenceManager`, the per-folder views, the key catalog and tab sessions moved out of the GTK package to the shared location AGENTS.md names, by the owner's ruling of 2026-11-11. One pull request per store, with GTK glue left behind as a thin wrapper | None |
| B3 | Commands and key dispatch | One command registry and staged key routing that shortcuts, menus and palettes all call. The Menu key (slice decision 22) | B1 |
| F1 | Fuzz driver and GTK calibration | The generator, replay, projection and shrinker on the QA runner. Two GTK runs of one seed agree | None |

### Wave 1: first fan-out

Twelve packages. The seven areas need only the offscreen host.

| ID | Package | Covers or delivers | Depends on |
| --- | --- | --- | --- |
| A1 | Icons view and thumbnails | The Icons runs of the slice's seven folders, `browser_workers`, `thumbnail_worker_settings`: 34 runs | B1 |
| A2 | List view | The List runs of the slice's seven folders: 31 runs | B1 |
| A3 | Sidebar | `sidebar_pinning`, `sidebar_reordering`, `letter_bookmarks`, `recent`: 20 runs | B1 |
| A4 | Tabs | `tabs`: 25 runs | B1, B2 |
| A5 | Location bar | `locations`: 16 runs | B1 |
| A6 | Menus, dialogs and keymap | `dialogs_and_menus`, `keymap`, `accessibility`: 57 runs | B1, B2, B3 |
| A7 | Entry management and undo | `entry_management`, `rename_visibility`: 46 runs, plus the slice's 17 rename runs that stop at undo or a new item (decisions 23 to 25) | B3 |
| H3 | Backend reach | The steps and predicates several areas share: `restart`, session-bus peers, screenshots. Starts from V10's reach figure | H1 |
| H4 | Session host | A real Rune window in headless sway under the same runner. Done when one drag run and one clipboard run pass on both targets | H1 |
| H5 | Coverage-gap scenarios | Scenarios written on `main` for feature-map areas the corpus covers thinly, such as devices and mounts | H0 |
| F2 | Differential mode and Columns pilot | GTK against Rune on Columns navigation, selection and rename. Sets the smoke budget | F1, H2 |
| F3 | Rune-side invariants | The single-target properties, including `Browser` against the view | F1 |

### Wave 2: second fan-out

Twelve packages that build on a view, the menus or the operations of wave 1.

| ID | Package | Covers or delivers | Depends on |
| --- | --- | --- | --- |
| A8 | View switching and appearance | `view_switching`, `text_size`, `column_background`, `created_entry_columns`, `monitor_selection`: 28 runs. The Appearance popover (decision 21) | A1, A2, B2 |
| A9 | Pointer and click modes | `click_modes`, `pointer_intent`, `marquee_scrolling`, `popover_scrolling`: 52 runs | A1, A2 |
| A10 | Search, filter and sort | `search_filter_sort`, `filter_results`: 77 runs | A1, A2, B3 |
| A11 | 10xer mode | The `tenxer` variants spread over 28 scenario files in other packages' folders | A6, A10 |
| A12 | Clipboard and conflicts | `clipboard`, `copy_conflicts`: 34 runs | A7. H4 for runs that read the system clipboard |
| A13 | Archives and providers | The five `archive_*` folders and `file_providers`: 44 runs | A6, A7 |
| A14 | Open with, terminal and custom actions | `open_with`, `terminal`, `custom_actions`: 22 runs | A6 |
| A15 | Preview drawer, text and documents | `preview_session` and the drawer, text and document files of `quick_preview`. 72 runs together with A16 | B1 |
| A16 | Media, PDF and model viewers | `preview_media_layout`, `preview_video` and the PDF, media and model files of `quick_preview` | A15. Audio playback runs wait on Rune, not on the `media-audio` shim (decision 15) |
| A17 | Properties | `properties_media`, `properties_size`: 12 runs | A6 |
| A18 | Settings and themes | `preferences`: 4 runs, the settings rows of A6's folders, and live preferences across two windows | A6, B2 |
| F4 | Fuzz rotation | The long-run tier, triage of its issues and corpus upkeep. Delegated in rounds until exit | F2 |

### Wave 3: bound to a real window

Four packages, all waiting on the session host.

| ID | Package | Covers or delivers | Depends on |
| --- | --- | --- | --- |
| A19 | Drag and drop | `drag_and_drop`, `drag_animation`, `content_drops`, `cross_volume_drop`, `sidebar_file_drops`: 76 runs | H4, A1 to A5 |
| A20 | Portal chooser | `chooser_selection`, `chooser_image_conversion`: 24 runs, on `strata-rune --qa --portal` | H4, A1, A2, A5 |
| A21 | Window and launch | `file_manager_interface`: 2 runs. Decorations, single instance and startup on the session host | H4 |
| F5 | Session-host fuzzing | Pointer and drag alphabets through the compositor | H4, F2 |

### Wave 4: exit

| ID | Package | Delivers | Depends on |
| --- | --- | --- | --- |
| E1 | Close the ledger | Zero `scope`, `harness` and `component` runs. Every run not `required` carries an exception and a reference | Every A and H package |
| E2 | Soak | The full parity set three times running, the fuzz soak, and the GTK suite with its mutation patches unchanged | E1, F4, F5 |
| E3 | Desktop hand checks | The windowed binary on Hyprland, GNOME, KDE and X11, by a person | A19, A20, A21 |
| E4 | Retire GTK | `src/ui`, `src/assets` and `src/qa` deleted, twins renamed into the corpus, the ledger retired, and no `gtk4` in `cargo tree` | E2, E3 |

## Coordination

The files of this folder hold the state agents share, and `strata-next` is where finished
work meets. Work in flight is seen in the open pull requests that target `strata-next`.

### Tracking

- **[`packages/<ID>.md`](packages/)** is the source for each package: its state (`not
  started`, `in progress`, `done` or `held`), its pull requests, its done-when list, its
  runs, a dated record of what merged and what was decided, what its dependents must know,
  and what is open. **[`STATUS.md`](STATUS.md)** gathers the states in one table; whoever
  coordinates keeps it, and package pull requests do not edit it.
- **A package's agent edits its own file,** in the package's own pull requests. A note for
  another package goes under "Notes from other packages" in that package's file, in the pull
  request that finds it; nothing else in another package's file changes.
- **The owner assigns.** The owner hands each package to an agent, and that assignment is
  the claim. The agent checks that every dependency's file reads `done`; its first pull
  request sets the package's state to `in progress`.
- **Findings.** Search [`findings.md`](findings.md), then the issues, by code and by
  scenario or seed name before adding one. A new finding gets a GitHub issue purely as its
  reference, which may be closed at once, and an entry in `findings.md`, which holds its
  state, in the pull request that meets it
  ([`findings.md`](findings.md#recording-a-finding)).
- **Report in the package's file.** Each pull request adds one dated entry to its record:
  its number, what it delivered and the runs it flipped. The commands and the evidence are
  on the pull request.
- **Questions for the owner** go in [`owner.md`](owner.md), with the default in force
  meanwhile, and so do the owner's rulings.
- **A stalled package goes back** to the owner, who hands it on. There is no timer.
- **The archived issues.** Until 2026-11-12 each package, the tracker and each finding was a
  GitHub issue on `lgse/strata` ([the issue map](#issue-map)). They were closed that day,
  finished or not. They stay as an archive that the package files and the findings link to:
  not reopened, by the coordinator's rule, and no longer where work is recorded. A
  reference to one stays valid.

The plan's labels live on as codes in the files above, in the ledger and in the fuzz
driver's files:

| Code | Used for |
| --- | --- |
| `divergence` | A failing parity run or a shrunk fuzz sequence. The entry names the area, the first differing field and the last action, so duplicates are found by search |
| `rune-gap` | A gap bound for the filing packet. Closed when its entry and its workaround merge |
| `exception` | A request for a twin or a normalisation rule, naming its code. `deviation` requests wait for the owner |
| `oracle` | A bug in Strata itself, or a flaky GTK run. Documented and set aside; any fix lands on `main` |
| `contract` | A change to a seam others build on: B1's slots, the protocol in `strata-qa`, the runner, a shared component. A pull request that makes one carries the label |

[`working.md`](working.md) restates what a package's agent does, from claiming to hand-off.

### Branches and worktrees

- **Branch** `<type>/<issue>-<package>-<slug>`, for example `feat/2134-a7-entry-management`,
  as `docs/rune/work/README.md` names them. The number is the package's archived issue's.
- **One worktree per package**, such as `../strata-a7`, with its own `target/`.
- **Pull requests target `strata-next`**, with Conventional Commit titles and the
  repository's template. They link the package's file where the template asks for the
  issue.
- **Small pull requests.** One per scenario folder is a natural size. Each flips the ledger
  rows it proves.
- **Contracts first.** A `contract` change lands as its own small pull request, and
  dependents merge `strata-next` afterwards.
- **Merge, never rebase.** A branch takes `strata-next` by merge. `strata-next` itself is
  never rebased.

### Integration with `main`

- GTK-side changes go to `main` through its own issue and pull request: a reworded scenario,
  a new scenario from H5, an oracle fix when one is taken up. They reach `strata-next` by
  merge, never by a second hand-written copy.
- An agent the owner delegates it to merges `main` into `strata-next` when the owner asks,
  then reruns the full GTK suite and the parity set.
- A Rune pin bump is its own change. Merges pause while it lands and the goldens are
  re-blessed.

### Shared files

| File | Rule |
| --- | --- |
| `tests/qa/parity/<folder>.toml` | A package edits only its own runs' rows |
| The shell in `strata-app` | B1 owns the slots. An area adds a module and one registration line |
| The normalisation map | One rule per line, sorted by field. A pull request adds only its own rules |
| `docs/rune/upstream/rune/README.md` | Append to "Added during parity work" only |
| `data/locales/*` | A new key goes into all thirteen catalogs in the same pull request |
| `Cargo.lock` | Never hand-merged. Take `strata-next`'s copy and regenerate |
| The protocol in `crates/strata-qa`, the runner in `tests/qa` | `contract` pull requests only |

### Roles

| Role | Does |
| --- | --- |
| Owner | Delegates each package and each merge from `main`. Rulings on deviations, filing upstream, hand checks, the checkpoints |
| Package agent | One assigned package in one worktree. Runs the merge gate before merging, and keeps the package's file current |
| Reviewer | A session other than the author's reviews each pull request |

## Validation and gates

Validation stays risk-based, as `AGENTS.md` sets it. An agent runs what its change affects,
in the pinned image, and records the commands. Full runs belong to the merge gate and the
checkpoints.

### Before a package's pull request

| Check | Command |
| --- | --- |
| The runs it claims, on both targets | `./scripts/qa-parity.sh <selection> --scenario <name>`. Selections by folder and package arrive with H1 |
| Which pending runs already pass on Rune | `./scripts/qa-parity.sh <selection> --pending` |
| Rune-side unit and gallery tests | `mise run test-rune <filter>`, with a filter that matches at least one test |
| The GTK side, when a shared crate or `src/` changed | `./scripts/qa.sh <scenario>` and `./scripts/test-headless.py <filter>` |
| The fuzz smoke tier for its alphabet | F1's command |
| That every mutation patch still applies, when anything under `crates/` or `src/` changed: seconds, no container, and no mutation run | The `git apply --check` loop below. Only `cut.patch` (finding #2161) is known not to apply |
| No GTK in the Rune binary | `cargo tree -p strata-rune -i gtk4` |
| Formatting, lint and code health, at the pre-push checkpoint only | `./scripts/quality.sh fmt`, `./scripts/quality.sh clippy`, `scripts/code-health.sh --changed` |

The patch check, from the repository root:

```bash
for p in tests/qa/parity/mutations/*.patch tests/qa/mutations/*.patch; do git apply --check "$p" 2>/dev/null || echo "does not apply: $p"; done
```

### The merge gate

The package's agent runs the gate itself before merging, on its branch with the integration
branch merged in. CI checks only linting and unit tests, and on `strata-next` it does not run
at all for now; the agent runs its [checks in place of CI](#checks-in-place-of-ci) instead,
and a merge does not wait for CI. The gate runs the package's ledger
selection on both targets (`./scripts/qa-parity.sh <package>`). When the runner,
`strata-qa`, a shared crate or `src/` changed, it is the full gate instead: every required
and parallel run on both targets (`./scripts/qa-parity.sh all`), the whole GTK suite
(`./scripts/qa.sh`) and `./scripts/quality.sh test`. Neither runs a mutation patch.
`./scripts/qa.sh` does not read the slice's flaky allowance. A failure there of
`new-folder-with-selection-groups/columns` passes as finding #2162 only if it is the only
failure, it fails as #2162 does (the opened folder's column does not take focus within
15 s), and the change does not touch what the run exercises; otherwise it is the change's.

### Checks in place of CI

On `strata-next` the `CI` workflow does not run, by the owner's instruction of 2026-11-12
([`owner.md`](owner.md#ruled)), and a merge does not wait for it. The package's agent runs
what CI carried, as far as the change reaches, and writes each result on the pull request; a
check skipped is named with its reason. This list is the orchestrating session's reading of
"agent's own checks are sufficient", for the owner to confirm.

| When | Check |
| --- | --- |
| Always | `typos`, and `mise run scripts`: the Python tests under `scripts/`, which hold the ledger, the mutation patches' rules and the fuzz files, and read `.github/workflows/`, `tests/container/`, the manifests and `packaging/`. CI ran them on every pull request; they take seconds |
| `src/`, a shared crate, `tests/` or a manifest changed | `./scripts/quality.sh fmt` and `./scripts/quality.sh clippy` |
| A Rune-side crate changed | The Rune lane's `cargo fmt --all --check`, and `cargo clippy $(python3 scripts/lanes.py rune) --all-targets --all-features --locked -- -D warnings` |
| `src/` or a shared crate changed | `./scripts/quality.sh test`. With CI off it is the only run of the GTK unit tests, so it is required there |
| `src/` or a shared crate changed | The build without UnRAR: the GTK lane checked with `--no-default-features --features test-support`, and four tests under it, below |
| A manifest, `Cargo.lock` or `deny.toml` changed | `cargo deny check`, the two `cargo tree` guards of `ci.yml`'s "Quality build and lint", and the same UnRAR guard for `strata-rune`, below |

The two guards, from the repository root, and the third for the Rune binary; each holds when
it prints nothing:

```bash
cargo tree --locked --no-default-features -e normal,build --prefix none | grep -E "^unrar(_sys)? v"
{ cargo tree --locked --no-default-features -e normal,build --prefix none; cargo tree --locked -e normal,build --prefix none; } | grep -E "^(x11rb|wayland-client|wayland-protocols-(misc|wlr)|xkbcommon) v"
cargo tree --locked -p strata-rune --no-default-features -e normal,build --prefix none | grep -E "^unrar(_sys)? v"
```

CI's guards read the root package only. Since `strata-rune` builds with UnRAR by default (a
`rar` feature, [A1](packages/A1.md)), the third is the only check that `strata-rune` without
default features carries no UnRAR. At the cutover `ci.yml`'s "Quality build and lint" needs
the same guard for `strata-rune`.

The build without UnRAR, from `ci.yml`'s step "Check and exercise the build without UnRAR".
CI runs it inside the pinned image, by a `docker run` of the quality image, and no script
wraps it; `./scripts/test-headless.py` cannot select `--no-default-features`. The same
commands run on the host from the repository root, the last under the private Xvfb and
D-Bus session the workflow gives it, never the desktop's:

```bash
gtk="$(python3 scripts/lanes.py gtk)"
cargo check --locked $gtk --all-targets --no-default-features --features test-support
cargo test --locked $gtk --all-targets --no-default-features --features test-support launch_mode
cargo test --locked $gtk --all-targets --no-default-features --features test-support services::operations::tests
cargo test --locked $gtk --all-targets --no-default-features --features test-support rar_disabled
xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp -noreset" dbus-run-session -- \
  env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --locked $gtk --all-targets --no-default-features --features test-support archive_extraction_actions_follow_build_support
```

Where the loss is largest:

- **A merge of `main` into `strata-next`** brings GTK changes with no CI run. Whoever is
  delegated one runs the full gate, `./scripts/quality.sh test` and the build without UnRAR
  included.
- **`cargo deny`** no longer sees a new advisory on `strata-next` unless a manifest changes
  there; `main`'s CI still checks nearly the same lock file.

### When a mutation run is right

Mutation runs are too heavy on wall-clock time to be routine verification. By the owner's
rule of 2026-11-12 ([`owner.md`](owner.md#ruled)), one is run only when there is a question
whether tests are effective:

- **A Rune mutation patch a package wrote or changed** is proven once, alone, by hand, since
  `./scripts/qa-parity.sh mutations` runs every Rune patch and has no per-patch selection
  yet. Start with the files the patch touches clean. Run its scenario on Rune unpatched
  (`./scripts/qa-parity.sh <folder> --target rune --scenario <scenario>`) and see it pass.
  Apply the patch (`git apply tests/qa/parity/mutations/<scenario>.patch`), run the same
  selection and see a step fail: a build or launch failure, or a mutated tree that ran
  nothing, is not a catch. Restore the files (`git restore -- <files>`, or `git apply -R`),
  and see `git status --porcelain` empty before committing anything.
- **A patch that stopped applying** because the code it breaks moved, once it is refreshed.
  The check before a pull request finds one.
- **A runner change to how a step is judged.**
- **A run suspected of passing for the wrong reason.**
- **A GTK scenario whose `mutation` changed**, or whose patch stopped applying:
  `./scripts/qa-mutation.sh <scenario>`, as `AGENTS.md`, "QA scenarios", says.
- **The exit checks:** E2's soak and exit criterion 3 are deliberate checks that the tests
  are effective, and stay as written.

Deliverable 5, a twin's patch and the risk row on twins are about a patch existing and
having been proven once, not about running the sets again.

### Checkpoints

Waves are dependency waves, not locks: a package starts when its own dependencies close.
Checkpoints are where the owner looks at the numbers.

| Checkpoint | Passes when |
| --- | --- |
| End of wave 0 | H1 lists 760 runs, each with a package. B1 is merged. Two GTK runs of one fuzz seed agree |
| End of wave 1 | Icons and List pass the slice's folders. The Columns fuzz pilot is clean. One drag run and one clipboard run pass on the session host |
| End of wave 2 | No `scope` run is left outside wave 3's folders. Every merged area has its alphabet in the long-run tier |
| End of wave 3 | Drag and clipboard round trips with a second process pass on the session host. A portal open and save pass on `strata-rune` |
| Exit | The six criteria below |

Every checkpoint report gives the same five counts: runs by status and reason, twins by
exception code, normalisation rules by code, open `divergence` and `oracle` findings, and
packet entries added. [`STATUS.md`](STATUS.md#checkpoints) has them as they stand, with
the commands.

### Exit criteria

1. Every run is `required` or `parallel`, or excluded by a ruling. A run still pending on
   `rune` needs a record of the workarounds tried and an owner ruling to ship without it.
2. The full parity set passes three times running, on both targets and both hosts.
3. Every Rune mutation patch is caught, and the GTK suite and its patches behave as before.
4. The fuzz soak passes with no open `divergence` finding.
5. The hand checks on four desktops are recorded.
6. `cargo tree` shows no `gtk4`, `gdk4`, `sourceview5` or `gdk-pixbuf`.

The thumbnail worker is the ruled exception to criterion 6 until E4: by the owner's ruling
of 2026-11-12 it uses `gdk-pixbuf` in `strata-rune`, and E4 removes it
([`owner.md`](owner.md#ruled)). `gdk-pixbuf` is in `strata-rune`'s tree already, without the
worker: `strata-fs` depends on it directly and through `strata-preview`, so E4's removal
covers those too, unless the owner narrows the criterion
([`owner.md`](owner.md#e4-exit-criterion-6)). It is an exception for
`gdk-pixbuf` alone: `gtk4`, `gdk4` and `sourceview5` stay out of `strata-app` and
`strata-rune`, and `cargo tree -p strata-rune -i gtk4` still finds nothing.

Three oracle defects are open today and should be the first `oracle` issues: `cut.patch` no
longer applies, `new-folder-with-selection-groups/columns` fails more often than not alone,
and `distant-archive-scrolled-into-view/icons` failed once under load.

## Risks

The largest risk is that exceptions grow until parity means little. The plan contains it by
making every exception a counted, referenced record.

| Risk | Containment |
| --- | --- |
| Twins and normalisation rules hide real differences | Every one carries a code and an issue. Counts are reported at each checkpoint. A twin must fail under a mutation patch |
| A flaky or wrong oracle sends agents after GTK's faults | `oracle` findings are recorded and set aside, not chased; a fix, if any, lands on `main` first. A divergence is confirmed three times out of three before it is filed |
| Offscreen parity is read as more than it is | `platform` runs name the session host. Hand checks gate the exit |
| The harness reads the view, not `Browser` | F3's invariant compares the two after every fuzz step |
| Parallel branches collide in the shell or the ledger | B1's slots and per-folder ledger files land in wave 0 |
| The session host is the long pole for about 100 runs | H4 starts in wave 1. Its three areas use the offscreen host for every run that allows it |
| A merge from `main` or a Rune pin bump moves the ground | Both are tasks the owner delegates on their own, with a rerun of the parity set. Merges pause for a pin bump |
| Thin scenario cover lets an area pass by saying little | H5 writes scenarios on `main` first. Fuzzing and invariants cover what scenarios do not name |
| Many worktrees load one machine | QA scripts run one at a time per machine. Each worktree keeps its own `target/` cache |
| Inert rows and unbound keys look like parity | Area done-when item 8. The slice found two required runs that pass only because a key is unbound |

## Open questions for the owner

Three remain. The first two wait on what the gaps turn out to be.

- [ ] **Shipping with open Rune gaps.** Which `rune` pending runs may remain at exit. Left
  open on 2026-11-11: it depends on the size of each gap and whether a workaround is found.
- [ ] **The cutover.** Whether `main` takes `strata-next` by merge at E4, and which release
  carries it. Left open on 2026-11-11 for the same reason.
- [ ] **Golden images altogether.** Whether to drop them for their wall-clock cost. Raised
  on 2026-11-11 and not ruled. Until it is, the existing GTK baselines and the Rune
  gallery's goldens stay as they are.

### Decided

- **Labelling GTK scenario files.** Yes, ruled 2026-11-11. The `parity` header key is the
  one edit allowed to a GTK scenario for parity's sake, with its steps unchanged.
- **Twin approval.** Yes, ruled 2026-11-11. The reviewer approves `timing`, `a11y`,
  `toolkit` and clear-cut `oracle` twins. Only `deviation` twins, and `oracle` twins whose
  bug is arguable, need the owner.
- **Slice decision 22, the Menu key.** B3 wraps each view's keyboard controller once, so the
  Menu key and Shift+F10 open the context menu in Columns, Icons and List.
- **Slice decision 23, undo.** A7 binds it as its first pull request. `Browser` already has
  the engine.
- **Slice decision 24, naming a new folder or file.** Confirmed as A7's.
- **Slice decision 25, a rename that fails.** Parity during the migration: A7 shows GTK's
  "Unable to rename item" dialog. The reopened inline editor is not taken as a deviation.
- **Slice decision 21, the Appearance popover.** A8 owns it as a host composition built from
  the existing slider and toggle components, not as a context menu.
- **Slice decision 15, a home for the audio shim.** Deferred as long as possible, on the
  expectation that Rune closes the gap (packet entry 26, PCM output on Linux). No shim is
  imported meanwhile. The question returns at each Rune pin bump, and at the end of wave 3
  at the latest.
- **B2's moves.** All four stores move: `PreferenceManager`, the per-folder views, the key
  catalog and tab sessions. Their toolkit-free parts go to `crates/strata-desktop`, beside
  `strata_desktop::settings`. `AGENTS.md` gains a "Shared state" section that says so.
- **No CI for parity or fuzzing.** CI runs only linting and unit tests. Each agent runs the
  verification before merging. The planned CI package, H7, is dropped. (This copy: on
  `strata-next` CI does not run at all for now, by the owner's instruction of 2026-11-12;
  see [Checks in place of CI](#checks-in-place-of-ci).)
- **Fuzz budgets.** Calibrated after work starts, from F2's pilot numbers.
- **No coordinator.** The owner delegates each package by hand. The coordinator's duties go
  to the package agents and to tasks the owner hands out, and there is no stale-claim timer.
- **Golden images on Rune.** Not in scope. The planned package for Rune visual baselines,
  H6, is dropped, and the test image gets no software Vulkan driver. The 9
  `visual_baselines` runs are `excluded`: they run on GTK only.

## Issue map

This section is not part of the owner's document. It was added with H0, so that a package's
issue can be found from the plan. The issues are an archive since 2026-11-12: a package's
state is in [`STATUS.md`](STATUS.md) and its file, and the findings are in
[`findings.md`](findings.md).

The tracker is [#2160](https://github.com/lgse/strata/issues/2160), and every package is its
sub-issue. The first three `oracle` issues are
[#2161](https://github.com/lgse/strata/issues/2161) (`cut.patch` no longer applies),
[#2162](https://github.com/lgse/strata/issues/2162)
(`new-folder-with-selection-groups/columns` is flaky) and
[#2163](https://github.com/lgse/strata/issues/2163)
(`distant-archive-scrolled-into-view/icons` failed once under load).

| Package | Title | Issue | Wave | Depends on |
| --- | --- | --- | --- | --- |
| H0 | Issue map and labels | [#2121](https://github.com/lgse/strata/issues/2121) | 0 | None |
| H1 | Whole-corpus ledger | [#2122](https://github.com/lgse/strata/issues/2122) | 0 | None |
| H2 | Exception conventions in the runner | [#2123](https://github.com/lgse/strata/issues/2123) | 0 | H1 ([#2122](https://github.com/lgse/strata/issues/2122)) |
| B1 | Shell seams | [#2124](https://github.com/lgse/strata/issues/2124) | 0 | None |
| B2 | Shared state out of the GTK package | [#2125](https://github.com/lgse/strata/issues/2125) | 0 | None |
| B3 | Commands and key dispatch | [#2126](https://github.com/lgse/strata/issues/2126) | 0 | B1 ([#2124](https://github.com/lgse/strata/issues/2124)) |
| F1 | Fuzz driver and GTK calibration | [#2127](https://github.com/lgse/strata/issues/2127) | 0 | None |
| A1 | Icons view and thumbnails | [#2128](https://github.com/lgse/strata/issues/2128) | 1 | B1 ([#2124](https://github.com/lgse/strata/issues/2124)) |
| A2 | List view | [#2129](https://github.com/lgse/strata/issues/2129) | 1 | B1 ([#2124](https://github.com/lgse/strata/issues/2124)) |
| A3 | Sidebar | [#2130](https://github.com/lgse/strata/issues/2130) | 1 | B1 ([#2124](https://github.com/lgse/strata/issues/2124)) |
| A4 | Tabs | [#2131](https://github.com/lgse/strata/issues/2131) | 1 | B1 ([#2124](https://github.com/lgse/strata/issues/2124)), B2 ([#2125](https://github.com/lgse/strata/issues/2125)) |
| A5 | Location bar | [#2132](https://github.com/lgse/strata/issues/2132) | 1 | B1 ([#2124](https://github.com/lgse/strata/issues/2124)) |
| A6 | Menus, dialogs and keymap | [#2133](https://github.com/lgse/strata/issues/2133) | 1 | B1 ([#2124](https://github.com/lgse/strata/issues/2124)), B2 ([#2125](https://github.com/lgse/strata/issues/2125)), B3 ([#2126](https://github.com/lgse/strata/issues/2126)) |
| A7 | Entry management and undo | [#2134](https://github.com/lgse/strata/issues/2134) | 1 | B3 ([#2126](https://github.com/lgse/strata/issues/2126)) |
| H3 | Backend reach | [#2135](https://github.com/lgse/strata/issues/2135) | 1 | H1 ([#2122](https://github.com/lgse/strata/issues/2122)) |
| H4 | Session host | [#2136](https://github.com/lgse/strata/issues/2136) | 1 | H1 ([#2122](https://github.com/lgse/strata/issues/2122)) |
| H5 | Coverage-gap scenarios | [#2137](https://github.com/lgse/strata/issues/2137) | 1 | H0 ([#2121](https://github.com/lgse/strata/issues/2121)) |
| F2 | Differential mode and Columns pilot | [#2138](https://github.com/lgse/strata/issues/2138) | 1 | F1 ([#2127](https://github.com/lgse/strata/issues/2127)), H2 ([#2123](https://github.com/lgse/strata/issues/2123)) |
| F3 | Rune-side invariants | [#2139](https://github.com/lgse/strata/issues/2139) | 1 | F1 ([#2127](https://github.com/lgse/strata/issues/2127)) |
| A8 | View switching and appearance | [#2140](https://github.com/lgse/strata/issues/2140) | 2 | A1 ([#2128](https://github.com/lgse/strata/issues/2128)), A2 ([#2129](https://github.com/lgse/strata/issues/2129)), B2 ([#2125](https://github.com/lgse/strata/issues/2125)) |
| A9 | Pointer and click modes | [#2141](https://github.com/lgse/strata/issues/2141) | 2 | A1 ([#2128](https://github.com/lgse/strata/issues/2128)), A2 ([#2129](https://github.com/lgse/strata/issues/2129)) |
| A10 | Search, filter and sort | [#2142](https://github.com/lgse/strata/issues/2142) | 2 | A1 ([#2128](https://github.com/lgse/strata/issues/2128)), A2 ([#2129](https://github.com/lgse/strata/issues/2129)), B3 ([#2126](https://github.com/lgse/strata/issues/2126)) |
| A11 | 10xer mode | [#2143](https://github.com/lgse/strata/issues/2143) | 2 | A6 ([#2133](https://github.com/lgse/strata/issues/2133)), A10 ([#2142](https://github.com/lgse/strata/issues/2142)) |
| A12 | Clipboard and conflicts | [#2144](https://github.com/lgse/strata/issues/2144) | 2 | A7 ([#2134](https://github.com/lgse/strata/issues/2134)). H4 ([#2136](https://github.com/lgse/strata/issues/2136)) for runs that read the system clipboard |
| A13 | Archives and providers | [#2145](https://github.com/lgse/strata/issues/2145) | 2 | A6 ([#2133](https://github.com/lgse/strata/issues/2133)), A7 ([#2134](https://github.com/lgse/strata/issues/2134)) |
| A14 | Open with, terminal and custom actions | [#2146](https://github.com/lgse/strata/issues/2146) | 2 | A6 ([#2133](https://github.com/lgse/strata/issues/2133)) |
| A15 | Preview drawer, text and documents | [#2147](https://github.com/lgse/strata/issues/2147) | 2 | B1 ([#2124](https://github.com/lgse/strata/issues/2124)) |
| A16 | Media, PDF and model viewers | [#2148](https://github.com/lgse/strata/issues/2148) | 2 | A15 ([#2147](https://github.com/lgse/strata/issues/2147)). Audio playback runs wait on Rune, not on the `media-audio` shim (decision 15) |
| A17 | Properties | [#2149](https://github.com/lgse/strata/issues/2149) | 2 | A6 ([#2133](https://github.com/lgse/strata/issues/2133)) |
| A18 | Settings and themes | [#2150](https://github.com/lgse/strata/issues/2150) | 2 | A6 ([#2133](https://github.com/lgse/strata/issues/2133)), B2 ([#2125](https://github.com/lgse/strata/issues/2125)) |
| F4 | Fuzz rotation | [#2151](https://github.com/lgse/strata/issues/2151) | 2 | F2 ([#2138](https://github.com/lgse/strata/issues/2138)) |
| A19 | Drag and drop | [#2152](https://github.com/lgse/strata/issues/2152) | 3 | H4 ([#2136](https://github.com/lgse/strata/issues/2136)), A1 to A5 ([#2128](https://github.com/lgse/strata/issues/2128) to [#2132](https://github.com/lgse/strata/issues/2132)) |
| A20 | Portal chooser | [#2153](https://github.com/lgse/strata/issues/2153) | 3 | H4 ([#2136](https://github.com/lgse/strata/issues/2136)), A1 ([#2128](https://github.com/lgse/strata/issues/2128)), A2 ([#2129](https://github.com/lgse/strata/issues/2129)), A5 ([#2132](https://github.com/lgse/strata/issues/2132)) |
| A21 | Window and launch | [#2154](https://github.com/lgse/strata/issues/2154) | 3 | H4 ([#2136](https://github.com/lgse/strata/issues/2136)) |
| F5 | Session-host fuzzing | [#2155](https://github.com/lgse/strata/issues/2155) | 3 | H4 ([#2136](https://github.com/lgse/strata/issues/2136)), F2 ([#2138](https://github.com/lgse/strata/issues/2138)) |
| E1 | Close the ledger | [#2156](https://github.com/lgse/strata/issues/2156) | 4 | Every A and H package |
| E2 | Soak | [#2157](https://github.com/lgse/strata/issues/2157) | 4 | E1 ([#2156](https://github.com/lgse/strata/issues/2156)), F4 ([#2151](https://github.com/lgse/strata/issues/2151)), F5 ([#2155](https://github.com/lgse/strata/issues/2155)) |
| E3 | Desktop hand checks | [#2158](https://github.com/lgse/strata/issues/2158) | 4 | A19 ([#2152](https://github.com/lgse/strata/issues/2152)), A20 ([#2153](https://github.com/lgse/strata/issues/2153)), A21 ([#2154](https://github.com/lgse/strata/issues/2154)) |
| E4 | Retire GTK | [#2159](https://github.com/lgse/strata/issues/2159) | 4 | E2 ([#2157](https://github.com/lgse/strata/issues/2157)), E3 ([#2158](https://github.com/lgse/strata/issues/2158)) |
