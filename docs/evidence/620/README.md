# Filter result interaction evidence (#620)

Before: upstream `08f77a35feed1c5dac6cbefd8c99d0959fb1bddc`.
After: filter fix `333060bbd39308450d5326222538b6ed35787615` (the subsequent test-only commit does not change the application).

Captures use synthetic files, disposable HOME/XDG directories, private Xvfb/X11 and session/accessibility buses, GTK 4.22.4 with the cairo renderer, and xcompmgr 1.1.10 for popup transparency. The display and application are both 1200×760. No active desktop or personal files were used. These are direct captures with no image retouching.

## Reproduction

1. Create `match-note.txt`, `alpha/match-note.txt`, and `beta/match-note.txt` with different contents, plus a `noise` directory containing 1,000 numbered text files.
2. Open the root in Columns, List, or Icons with recursive filtering enabled and single-click preview disabled.
3. Press Ctrl+F, type `match-note`, press Down and Space, then right-click the first result.
4. Capture the preview and menu. Dismiss the menu, focus the query, and append/remove a period four times. Capture the final state.
5. Repeat with the before and after builds on the same private display configuration.

The videos show query-update flashing and selection loss. The small fixture indexes quickly; deterministic Rust regression coverage exercises progressive insertion, reordering, and removal separately.

## Comparisons

| Mode | Before menu | After menu | Before video | After video |
|---|---|---|---|---|
| List | [Screenshot](before/list-item-menu.png) | [Screenshot](after/list-item-menu.png) | [Recording](before/list.mp4) | [Recording](after/list.mp4) |
| Icons | [Screenshot](before/icons-item-menu.png) | [Screenshot](after/icons-item-menu.png) | [Recording](before/icons.mp4) | [Recording](after/icons.mp4) |
| Columns control | [Screenshot](before/columns-item-menu.png) | [Screenshot](after/columns-item-menu.png) | [Recording](before/columns.mp4) | [Recording](after/columns.mp4) |

Each directory also contains `<mode>-preview.png` and `<mode>-updated.png` for preview and final query-update state.

Before: List/Icons show the background menu on a file and lose selection during every sampled query update. After: both show the item menu and retain selection and open preview through all four sampled updates.

Columns remains the control: its separate query-edit model still resets selection. Its nested-result context routing and menu-related preview dismissal are corrected. This change stabilizes the List/Icons result surface.
