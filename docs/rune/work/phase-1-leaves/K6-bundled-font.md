# K6 Bundled JetBrains Mono

| | |
| --- | --- |
| Phase / lane | 1 · Look (added by the owner after K4) |
| Plan | §4 K2 (type), Strata's `src/assets.rs`, `src/ui/theme/font.rs`, `src/style.css` font rules |
| Depends on | K2, W2 |
| Unblocks | Every preview's look matching Strata; C10b, C11, C14b, X1 adopt it |
| Owns | `crates/strata-kit/src/fonts/` (new), `crates/strata-kit/fonts/` (the asset and its licence), the K2 families defaults, the gallery's `AppSpec` families |
| Branch | `feat/k6-bundled-font` |

## Goal

Strata ships JetBrains Mono and uses it for every preview's monospace text and as its default
interface font. The components should look the same: bundle the font in strata-kit and make it
the default for both slots.

## Scope

- **The asset:** JetBrains Mono 2.304, the variable `JetBrainsMono[wght].ttf`, byte-identical to
  Strata's `data/fonts/JetBrainsMono[wght].ttf` (read it from the local Strata clone with
  `git show upstream/main:…`; no download), plus its SIL OFL 1.1 licence text. Record the SHA-256.
  Distributed unmodified.
- **The API:** strata-kit exports the face bytes and the families as data, the way rune-kit's
  theme crates do (`AppSpec::fonts`, `AppSpec::families`). Strata's default families are
  `"JetBrains Mono", "JetBrainsMono Nerd Font"` (`ui/theme/font.rs`), for both the interface
  and the mono slot.
- **Defaults:** `LookParams::app_families` and the K2 typography rules default to the bundled
  families. A chosen interface font still goes in front of them, and the mono slot stays the
  bundled font, as Strata's explicit monospace for previews wins over the interface choice.
- **Hosts:** the gallery's `spec` and its K2 page use the bundled fonts, and so does the X1
  shell demo (if X1 has merged by then; otherwise X1 adopts it at its rebase).
- **Previews:** audit every component that draws preview or code text (code view, document
  code blocks, text and command previews, list cells) so it uses the mono slot and not a
  system or named family.
- **Credits:** the licence file ships with the crate (`include` in its manifest), and the
  crate's README lists JetBrains Mono, OFL 1.1, among its credits.

## Tests

- A unit test that the bundled bytes parse as a font with the family name Rune will match.
- A11y goldens don't change. PNG goldens change almost everywhere: re-bless them on purpose
  and review the diffs (the font changes, nothing else).
- K2's font-picker tests still pass: Default restores the bundled font.

## Done when

The gallery and every preview draw in JetBrains Mono by default, the licence ships with the
crate, and `mise run check` passes.

## Notes

- Not a dependency, so `deny.toml` is unaffected. The crate grows by about 300 KB.
- In-flight WPs re-bless their own PNG goldens after rebasing onto this.
