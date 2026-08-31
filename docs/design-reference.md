# Prototype Design Reference

The original web prototype is the visual and interaction reference for Strata:

- Repository: <https://github.com/l0gicgate/omarchy-file-explorer-prototype>
- Reference commit: `afb957925063adc1d9e2d71839ac06006c0675c9`

The prototype is a reference, not a native implementation specification. Native behavior, accessibility, performance, and desktop conventions take priority when a direct translation would compromise the product.

## Visual character

- Dark, low-chroma surfaces with a bright cyan primary/accent
- Fine, low-contrast separators between structural regions
- Compact, terminal-influenced typography
- Small radii rather than fully square or highly rounded controls
- Restrained glow used only for active/focused elements
- Dense information layout with generous empty workspace
- Colored, type-specific outline icons

## Layout baseline

| Element | Prototype value |
|---|---:|
| Top toolbar | 40 px |
| Status bar | 24 px |
| Expanded sidebar | 208 px |
| Normal directory column | 288 px |
| Last directory column | 320 px |
| Column header | 36 px |
| Compact list row | 30 px |
| Airy list row | 42 px |
| Compact grid row | 86 px |
| Airy grid row | 110 px |
| Grid columns per pane | 3 |
| Preview default width | 384 px |
| Preview minimum width | 280 px |
| Preview maximum width | 720 px |
| Peek width | 256 px |
| Peek item limit | 8 |

These are starting points. Native font metrics, scaling, touch targets, and accessibility may require adjusted values.

## Motion baseline

The prototype consistently uses the emphasized deceleration curve:

```text
cubic-bezier(0.16, 1, 0.3, 1)
```

| Transition | Duration | Motion |
|---|---:|---|
| Directory column | 220 ms | Fade and translate 24 px from the right |
| Preview drawer | 260 ms | Slide from the right |
| Sidebar collapse | 300 ms | Width transition |
| Folder peek | 160 ms | Fade, translate 4 px, scale from 0.98 |
| Command palette | 160 ms | Fade, translate -8 px, scale from 0.98 |
| Context menu | 120 ms | Fade and scale from 0.96 |
| Overlay | 140 ms | Fade |
| Row color feedback | 100 ms | Color transition |
| Theme change | 300 ms | Background and foreground transition |

The prototype uses a 450 ms folder-peek delay. This should become configurable and should be tested against shorter values.

All native motion must be interruptible where user input can reverse it and must respect reduced-motion preferences.

## Typography

The prototype requests:

```text
JetBrains Mono, ui-monospace, monospace
```

for both interface and preview text. No font files are stored in the prototype repository.

Strata uses JetBrains Mono 2.304 as its bundled default visual profile. The font remains configurable and includes:

- Its complete OFL-1.1 license and attribution
- A generic system monospace fallback
- Planned separate interface and monospace-preview overrides
- Native text scaling and accessibility behavior

The font choice belongs to a semantic theme/user preference rather than being embedded throughout individual widget styles.

## Icons

The prototype uses `lucide-react` for its interface and file-type icons. Relevant icons include:

- Folder and generic file
- File text, code, JSON, and image
- Terminal, film, and archive
- Search, sidebar, list, and grid
- Close, pin/unpin, rename, copy, cut, trash, preview, and external open

The native application should expose semantic icon names internally rather than referring to a concrete icon library from product logic. A theme or icon adapter can then resolve each semantic role.

Before distributing copied Lucide SVG data, include the required Lucide/ISC license attribution. Prefer system symbolic icons where they preserve the intended appearance and interoperability; use a curated bundled set where consistency is important.

## Semantic color vocabulary

The prototype already provides a useful semantic vocabulary:

- `background`
- `surface`
- `foreground`
- `card` and `card-foreground`
- `popover` and `popover-foreground`
- `primary` and `primary-foreground`
- `secondary` and `secondary-foreground`
- `muted` and `muted-foreground`
- `accent` and `accent-foreground`
- `destructive`
- `border`
- `input`
- `ring`
- `glow`
- `dim`
- Syntax keyword, string, number, function, and comment colors

Strata's native theme schema should retain these concepts where useful and map Omarchy colors into them. Widgets should consume semantic roles rather than raw colors.

## Interaction details worth preserving

- Single-click activation in directory columns
- Active row indicated through color, outline, and restrained glow
- Child directory chevron at the trailing edge
- File size appears on row hover
- Folder peeks float in an anchored popover beside the hovered row; they never join or reflow the Miller-column strip
- Miller columns have no fixed depth limit; committed directories keep stacking in a horizontally scrollable strip
- Opening a column after horizontal overflow automatically scrolls the strip to its end so the newest column is visible
- Breadcrumbs remain available above Miller columns
- Sidebar collapses to zero rather than becoming a narrow icon rail
- User pins live separately from standard places
- Theme swatches communicate surface, accent, and supporting color
- Preview metadata is separated from preview content
- Preview divider can be dragged or adjusted from the keyboard
- Inline rename selects the stem while preserving the extension
- Search palette supports fuzzy path fragments
- Status bar shows location, entry count, view mode, and density

## Provenance rules

The prototype repository currently has no repository-level license. Do not mechanically copy its source or placeholder assets into the public Strata repository until provenance is explicit.

Safe paths forward:

1. Treat the prototype as a behavioral/design specification and independently implement it.
2. Add an appropriate license to original prototype material the owner wants to reuse.
3. Track third-party assets independently and preserve their licenses and attribution.
4. Do not carry over generated placeholder images unless their provenance and need are clear.

Strata remains licensed under GPL-3.0-or-later regardless of which implementation details are inspired by the prototype.
