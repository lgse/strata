# Owner decisions

What the parity work waits on the owner for, and what the owner has ruled. A package's agent
adds an open question here in the pull request that meets it, with the default it works to
meanwhile, and moves it to "Ruled" with the date when the owner answers. The plan's own
decisions are its ["Decided" list](README.md#decided) and are not repeated here.

"The orchestrating session" is the session the owner delegated the waves to. A default it
set stands until the owner rules otherwise. Where the only record of an item is that
session's notes, the item says so.

## Open

### Tracking and findings

- **What a reference to a new finding looks like.** The ledger, twins, the fuzz driver's map,
  mask and `parked` lists accept an issue or a packet entry and nothing else
  ([`findings.md`](findings.md#recording-a-finding)), and `docs/testing.md`, "Twins", asks a
  `deviation` twin for "the issue that records the owner's ruling". Now that findings and
  rulings are recorded in these files, what stands in those places for an entry with no
  issue? Meanwhile: nothing cites a finding recorded only here; a pull request that needs to
  says so and waits. Source: recorded by the orchestrating session on 2026-11-12.
- **The plan's copy and the owner's document.** From the pull request that moved tracking
  here, the copy in [`README.md`](README.md) differs from the owner's document in
  "Coordination" and in the sentences that sent a reader to an issue for a package's state;
  the note at its top says so. Whether the document takes the same change. Meanwhile: the
  copy is what agents follow. Source: that pull request.

### H1: Whole-corpus ledger

Each of these is H1's reading of the plan or a default the orchestrating session set while
the owner was away; the rules as they landed are in `docs/testing.md`, "Who owns a run", and
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
  session, in "Twins", in force. Source: [#2123](https://github.com/lgse/strata/issues/2123). Where that record is now is the reference
  question above.
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

### B2: Shared state out of the GTK package

Sources: the archived issue [#2125](https://github.com/lgse/strata/issues/2125), its
comments of 2026-11-12, unless another is named.

- **Whether the bookkeeping-only shape meets the ruling.** B2 moves `PreferenceManager`'s
  save bookkeeping and wires the Rune side to it, and leaves every getter and setter in
  `src/ui`; a setter's body moves when a Rune package first needs it. That is narrower than
  the plan's "moved out of the GTK package". In force, as built in [#2172](https://github.com/lgse/strata/pull/2172) and [#2178](https://github.com/lgse/strata/pull/2178).
- **Several pull requests per store**, against the plan's "one pull request per store": two
  so far for `PreferenceManager` ([#2172](https://github.com/lgse/strata/pull/2172), [#2178](https://github.com/lgse/strata/pull/2178)) and for the per-folder views ([#2168](https://github.com/lgse/strata/pull/2168), [#2188](https://github.com/lgse/strata/pull/2188)),
  each with more to come, and the key catalog in three parts. In force, taken as within the
  ruling's intent, for review safety.
- **The key lookups behind one backend.** The five key lookups and the accelerator reader
  and writer sit behind one backend in `strata_desktop::keys`; GTK installs a GDK-backed one
  and stays an exact oracle; the `xkbcommon` backend is a `strata-desktop` feature that only
  `strata-app` enables. So CI's guard against `xkbcommon` in a release build stays as it is.
  A ruling of the orchestrating session after an independent read, waiting for the owner's
  confirmation. In force: it orders B2's remaining work.
- **The Rune side writes `settings.toml` as GTK does**, because the plan puts what
  `settings.toml` holds among what must match with no exception. A default of the
  orchestrating session; in force since [#2178](https://github.com/lgse/strata/pull/2178).
- **One settings store per process, or per window, on Rune**, and how wide "live across two
  windows" is: from another process GTK adopts five keys only, the sort and the icon size.
  Meanwhile: one `Saving` for the file in a process, and the Rune side follows nothing
  another process saves.
- **Where policy that spans the settings and the per-folder views lives**
  (`set_remember_folder_views`, `set_default_sort_options`, `set_folder_icons_size`,
  `set_default_icons_size`). Meanwhile: in `src/ui`'s wrapper, which the Rune side would
  re-implement.
- **Where `active_omarchy_theme` lives**: the Omarchy variant methods need it, and it is
  neither a saved value nor bookkeeping. Meanwhile: in `src/ui`.
- **A home for the exhaustive saved-preferences fixture** before `src/ui` is deleted; the
  Rune write path has no equivalent coverage. Meanwhile: GTK-only. Also on the archived
  issue [#2150](https://github.com/lgse/strata/issues/2150).
- **The `grid` and `explorer` aliases of the view mode**, which need `browser_modes` and are
  outside the ruling. Meanwhile: each Rune view's `shows` answers to both its name and its
  alias. Also on the archived issue [#2124](https://github.com/lgse/strata/issues/2124).
- **Whether B2 can close with the folder-view timer rule left in each interface.**
  Meanwhile: it is left in each.
- **`src/ui/keys.rs` becoming `src/ui/keys/mod.rs`** when the catalog moves without its event
  reader, the shape that keeps merges from `main` landing in the shared file. Meanwhile:
  that shape.
- **A key with no name is written as `0x…` or `U+XXXX` and is not read back**, as GTK does
  today. Whether that is behaviour to keep. Meanwhile: kept exactly.
- **For the owner to know:** B2's agent read `gtk/gtkaccelgroup.c` during the design, before
  the ruling that nothing derived from GTK's source is committed. The accelerator reader and
  writer for the `xkbcommon` backend are therefore written by another session, one that has
  not read GTK's source.

### B3: Commands and key dispatch

- **B3 depends on B2's key-catalog move**, which the plan does not record: what B3 builds on
  arrives with it. Meanwhile: B3 is held until the move merges. Source: the archived issues
  [#2125](https://github.com/lgse/strata/issues/2125) and [#2126](https://github.com/lgse/strata/issues/2126).
- **Seven cases where GTK and Rune would read a key differently**, measured against Rune at
  the pinned revision: a non-Latin layout with a non-US Latin layout also configured; the
  digit row above level 1 with AltGr; the keypad decimal and separator; every press in the
  QA target; the keys Rune's host never delivers; a modifier's own press; and character to
  keysym. Whether each is a parity requirement, a deviation, an upstream packet entry with a
  workaround, or a shim. Meanwhile: none is decided, and they do not block the move.
  Source: [#2126](https://github.com/lgse/strata/issues/2126) ([B3](packages/B3.md) lists them).

### A6 and A18: keyboard settings

- **Which conflicts Rune's keyboard settings report**: GTK's as they are, or those of Rune's
  own text field and views, which nobody has listed; and what they say of a key Rune's host
  never delivers (dead keys, Compose, Caps Lock, AltGr, media keys, F25 to F35), whose
  binding loads, gets a label and never fires. Meanwhile: `src/ui/keys/conflicts.rs` stays in
  `src/ui`, and the Rune side has no conflict check. Source: the archived issues
  [#2133](https://github.com/lgse/strata/issues/2133) and [#2150](https://github.com/lgse/strata/issues/2150).

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
  (slice decision 27). The merge gate keeps both allowances. Source:
  `docs/rune/slice/README.md`, decision 27.
- **2026-11-11: package agents merge their own pull requests** into `strata-next`, with a
  merge commit, once a session other than the author's has reviewed them, the plan's merge
  gate has passed and CI is green. Source: recorded by the orchestrating session on
  2026-11-11; the plan's ["The merge gate"](README.md#the-merge-gate) has the gate.
- **2026-11-12: link `xkbcommon` at run time for the key tables.** Nothing derived from GTK's
  source is committed: no generated tables and no script that generated them. The shared
  crate carries the rules where GTK behaves differently, found by running GTK. Source: the
  archived issue [#2125](https://github.com/lgse/strata/issues/2125).
- **2026-11-12: one line of `tests/qa/mutations/quick-preview.patch` may change**: its key
  expression, so that it compiles against the shared key type, proved with
  `./scripts/qa-mutation.sh quick-preview`. The rule that a GTK mutation patch is never
  edited stands for every other line and patch. Source: the archived issue [#2125](https://github.com/lgse/strata/issues/2125).
- **2026-11-12: wave 1 packages start as their own dependencies finish**, not after the end
  of wave 0's checkpoint, which is still reported. Source: recorded by the orchestrating
  session on 2026-11-12; the plan's "Checkpoints" says the same of waves.
- **2026-11-12: three package agents build at once**, and pinned-container runs still go one
  at a time on a machine. Source: recorded by the orchestrating session on 2026-11-12.
- **2026-11-12: tracking moves to Markdown in the repository.** After a maintainer closed
  every package and finding issue that day, finished or not, the owner said to keep going,
  and to move tracking into documents checked in under `docs/`, as agents finish and new
  ones start. So the state of a package, its record and its open questions live under
  `docs/rune/parity/`, and the closed issues are an archive that is linked, never reopened.
  Source: recorded by the orchestrating session on 2026-11-12, and carried out by the pull
  request that added these files.
