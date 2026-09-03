# Rendered document previews

Strata can render local Markdown and a deliberately small HTML subset with native GTK widgets. Each document preview has **Rendered** and **Source** views. **Render documents by default** under **Settings → General → Browsing** chooses the initial view for each newly opened document; switching the current preview does not change that preference.

Remote Markdown and HTML locations remain source-only. Rendered previews are selected from the local filename or content type:

- Markdown: `.md`, `.markdown`, `.mdown`, `.mkd`, `.mkdn`, `.mdwn`, `text/markdown`, and `text/x-markdown`;
- HTML: `.html`, `.htm`, `.xhtml`, `text/html`, and `application/xhtml+xml`.

## Supported content

Markdown uses `pulldown-cmark` with tables, strikethrough, and task lists enabled. Headings, paragraphs, emphasis, lists, block quotes, web links, fenced and inline code, rules, tables, and line breaks render natively. Known fenced-code language hints use Strata's existing, theme-aware GtkSourceView syntax highlighting. Raw HTML is displayed as inert text. Images become text placeholders, and their resource URLs are discarded.

HTML uses `html5gum` tokenization and supports semantic containers, headings, paragraphs, emphasis, lists, block quotes, `http` and `https` links, `pre`/`code`, rules, simple tables, and line breaks. Normal flow whitespace is collapsed like HTML, while `pre` content remains exact. A known `language-*` class on `code` inside `pre` is used only as a syntax hint; it does not enable CSS. Entities are decoded before their text is escaped for Pango.

CSS, classes, IDs, metadata, and other presentation attributes are ignored. Scripts, styles, forms, frames, objects, embedded content, images, media, event handlers, unsafe links, and resource-bearing tags or attributes are omitted. If a document still has useful safe content, Strata renders it with an omission warning. Link schemes are checked again when a link is activated before the existing external URI launcher is used.

## Limits and fallback

Rendered parsing has fixed limits:

- 1 MiB input;
- 20,000 parser events;
- nesting depth 32;
- 512 cells in any one atomic table;
- 4 MiB of escaped Pango markup;
- 500 ms parser time.

A truncated, malformed, contentless, timed-out, cancelled, or limit-exceeding document opens in **Source**. The **Rendered** control is disabled and the reason is shown above the source preview.

Small source previews up to 128 KiB and 512 lines retain the syntax-highlighted SourceView unless they contain a logical line over 2 KiB. Larger source previews and pathological lines use recycled plain-text rows; long logical lines are fully represented as contiguous, unwrapped 2 KiB virtual rows. Selection and model-backed copy preserve the original line breaks. File reading remains limited to 1 MiB, and **Open** remains available for the complete file.

When **Source** is the initial view, parsing is deferred until the user requests **Rendered**. Rendered paragraphs and code are grouped around a 32 KiB target, while individual pathological lines are divided into contiguous 2 KiB virtual rows before GTK layout; tables remain atomic, with exceptionally large cell displays bounded while table copy retains complete text. Complete code blocks use syntax highlighting, while blocks split for virtualization remain plain to keep highlighting bounded and lexically correct. Both large source and rendered previews use GTK list virtualization, so only visible rows own Pango layouts, text buffers, table grids, or cells. Drag selection is stored in document coordinates and survives row recycling; copy reads from the model, with tables represented as tab-separated text.

## Trust boundary

Document parsing and layout derivation run off the GTK thread through `gio::spawn_blocking`. The in-process, pure Rust parsers consume only the already bounded source string and produce escaped Pango markup. The layout boundary immediately decodes that reviewed markup into plain text and semantic spans before GTK receives it. Parser cancellation and preview request identity are checked before results are published.

This path does not use WebKit, Chromium, JavaScript execution, CSS rendering, subresource loading, external converters, or parser-initiated filesystem or network access. Native GTK creates the final widgets; only a user-activated, revalidated `http` or `https` link can launch an external application.
