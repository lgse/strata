# Scoring worksheet

Copy one block per scenario into a scratch file outside the tree (for example
`target/qa-quality/<folder>.md`). Keep the batch log in the issue or PR, never
in the source tree.

```text
Scenario:      <folder>/<name>.jsonl
Behaviour:     <action> via <route> in <modes> → <asserted outcome>
Runs:          modes=<...> variants=<...>; distinct paths: <which ones differ, and how>
Overlaps:      <scenario>: <shared fixture/route/mode/assertions>; superset? <yes/no/mirror>
Unit/GTK:      <src/.../tests.rs::fn>: pins <what>; leaves <what> to the GUI
History:       churn <n> commits/90d in <paths>; fixes <ids>; origin <hash> (<fix|feat|test>)
Impact:        <1-5>  because <worst realistic failure>
Coverage:      <1-5>  because <what only this scenario verifies>
Criticality:   <1-5>  because <likelihood + whether only the GUI catches it>
Diagnostic:    <1-5>  because <what a failing step would tell the reader>
Score:         <quality.py score I C K D>
Note:          <behaviour>; <deciding factor>
```

## Checklist

- [ ] Read the header and every step, with their `modes`, `variants`, `when`,
      `if` and `unless` guards.
- [ ] Ran `quality.py similar`, and grepped for the controls, chords,
      predicates and preferences the scenario uses.
- [ ] For each duplicate pair, decided which side is the superset and named it
      in both notes.
- [ ] Checked the implementing module's `tests.rs` for the same logic.
- [ ] Measured churn and fixes with `git log`; ran `-S` for the origin.
- [ ] Counted modes and variants only where they take distinct paths.
- [ ] Scored cosmetic content at most coverage 2, and geometry-only content
      at coverage 1.
- [ ] Scored diagnostic from what a real failure would say, not from length
      or message count.
- [ ] For a score of 0, checked which cause applies (coverage 1, or impact,
      criticality and diagnostic all 1) and that the note says it.
- [ ] Wrote the note as one line, with no verdict.
- [ ] Wrote the field with `quality.py write`; `git diff` shows one changed
      line.

## Batch log row

```text
| scenario | I | C | K | D | score | scorer | date | override |
```

Put an override's reason in the scenario note (`; override: <reason>`) and in
this row.
