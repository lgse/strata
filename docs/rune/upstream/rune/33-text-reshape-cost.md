# 33. Text shaping: colour changes and single keystrokes reshape the whole text

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C10b-1, U-C10b-5 |
| Affects | C10b (code view and script editor), C10a (highlighting) |
| Retires | C10b's 128-line colour window and its D9 deviation (a 25–27 ms keystroke at 5,000 lines) |
| Checked | Re-checked at `2a44156`. By source, `text_hash` still hashes each span's start and end, with no colour (`text/hash.rs:39–40`), and `shape_key` (`:53`) keys shaping on it. Re-measured with a new headless probe, `rv_33`, in a release build: a 377 KB, 5,000-line text, counting `Runtime::stats.shaped`. 1,500 colour-only spans landing on 200 lines shape it again (117 ms), and so does that window moved by one line (117 ms); all 39,900 spans, 184 ms; the same spans with new colours shape nothing (2 ms). Each keystroke in the same text as a multi-line `text_input` shapes it again (116 ms). The text changes past the pin (Windfoil, subpixel modes, `dir_by_paragraph`) don't touch the key. In the gallery, C10b's 128-line span window (`code_view/editor.rs:21`, `SPAN_PAGE` at `:61`) and its D9 (`code_view/mod.rs:96–97`, `:141`, `:158`) are still there. #267 (a roadmap proposal) lists "typing reshapes and copies the whole field" as a theme linking an outside plan; it isn't an issue for this, so this entry stands and can cite it. Earlier: the hash by source at `5d421fd` (`:36`) and `04b38ae`; timings from C10b's probe P3 at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Colour-only span changes, and every keystroke in a multi-line field, reshape the whole text

Context: Strata (lgse/strata), a file manager on Rune, previews and edits source files of thousands
of lines with syntax colouring.

**What happens** (`5d421fd`; the hash is the same at `2a44156`):
- A text's colour spans are part of its shaping key: `text_hash` hashes every span's start and
  end (`text/hash.rs:39–40` at `2a44156`). So when highlighting lands, the whole text reshapes:
  in a 153 KB, 5,000-line field, about 25 ms with a 200-line span window, 160 ms with every
  line's 27,780 spans. Recolouring the same ranges doesn't reshape; any change to where spans
  start or end does, so a span window that follows the viewport reshapes as it moves.
- A keystroke in a multi-line `text_input` reshapes its whole text: about 19 ms a key in the
  same 5,000-line field with no spans.

At `2a44156`, in a release build with a 377 KB, 5,000-line text (`Runtime::stats.shaped` counts
the texts shaped): 1,500 colour-only spans landing on 200 lines, 1 text shaped, 117 ms; that
window moved by one line, 1 text, 117 ms; all 39,900 spans, 184 ms; the same spans with new
colours, none, 2 ms. A keystroke in the same text as a multi-line `text_input`, 1 text, 116 ms.

**Ask.** Key shaping on what changes shaping only (font, weight, size, italic, mono) and repaint
colour spans over the cached layout; and shape a field by paragraph, reusing the unchanged
paragraphs' layouts.

Related: #267 (its theme "typing reshapes and copies the whole field").
