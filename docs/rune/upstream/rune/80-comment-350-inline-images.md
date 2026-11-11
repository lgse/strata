# 80. Comment on #350: inline images in read-only rich text

| | |
| --- | --- |
| Kind | Comment on [#350](https://github.com/anomalyco/rune/issues/350) (inline images as part of a text run) |
| Drafts | R3 (C11) |
| Affects | C11 (document view: inline images in Markdown and documents) |
| Retires | C11's images as blocks of their own |
| Checked | Re-checked at `2a44156` (2026-11-10) by source, which corrects the draft: inline boxes aren't for edit-field pills only. Read-only `text`/`rich` take `El::pill` (`el/pills.rs:22`) and `El::inline_icon` (`el/pills.rs:57`, an icon or a `Pixels` image, square at `size`, before a word, skipped by selection and copy, nothing without a word after it); both were at the pin (`el/pills.rs:57` there too). #350 (open, no comments, `gh issue view` 2026-11-10) builds on `inline_icon` and already asks for everything C11 needs: "an inline box/image of a given size that takes room on the line, has a copy text, and can stand alone or next to another one". Gallery: C11's D2 (an inline equation shows its source) and R3 stand (`document_view/mod.rs:80-81`, `:116-117`; R3's wording repeats the draft's mistake). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Drop: duplicate of #350. The thread already asks for an inline image of a given size with a copy text that can stand alone; the comment's one claim of its own ("inline boxes exist for edit-field pills only") is wrong, since `El::inline_icon` sets images in read-only text. A read-only document view adds a use (README badges, inline figures and equations), not a need. |

Paste from here.

---

The same need in a read-only document preview: images inside a paragraph (badges in a README,
inline figures and equations in a document) can't sit in the text run: `El::inline_icon` sets
a square image only before a word, and selection and copy skip it. The selection and copy
behaviour asked here (copy the image's text or alt) is what a document view needs too.
