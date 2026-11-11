# Strata on Rune

Strata's interface is moving from GTK to Rune on the `strata-next` branch. This folder holds
the plans for that work, split out of two planning documents so they live beside the code.

| File | Holds |
| --- | --- |
| [workspace-migration.md](workspace-migration.md) | How the repository becomes one workspace for the GTK app, the shared crates and the Rune side, and how the branch lands on `main` |
| [slice/README.md](slice/README.md) | The vertical slice: one view, one read path and one write path on Rune, passing the same QA scenarios as the GTK app |
| [slice/V0-findings.md](slice/V0-findings.md) | What the slice's assumption checks found |
| `slice/V3-…` to `slice/V10-…` | One file per work package that is still to do, linked from the slice index |

## Where the plans came from

- "One workspace for Strata and its Rune components", revision 7 of 2026-11-09
  (<https://claude.ai/artifact/Jg9sdbrEKgrqKoLQAQFaao>).
- "Strata on Rune: vertical slice with QA-harness parity", 2026-11-09
  (<https://claude.ai/artifact/TVo6avcNyML2LRKuTNdoGb>).

The files here replace both as the working copy. Where the two disagreed, the owner's
rulings of 2026-11-10 decide, and each file says where it departs from its source.

## Reserved paths

`docs/rune/plan.md` and `docs/rune/work/` are kept free. The component plan and its work
packages arrive there from strata-components when its crates are imported
([slice/V4b](slice/V4b-import-components.md)).

## These files and `AGENTS.md`

`AGENTS.md` asks for task checklists and test results to live in issues and pull requests.
The work package files here are an exception the owner asked for on 2026-11-10, and the
workspace proposal already planned one for `docs/rune/work/`. `AGENTS.md` does not state the
exception yet.
