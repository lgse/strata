# Strata on Rune

Strata's interface is moving from GTK to Rune on the `strata-next` branch. This folder holds
the plans for that work, split out of two planning documents so they live beside the code.

| File | Holds |
| --- | --- |
| [workspace-migration.md](workspace-migration.md) | How the repository becomes one workspace for the GTK app, the shared crates and the Rune side, and how the branch lands on `main` |
| [slice/README.md](slice/README.md) | The vertical slice: one view, one read path and one write path on Rune, passing the same QA scenarios as the GTK app |
| [slice/V0-findings.md](slice/V0-findings.md) | What the slice's assumption checks found |
| `slice/V3-…` to `slice/V10-…` | One file per work package of the slice, linked from the slice index |
| [slice/slice-report.md](slice/slice-report.md) | What the slice proved and did not, what it cost, its gaps and measurements, and what is open after it |
| [plan.md](plan.md) | The component plan: which Strata area each Rune component covers, and the phases |
| [work/](work/README.md) | The component work packages and [their status](work/STATUS.md) |
| [decisions/](decisions/) | The component spikes S1 to S5, with the owner's rulings |
| [upstream/](upstream/) | What was asked of Rune upstream, and the audits behind it |
| [testing.md](testing.md) | How the Rune side's gallery, goldens, scripts and compositor tests work |

## Where the plans came from

- "One workspace for Strata and its Rune components", revision 7 of 2026-11-09
  (<https://claude.ai/artifact/Jg9sdbrEKgrqKoLQAQFaao>).
- "Strata on Rune: vertical slice with QA-harness parity", 2026-11-09
  (<https://claude.ai/artifact/TVo6avcNyML2LRKuTNdoGb>).

The files here replace both as the working copy. Where the two disagreed, the owner's
rulings of 2026-11-10 decide, and each file says where it departs from its source.

## The component documents

`plan.md`, `work/`, `decisions/`, `upstream/` and `testing.md` came from
`wmfeht/strata-components` at `e4f2df1a`, with its crates
([slice/V4b](slice/V4b-import-components.md)). They were written in that repository: the
plan and the testing document each open with what the import changed, and the work packages
stand as written, with their paths pointed at `docs/rune/`. Commit hashes they cite are that
repository's.

## These files and `AGENTS.md`

`AGENTS.md` asks for task checklists and test results to live in issues and pull requests.
The work package files here are an exception the owner asked for on 2026-11-10, and the
workspace proposal already planned one for `docs/rune/work/`. `AGENTS.md` states it under
"Two branches".
