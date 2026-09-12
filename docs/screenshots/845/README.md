# Settings reference rework (#845)

The layout follows the six supplied references: a 1400 × 1024 dialog at the
reference desktop size, grouped rows, sidebar descriptions, activation matrix,
searchable theme/shortcut lists, release-note disclosure, and About cards.
The navigation collapses and controls reflow on smaller displays or larger text.
The [responsive follow-up](responsive/README.md) records the owner's subsequent
resize fixes, Settings-wide search, uniform panel backgrounds, white website
branding request, and sidebar-footer removal.

[Previous layout](before.png) is retained from #831's Settings capture, at its
original palette and size. The new captures use a temporary reference-like
palette, 13 px saved text, and an isolated 1806 × 1102 Xvfb/D-Bus session. No
user preferences, desktop session, or default theme colors were changed.

| Page | Supplied reference | Reworked application |
| --- | --- | --- |
| General | [Reference](reference-general.png) | [After](general.png) |
| General, scrolled | [Reference](reference-general-bottom.png) | [After](general-bottom.png) |
| Appearance | [Reference](reference-appearance.png) | [System-managed](appearance-follow.png), [manual](appearance.png) |
| Keybindings | [Reference](reference-keybindings.png) | [After](keybindings.png) |
| Updates | [Reference](reference-updates.png) | [After](updates.png) |
| About | [Reference](reference-about.png) | [After](about.png) |

## Content and behavior

These are application captures, not literal pixel-identical mock-data copies:
version, commit, GTK version, MIT license, release status/notes, shortcut count,
and available themes remain real. The system-managed Appearance capture uses
an isolated Omarchy fixture; enabling Follow Omarchy disables manual selection.
Themeable interface colors still follow the active theme. Existing desktop/file-chooser
integration remains below General's referenced sections rather than being removed.

The existing live-preference fixture now exercises the replacement menus and
relocated controls across two windows. Added coverage owns theme search/filtering
(including newly created custom themes), live current-theme swatches, shortcut
search/no-results recovery, managed release channels, and the real clipboard.
Layout checks retain small-display/large-text reachability and include keycaps
and About values. Focus reachability waits for GTK's native scrolling instead of
assuming that a fixed 150 ms delay finishes every frame.

## Local validation

Validation records are in the task worktree's `target/settings-reference/`.
The isolated rootless Podman wrapper is `target/settings-reference/podman/bin`;
the repository's pinned runner verified its normal base-image provenance.
Image ID: `dbc88d033bc4effdd19e4501ca6305775a221f416f82185432167fc437839d0c`.

```bash
export PATH="$PWD/target/settings-reference/podman/bin:$PATH"
export STRATA_CONTAINER_ENGINE=podman
./scripts/quality.sh fmt
./scripts/quality.sh clippy
./scripts/quality.sh test
./scripts/e2e.sh
git diff --check
```

Initial redesign results: formatting and Clippy passed; full Rust **1,470 passed, 18 ignored**
(`quality-test-final-v2.log`); canonical E2E **792 passed** in 191.99 seconds
(`e2e-final.log`); `git diff --check` passed. Full suites were selected because
Settings spans shared preferences, theme notifications, and every view's controls.
No local gate was omitted; required GitHub checks still apply before merge.

Earlier iterations encountered isolated preview-cleanup and timing-sensitive
search-test failures; both unchanged coverage owners passed in the final full
run. No unrelated fixes were included. No native-host build, user-display test,
base-image rebuild, or validation override was used.
The one-off capture generator remains outside the test suite in the task's
ignored `target/settings-reference/capture.py`.

This work originally branched from `b22af9a4` on #838
(`feat/831-custom-text-size`). After #838 merged, main at `b6220406` was
integrated and the PR was retargeted to main as a Settings-only change.
See the [follow-up evidence](responsive/README.md#compact-controls-and-main-integration)
for conflict resolution and validation.
