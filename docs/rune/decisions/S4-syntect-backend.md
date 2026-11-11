# S4: syntect regex backend and checkpoint interval

| | |
| --- | --- |
| Status | Decided by the owner after review round 3 (see [Decision](#decision)). The owner first chose `onig`, N = 64, the RUSTSEC-2025-0141 ignore and WTFPL after round 1; review then found two `onig` costs (B1, B2), the options were put again ([Options put to the owner](#options-put-to-the-owner)), and the owner confirmed `onig`, choosing option (a) for B1 |
| Work package | [S4-syntect-benchmark](../work/phase-0-groundwork/S4-syntect-benchmark.md) |
| Consumers | C10a (highlighter), C10b (code view), C11 (Markdown code blocks) |
| Fixtures | [`tests/fixtures/code/`](../../tests/fixtures/code/) (see its README) |
| Harness | `/home/wmfeht/dev/strata-spikes/s4-bench`, outside the repository and not merged. Evidence files below are under its `out/` |

## Decision

The owner decided:

- **Backend: `onig`.** B1 is solved with option (a): a dedicated highlighter thread owns every
  `ParseState`, and there is no `unsafe`. B2 is handled with workaround 1.
- **Checkpoint interval: N = 64.** At p95 it meets the 8 ms reading on all six fixtures (largest
  p95 7.4 ms, Markdown), and the 16.7 ms reading with margin (see e).
- **RUSTSEC-2025-0141:** the ignore is approved.
- **Grammar content:** the WTFPL GraphQL grammar, and the Unlicense and Sublime-licensed grammars
  that come with two-face, are accepted.

C10a adds these to the root `[workspace.dependencies]`:

```toml
# Regex backend and grammar set chosen in S4 (docs/decisions/S4-syntect-backend.md).
syntect = { version = "5.3.0", default-features = false, features = ["parsing", "regex-onig"] }
two-face = { version = "0.5.2", default-features = false, features = ["syntect-onig"] }
```

and this to `deny.toml`'s `[advisories] ignore`:

```toml
{ id = "RUSTSEC-2025-0141", reason = "bincode 1 is syntect's and two-face's syntax dump format; unmaintained, no known vulnerability, and its authors call 1.3.3 complete" },
```

Owner items. The owner makes these follow-up edits; this work package does not edit them:

- [`docs/plan.md`](../plan.md) §4 C10:
  - "Use the `fancy-regex` backend … Measure it against `onig`" becomes `onig`, citing this
    record.
  - "keep a parse and highlight state checkpoint every 256 lines" becomes 64.
  - The worker wording gains the dedicated highlighter thread (B1 (a)).
- §7, phase 0: the syntect spike row can point to this record.
- C10a's doc:
  - "N from S4, default 256" becomes 64.
  - "Requests run on a worker through `rune::Latest`" becomes requests to the highlighter thread,
    which owns every `ParseState` (B1).
  - The budget wording ("one frame budget on the worker") names a reading. N = 64 is sized for
    the 8 ms reading at p95.
  - The Edits bullet ("re-parses until the state matches a cached checkpoint again") and the unit
    test "an edit re-parses only a few lines" conflict with B2 and workaround 1 on `onig`.
    Inside a heredoc or fence the re-parse runs to the block's end, and checkpoint states inside
    a capture context never match. The doc should instead name a per-line state cache,
    workaround 1's foreground cap and background pass, and test exactness once the background
    pass finishes.
- C10b's doc, if the budget reading changes its perf runs.

## Summary

| | `fancy-regex` | `onig` |
| --- | --- | --- |
| Speed, source fixtures | 2.6–2.9× slower than `onig` | — |
| Speed, Markdown with heavy fenced code | 21× slower (a high-end case, see [Markdown](#why-markdown-is-slow-on-fancy-regex)) | — |
| Worst-case viewport within 8 ms (p95) | Source: only with a state cached for every line (N = 1). Markdown: never | All six fixtures at N ≤ 64 |
| Worst-case viewport within 16.7 ms (p95) | Source: N ≤ 64. Markdown: never | All six fixtures at N ≤ 256 |
| `ParseState` is `Send` (B1) | Yes | **No**: needs a highlighter thread or an `unsafe` wrapper |
| `ParseState` equality (B2) | By value | **By pointer** while a capture-holding context is open, so a state never equals its own clone there. It matters only while editing. An edit inside a heredoc or fence re-parses to the block's end: about 25–55 ms per 1,000 fenced lines in the background, where `fancy-regex` stops after 1–3 lines (see [B2](#b2-with-onig-parsestate-equality-compares-pointers)) |
| Grammars | 213: lacks PowerShell, JavaScript (Babel) and 5 more | All 220 |
| Binary size added (stripped) | 2.85 MB | 1.71 MB |
| C code in the build | None | Oniguruma, built with `cc` |
| bincode advisory | Same on both | Same on both |

The decision rule, fixed before measuring, chooses `fancy-regex` unless (1) it misses a budget
that `onig` meets, (2) its highlighting is visibly wrong on the fixtures, or (3) the grammars it
drops are needed. Rule (2) does not apply (see g). Rule (1) depends on which budget applies (see
[Budgets](#budgets)):

- **8 ms:** rule (1) applies. `fancy-regex` meets 8 ms on the source fixtures only with a state
  for every line (23–60 MiB per 50k-line file), and never on Markdown. `onig` meets it on all six
  fixtures at N = 64.
- **16.7 ms:** rule (1) applies only through the Markdown fixture. `fancy-regex` meets the budget
  on all five source fixtures at N = 64, but Markdown misses it at every N, even N = 1
  (121 ms p95). `onig` meets it everywhere at N = 256. Whether Markdown source previews alone
  should decide the backend is an owner call.

Either way `onig` has two costs the rule does not weigh: B1 and B2 below. Rule (3) is an owner
call too: `fancy-regex` loses PowerShell, among others.

## Budgets

| Number | Source | What it is |
| --- | --- | --- |
| 8 ms | [`docs/plan.md`](../plan.md) §8 ("`RUNE_COMMIT_BUDGET_MS=8` runs over scripted scrolls") and C10b ("the 50k-line scroll under `RUNE_COMMIT_BUDGET_MS=8`") | A **UI-commit** budget. Rune prints each commit (view, layout, apply) that takes longer. It does not time worker threads |
| "one frame budget on the worker" | C10a, Tests | No number. At 60 Hz one frame is 16.7 ms |
| 4 ms | The S4 planning stage (coordinator-approved plan), not a repository document | A stretch target, half of 8 ms |
| 2 ms | C10a ("a one-line edit in the 5,000-line script stays under 2 ms") | Edit re-highlight |

The highlight runs on a worker, so the commit budget does not bound it directly. It bounds how
long lines stay plain after a jump. This record therefore gives both readings, 8 ms and 16.7 ms,
and C10a's doc should name one. The decision uses p95 over 200 positions. Max is shown beside it,
and one preemption on this shared machine can make it jump (fancy, JavaScript, N = 128 had one
842 ms sample).

## B1: with `onig`, `ParseState` is not `Send` or `Sync`

`onig::Region` (onig 6.5.3) wraps the C `OnigRegion`, whose `beg`, `end` and `history_root` are
raw pointers, so it is neither `Send` nor `Sync`. syntect keeps a `Region` in each parse-stack
level that holds captures (`StateLevel.captures`, `parser.rs:67`). So `ParseState`, and anything
holding one (a checkpoint, a per-line cache), cannot cross threads. `SyntaxSet` (`Send + Sync`),
`HighlightState` and `Style` (`Send`) are fine on both backends; `fancy-regex` passes every check.
Evidence: `out/b1-sendcheck.txt`, from `src/bin/sendcheck.rs`, which compiles on `fancy` and
fails on `onig` with ``*mut i32` cannot be sent between threads safely`` at
`send::<ParseState>()` and `sync::<ParseState>()`.

`rune::Latest::request` takes `K: Send + 'static` and a `FnOnce(K) -> T + Send + 'static`
closure (`crates/rune/src/latest.rs:171`), with `T: Send`, and runs on a pool of up to four
threads. So checkpoints cannot be moved into a `Latest` request, or shared through
`Arc<Mutex<…>>`, on `onig`. Options:

**(a) One long-lived highlighter thread that owns every `ParseState`.** No `unsafe`. Sketch:

```rust
enum Request {
    Open { doc: DocId, text: Arc<str>, syntax: String },
    Viewport { doc: DocId, lines: Range<usize>, generation: u64 },
    Edit { doc: DocId, line: usize, text: Arc<str>, generation: u64 },
    Close { doc: DocId },
}
// Thread state: Arc<SyntaxSet>, HashMap<DocId, Doc { checkpoints: Vec<State>,
// per_line: Option<Vec<State>>, latest: Arc<AtomicU64> }>.
// Replies (colour spans per line, which are Send) go back over an mpsc channel and wake the
// window with rune::Waker (Arc<dyn Fn() + Send + Sync>, documented for background threads).
```

Costs:
- One thread for the app, idle when nothing is open.
- All highlighting is serialized through it, from C10, C11 blocks and the editor. So the opening
  checkpoint pass of a 50k-line file (0.6–1.4 s on `onig`) must run in slices of a few hundred
  lines, so viewport and edit requests can cut in.
- Latest-wins has to be rebuilt: the thread checks a per-document generation counter and skips
  superseded work. `Latest` can still front it, with a closure that sends and blocks on the
  reply, but that ties up one of Latest's four pool threads per waiting request. Waking the
  window directly avoids that.
- A channel round trip costs microseconds, which is negligible here.

**(b) A `SendState` newtype with `unsafe impl Send`.**

Soundness argument, for onig 6.5.3 and onig_sys 69.9.3:
- Each `Region` owns its `beg`/`end` arrays (and `history_root`, which is null unless capture
  history is used) exclusively. `Clone` deep-copies them through `onig_region_copy`, and `Drop`
  frees them through `onig_region_free`.
- These are plain `malloc`/`free`/`memcpy` with no thread-local or global state, and syntect
  never hands out references into them.
- So moving a whole `ParseState` to another thread moves unique ownership, and `Send` is sound.

`Sync` is not needed, and not argued: `Region::raw_mut` casts `&self` to `*mut`, so the newtype
should not hand out a `&ParseState` to two threads at once.

Costs:
- The workspace sets `unsafe_code = "deny"`, so the newtype needs a local
  `#[expect(unsafe_code, reason = "…")]` and a `// SAFETY:` comment. That is an explicit owner
  exception.
- The argument rests on onig internals, so every onig or syntect bump must re-check it.
- With it, states can move into `Latest` requests. `Arc<Mutex<SendState>>` is sound too, since
  `Mutex<T>: Sync` needs only `T: Send`. So for threading, (b) gives C10a essentially the same
  design as `fancy-regex`: checkpoints behind a mutex, or moved into requests. B2 still applies.

**(c) Count it for `fancy-regex`,** whose `ParseState` is `Send + Sync`: checkpoints can sit in
`Arc<Mutex<…>>` or move into `Latest` closures with no extra machinery.

**Edits under B1.** The round-1 advice said "re-highlight synchronously up to the viewport plus
overscan, then finish the cascade on the worker". That moves states between threads, which
`onig` forbids without (b). Corrected: the per-line cache and every re-parse live on whichever
thread owns the states. Under (a) the highlighter thread does both the capped pass and the
continuation, and the UI thread only receives spans. Under (b) or with `fancy-regex`, states may
move with the request.

## B2: with `onig`, `ParseState` equality compares pointers

`onig::Region` derives `PartialEq` over the raw `OnigRegion`, so two regions compare by pointer
address. A `Region` is therefore never equal to its own clone (the clone deep-copies), so a
`ParseState` holding captures is never equal to its clone either. `fancy-regex`'s region is a
`Vec` of positions and compares by value. Evidence: `out/b2-onig-region-repro.txt`
(`src/bin/onig_region.rs`: same positions, `region == region.clone()` is false) and
`out/b2-selfeq.txt` (lines whose starting state differs from its own clone):

| Fixture | `onig` | `fancy-regex` |
| --- | --- | --- |
| markdown-50k | 29,138 / 50,000 | 0 |
| python-50k | 2,337 / 50,000 | 0 |
| shell-5k | 586 / 5,000 | 0 |
| rust, c, js | 0 | 0 |

`HighlightState` compares equal to its clone on every line, on both backends.

Only the "stop when the state matches the cached one" step uses equality, and only edits use that
step. So B2 matters only while editing: read-only views (C10's source preview, C11's code blocks)
never compare states. Resuming from a checkpoint is unaffected: all d windows were exact. On `onig`,
an edit inside a capture-holding context (a heredoc, a fenced block, some Python constructs) cannot
converge until that context closes. Checkpoints that fall inside one never match. This explains the
round-1 checkpoint-cache difference: `case` edits re-parsed 512 lines (median) on `onig` and 256 on
`fancy-regex`.

Measured (`review edits`, A B A B, worse round's p95, max across rounds; `out/results2.txt`):

| Edit, re-highlight strategy | `onig` time p95 / max (lines re-parsed p95 / max) | `fancy-regex` time p95 / max (lines p95 / max) |
| --- | --- | --- |
| Inside a heredoc (100 edits): per-line cache, stop on equal state | 0.08 / 0.09 (3 / 3) | 0.30 / 0.35 (1 / 1) |
| Inside a heredoc: checkpoints only, N = 64 | 2.45 / 4.43 (128 / 192) | 5.73 / 6.09 (64 / 64) |
| Inside a Markdown fence (30 edits): per-line cache, stop on equal state | 6.37 / 7.97 (44 / 61) | 49.1 / 130.7 (26 / 61) |
| Inside a fence: checkpoints only, N = 64 | 18.7 / 29.4 (320 / 448) | 126 / 234 (64 / 128) |
| Inside a fence: checkpoints only, N = 256 | 92.8 / 151.6 (2,048 / 3,072) | 403 / 528 (256 / 512) |
| Rename anywhere in shell-5k (200 edits): per-line cache, stop on equal state | 0.21 / 1.64 (7 / 73) | 0.73 / 1.46 (7 / 73) |
| Rename anywhere: checkpoints only, N = 64 | 4.03 / 6.33 (128 / 192) | 4.95 / 6.88 (64 / 128) |

On `onig` with a per-line cache, an edit inside a heredoc or fence re-parses to that block's
end. The fixture's heredocs are 2–3 lines and its fences at most 60, so a long real heredoc or
fenced block costs more. `fancy-regex` stops at the true convergence point (1 line for a
heredoc).

The fence rows are grammar-confounded. On `onig`, ` ```js ` fences use `JavaScript (Babel)`; on
`fancy-regex`, `JavaScript` (see g). The medians are close: `onig` 1.96 ms and 11 lines,
`fancy-regex` 1.85 ms and 1 line. `fancy-regex`'s tail (26 / 61 lines, 49 / 131 ms) comes from
two renames of a keyword in js fences, at lines 5630 and 20373, which under `JavaScript` really
change the following lines (`onig`, under Babel, re-parses 61 and 27 lines there too). The
reviewer's probe confirms it (`/home/wmfeht/dev/strata-spikes/s4-review2/fence-out.txt`):
`fancy-regex` converged after 1–2 lines at the other 28 sites. So the tail is real re-parsing
under a different grammar, plus `fancy-regex`'s cost per fenced line (see
[Markdown](#why-markdown-is-slow-on-fancy-regex)).

Scale. Round 2 gave 0.15 ms per re-parsed line, from one sample (6.37 ms over 44 lines). That
overstated the cost 3–6×. The reviewer measured directly
(`/home/wmfeht/dev/strata-spikes/s4-review3/`, `src/bin/scale.rs`):

- **The same 30 fence edits**, best of 5: 0.006–0.068 ms a line, about 0.045 typical
  (`site-out.txt`). Together they re-parsed 475 lines in 20.2 ms, 0.043 ms a line
  (`bench-edit-out.txt`).
- **An edit at the top of a 1,000-line block** (`scale-out.txt`, 30 edits each). `onig`
  re-parsed 937–1,000 lines; `fancy-regex` stopped after 1–3 lines (12 in the shell fence).

  | Block | `onig` p95 | `fancy-regex` p95 |
  | --- | --- | --- |
  | C fence | 25.2 ms | 1.2 ms |
  | Shell fence | 39.2 ms | 15.7 ms |
  | Rust fence | 40.9 ms | 2.3 ms |
  | Python fence | 53.6 ms | 4.5 ms |
  | Shell heredoc | 8.4 ms | 0.27 ms |

So on `onig`, B2 costs about 25–55 ms of background work per 1,000 fenced lines (about 8 ms per
1,000 heredoc lines), and it grows linearly with the block. A 120-line foreground cap costs about
3–7 ms, and up to about 17 ms in cold bursts (round 2's 0.15 ms-a-line sample).

Workarounds for C10a on `onig`:

1. **Cap the foreground re-parse**, from the edited line to the bottom of the viewport plus
   overscan (120 lines in these measurements; a taller viewport needs more). Then a background
   pass, on the thread that owns the states (B1), continues with plain state equality until the
   state matches or the file ends. On `onig` that means the end of the capture context. This
   bounds the edit's foreground cost, and the result is exact once the background pass
   finishes. Until then:
   - lines past the pass's frontier keep their stale cached colours;
   - a viewport resumed from a cached state below the edit gets stale colours;
   - a second edit restarts the pass from the earliest dirty line.

   So C10a treats every cached state past the frontier as provisional.
2. **Upstream fix** (draft below), after which plain equality converges inside capture contexts
   too.

A third idea was dropped: stop as soon as the `HighlightState` equals the cached one and the next
k lines re-parse to the cached scope ops. Round 2 tested it only on renames (330 per backend,
k = 1 and 2, all correct), and renames don't change a block's terminator. The reviewer's probe
(`/home/wmfeht/dev/strata-spikes/s4-review2/probe-out.txt`, `src/bin/probe.rs`) compared it
against a full re-parse to the end of the file, for edits that do change a terminator:

| Edit (40 sites each) | k = 1 | k = 2 | k = 4 |
| --- | --- | --- | --- |
| `<<EOF` → `<<EOT`, both backends | 40 wrong | 40 wrong | 0 wrong |
| `<<'EOF'` → `<<'EOT'`, both backends | 40 wrong | 0 wrong | 0 wrong |
| ```` ```js ```` → ```` ````js ```` (fence opener), `onig` | 40 wrong | 40 wrong | 38 wrong |
| Unmatched `"` (both backends); change of fence language, deleted fence opener (`onig`) | 0 wrong | 0 wrong | 0 wrong |

It stops too early on ordinary edits, at any k worth having. At most it may serve as a
foreground stop inside the cap, with the background pass of workaround 1 always running plain
equality to the end of the capture context, and then it adds no exactness.

Draft upstream note for rust-onig (not filed):

> **`Region`: `PartialEq` compares pointers, and `Region` is `!Send` though it owns its
> buffers.**
>
> `Region` derives `PartialEq`/`Eq` over `onig_sys::OnigRegion`, whose `beg`, `end` and
> `history_root` are raw pointers, so two regions with the same captures compare unequal, and a
> region never equals its own clone (`Clone` deep-copies through `onig_region_copy`):
>
> ```rust
> let re = onig::Regex::new("(a)(b)?").unwrap();
> let mut region = onig::Region::new();
> re.search_with_param("ab", 0, 2, onig::SearchOptions::SEARCH_OPTION_NONE,
>     Some(&mut region), onig::MatchParam::default()).unwrap();
> assert_eq!(region, region.clone()); // fails with onig 6.5.3
> ```
>
> Proposal: implement `PartialEq` by value (`len()` and `pos(i)` for each group). The same raw
> pointers make `Region` `!Send`, even though each region uniquely owns its allocations, which
> are freed in `Drop`; `unsafe impl Send for Region` looks sound for the same reason.
> Downstream, syntect's `ParseState` (which stores a `Region`) is `!Send` and gets false
> negatives from `==` under `regex-onig`, which breaks the "re-parse until the state matches"
> pattern in editors.

## Decision inputs and results

Setup:
- syntect 5.3.0 (`parsing` plus `regex-fancy` or `regex-onig`, no default features), which
  requires fancy-regex `^0.16.2` (resolved 0.16.2), so the newer fancy-regex 0.20 could not be
  tried; or onig 6.5.3 / onig_sys 69.9.3. two-face 0.5.2+bat-0.26.1, `extra_newlines()`. The
  `fancy` dump has 213 syntaxes, the `onig` dump 220. Theme: two-face's OneHalfDark.
- Rust 1.99.1 and cargo-deny 0.20.2 (the repository's pins). Release profile, `lto = true`,
  `codegen-units = 1`.
- Intel Core Ultra 9 285K, Linux 7.2, governor `powersave`. Benchmarks are single-threaded and
  pinned to P-core 7; builds ran on E-cores with `CARGO_BUILD_JOBS=6`.
- The machine was shared: the 1-minute load average was 6.4–9.0 in round 1, and 5.5–17 in
  round 2 with two spikes (119 and 219). Rows taken during the spikes agree with the calm rows
  within 2 %, so the pinned core was not contended, but read the ratios as the headline.
- JavaScript is forced to the `JavaScript` grammar on both backends. `onig`'s detection picks
  `JavaScript (Babel)`, which `fancy-regex` lacks, and comparing two grammars would measure the
  grammar rather than the backend. Babel's cost on `onig` is listed under c.
- "Viewport" means 80 visible lines plus 40 of overscan. The worst case resumes from a checkpoint
  N − 1 lines above the viewport, so it highlights N − 1 + 120 lines.

Run shape and evidence:

| Section | Shape | Evidence |
| --- | --- | --- |
| a, b cold | 30 fresh processes per backend, alternating | `out/results.txt`, `out/summary.txt` |
| c full parse, d window, f edits | A B A B (two rounds each) | `out/results.txt`, `out/summary.txt` |
| e viewport sweep (N = 1–1024) | A B A B | `out/results2.txt`, `out/summary2.txt` (replaces round 1's single run) |
| e checkpoint memory, pass overhead | Single run | `out/results2.txt` (memory), `out/results.txt` (overhead) |
| B2 equality, edits inside blocks | Single run (equality); A B A B (edits) | `out/b2-selfeq.txt`, `out/results2.txt` |
| g op diff, i long lines | Single run | `out/results.txt`, `out/ops/` |
| h grammars | Single run | `out/evidence-h-grammars.txt` |
| j size, crates, build time | Single run (build time: two rounds) | `out/evidence-j-size.txt` |
| k deny, l defaults | Single run | `out/evidence-k-deny.txt`, `out/evidence-l-defaults.txt` |

Scripts: `build.sh` (all binaries, both backends), `run.sh` (round 1), `run2.sh` (round 2),
`evidence.sh` (h, j, k, l), and `summarize.py` / `summarize2.py` for the tables. Times are in ms
unless marked.

### a, b: cold start

| | fancy median (p95) | onig median (p95) | ratio |
| --- | --- | --- | --- |
| Load `extra_newlines()` | 1.59 (3.00) | 2.08 (3.06) | 0.77× |
| First colours, Rust (load, detect, lines 0–255, regex compiles) | 37.7 (41.9) | 6.9 (8.5) | 5.5× |
| First colours, C | 29.4 (40.1) | 5.9 (10.1) | 5.0× |
| First colours, JavaScript | 101.9 (105.5) | 15.4 (17.7) | 6.6× |
| First colours, Python | 46.8 (48.9) | 8.9 (13.1) | 5.2× |
| First colours, Markdown | 254.1 (260.2) | 14.1 (22.6) | 18.0× |
| First colours, shell | 38.7 (39.0) | 6.6 (8.8) | 5.9× |
| Compile every pattern in the set (h) | 3,437 | 200 | 17× |

### c: full parse, warm (1 warm-up, 10 runs, two rounds; medians)

| Fixture | Parse only, fancy / onig | Parse + highlight, fancy / onig | ratio |
| --- | --- | --- | --- |
| rust-50k | 2.20 s / 0.75 s | 2.30 s / 0.84 s | 2.7× |
| c-50k | 1.49 s / 0.54 s | 1.55 s / 0.59 s | 2.7× |
| js-50k | 2.66 s / 0.85 s | 2.83 s / 0.99 s | 2.9× |
| python-50k | 2.83 s / 1.02 s | 2.92 s / 1.10 s | 2.7× |
| markdown-50k | 29.2 s / 1.28 s | 29.3 s / 1.37 s | 21× |
| shell-5k | 193 / 64 | 204 / 74 | 2.8× |

With `onig`'s own detection, js-50k uses `JavaScript (Babel)`, and parse plus highlight takes
1.27 s, 28 % more than `JavaScript`.

### d: 256-line window resumed from a 256-line checkpoint (200 positions, two rounds)

| Fixture | fancy median (p95) | onig median (p95) | ratio |
| --- | --- | --- | --- |
| rust-50k | 17.9 (23.4) | 6.5 (8.6) | 2.7× |
| c-50k | 12.1 (16.0) | 4.7 (6.2) | 2.6× |
| js-50k | 21.8 (30.3) | 8.0 (10.9) | 2.8× |
| python-50k | 22.0 (29.6) | 8.3 (11.2) | 2.6× |
| markdown-50k | 232 (364) | 11.3 (16.1) | 21× |
| shell-5k | 15.0 (20.8) | 5.6 (7.5) | 2.7× |

On both backends, every window gave exactly the spans of a full parse (0/200 mismatches per
fixture and round). So resuming from a cloned `ParseState` + `HighlightState` is exact.

### e: worst-case viewport by checkpoint interval

p95 / max over 200 positions. Two rounds; the worse round's p95 and the overall max.

| N (lines) | Backend | rust | c | js | python | markdown | shell |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 (120) | onig | 2.4 / 2.6 | 1.7 / 1.8 | 2.8 / 3.9 | 3.1 / 3.3 | 4.8 / 8.4 | 2.2 / 2.3 |
| 1 | fancy | 6.6 / 7.0 | 4.4 / 4.9 | 7.8 / 10.8 | 7.9 / 8.3 | 120.6 / 227.5 | 6.2 / 6.6 |
| 8 (127) | onig | 2.6 / 2.8 | 1.8 / 1.9 | 3.0 / 4.3 | 3.3 / 3.6 | 4.7 / 8.4 | 2.3 / 2.5 |
| 8 | fancy | 7.0 / 7.4 | 4.6 / 4.9 | 8.6 / 12.0 | 8.4 / 9.0 | 120.4 / 225.4 | 6.7 / 7.1 |
| 16 (135) | onig | 2.8 / 2.9 | 1.9 / 2.0 | 3.3 / 6.1 | 3.4 / 3.6 | 5.1 / 7.7 | 2.4 / 2.5 |
| 16 | fancy | 7.4 / 7.9 | 5.0 / 5.4 | 9.5 / 16.6 | 9.0 / 9.6 | 122.0 / 205.4 | 7.0 / 7.5 |
| 32 (151) | onig | 3.0 / 3.1 | 2.1 / 2.3 | 4.4 / 4.9 | 3.8 / 3.9 | 6.0 / 6.6 | 2.7 / 3.0 |
| 32 | fancy | 8.1 / 8.5 | 5.5 / 5.8 | 12.5 / 13.8 | 10.0 / 10.3 | 147.0 / 169.0 | 7.5 / 8.2 |
| 64 (183) | onig | 3.6 / 3.9 | 2.5 / 2.6 | 4.3 / 5.5 | 4.6 / 4.8 | 7.4 / 9.9 | 3.2 / 3.5 |
| 64 | fancy | 9.8 / 10.4 | 6.5 / 6.9 | 12.8 / 15.8 | 11.8 / 12.5 | 194.2 / 471.5 | 9.1 / 9.3 |
| 128 (247) | onig | 4.7 / 5.1 | 3.3 / 3.4 | 6.5 / 8.3 | 6.1 / 6.5 | 9.1 / 13.9 | 4.2 / 4.4 |
| 128 | fancy | 12.7 / 13.5 | 8.6 / 9.0 | 22.5 / 842.1 | 16.3 / 24.6 | 228.4 / 784.4 | 11.8 / 12.6 |
| 256 (375) | onig | 7.0 / 7.2 | 4.9 / 5.2 | 9.1 / 9.4 | 9.2 / 9.4 | 13.2 / 16.5 | 6.4 / 6.5 |
| 256 | fancy | 18.7 / 19.2 | 12.8 / 13.7 | 25.9 / 26.9 | 23.9 / 26.5 | 309.9 / 397.8 | 16.8 / 17.7 |
| 512 (631) | onig | 11.5 / 12.4 | 8.1 / 8.4 | 14.0 / 15.1 | 15.2 / 16.0 | 21.1 / 27.1 | 10.6 / 10.7 |
| 512 | fancy | 31.5 / 33.5 | 21.2 / 21.7 | 41.3 / 67.2 | 39.7 / 41.5 | 498.5 / 1,468 | 27.2 / 27.3 |
| 1024 (1143) | onig | 20.6 / 21.1 | 14.4 / 14.6 | 25.3 / 28.1 | 27.1 / 27.5 | 35.3 / 40.8 | 18.6 / 18.8 |
| 1024 | fancy | 56.1 / 57.3 | 37.6 / 38.1 | 71.7 / 76.5 | 70.7 / 75.8 | 823.7 / 995.4 | 47.5 / 47.6 |

Largest N that meets each budget at p95:

| Budget | onig, all six | onig, five source fixtures | fancy, five source fixtures | fancy, Markdown |
| --- | --- | --- | --- | --- |
| 8 ms | 64 | 128 | 1 | none |
| 16.7 ms | 256 | 512 | 64 | none |

Judged by max instead of p95, `onig` at 8 ms needs N ≤ 32 (Markdown max 9.9 ms at 64), and
`fancy-regex` meets 8 ms at no N (JavaScript max 10.8 ms at N = 1).

Checkpoint memory (deep size of the cloned states, counting allocator, single run; `onig`, with
`fancy-regex` within 20 %):

| N | rust | c | js | python | markdown | shell-5k |
| --- | --- | --- | --- | --- | --- | --- |
| 1 (every line) | 50 MiB | 32 MiB | 63 MiB | 24 MiB | 51 MiB | 3.5 MiB |
| 8 | 6.3 MiB | 4.0 MiB | 7.8 MiB | 3.0 MiB | 6.4 MiB | 452 KiB |
| 16 | 3.1 MiB | 2.0 MiB | 3.9 MiB | 1.5 MiB | 3.2 MiB | 226 KiB |
| 32 | 1.6 MiB | 1011 KiB | 2.0 MiB | 758 KiB | 1.6 MiB | 115 KiB |
| 64 | 792 KiB | 502 KiB | 1001 KiB | 381 KiB | 795 KiB | 59 KiB |
| 256 | 199 KiB | 126 KiB | 254 KiB | 95 KiB | 193 KiB | 14 KiB |

Keeping checkpoints adds 0–3.3 % to a full pass for the source fixtures (Python on `onig` at
N = 64 is the 3.3 %) and 2.8–8.6 % for Markdown, at N = 64–1024 (single run; N < 64 not
measured).

### f: re-highlight after a one-line edit in shell-5k (round 1)

5 edit positions per worst case and 200 renames, in two rounds. With n = 5, "p95" is the max.
"Capped" stops at 120 lines (the rest would run in the background).

| Edit | Per-line cache, until the state matches | Per-line cache, capped at 120 | Checkpoints only (N = 256) |
| --- | --- | --- | --- |
| 200 word renames, onig | 0.025 (p95 0.076) | 0.023 (0.072) | 3.8 (6.3) |
| 200 word renames, fancy | 0.046 (0.51) | 0.042 (0.50) | 10.4 (12.9) |
| Unmatched `"` at 5 lines, onig | 6.0 (max 24.2) | 0.78 (max 0.92) | 8.5 (max 25.0) |
| Unmatched `"`, fancy | 7.3 (max 29.3) | 0.93 (max 1.13) | 12.9 (max 36.2) |
| `cat <<EOF` at 5 lines, onig | 0.13 (max 0.24) | 0.13 (max 0.24) | 3.8 (max 8.7) |
| `cat <<EOF`, fancy | 0.38 (max 2.06) | 0.37 (max 2.02) | 10.9 (max 19.0) |
| `case "$x" in` at 5 lines, onig | 0.24 (max 0.81) | 0.23 (max 0.81) | 6.9 (max 8.6) |
| `case "$x" in`, fancy | 0.29 (max 1.90) | 0.28 (max 1.88) | 11.5 (max 19.6) |

A rename re-parses 1 line (median) and 4 at p95. An unmatched quote re-parses to the end of the
file (up to 4,960 lines), and a heredoc or `case` 12–78 lines. `fancy-regex`'s heredoc max,
2.06 / 2.02 ms in the two rounds, **exceeds** the 2 ms budget. Its cause: syntect compiles a
fresh regex for each back-referencing end pattern (the heredoc's `EOF`), and `fancy-regex`
compiles slowly. Under B2, the `onig` "checkpoints only" column re-parses more than
`fancy-regex` (for `case`, a 512-line median against 256). The edits inside blocks are under B2
above.

### g: correctness diff of scope ops (single run)

Identical on every line of rust, c, js, python and shell. Markdown differs on 5,789 lines, and
all of them lie in or on the fences of its ` ```js ` blocks. The fences embed `source.js`, which
resolves to `JavaScript (Babel)` in the `onig` set and to `JavaScript` in the `fancy` set. The
grammar differs, not the regex engine.

### h: grammar sweep (`Regex::try_compile` over every match pattern)

Evidence: `out/evidence-h-grammars.txt`.

`onig` compiles all 22,257 patterns of the full 220-syntax set. `fancy-regex` fails on patterns
in 7 syntaxes, which is exactly the set two-face leaves out of its `fancy` dump: ARM Assembly,
Hosts File, JavaScript (Babel), PowerShell, Regular Expressions (Elixir), Salt State (SLS) and
VimHelp. The failures are subroutine calls, a variable-length look-behind, an unknown escape,
and pattern parse and NFA build errors.

### Why Markdown is slow on `fancy-regex`

About 55 % of the Markdown fixture's lines lie inside fenced blocks, or 62 % counting the fence
lines. A fence opens a context that holds captures (its backtick run), and while one is on the
stack, syntect builds a new regex for every back-referencing pattern it tries
(`regex_with_refs`, with `can_cache = false`; `parser.rs` `search`). `fancy-regex` compiles
about 17× slower than `onig` (h), so fenced code costs it about 1 ms a line on average, and up
to about 2 ms. Treat the Markdown
numbers as a high-end case for code-heavy Markdown: prose costs far less. The 21× ratio has the
same grammar confound as the fence edits: `onig` highlights ` ```js ` fences with
`JavaScript (Babel)`, which is about 28 % slower than the `JavaScript` grammar `fancy-regex`
uses (c). So if anything the ratio understates the backend gap. C11's rendered
Markdown highlights each code block on its own (with that block's grammar, no fence context), so
this applies to Markdown *source* views.

### i: one very long JavaScript line (warm, 5 runs, single run)

| Line | fancy | onig |
| --- | --- | --- |
| 100 KB array literal | 184 | 64 |
| 400 KB array literal | 537 | 275 |
| 100 KB of joined statements ("minified") | 366 | 181 |
| 400 KB of joined statements | 1,737 | 1,217 |

Cost grows faster than length: 4× the minified text costs 6.7× the time on `onig`.

### j: binary size and build

Evidence: `out/evidence-j-size.txt`, recorded at load 17.

| | fancy | onig |
| --- | --- | --- |
| Size added (`size_probe` − `size_base`) | 3.10 MB, 2.85 MB stripped | 1.79 MB, 1.71 MB stripped |
| Crates, normal dependencies | 29 | 26 (onig and onig_sys are normal dependencies) |
| Crates, build-only | 0 | 4: `cc`, `pkg-config`, `shlex`, `find-msvc-tools` |
| Clean release build of the probe (LTO, 6 jobs, two rounds) | 15.8–16.4 s | 12.8–15.0 s |
| Clean dev build | 4.1 s | 4.2 s |

The probe links two-face's syntax set, parsing and highlighting, with a theme built in code (K1
builds themes, so two-face's theme dump is left out). `onig` comes out 1.1 MB smaller:
`fancy-regex` brings regex-automata and its Unicode tables, while `onig` statically links
Oniguruma, built from bundled C.

### k: cargo deny (the repository's `deny.toml`, graph narrowed to each backend)

Evidence: `out/evidence-k-deny.txt`.

- `cargo deny check licenses`: **ok** for `fancy`, `onig`, and both together.
- `cargo deny check`: bans, licenses and sources ok. Advisories **fail** on both backends with one
  finding: `bincode 1.3.3`, **RUSTSEC-2025-0141** (unmaintained; not a vulnerability). syntect
  (whose `parsing` feature turns on `dump-load`/`dump-create`) and two-face both need it to read
  the syntax dumps, so no feature choice avoids it.
- `onig_sys` declares `MIT`, but it compiles the bundled Oniguruma C sources, which are
  BSD-2-Clause (allowed). cargo-deny sees only the crate's declared licence.
- The yaml-rust advisory is not pulled in: the lines below leave out `yaml-load`, which comes
  with `default-onig`.

### l: syntect's own default syntaxes

Evidence: `out/evidence-l-defaults.txt`. Every one of syntect's 75 default syntaxes is in
two-face's set, by name or scope, on both backends. C10a doesn't need syntect's
`default-syntaxes` feature.

## Options put to the owner

Kept as the record of what the [Decision](#decision) weighed. The owner first chose `onig`,
N = 64, the RUSTSEC-2025-0141 ignore and accepting the WTFPL grammar content, based on the
round-1 record. That record missed B1 and B2, so each point was put again:

1. **Backend.** The budget reading decides rule (1), as shown in the [Summary](#summary).
   - Against `onig`:
     - B1: C10a needs a highlighter thread (a), or an `unsafe` wrapper (b) with a lint
       exception. With (b) the threading design matches `fancy-regex`'s; B2 still applies.
     - B2, which matters only while editing: until rust-onig changes, an edit inside a heredoc
       or fence re-parses to the block's end. In the fixture's short blocks that costs about the
       same as `fancy-regex` (fence-edit medians 1.96 ms against 1.85 ms). In a 1,000-line block
       `onig`'s cost grows linearly: about 25–55 ms of background work per 1,000 fenced lines
       (8 ms for a heredoc), and about 3–7 ms for a 120-line foreground cap (up to about 17 ms
       in cold bursts). `fancy-regex` stops after 1–3 lines.
     - A C library, built with `cc`.
   - Against `fancy-regex`:
     - 2.6–2.9× slower on source and about 21× on fenced Markdown.
     - It meets 8 ms only with per-line state.
     - 7 grammars are missing, including PowerShell.
     - The binary is 1.1 MB bigger, and cold first colours are 5–18× slower.
     - It misses the 2 ms edit budget on one heredoc insertion (2.06 ms max, f).
2. **Budget reading:** 8 ms (the commit budget) or 16.7 ms (one 60 Hz frame on the worker), at
   p95. C10a's doc should state it.
3. **Checkpoint interval N:**

   | Backend | 8 ms | 16.7 ms |
   | --- | --- | --- |
   | `onig` | 64 | 256 |
   | `fancy-regex` | 1, source only (a state per line, 23–60 MiB per 50k-line file); Markdown at no N | 64, source only; Markdown at no N |

4. **If `onig`, B1 option:** (a) a highlighter thread, with no `unsafe`; or (b) `SendState`, with
   an `unsafe_code` exception and a re-check of the soundness argument on each bump.
5. **RUSTSEC-2025-0141 ignore:** needed with either backend.
6. **If `onig`:** link the bundled Oniguruma (the default) or the system one
   (`RUSTONIG_SYSTEM_LIBONIG=1`), which is a Strata packaging call.
7. **Grammar content licences:** these come with either backend, and cargo-deny cannot see them.
   The GraphQL grammar is WTFPL, which the allow list doesn't name; GLSL and PureScript are
   Unlicense; Sublime's default packages use Sublime's permissive licence. None needs a notice.
8. **Notice text:** render the syntax acknowledgements in Strata's credits, or link to two-face's
   URL instead. The link is an owner or legal call (see For C10a).

The follow-up edits are listed as owner items under [Decision](#decision).

## For C10a

**Dependency lines and advisory ignore:** as under [Decision](#decision). Avoid `default-onig`,
which adds html, plist, yaml-load and the default themes. syntect and two-face must use the same
backend, because two-face embeds a dump per backend.

**`onig` build requirement:** `onig_sys` needs a C compiler at build time. It does not need
bindgen or libclang, because syntect turns off onig's `generate` feature.

**Syntax set:** `two_face::syntax::extra_newlines()`. The measurements used the `newlines` set,
so feed lines with their `\n`, and add one to a final line that has none.

**Notice duty:**
- `listing().to_md()` is about 200 KB, not the ~11 KB the round-1 record said: 66 syntax and 12
  theme licences plus bat's curation note. Strata ships no two-face themes (K1 builds them), so
  render only `for_syntaxes()` (each entry's `write_md`, about 186 KB) and keep the bat note.
- Linking to `two_face::acknowledgement::url()` instead is smaller, but whether a link meets the
  MIT/Apache/BSD notice terms is an owner or legal call.
- With `onig`, add Oniguruma's BSD-2-Clause notice (`onig_sys/oniguruma/COPYING`).

**Jump to end:** a warm full parse and highlight of a 50k-line file takes 0.59–1.37 s on `onig`
and 1.55–2.92 s on `fancy-regex`, or 29 s for the Markdown fixture. Checkpoints near the end of
a file exist only once a pass has reached them. So build them in one background pass right after
open, in slices (B1 (a)), serve each viewport from the nearest checkpoint built so far, and draw
lines plain until the pass reaches them.

**Long lines:** skip highlighting any line longer than 16 KiB: draw it plain and carry the parse
state over unchanged. A 100 KB line costs 64–181 ms on `onig`, and cost grows faster than length.
Carrying the state over a skipped line can give the wrong colours to the lines after it, if the
skipped line would have opened or closed a string or comment. So C10a's "a resume from a checkpoint
gives the same spans as a full parse" test must exclude files with skipped lines, or compare against
a full parse that skips the same lines. Rune-kit's built-in highlighter has a similar guard (it
colours the first 32 KiB of a block).

**Edits:** keep a state per line in the editable variant (scripts are small: 3.5 MiB for 5,000
lines). With checkpoints only, a rename costs 4–5 ms p95 even at N = 64. Use B2's workaround 1 on
either backend:
- re-highlight in the foreground from the edited line to the bottom of the viewport plus
  overscan;
- then continue in the background, on the highlighter thread that owns the states (B1), with
  plain state equality until the state matches the cache or the file ends.

Treat cached states past the background pass's frontier as provisional (see B2, workaround 1).

An unmatched quote cascaded to the end of the file in up to 24 ms (`onig`). Don't stop early on
`HighlightState` plus look-ahead ops: it gives wrong colours when a heredoc or fence terminator
changes (B2).

## Not measured

- Real-world corpora. The fixtures are synthetic, generated from templates, so token mix and
  line lengths are plausible but not sampled from real projects. The Markdown fixture's
  fenced-code share is high.
- Checkpoint-pass overhead below N = 64, and intervals between the measured ones.
- K1's real themes. Highlight cost grows with the number of theme selectors; this used
  OneHalfDark.
- B1 option (a)'s queueing under real load. For (b), no test backs the written argument. The real
  gap is a ThreadSanitizer or AddressSanitizer run of a cross-thread move-and-drop test; Miri can't
  run Oniguruma's C.
- B2's cost in real files. The reviewer measured single 1,000-line blocks
  (`/home/wmfeht/dev/strata-spikes/s4-review3/scale-out.txt`): `onig` 25–54 ms p95 per fenced
  block and 8.4 ms per heredoc, against 1.2–15.7 ms and 0.27 ms on `fancy-regex`. Nested capture
  contexts and real files were not measured.
- The resident memory of a loaded syntax set, and contention when several threads share it.
- Linking against a system Oniguruma, other CPUs, and the UI-thread commit budget (C10b's
  `RUNE_COMMIT_BUDGET_MS` runs).
- Behaviour with CRLF input, the `no-newlines` set, and first-line detection.
