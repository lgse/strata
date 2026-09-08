# Outside-wheel popover dismissal

Both captures show one downward wheel tick over a Columns listing with Sort by
open, using synthetic files on a private Xvfb display (native GTK 4.22.4).

- `before.png`: main at `6f7326f`; the popover stays open and the listing does not move.
- `after.png`: this fix; the popover closes and that same tick scrolls the listing.

The fixture directory names differ because each scenario gets a fresh temporary tree.
