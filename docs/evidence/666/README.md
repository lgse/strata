# Keyboard context-menu review (#666)

Captured from the pinned GTK 4.14 environment on private Xvfb and D-Bus,
using disposable fixtures with hidden files disabled. No desktop session was used.

Select `todo.txt`, then press Menu. Before the review fixes (merge commit
`968ab4b5`), this opened the background menu. Afterward, it opens the file menu
at the selected item; Home highlights Open. Shift+F10, Home/End, Tab, and Escape
were also exercised in all three views. Dismissal preserved selection and file
contents.

| View | Before | After |
| --- | --- | --- |
| Columns | [Background menu](before-columns.png) | [Item menu](after-columns.png) |
| Icons | [Background menu](before-icons.png) | [Item menu](after-icons.png) |
| List | [Background menu](before-list.png) | [Item menu](after-list.png) |

The adjacent Rust menu tests own grouped/hidden-item targeting, chooser and Trash
menus, selection/focus preservation, and navigation/scrolling. Real key routing
and activation live in `test_dialogs_and_menus.py`; recursive-result targeting
and focus restoration extend `test_filter_results.py`. These replace the PR's
trigger-exists and no-crash-only tests.

## Right-click dismissal

Select `readme.md`, right-click `documents`, then press Escape. Before this fix
(`f94e8b15`), `documents` was selected but the keyboard cursor returned to
`readme.md`. Afterward, `documents` owns both selection and keyboard focus, and
Up continues from that folder rather than the previous file. The folder is not
opened. These captures use disposable fixtures, not the owner's recording.

[Before](before-right-click-escape.png) · [After](after-right-click-escape.png)

`test_selection.py` covers files, folders, subsequent arrow navigation, and
multi-selection in all three views. The adjacent Rust fixture also checks
hidden/grouped views and chooser selection; the recursive-result tests verify
that Escape restores the clicked result without changing the query.

## Activation latency

GTK's animated button activation waited for its 250 ms fallback because the menu
controller consumed key release. Menu actions now dispatch on key press instead.

The same isolated List fixture activated background Select All with Enter, keypad
Enter, and Space, three times each. Median key-to-menu-dismissal observation fell
from **264 ms** ([before](activation-before.json), `6aff5382`) to **44 ms**
([after](activation-after.json)). Every activation selected all five visible
entries and left file contents unchanged. These timings include input transport
and accessibility observation overhead; they are informational, not CI limits.

The existing Rust activation regression now requires the focused action to fire
exactly once before key handling returns, without a wall-clock assertion. The
real-key menu fixture covers both Enter and Space activation across all views.

## Returning from Properties

In Columns, open `documents/todo.txt`'s menu with Shift+F10, choose Properties
with arrows and Enter, close it with Escape, then press Up once. Before this fix
(`9baead1e`), focus ended up on F1 Shortcuts and the file remained selected.
Afterward, focus returns to the child file and one Up selects `nested` in that
same column. Both captures use disposable fixtures and leave file contents intact.

[Before](before-properties-focus.png) · [After](after-properties-focus.png)

The existing Properties E2E fixture covers keyboard menus, pointer menus, the
Properties shortcut, Escape, the close button, backdrop dismissal, and Rename
handoff across all views. Recursive-result and chooser fixtures retain their
selection and focus checks. The property lifecycle unit test also guards against
restoring an unmapped origin or stealing focus from a follow-up modal.
