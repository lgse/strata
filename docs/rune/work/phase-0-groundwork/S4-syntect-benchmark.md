# S4 syntect regex backend benchmark (spike)

| | |
| --- | --- |
| Phase / lane | 0 · Preview |
| Plan | §4 C10 ("Measure it against `onig` on the 50k-line fixture before settling"), decision 4 |
| Depends on | — |
| Unblocks | C10a |
| Owns | `docs/decisions/S4-syntect-backend.md`, a bench fixture under `tests/fixtures/code/` |
| Branch | `docs/<issue>-s4-syntect-benchmark` |

## Goal

Choose syntect's regex backend (`fancy-regex` or `onig`) with numbers, and check that the
grammar set from `two-face` passes `cargo deny`.

## Method

- Fixtures: a 50k-line file each in Rust, C, JavaScript, Python and Markdown, plus a 5,000-line
  shell script for the editable variant.
- Measure, for each backend: cold load of the syntax set, a full parse of each fixture, a
  256-line window parsed from a checkpoint, and a re-highlight after a one-line edit.
- Check `cargo deny check licenses` with `two-face` and both backends.
- Record the binary size added.

## Deliverable

A decision record with the table, the chosen backend, and the checkpoint interval (the plan
says 256 lines; confirm or change it).

## Done when

C10a can add the dependency with the chosen features, and the fixtures are committed for
C10a and C10b's perf tests.
