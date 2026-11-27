# Migration note for Strata's changelog

This note is written for the release that moves Strata from GTK 4 to Rune. It covers the
user stylesheet (K4) and the two appearance settings that have no Rune equivalent (K5).

## The user stylesheet becomes `overrides.toml`

Strata no longer reads `~/.config/strata/style.css`. Its replacement is
`$XDG_CONFIG_HOME/strata/overrides.toml` (usually `~/.config/strata/overrides.toml`), which
sets Strata's look tokens by name instead of styling GTK widgets:

```toml
# Every key is a token: colours, radii, spacing, row heights, type, glow, syntax colours.
radius.md = 4
color.border_focus = "#ff9e64"

# Component tokens, under the component's table.
[sidebar.row]
height = 32

# A colour can follow the theme: another colour token, or a share of its opacity.
[sidebar.hover]
fill = "alpha(@color.text_base, 0.06)"

# Values for light or dark themes only.
[dark]
color.bg_base = "#101018"
```

- Strata re-reads the file when it changes, as it did the stylesheet, and the change animates
  like a theme switch. Deleting the file restores the theme's look.
- Every unknown token, wrong value and syntax error is reported with its line and column, and
  every other row still applies. Settings → Appearance shows how many problems the file has.
- Overrides apply last: after the theme, Omarchy following, corner roundness, density and
  text size. Corner roundness doesn't scale an overridden radius, as it didn't scale one in
  the stylesheet. An override of `glow.on` wins over the Element glow setting.
- Component tokens (such as `sidebar.row.height`) are part of Strata's interface: renaming
  one is a breaking change, listed in the changelog.

### Your old stylesheet

On first launch, Strata converts what of an existing `style.css` it can into
`overrides.toml`, and shows the rules it couldn't convert once. It never changes or deletes
`style.css`.

A rule converts only where a token stands for exactly what it styled: the current sidebar
row's fill and icon colour (`.sidebar-row.active` and `.sidebar-row.active image`), from a
literal colour, one of Strata's `@strata_*` colours or `alpha()` of either (those keep
following the theme). As in CSS, a later declaration wins: if it can't convert, the earlier
one is dropped too. Everything else stays in the notice, with its line:

- rules whose replacement would style more than the rule did: the file rows' margins and
  radius (`.file-list row` styled Miller columns, folder peek and inline search, where the
  row tokens style every view) and the sidebar's hover fill (`.sidebar-row:hover`; the token
  also fills the section headings);
- other selectors (GTK class names were never a stable interface), lengths, `shade()` and
  `mix()`, `!important`, at-rules, and `@define-color` (to change a theme colour, make a
  custom theme).

## Settings that are gone

Rune draws Strata's window itself, through the GPU (wgpu), so two settings for GTK's
renderers have nothing left to control. Strata ignores a saved value and removes it on the
next save.

- **Interface renderer** (Settings → Appearance → Rendering; `interface_renderer` in
  `settings.toml`). GTK's GL, Vulkan and Cairo renderers aren't used any more, and the
  `GSK_RENDERER` environment variable has no effect. Restart now goes with it.
- **The uncomposited popup fallback.** On a display without a compositor (X11 without a
  compositing manager), GTK popovers, menus and tooltips lost their shadows and rounded
  corners, because their transparent pixels drew black. Strata's menus and popovers now draw
  inside its window, so there are no transparent popup pixels to turn black, and they keep
  their shadows and corners everywhere. If Rune draws popups as separate surfaces later, this
  needs another look.

### Feature-map bullets proposed not applicable (owner to confirm)

`settings/themes`, "Interface renderer":

- Interface renderer offers GTK default and Cairo; choosing a value other than the startup one
  shows Restart now.
- After restart, Cairo renders the interface; an explicit `GSK_RENDERER` environment variable
  overrides the saved choice.

`settings/themes`, "Uncomposited displays":

- On an uncomposited display, such as bare Xvfb, context menus, popovers, and tooltips draw
  square corners and no shadow.
- On that display, a context menu shows no black frame or black corner tips around it.
- On a composited display, popovers, menus, and tooltips keep their shadows and corner radii.
- Starting or stopping a compositing manager while Strata runs squares or restores popup
  corners and shadows without a restart.
- Without a compositor, a `popover contents { border-radius: 12px; box-shadow: 0 0 8px black; }`
  rule in `style.css` still applies.
- Without a compositor, Corner roundness leaves popover, menu, and tooltip corners square at
  every value, 200 included.
- Without a compositor, the folder peek card keeps its shadow and rounded corners.

Of these, "a context menu shows no black frame" and "the folder peek card keeps its shadow
and rounded corners" still describe the Rune build (in-window menus have no transparent
popup pixels), but nothing in the port does work to make them hold.
