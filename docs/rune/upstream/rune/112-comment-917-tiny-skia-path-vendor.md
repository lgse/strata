# 112. Comment on #917: the `tiny-skia-path` fork breaks `cargo vendor` too

| | |
| --- | --- |
| Kind | Comment on [#917](https://github.com/anomalyco/rune/issues/917) ("tiny-skia-path fork collides with the registry package in Nix consumers", filed 2026-11-13 by another consumer). The draft started as a new issue in lgse/strata#2699, and became a comment on #917 when the coordinator found that issue had the same cause |
| Drafts | lgse/strata#2698, the RPM workflow's vendoring failure on `strata-next` |
| Affects | The RPM package (`scripts/build_srpm.py`, `packaging/rpm/strata.spec.in`, `.github/workflows/rpm.yml`). `cargo vendor` rejects `strata-next`'s lockfile because it has `tiny-skia-path 0.11.4` from two sources: Rune's fork, a path dependency of `rune-text` that reaches Strata as a git source, and the crates.io copy that `rune-image`'s resvg 0.45.1 and `sctk-adwaita`'s tiny-skia 0.11.4 take. Any offline vendored build of the Rune binary will hit the same error |
| Needs | The fork under its own package identity, so both copies vendor side by side and resvg keeps the registry API. The tested change: package name `rune-tiny-skia-path`, lib name `tiny_skia_path`, and `rune-text` depending on it with `package = "rune-tiny-skia-path"`. Strata can't do this itself: a consumer `[patch.crates-io]` would give resvg the fork, whose `PathSegment::ConicTo` breaks tiny-skia's and usvg's matches (#917) |
| Retires | `rpm.yml`'s `branches-ignore: [strata-next]`. After the fix, the RPM repair in [`workspace-migration.md`](../../workspace-migration.md) still needs a source replacement for Rune's git source in `build_srpm.py` and the spec |
| Checked | At `7a707454` (2026-11-21): an empty binary that depends on Rune the way "Depending on Rune" says fails `cargo vendor` with `found duplicate version of package tiny-skia-path v0.11.4 vendored from two sources`. In a scratch clone of Rune with the rename above, the same binary's `cargo vendor` succeeds and its `cargo check` passes |
| Status | Posted on 2026-11-21 by the coordinator, as [a comment on #917](https://github.com/anomalyco/rune/issues/917#issuecomment-7065687540). #917 is open |
