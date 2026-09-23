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
