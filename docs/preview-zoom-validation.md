# Preview zoom validation

The change is limited to preview magnification and its input ownership. It uses
window-local state, adds no saved preference, and changes no dependency, build
configuration or theme token value. The theme provider priority is exposed so
preview fonts sit above Strata’s CSS reset and below the user stylesheet.

## Selected checks

- `./scripts/qa.sh preview-zoom-keeps-interface-size pdf-zoom-keeps-text-size chooser-pdf-zoom-keeps-text-size preview-source-text-copies rendered-document-word-selection`: **13 passed**. Covers source text, Markdown, PNG, PDF, chooser/default and 10xer reset, copying selected text, selection changes, slider limits, reset button, and preview-vs-interface wheel ownership.
- In the pinned container, `python3 scripts/test-headless.py -- ui::preview::tests ui::window::tests::keyboard_dispatch i18n:: --test-threads=2` selected 229 tests: 228 passed initially. The remaining test expected the old PDF-only action namespace; the shared preview now exposes `preview.reset-zoom`, and the keymap regression was updated to exercise that action. `python3 scripts/test-headless.py -- ui::window::tests::keyboard_dispatch::keymap_overrides::document_zoom_reset_follows_its_own_keys --test-threads=2` then passed (1 selected test). Unaffected passing results were preserved.
- After the reset-action correction, `./scripts/qa.sh preview-zoom-keeps-interface-size pdf-zoom-keeps-text-size chooser-pdf-zoom-keeps-text-size` passed all 9 affected runs. The earlier 4 source/rendered-copy passes remain applicable.
- `./scripts/quality.sh fmt` and `./scripts/quality.sh clippy` both passed again on the final code at the pre-push checkpoint.
- Negative control: temporarily changed the new scenario’s expected interface size from 17 to 18, ran `./scripts/qa.sh preview-zoom-keeps-interface-size/columns-source`, and observed the intended failure (`actual 17`). Restored the assertion and reran the passing selection above.
- Manually drove the application through `qa-drive.sh` in the pinned container, on its private Xvfb with accessibility bridging disabled. Inspected source text at approximately 201% and SVG images at 25%, 100% and approximately 201%; the header and file list retained their size, and enlarged images offered scrolling. Image captures are local artifacts in `target/preview-zoom-review/`.
- The new scenario’s four cost entries use timings from this 13-run selection, normalized within that selection. No full suite was run merely to refresh costs.

The full Rust and QA suites are intentionally omitted under AGENTS.md’s bounded,
risk-based validation policy. Adjacent preview, key dispatch, internationalization
checks are selected; CI remains authoritative for the full suite.

## Independent scenario grading

The author and an independent blind grader both scored the new scenario
**impact 2 / coverage 4 / criticality 4 / diagnostic 3 = 38** using `qa-quality`.
The separate grader edited nothing. `pdf-zoom-keeps-text-size` overlaps PDF wheel
isolation; the new scenario adds the slider, reset button, source/rendered/image
surfaces and selection lifetime. `chooser-pdf-zoom-keeps-text-size` covers a
distinct chooser context. Prior fix `58981b33` separated PDF and interface zoom;
recent preview refactors include `9c0cbc00`, `7dffb0e0` and `ef108c13`.

CodeScene was attempted but could not run because neither `CS_ACCESS_TOKEN` nor
its `CODE_SCENE_TOKEN` fallback is available. It has not been reported as passed.
