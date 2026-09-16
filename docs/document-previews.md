# Rendered document previews

Strata can render local Markdown and a deliberately small HTML subset with native GTK widgets. Each document preview has **Rendered** and **Source** views. **Render documents by default** under **Settings → General → Browsing** chooses the initial view for each newly opened document; switching the current preview does not change that preference.

Remote Markdown and HTML locations remain source-only. Rendered previews are selected from the local filename or content type:

- Markdown: `.md`, `.markdown`, `.mdown`, `.mkd`, `.mkdn`, `.mdwn`, `text/markdown`, and `text/x-markdown`;
- HTML: `.html`, `.htm`, `.xhtml`, `text/html`, and `application/xhtml+xml`.

## Supported content

Markdown uses `pulldown-cmark` with tables, strikethrough, and task lists enabled. Headings, paragraphs, emphasis, lists, block quotes, web links, fenced and inline code, rules, tables, and line breaks render natively. Known fenced-code language hints use Strata's existing, theme-aware GtkSourceView syntax highlighting. Raw HTML is displayed as inert text.

Markdown images with relative paths render PNG, JPEG, GIF (first frame), WebP, BMP, and SVG content. Percent-encoded filenames are supported. Image files must be inside the document's folder or its descendants; symlinks cannot escape that folder. Inline images appear below their containing paragraph or list item. Images in tables remain alt-text placeholders. Absolute paths, `file:` URLs, data URLs, and remote images are not loaded; remote URLs are discarded and a notice explains the restriction.

Fenced `mermaid` blocks render offline with a native Rust renderer, including flowcharts and sequence diagrams. Diagrams use the active theme's text and surface colors and update live. **Copy diagram source** copies the original Mermaid text. Unsupported or invalid syntax retains readable source with a fallback notice; the renderer does not promise complete Mermaid.js compatibility.

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

A truncated, malformed, contentless, timed-out, cancelled, or limit-exceeding document opens in **Source**. The view-switch control is hidden and the reason is shown above the source preview.

Small source previews up to 128 KiB and 512 lines retain the syntax-highlighted SourceView unless they contain a logical line over 2 KiB. Larger source previews and pathological lines use recycled plain-text rows; long logical lines are fully represented as contiguous 2 KiB virtual rows. The saved **Wrap lines** choice applies to source and rendered prose. Selection and model-backed copy preserve the original line breaks. File reading remains limited to 1 MiB, and **Open** remains available for the complete file.

When **Source** is the initial view, parsing is deferred until the user requests **Rendered**. Rendered paragraphs and code are grouped around a 32 KiB target, while individual pathological lines are divided into contiguous 2 KiB virtual rows before GTK layout; tables remain atomic, with exceptionally large cell displays bounded while table copy retains complete text. Cell formatting that expands beyond 16 KiB is shown as plain text with an explanatory tooltip. Complete code blocks use syntax highlighting, while blocks split for virtualization remain plain to keep highlighting bounded and lexically correct. Both large source and rendered previews use GTK list virtualization, so only visible rows own Pango layouts, text buffers, table grids, or cells. Drag selection is stored in document coordinates and survives row recycling; copy reads from the model, with tables represented as tab-separated text.

Images and diagrams load asynchronously as rows are visited, with at most 16 media items retained per preview. Local images are limited to 8 MiB, Mermaid input to 16 KiB and 256 lines, and decoded output to 800 × 800 pixels. Larger Mermaid blocks remain virtualized code. Each media renderer has a three-second deadline; closing the preview cancels pending work. Missing files, unsupported formats, and decoder errors preserve alt text or diagram source without preventing the rest of the document from rendering.

## Trust boundary

Document parsing and layout derivation run off the GTK thread through `gio::spawn_blocking`. The in-process, pure Rust parsers consume only the already bounded source string and produce escaped Pango markup. The layout boundary immediately decodes that reviewed markup into plain text and semantic spans before GTK receives it. Parser cancellation and preview request identity are checked before results are published.

This path does not use WebKit, Chromium, JavaScript execution, or document CSS rendering. Document text parsers do not access files or the network. Markdown image references are opened atomically beneath the document directory, checked for regular-file type and size, then copied to private temporary inputs. Image decoding and Mermaid rendering run in the existing resource-limited Bubblewrap sandbox, with no network or document-directory mount. SVG subresource resolvers are disabled, including embedded images. Only validated bounded PNG output crosses back to GTK. Native GTK creates the final widgets; only a user-activated, revalidated `http` or `https` link can launch an external application.
