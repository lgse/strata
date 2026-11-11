# C25 ThumbnailStore

| | |
| --- | --- |
| Phase / lane | 1 · Collection |
| Plan | §4 C25, `browser/thumbnails/*` |
| Depends on | W1 |
| Unblocks | C1e (thumbnail slot), C3 |
| Owns | `crates/strata-components/src/thumbnails/`, `scenes/c25.rs` |
| Branch | `feat/<issue>-c25-thumbnail-store` |

## Goal

Hold thumbnails as `rune::Pixels` handles under a byte budget, and say which ones to load
next. Loading and decoding stay in Strata.

## Scope

- Map an `EntryId` and size step to a `Pixels` handle. Insert RGBA or replace it in place,
  with no view commit, as `Pixels` allows.
- An LRU byte budget that evicts the least recently drawn handles. The rows on screen are
  never evicted.
- Priority: take the mounted range from virtual-row layout reports, and the view's scroll
  direction. Produce a request queue with the visible rows first, then the overscan in the
  scroll direction.
- A "requested, failed, ready" state per entry, so a view can draw the placeholder icon and
  not ask again.
- The gallery generates thumbnails in a fake worker with a delay.

## Tests

- Unit: the eviction order, the budget never exceeded, visible rows never evicted, and the
  queue order after a scroll in each direction.
- A headless script: scrolling the gallery list fills tiles without a commit per thumbnail.
  Check it with `RUNE_COMMIT_BUDGET_MS`.

## Done when

The tests pass, and C3's owner reviews the API.
