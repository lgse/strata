# Word document preview fixture

`report.docx` is a hand-authored minimal OOXML package (no Word or LibreOffice
involved) holding one of each structure the DOCX preview maps:

- `Heading1` and `Heading2` styled paragraphs, and a `Quote` styled paragraph;
- a paragraph mixing bold, italic, underlined, and struck runs with `&` and `<`
  (underline carries `w:val="single"`, as Word writes it: `docx-rs` panics on a bare
  `<w:u/>`);
- a bulleted list with a nested second level (`numId` 1, `bullet` format);
- a numbered list (`numId` 2, `decimal` format);
- a three-row table whose first row supplies the column titles;
- a run containing a `<w:br/>` line break;
- an empty `<w:drawing>` run, which the preview must drop.

The package contains only `[Content_Types].xml`, `_rels/.rels`,
`word/_rels/document.xml.rels`, `word/document.xml`, `word/numbering.xml`, and
`word/styles.xml` — the parts `docx-rs` requires plus numbering and styles.

# OpenDocument text preview fixture

`report.odt` is a hand-authored minimal ODF 1.3 package (written without an office
suite, and checked to open in LibreOffice) holding one of each structure the ODT
preview maps or omits:

- `text:h` headings at outline levels 1 and 2;
- a paragraph mixing bold, italic, underlined, and struck automatic span styles,
  the named `Strong Emphasis` and `Source Text` styles from `styles.xml`, and `&`
  and `<`, followed by a footnote and a comment that the preview must drop;
- a paragraph whose automatic style is bold, with a span that switches bold off;
- a paragraph inheriting `Quotations`, and three inheriting `Preformatted Text`,
  one indented with `<text:s text:c="4"/>`;
- a bulleted list (`L1`) whose nested list inherits the outer list's style, and a
  numbered list (`L2`);
- a three-row table whose first row sits in `table:table-header-rows`;
- a `<text:line-break/>`, an `https` link, and an internal `#Totals` link;
- a text box in a `draw:frame`, a tracked deletion, and a header in the master
  page, none of which may appear in the preview.

The package contains `mimetype` (stored first and uncompressed),
`META-INF/manifest.xml`, `styles.xml`, and `content.xml`.

# PDF text-layer fixture

`text.pdf` is a hand-written minimal PDF (single Helvetica text stream, no
compression) whose two text lines let the quick preview exercise its text
layer: glyph hit-testing, drag selection, and copy.

`password-protected.pdf` and `owner-restricted.pdf` encrypt the same document
with AES-256. The first requires the artificial user password `preview`; the
second has an empty user password and must still preview without prompting.
Both use the artificial owner password `owner`. Generate them with:

```sh
cd tests/fixtures/documents
qpdf --encrypt preview owner 256 -- text.pdf password-protected.pdf
qpdf --encrypt '' owner 256 --print=none --modify=none --extract=n -- text.pdf owner-restricted.pdf
```

QPDF is needed only to regenerate these fixtures, not to run the tests.
