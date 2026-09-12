# List navigation restoration (#868)

Both captures use the pinned GTK 4.14 E2E environment with a private Xvfb/D-Bus
session and a generated directory containing 160 folders. Scroll down, select
`folder-057`, enter it, then press Alt+Left.

- [Before](before.png): List-state capture disabled to reproduce the previous
  behavior: the viewport returns to the top and selects `folder-000`.
- [After](after.png): `folder-057` retains its selection, keyboard cursor, and
  position within the scrolled viewport.

The screenshot generator is a one-off artifact, not an additional suite test.
The regression coverage belongs to `test_keyboard_navigation.py` and
`ui::browser_modes::navigation::tests`.
