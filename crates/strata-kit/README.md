# strata-kit

Strata's look on top of `rune_kit::Kit`: theme mapping, density and type tokens, icon
catalogs, the bundled font and user overrides. Components read these tokens, never a theme by
name.

A host registers the bundled font and names it for both of Rune's slots:

```rust,ignore
AppSpec {
    fonts: strata_kit::fonts::FONTS,
    families: strata_kit::fonts::FAMILIES,
    ..AppSpec::new(title, size, init, view)
}
```

## Credits

strata-kit is MIT. It embeds third-party data that keeps its own licence; each notice must
ship with any copy, including a binary that embeds the crate.

| Data | Licence | Details |
| --- | --- | --- |
| Lucide icons | ISC (Feather-derived icons MIT) | [`icons/README.md`](icons/README.md) |
| Catppuccin VSCode Icons | MIT | [`icons/README.md`](icons/README.md) |
| Strata's icon curation and theme catalogue | MIT | [`icons/README.md`](icons/README.md), [`themes/README.md`](themes/README.md) |
| Tinted Theming schemes | MIT | [`themes/README.md`](themes/README.md) |
| JetBrains Mono 2.304 | SIL OFL 1.1 | [`fonts/README.md`](fonts/README.md) |
