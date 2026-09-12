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
