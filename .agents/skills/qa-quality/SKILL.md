---
name: qa-quality
description: >
  Rubric for scoring the value of Strata QA scenarios (tests/qa/scenarios) on
  impact, coverage, criticality and diagnostic precision, and for recording the optional `quality`
  header field. Use when scoring or reviewing the value of QA scenarios,
  writing or checking a scenario's `quality` field, comparing overlapping
  scenarios, or deciding which tests matter.
---

# QA scenario quality rubric

Score each scenario in `tests/qa/scenarios/<folder>/*.jsonl` on four
dimensions, combine them into a 0–100 `score`, and record the result in the
scenario's header. QA is Strata's only GUI test suite; every comparison below
is against other QA scenarios and the Rust unit and GTK tests in
`src/**/tests.rs`.

This rubric produces **scores only**. It defines no keep/drop threshold, tier
or label. The owner sets any cut-offs when applying the scores. Never write a
verdict such as "drop" or "keep" into a note.

Supporting files:

- [calibration.md](calibration.md): ten scored real scenarios. Read them before
  scoring anything.
- [worksheet.md](worksheet.md): a per-scenario worksheet and the batch log
  format.
- [quality.py](quality.py): computes `score`, writes the field byte-exactly,
  ranks overlapping scenarios and lists recorded scores.

## The field

The header is the JSON object on line 1. `quality` is optional and has a fixed
shape:

```json
"quality":{"impact":4,"coverage":4,"criticality":4,"diagnostic":3,"score":52,"note":"Keep Both via keyboard + undo; only keyboard route through the paste-conflict dialog"}
```

| Key | Type | Rule |
| --- | --- | --- |
| `impact` | integer 1–5 | Anchors below |
| `coverage` | integer 1–5 | Anchors below |
| `criticality` | integer 1–5 | Anchors below |
| `diagnostic` | integer 1–5 | Anchors below |
| `score` | integer 0–100 | Exactly the formula below |
| `note` | string | One non-empty line: `note.splitlines() == [note]` and not blank. No `\n`, `\r`, `\v`, `\f`, `\x1c`–`\x1e`, U+0085, U+2028 or U+2029 |

No other keys. The qa-scenarios skill (`.agents/skills/qa-scenarios/SKILL.md`)
describes the rest of the header.

## Impact: how much a user is hurt if the behaviour breaks

Score the worst realistic failure of the behaviour the scenario guards, not the
failure of an incidental step. Weigh frequency of use, data loss or safety,
recoverability and visibility. A silent failure is worse than an obvious one.

| Score | Anchor | Strata examples |
| --- | --- | --- |
| 5 | Data loss, data corruption or a safety/security breach; unrecoverable or silent. | Dismissing a paste conflict overwrites a file (`dismissing-conflict-preserves-both`); permanent delete through a symlinked folder removes the wrong tree (`permanent-delete-symlinked-parent`); an executable runs without confirmation (`executable-requires-confirmation`); a disconnected provider replays an action (`provider-disconnect-never-replays`). |
| 4 | A core daily workflow fails with no reasonable workaround, or a recoverable but damaging file outcome (wrong file trashed, undo leaves debris). | Arrow keys stop moving focus (`arrow-keys-move-focus`); rename does not commit on Enter (`rename-commits-on-enter`); Keep Both or its undo affects the wrong file (`keep-both-then-undo-remaining`); settings lost on restart (`preferences-sync-windows-restart`). |
| 3 | A secondary feature fails, or a core one fails with an obvious workaround; or a whole user group (keyboard-only, screen reader) loses one control. | Sort by size wrong (`sorting-by-size`); Open With forgets the default (`open-with-keeps-default`); window shortcuts fire over a modal (`modal-dialog-blocks-window-shortcuts`); toolbar buttons unnamed for screen readers (`toolbar-controls-named`). |
| 2 | A visible annoyance on a working path: lost scroll position, wrong preview size, rendering or state glitch the user can recover from at once. | Rename loses the scroll position (`visible-rename-preserves-scroll`); small images upscaled (`small-image-preview-native-size`); hover tile renders wrong (`icons-hover-baseline`); filter text selection ignores the theme (`filter-selection-uses-theme`). |
| 1 | Cosmetic only: alignment, spacing, centring, a colour shade, tooltip wording. Nothing stops working. | A column title off-centre by a few pixels (the centring assertions in `column-header-titles-bounded`); tooltip wording. |

Some examples here were removed from the suite for their low score per unit
of cost; they remain as scoring examples.

Accessible names, roles and states are product surface: a missing name makes
a control unusable for screen-reader users, so score it 3 or higher, never as
cosmetic.

## Coverage: unique behaviour verified that nothing else verifies

Coverage is **marginal**: what would go unverified if this scenario were
deleted. Breadth counts only where it is unique. Count distinct behaviours,
input routes (pointer, keyboard, context menu, drag, shortcut), view modes,
and lifecycle paths (startup, restart, relaunch, dismissal, undo, worker
retirement) that this scenario alone exercises.

| Score | Anchor | Strata examples |
| --- | --- | --- |
| 5 | Sole guard of a whole behaviour, with several distinct routes, modes or lifecycle paths; no scenario or unit test checks the GUI-level outcome. | `dismissing-conflict-preserves-both` (Escape, Cancel, the close button, and a move: the file outcome of dismissing a paste conflict is checked nowhere else). |
| 4 | Sole guard of the main behaviour, or of a distinct input route or lifecycle path of it; setup or underlying logic may be shared or unit-tested. | `modal-dialog-blocks-window-shortcuts` (no other scenario or unit test presses window shortcuts over a modal); `keep-both-then-undo-remaining` (only keyboard route through the paste-conflict dialog). |
| 3 | The core is covered elsewhere, but one meaningful unique part remains: one route, one view mode with its own code path, one lifecycle step, or the GUI wiring of logic that unit tests cover. | `permanent-delete-symlinked-parent` (deletion through an alias is unit-tested; resolving the alias pane in the GUI is not); `toolbar-controls-named` (two of five names checked nowhere else). |
| 2 | Mostly duplicated: the unique part is minor, an equivalent route, another value of an already-tested setting; or every unique assertion is cosmetic. | `keep-both-then-undo` (only its click on Keep Both is not in `keep-both-then-undo-remaining`); `appearance-menu-switches-view` (Icons value of a menu switch the List scenario tests); `icons-hover-baseline`. |
| 1 | Fully contained: every behaviour and assertion is verified by another scenario or unit test through an equivalent route; or every unique assertion is layout or geometry. | `column-header-titles-bounded` (only geometry assertions). |

Rules:

- **Layout and geometry assertions add no coverage.** AGENTS.md "Test value"
  forbids layout tests: widget positions, sizes, alignment, spacing, wrapping.
  A scenario whose only unique assertions are layout scores coverage 1.
- **Cosmetic assertions cap coverage at 2**: colours, theme pixels, visual
  baselines. A baseline is the only way to see a rendering regression, so it
  is not 1, but its unique content is visual.
- **Behaviour AGENTS.md calls distinct stays distinct**: input routing,
  lifecycle, filesystem safety, error handling and live preferences. A
  keyboard route and a pointer route to the same action are two behaviours
  when Strata's code routes them differently (focus, input ownership, key
  bindings). Activating one GTK button by click or by Return is one route.
- **Never mark both sides of a duplicate pair as duplicates.** When two
  scenarios cover the same behaviour, the superset (more routes, assertions,
  modes) keeps the coverage for the shared part; the subset scores only its
  remainder. If they are exact mirrors, the one outside the behaviour's own
  folder, or else the later name alphabetically, takes the low score. Name the
  other scenario in both notes.
- **Unit coverage reduces coverage only for the logic it pins.** A unit test of
  `local_operations` naming does not cover the dialog, focus, selection or
  listing refresh that only the GUI path reaches.

### Judging overlap

Compare, in this order:

1. **Fixture**: header `tree`, `setup`, `images`, `prefs`, `env`,
   `session_bus`, `portal`, and the first `fs` steps.
2. **Input route**: `key`, `type`, `click` (with `button`, `count`, `mods`),
   `select`, `select_with_keyboard`, `open_menu`, drags, `wheel`.
3. **View mode**: header `modes`, and any step-level `"modes"` guard.
4. **Assertions**: `expect`, `wait`, and `fs` checks. Ignore `message` text.

Two scenarios overlap on a behaviour when fixture, route and mode are
equivalent and the assertions on that behaviour are equal or one is a superset.

## Criticality: how likely a regression is, and whether only this level catches it

| Score | Anchor | Strata examples |
| --- | --- | --- |
| 5 | Concurrent or external code path (async workers, filesystem operations, races, portals, D-Bus, sandbox helpers) with repeated past fixes and high churn; only the running application shows the failure. | Provider disconnect and replay over D-Bus (`provider-disconnect-never-replays`); idle sandbox workers exiting and restarting (`idle-workers-exit-and-restart`). |
| 4 | Complex GUI glue across modules (dialog → operation → listing refresh → focus), or a path with past fix commits or high churn; a unit test cannot reproduce the integration. | Paste-conflict dialogs (`src/ui/browser/transfer*`, `src/adapters/local_operations`: ~100 commits, ~55 fixes); modal input routing added by a fix (`modal-dialog-blocks-window-shortcuts`). |
| 3 | Non-trivial wiring with moderate churn or one prior regression; a refactor could plausibly break it. Visual baselines of widely restyled surfaces. | Appearance menu switching views (`src/ui/browser_modes`, high churn); filter text selection colour (added with fix #778); baselines refreshed by theming and font-rendering fixes. |
| 2 | Stable, simple code, or logic a unit test could pin as well; regressions unlikely or better caught elsewhere. | Static header geometry (`column-header-titles-bounded`). |
| 1 | Declarative or effectively frozen: a constant, a label with no history, behaviour fully determined by unit-tested logic. | A fixed string never touched since it was added. |

Measure, don't guess:

- **Churn**: commits touching the implementing paths in the last 90 days.
  Roughly ≥ 30 is high, 10–29 medium, < 10 low.
- **Past bugs**: `fix` commits touching those paths, and especially a fix that
  introduced the scenario's distinctive assertion (see the recipes below).
  Several commits are batch fixes ("triage batch N") that touch many areas;
  count one only when its subject or diff touches the behaviour.
- **Level**: if a Rust unit or GTK test could catch the same regression as
  well, lower criticality by one. If only real input, real windows, real
  processes or real buses can show it, do not.

## Diagnostic: how precisely a failure points at the cause

Judge what a failing run tells the person reading it: the failing step, its
message, and how many modules could be behind it. Read the scenario as if
each assertion had just failed.

| Score | Anchor | Strata examples |
| --- | --- | --- |
| 5 | One behaviour, a few assertions, each naming exactly what broke; the failure line alone identifies the regression. | `toolbar-controls-named` (`'Settings' is unnamed`); `modal-dialog-blocks-window-shortcuts` (each check names the shortcut that leaked). |
| 4 | Focused on one behaviour with a short setup; a failure identifies the phase and the outcome, and one area of code is behind it. | `dismissing-conflict-preserves-both` (per-route variants, file checks); `permanent-delete-symlinked-parent`; `appearance-menu-switches-view`. |
| 3 | Several phases chained (dialog, operation, selection, focus, undo), or a heuristic check (a pixel count); a failure names the phase, but several modules could cause it. | `keep-both-then-undo-remaining`; `filter-selection-uses-theme` (accent pixel count). |
| 2 | A long walk-through, or many assertions without specific messages (derived geometry, bare `expect`s); a failure needs the artifacts to interpret. | `shortcut-round-trip-keeps-selection` (55 steps across three views); `column-header-titles-bounded` (unlabelled geometry comparisons). |
| 1 | A broad capture or tour where a failure says only that something differs: a percentage of changed pixels across the window. | `icons-hover-baseline`, `list-view-baseline`. |

Messages help only when they name the behaviour. `"a dialog to appear"`
before the behaviour under test is setup, not diagnosis. Diagnostic is
independent of coverage: a broad scenario can cover much and diagnose little.

## Scoring formula (normative)

```text
P     = 3*(impact - 1) + 3*(criticality - 1) + 2*(diagnostic - 1)           # 0..32
score = round_half_up(100 * (coverage - 1)/4 * P/32)                       # 0..100
      = (50*(coverage - 1)*P + 32) // 64                                   # integer form
```

- `score` is **uniqueness × risk**: `(coverage - 1)/4` is the share of the
  behaviour only this scenario verifies, and `P/32` is how much that behaviour
  matters and how well a failure is reported.
- Risk weights: impact 3, criticality 3, diagnostic 2.
- `//` is integer division rounding toward negative infinity. Every operand is
  non-negative, so it equals truncating division. The integer form is exactly
  `floor(100*(coverage-1)*P/128 + 1/2)`: an exact half rounds up.
- The runner and `quality.py score` implement the integer form. It matches the
  exact fraction for all 625 combinations of the four dimensions.

Why this shape: deleting a scenario loses only what it alone verifies, so
uniqueness multiplies rather than adds. A duplicate is worth nothing however
important its behaviour. Impact (harm if broken) and criticality (chance it
breaks unseen) are the two halves of risk, so they weigh the same. Diagnostic
weighs less: it changes what a failure costs to investigate, not whether the
regression is caught.

| impact | coverage | criticality | diagnostic | P | score |
| --- | --- | --- | --- | --- | --- |
| 5 | 5 | 5 | 5 | 32 | 100 |
| 5 | 5 | 4 | 5 | 29 | 91 |
| 4 | 4 | 4 | 4 | 24 | 56 |
| 3 | 5 | 3 | 1 | 12 | 38 |
| 4 | 3 | 4 | 3 | 22 | 34 |
| 3 | 3 | 3 | 3 | 16 | 25 |
| 5 | 2 | 5 | 5 | 32 | 25 |
| 2 | 2 | 2 | 2 | 8 | 6 |
| 5 | 1 | 5 | 5 | 32 | 0 |
| 1 | 5 | 1 | 1 | 0 | 0 |

### Reading a score

- **Coverage caps the score**: coverage 1 always scores 0, 2 at most 25, 3 at
  most 50, and 4 at most 75. A cosmetic scenario (coverage at most 2) never
  exceeds 25. A geometry-only scenario (coverage 1) scores 0.
- **A score of 0 has two causes.** Read the dimensions to see which:
  - Coverage 1: the scenario adds nothing that is not verified elsewhere, or
    its unique part is geometry only. Its behaviour may still matter a great
    deal. Check that the note names the scenario or unit test that covers it,
    and that the named one really does, before acting on the 0.
  - Impact, criticality and diagnostic all 1: the scenario is unique, but it
    guards something harmless and frozen, and its failure says little.
- **A low score is not a verdict.** The rubric only ranks. The dimensions say
  why a score is low, and the owner decides what to do with it.

## The note

One line: the behaviour under test, then the deciding factor. Name the
scenario or unit test it duplicates, or the unique route it alone covers.

```text
paste-conflict dismissal by Cancel/Escape/Close keeps both files; only scenario covering dismissal routes
Icons switch via Appearance menu; appearance-menu-switches-view-list covers the same path for List
```

Keep it under about 140 characters, with no line-break character of any kind
(see the field table). Do not restate the numbers or add a
keep/drop verdict.

## Scoring one scenario

1. **Read the scenario.** Header (`modes`, `variants`, `prefs`, `tree`,
   `setup`, `skip`, `known_bug`), then every step, noting step-level `modes`,
   `variants`, `when`, `if` and `unless` guards. List what each variant and
   mode changes.
2. **State the behaviour under test** in one sentence: the action, the route,
   and the outcome asserted. Separate the asserted outcome from setup and
   waits.
3. **Find overlapping scenarios.** Start with the folder, then the corpus:

   ```bash
   ls tests/qa/scenarios/<folder>/
   python3 .agents/skills/qa-quality/quality.py similar tests/qa/scenarios/<folder>/<name>.jsonl
   grep -lF '"name":"Keep Both"' tests/qa/scenarios/*/*.jsonl          # same control
   grep -l '"key":"shift+Delete"' tests/qa/scenarios/*/*.jsonl         # same chord
   grep -l '"accent_pixels"' tests/qa/scenarios/*/*.jsonl              # same predicate
   grep -l '"pref":{"key":"browser_mode"' tests/qa/scenarios/*/*.jsonl # same preference
   grep -l '"symlink"' tests/qa/scenarios/*/*.jsonl                    # same fixture shape
   grep -l '"session_bus":true' tests/qa/scenarios/*/*.jsonl           # same environment
   diff <(tail -n +2 A.jsonl) <(tail -n +2 B.jsonl)                    # step diff of a pair
   ```

   `similar` compares literal steps, so it misses pairs that differ in one
   value (Icons vs List). Always follow up with greps on the controls,
   chords, predicates and preferences the scenario uses.
4. **Check unit and GTK test coverage.** Find the implementing code from a
   name or string the scenario uses, then its adjacent tests:

   ```bash
   grep -rn '"Keep Both"' src --include=*.rs | grep -v tests.rs
   grep -rn 'fn .*keep_both\|fn .*conflict' src --include=tests.rs
   ls src/ui/browser/transfer/tests.rs src/adapters/local_operations/tests/
   ```

   Decide which part of the behaviour those tests pin (logic, dialog
   contents, focus restoration) and which only the running application shows.
5. **Check history for the area.**

   ```bash
   git log --oneline --since='90 days ago' -- <paths> | wc -l               # churn
   git log --oneline -i --grep='^fix' -- <paths>                           # past fixes
   git log --format='%h %ad %s' --date=short -S'<distinctive string>' | tail -1   # origin
   ```

   Search `-S` for a distinctive accessible name, message or test name from
   the scenario across the whole tree, not only the scenario file; the
   scenario file's own history starts at its port. An origin commit typed
   `fix` means the scenario guards a known regression.
6. **Score each dimension** against the anchors and the calibration examples.
   When torn between two levels, pick the lower and say why in the note.
7. **Compute `score`**: `python3 .agents/skills/qa-quality/quality.py score I C K D`
   (impact, coverage, criticality, diagnostic).
8. **Write the field.**

   ```bash
   python3 .agents/skills/qa-quality/quality.py write tests/qa/scenarios/<folder>/<name>.jsonl I C K D "<note>"
   ```

   It replaces an existing `quality` value in place, or adds it as the last
   header key, using compact JSON like the rest of the header, and refuses to
   write if anything else in the header would change or the note is not one
   line. It writes a temporary file beside the scenario and renames it over
   the original, so an interrupted write never leaves a truncated file. Every
   other byte of the file stays as it was. Editing by hand is allowed under the same rule:
   change only the `quality` value on line 1.
9. **Have an independent grader check it.** When an agent wrote or changed
   the scenario, a separate subagent on a cheap model (Haiku) grades it blind
   with this rubric: tell it to ignore the existing `quality` field, to cite
   the overlapping tests, churn and fix commits behind each dimension, and to
   edit nothing. Compare its four dimensions with yours. Re-read the anchors
   and settle any dimension that differs by 2 or more, or a `score` that
   differs by more than 10; rewrite the field if you change your mind. Report
   both gradings in the handoff or PR, not in the note.

## Applying the rubric to the whole corpus

1. **Calibrate first.** Score the ten scenarios in calibration.md blind, then
   compare. Discuss any dimension that differs by 2 or more, or any `score`
   off by more than 10, and re-read the anchors before going on. Every agent
   or reviewer taking part calibrates separately.
2. **Score folder by folder.** Read every file in a folder before scoring any
   of them; most duplicates sit in the same folder (`*-remaining` names are
   usually a split of one behaviour). Settle duplicate pairs inside the folder
   first, then search the corpus. Fill in the worksheet row as you go.
3. **Cross-folder consistency pass.** When every folder is done, run
   `quality.py report` (all folders, or named ones by bare name or path) and
   sort by each dimension. Rows marked `!` have a malformed `quality` value;
   fix them first. Compare scenarios with the
   same score across folders: similar harm should have similar impact, and
   each duplicate pair must have exactly one low-coverage side. Re-score two
   calibration anchors after every 50 scenarios to catch drift.
4. **Record overrides.** A reviewer who disagrees changes the dimensions,
   recomputes `score` with `quality.py write`, and ends the note with
   `; override: <reason>`. Log who overrode what in the issue or PR, not in
   the tree.
5. **Re-score when the evidence changes:**
   - the behaviour or its implementation changes, or the scenario's steps do;
   - a scenario is added that overlaps it: re-score both;
   - an overlapping scenario is removed: the survivor's coverage may rise;
   - a unit or GTK test now pins the same logic;
   - a bug is found in the area, or escapes past this scenario: raise
     criticality.

### Variants and modes

- A mode or variant adds coverage only when it takes a distinct code path:
  a different key binding (Right in Icons, Down in List), a different widget
  (Columns panes vs List rows), a different preference, layout or lifecycle.
- Running view-independent behaviour (a dialog, a file operation after the
  selection is made) in three modes is one behaviour, not three.
- Look for step-level `"modes"` or `"variants"` guards and mode-specific
  predicates. Their absence, on behaviour that does not depend on the view,
  means the extra runs are duplicates.
- Variants that change only a value without changing the route
  (`short`/`long` names that hit the same check) add nothing.

## Pitfalls

- **Don't score length or run count.** A 5-step scenario guarding data loss
  outranks a 55-step menu walk-through. Length matters only through
  diagnostic, when it blurs what a failure means.
- **Don't inflate coverage to avoid a 0.** Coverage 1 scoring 0 is the
  intended reading of a full duplicate; record the duplicate in the note.
- **Don't score diagnostic from message count.** Judge what a real failure
  would tell you. Many generic messages diagnose less than one precise one.
- **Don't reward duplicate modes or variants.** See above.
- **Don't penalise slowness.** Run cost lives in the header's `cost` field and
  is a separate decision.
- **Don't confuse flaky with low value.** A flaky scenario on a race-prone path
  often guards the most critical behaviour. Score the behaviour; report the
  flakiness separately. Flakiness does not lower diagnostic either: score
  what a genuine failure would say.
- **Score a `known_bug` or `skip` scenario on the behaviour it guards**, as if
  it passed. Being skipped is not a reason to lower any dimension.
- **Don't fold impact into coverage.** A fully duplicated data-safety scenario
  keeps impact 5 and its criticality; coverage 1 alone makes its score 0, and
  the dimensions still show that the behaviour matters.
- **Don't treat accessibility checks as cosmetic**, or visual baselines as
  layout tests. Baselines are the sanctioned way to catch rendering
  regressions; their coverage cap is 2.
- **Don't count waits as assertions.** A `wait` that only synchronises (a
  dialog to appear, a pane to settle) is setup unless the behaviour under test
  is that it appears.
