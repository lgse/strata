# Rendered document previews

Strata can render local Markdown, a deliberately small HTML subset, RTF, CSV, and TSV with native GTK widgets. Each document preview has **Rendered** and **Source** views. **Render documents by default** under **Settings → General → Browsing** chooses the initial view for each newly opened document; switching the current preview does not change that preference.

Remote Markdown and HTML locations remain source-only. Rendered previews are selected from the local filename or content type:

- Markdown: `.md`, `.markdown`, `.mdown`, `.mkd`, `.mkdn`, `.mdwn`, `text/markdown`, and `text/x-markdown`;
- HTML: `.html`, `.htm`, `.xhtml`, `text/html`, and `application/xhtml+xml`;
- RTF: `.rtf`, `application/rtf`, and `text/rtf`;
- delimited tables: `.csv`, `.tsv`, `text/csv`, and `text/tab-separated-values`.

Local XLS, XLSX, and ODS workbooks use the same table renderer but do not expose a raw Source view. Only the first worksheet is previewed. Formulas are not recalculated, macros are never executed, and workbook styling, charts, and merged cells are not reproduced. Values use Calamine's string conversion, not Excel's full number-format display.

Local DOCX documents render the same way and also have no Source view, since the file is a ZIP container rather than text.

## Supported content

Markdown uses `pulldown-cmark` with tables, strikethrough, and task lists enabled. Headings, paragraphs, emphasis, lists, block quotes, web links, fenced and inline code, rules, tables, and line breaks render natively. Known fenced-code language hints use Strata's existing, theme-aware GtkSourceView syntax highlighting. Raw HTML is displayed as inert text.

Markdown images with relative paths render PNG, JPEG, GIF (first frame), WebP, BMP, and SVG content. Percent-encoded filenames are supported. Image files must be inside the document's folder or its descendants; symlinks cannot escape that folder. Inline images appear below their containing paragraph or list item. Images in tables remain alt-text placeholders. Absolute paths, `file:` URLs, data URLs, and remote images are not loaded; remote URLs are discarded and a notice explains the restriction.

Fenced `mermaid` blocks render offline with `mermaid-svg`, a native Rust renderer, including flowcharts and sequence diagrams. Diagrams use the active theme's text and surface colors and update live. **Copy diagram source** copies the original Mermaid text. Unsupported or invalid syntax retains readable source with a fallback notice; the renderer does not promise complete Mermaid.js compatibility.

LaTeX equations use `$...$` inline, `$$...$$` for display math, or fenced `latex`/`math` blocks. Inline equations stay within the surrounding prose; selection/copy preserves the original LaTeX delimiters. Display equations offer **Copy equation source**. Base TeX and AMS notation supports fractions, roots, superscripts/subscripts, sums, integrals, and matrices. Unsupported commands fall back to readable source. Equations in table cells remain source text, and escaped dollars and code spans are not interpreted as equations. This is math notation support, not a full LaTeX document compiler.

RTF is converted in-process to the same HTML subset. Paragraphs, line breaks, bold, italic, underline, strikethrough, `\'hh` Windows-1252 escapes, and `\uN` Unicode escapes with their `\ucN` fallback characters are supported. Font, colour, style, list, and table control words carry no formatting: table cells and list markers become plain paragraph text, and font tables, stylesheets, pictures, headers, footers, footnotes, annotations, and other non-body destination groups are discarded. Code pages other than Windows-1252 are not honoured, so `\'hh` bytes in a document declaring another code page can decode to the wrong character.

DOCX is read with `docx-rs` inside the [preview sandbox](preview-sandbox.md) and converted to the same HTML subset. Headings (`Heading1`-`Heading6`, `Title`, `Subtitle`), quote styles, paragraphs, bold, italic, underline, strikethrough, line breaks, tabs, bulleted and numbered lists with their nesting level, and tables render natively; the first table row becomes the column titles. Hyperlink text is kept but the link target is not, and images, drawings, text boxes, headers, footers, footnotes, comments, fields, and theme or style-sheet formatting are omitted. Table cells show their paragraphs joined into a single value, and a nested table's text is flattened into its containing cell. Legacy DOC, ODT, and presentation formats are not supported.

HTML uses `html5gum` tokenization and supports semantic containers, headings, paragraphs, emphasis, underline (`u`, `ins`), lists, block quotes, `http` and `https` links, `pre`/`code`, rules, simple tables, and line breaks. Normal flow whitespace is collapsed like HTML, while `pre` content remains exact. A known `language-*` class on `code` inside `pre` is used only as a syntax hint; it does not enable CSS. Entities are decoded before their text is escaped for Pango.

CSS, classes, IDs, metadata, and other presentation attributes are ignored. Scripts, styles, forms, frames, objects, embedded content, images, media, event handlers, unsafe links, and resource-bearing tags or attributes are omitted. If a document still has useful safe content, Strata renders it with an omission warning. Link schemes are checked again when a link is activated before the existing external URI launcher is used.

## Limits and fallback

Rendered parsing has fixed limits:

- 1 MiB input;
- 20,000 parser events;
- nesting depth 32;
- 256 columns per table row (there is no 512-cell table ceiling);
- 4 MiB of escaped Pango markup;
- 500 ms parser time.

A truncated, malformed, contentless, timed-out, cancelled, or limit-exceeding document opens in **Source**. The view-switch control is hidden and the reason is shown above the source preview.

Small source previews up to 128 KiB and 512 lines retain the syntax-highlighted SourceView unless they contain a logical line over 2 KiB. Larger source previews and pathological lines use recycled plain-text rows; long logical lines are fully represented as contiguous 2 KiB virtual rows. The saved **Wrap lines** choice applies to source and rendered prose. Selection and model-backed copy preserve the original line breaks. File reading remains limited to 1 MiB, and **Open** remains available for the complete file.

When **Source** is the initial view, parsing is deferred until the user requests **Rendered**. Rendered paragraphs and code are grouped around a 32 KiB target, while individual pathological lines are divided into contiguous 2 KiB virtual rows before GTK layout; tables remain atomic document-selection units but use their own recycled, vertically scrolling rows, with exceptionally large cell displays bounded while table copy retains complete text. Cell formatting that expands beyond 16 KiB is shown as plain text with an explanatory tooltip. Complete code blocks use syntax highlighting, while blocks split for virtualization remain plain to keep highlighting bounded and lexically correct. Both large source and rendered previews use GTK list virtualization, so only visible rows own Pango layouts, text buffers, table grids, or cells. Drag selection is stored in document coordinates and survives row recycling; copy reads from the model, with tables represented as tab-separated text.

## Interactive tables

CSV and TSV use quote-aware parsing, preserve multiline fields, and tolerate ragged rows. The first row becomes column titles. Markdown and HTML tables share the same native `ColumnView`; HTML row headers remain styled cells when the first row is not entirely headers.

Tables open sorted ascending by the first column. Hover a column title for the pointer cursor, click to sort ascending or descending, and drag its divider to resize it. A compact up/down caret at the right of the header indicates the current direction. Standalone tables fill the preview's available height with a minimum height; tables embedded in prose retain their own bounded scrolling area. Sorting uses finite numeric values first, case-insensitive text next, and empty values last in ascending order; descending reverses that order. Sorting changes only the preview, never the file. Initial widths sample up to 64 rows, reserve enough room for compact fields, and give spare width to longer text. Every column, including the last, can be resized. Manual widths and sort order survive row recycling while that rendered view remains open.

Drag through cells and rows to select a continuous range in displayed sort order; dragging near an edge scrolls the table and preserves selection across recycled rows. **Ctrl+C** copies that range with tabs between cells and newlines between rows. A single click in any cell or empty space, or changing the sort, clears the selection. Native keyboard selection within a cell remains available. With focus on the table rather than a cell label, **Ctrl+C** copies all loaded rows in their current sorted order, with original full cell text and tab separators. Cross-document selection also copies tables in their current order. Native sortable column titles are plain text; body cells retain supported inline styling and safe links.

The old 200-row display cap is removed. Delimited/workbook output is instead bounded by 100,000 values, 4 MiB of cell text, and 256 columns, with a visible warning when output is limited. CSV/TSV retain the 1 MiB source-reading limit and Source fallback. Virtualization bounds resident row widgets; it does not make parsing or model memory unlimited.

Images and diagrams load asynchronously as rows are visited, with at most 16 media items (images, diagrams, and equations) retained per preview. Local images are limited to 8 MiB, Mermaid input to 16 KiB and 256 lines, equation input to 4 KiB, and decoded output to 800 × 800 pixels. Larger Mermaid blocks remain virtualized code. Each media renderer has a three-second deadline; closing the preview cancels pending work. Missing files, unsupported formats, and decoder errors preserve alt text or diagram source without preventing the rest of the document from rendering.

## Trust boundary

Document parsing and layout derivation run off the GTK thread through `gio::spawn_blocking`. The in-process, pure Rust parsers consume only the already bounded source string and produce escaped Pango markup. The layout boundary immediately decodes that reviewed markup into plain text and semantic spans before GTK receives it. Parser cancellation and preview request identity are checked before results are published.

Workbooks and Word documents are different: Calamine can allocate a complete worksheet before rows are extracted, and `docx-rs` decompresses every document part into memory. XLS/XLSX/ODS and DOCX parsing therefore runs in the [preview sandbox](preview-sandbox.md), with a 20 MiB input-file limit, 2 GiB address-space limit, 10-second CPU limit, and 12-second wall deadline. Cancellation terminates the helper. The application validates bounded JSON cell or HTML output before parsing and deriving the shared document layout; DOCX HTML is generated by the helper, escaped, and reparsed by the same bounded HTML parser local `.html` files use. Compressed file size alone is not a decompression-memory limit.

This path does not use WebKit, Chromium, document script execution, or document CSS rendering. Document text parsers do not access files or the network. Markdown image references are opened atomically beneath the document directory, checked for regular-file type and size, then copied to private temporary inputs. Image decoding, Mermaid rendering, and equation rendering run in the existing resource-limited Bubblewrap sandbox, with no network or document-directory mount. Equations use a bundled, pinned MathJax program in QuickJS with no host bindings or module loader; user input is a string argument, never executable JavaScript. The JavaScript heap and stack are capped at 64 MiB and 1 MiB, with a two-second interrupt deadline. Only base and AMS TeX packages are registered; autoload, require, and HTML extensions are unavailable. No external TeX executable or npm installation is needed at runtime. See [renderer build inputs](../packaging/math-renderer/README.md). SVG subresource resolvers are disabled, including embedded images. Only validated bounded PNG output crosses back to GTK. Native GTK creates the final widgets; only a user-activated, revalidated `http` or `https` link can launch an external application.
