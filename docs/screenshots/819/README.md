# Breadcrumb review evidence

Captured from PR #820 after the review fixes, using the pinned GTK 4.14.5 E2E
container with private Xvfb and D-Bus, a disposable six-level directory, and the
fixture's theme. No desktop session was used.

- [Deep path](deep-path.png): full current-directory label above the scrollbar.
- [Hierarchy](hierarchy.png): right-clicking the current breadcrumb opens the
  ancestor menu rather than the window menu. Selecting its parent navigated up.

The interaction is covered by
`tests/e2e/scenarios/test_locations.py::test_current_breadcrumb_opens_hierarchy_instead_of_window_menu`.
