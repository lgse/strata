# 112. Comment on #917: the `tiny-skia-path` fork breaks `cargo vendor` too

| | |
| --- | --- |
| Kind | Comment on [#917](https://github.com/anomalyco/rune/issues/917) ("tiny-skia-path fork collides with the registry package in Nix consumers", filed 2026-11-13 by another consumer). The draft started as a new issue in lgse/strata#2699, and became a comment on #917 when the coordinator found that issue had the same cause |
| Drafts | lgse/strata#2698, the RPM workflow's vendoring failure on `strata-next` |
| Affects | The RPM package (`scripts/build_srpm.py`, `packaging/rpm/strata.spec.in`, `.github/workflows/rpm.yml`). `cargo vendor` rejects `strata-next`'s lockfile because it has `tiny-skia-path 0.11.4` from two sources: Rune's fork, a path dependency of `rune-text` that reaches Strata as a git source, and the crates.io copy that `rune-image`'s resvg 0.45.1 and `sctk-adwaita`'s tiny-skia 0.11.4 take. Any offline vendored build of the Rune binary will hit the same error |
| Needs | The fork under its own package identity, so both copies vendor side by side and resvg keeps the registry API. The tested change: package name `rune-tiny-skia-path`, lib name `tiny_skia_path`, and `rune-text` depending on it with `package = "rune-tiny-skia-path"`. Strata can't do this itself: a consumer `[patch.crates-io]` would give resvg the fork, whose `PathSegment::ConicTo` breaks tiny-skia's and usvg's matches (#917) |
| Retires | `rpm.yml`'s `branches-ignore: [strata-next]`. After the fix, the RPM repair in [`workspace-migration.md`](../../workspace-migration.md) still needs a source replacement for Rune's git source in `build_srpm.py` and the spec |
| Checked | At `7a707454` (2026-11-21): an empty binary that depends on Rune the way "Depending on Rune" says fails `cargo vendor` with `found duplicate version of package tiny-skia-path v0.11.4 vendored from two sources`. In a scratch clone of Rune with the rename above, the same binary's `cargo vendor` succeeds and its `cargo check` passes |
| Status | Posted on 2026-11-21 by the coordinator, as [a comment on #917](https://github.com/anomalyco/rune/issues/917#issuecomment-7065687540). #917 is open. The text below is the comment as posted, fetched from the API on 2026-11-21. The one change: the two `source` lines of cargo's error, indented in the comment with two spaces and a tab, are indented here with four spaces, which `git diff --check` accepts |

Paste from here.

---

The same collision breaks `cargo vendor` for any application that depends on Rune by git, so offline distro builds (RPM `%cargo_prep`, Debian, Flatpak's cargo generator) can't vendor a Rune app.

At `7a707454`, `rune-text` takes `vendor/tiny-skia-path` as a path dependency under the crates.io name and version, while `rune-image` (resvg 0.45.1 → usvg) and the patched `sctk-adwaita` (tiny-skia 0.11.4) take `tiny-skia-path 0.11.4` from crates.io. In Rune's own workspace the fork is a path source, which `cargo vendor` skips; in a git dependent it's a git source, and `cargo vendor` refuses.

**Repro:** `cargo new repro`, then add `rune = { git = "https://github.com/anomalyco/rune", rev = "7a707454fbfb8d3fd200723c65778fb7b98fd31e" }` and the `[patch.crates-io]` entries from "Depending on Rune" in `crates/rune/README.md`. Then run `cargo generate-lockfile && cargo vendor vendor`:

```
error: failed to sync
  found duplicate version of package `tiny-skia-path v0.11.4` vendored from two sources:
    source 1: registry `crates-io`
    source 2: https://github.com/anomalyco/rune?rev=7a707454…#7a707454
```

`cargo-vendor-filterer` runs a full `cargo vendor` first, so filtering doesn't help.

**A fix that worked locally, as the issue suggests (a distinct package identity):** give the fork its own package name, keep its lib name, and depend on it under the old name. resvg keeps its own copy, as intended:

```toml
# vendor/tiny-skia-path/Cargo.toml
[package]
name = "rune-tiny-skia-path"
[lib]
name = "tiny_skia_path"

# crates/rune-text/Cargo.toml
tiny-skia-path = { package = "rune-tiny-skia-path", path = "../../vendor/tiny-skia-path" }
```

In a scratch clone with this change, the repro's `cargo vendor` succeeds and a downstream `cargo check` passes. `scripts/vendor-tiny-skia-path.sh` would need to keep the rename when it refreshes the fork.
