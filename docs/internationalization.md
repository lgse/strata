# Internationalization

Strata uses `rust-i18n` with compiled-in UTF-8 JSON catalogs and English fallback.
The language preference is initialized before application UI and takes effect on restart;
see [Language preferences](preferences.md#language) for detection and persistence.
The application's language never changes filesystem paths, protocol identifiers,
user-defined names, or the locale environment inherited by other programs.

## Catalogs

- `locales/<locale>.json`: common controls, settings, and interface messages.
- `locales/messages/<locale>.json`: formatted messages and shortcut descriptions.
- `locales/counts.json`: whole count messages and compact relative-time messages,
  using rust-i18n's multilingual `_version: 2` format.
- `locales/dates.json`: month/weekday names and date presentation patterns,
  also multilingual. ISO 8601 remains language-independent.

Locales are `en`, `fr`, `de`, `es`, `ja`, `pt-BR`, `ko`, `vi`, `it`, and `ru`.
JSON keys normally contain the original English message. Semantic keys are used
for grammatical count forms and calendar data. All catalogs are build inputs;
changing only a translation still rebuilds the embedded translations.

## Adding or changing text

Translate at the presentation boundary, not in filesystem or protocol code:

```rust
let label = gtk::Label::new(Some(&crate::i18n::tr("Language")));
let message = rust_i18n::t!("Copied to %{device_name}", device_name = device_name);
```

Add the English message and its translations to the matching catalogs. Translate
whole sentences; do not construct them by appending English plural suffixes or
joining words in an English-only order. Use named `%{placeholders}` so translators
can reorder values. Preserve every placeholder name, markup tag/attribute, URL,
command, path, and accelerator. Escape untrusted values before inserting them
into markup, just as for an untranslated markup string. Never translate inserted
filenames, user-defined actions/themes, script output, or document contents.

`i18n::count` selects the supported languages' integer plural categories.
Russian distinguishes one/few/many, French and Brazilian Portuguese use the
singular form for zero and one, and Japanese/Korean/Vietnamese use invariant
forms. Keep counts as complete messages. Arbitrary fractional quantities require
a separate design rather than reusing the integer helper.

Settings search indexes the translated title as well as the English title and
aliases. Stable source IDs remain independent of displayed language; do not use
a translated label to dispatch an action, recognize a page, or sort type groups.
Shortcut descriptions are translated when rendered and searched, while keycaps
retain their accelerator spelling.

External release notes, system/GIO descriptions and errors, and toolkit-owned
controls may follow their provider or system language. Do not rewrite the
process environment to translate them: spawned commands must retain the user's
locale. Translations should be reviewed by native speakers, particularly longer
help text and destructive-operation confirmations.

GTK uses Fontconfig fallback for characters absent from the selected font.
Japanese and Korean require installed CJK fonts, such as Noto Sans CJK (commonly
packaged as `noto-fonts-cjk` or `fonts-noto-cjk`). Strata does not download or
install fonts. A minimal container without CJK fonts can expose correct
accessible text while rendering missing-glyph boxes; check actual rendering
with suitable fonts installed, rather than relying only on accessibility tests.

## Validation

Run targeted regressions using the private display/session bus:

```bash
./scripts/test-headless.py i18n::
./scripts/test-headless.py language_
./scripts/test-headless.py util::tests
./scripts/test-headless.py ui::settings::tests::general
```

Catalog tests require every supported locale to contain the same source keys
and preserve interpolation placeholders. Behavioral tests cover saved startup
language, restart-only application, synchronized Settings controls, restart
close guards, plural rules, literal user data, and app-language calendar labels.
For broad code changes, also follow the full validation policy in `AGENTS.md`.

Manually inspect Settings, menus, file views, and dialogs in both Latin and CJK
languages on an isolated display. Check translated search, autonyms in the
language selector, long descriptions, and readable destructive confirmations.
Do not add widget geometry/layout assertions for translated text.
