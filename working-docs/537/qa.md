# Round 1 QA: #537 unlock prompt ownership

- **verdict:** `pass-with-nits`
- **staging_pr:** https://github.com/lgse/strata/pull/935 (draft, left draft)
- **head_sha tested:** `1e890e061b79f138830d50245516e9e3ba1e3c1d` (PR tip at QA start; product same as `f8b050e`)
- **product_sha:** `6f5c7e79d51bd093f1df3cfa458d075948bf325d`
- **n:** 537 (`working-docs/537/`)
- **agent_id:** `bc-1a9f0ab9-df13-56a0-93a1-c9789e80fb24`
- **round:** 1

Functional/exploratory only. Did not implement, undraft, merge, squash, rebase, amend, force-push, file GitHub issues, or run strata-cleanup. William will resign/sign.

## Product non-nits

None. Every planned case this environment can run passed. Competing-prompt ownership (cases 6–9) is skipped for host reasons, not a product fail.

## Nits / process

Same class as round 1 review (not product breaks):

- `wait_for_foreign_volume_mount` still completes on any `volume-changed`, not only mount-present or job-end.
- 8s timeout then a second Pending → Quiet can swallow a sidebar click.
- Test 4 drives classification / follow-up enums only. It does not run `wait_for_foreign_volume_mount`.
- `poll_id.remove()` / `timeout_id.remove()` after those sources already fired can log GLib “source ID not found”.
- PR body `Closes #537` overclaims; cases 6–9 remain owner/LUKS-capable Wayland work.
- `status.md` at QA start still listed `f8b050e`, one docs commit behind the PR tip `1e890e0`.

## Coverage

| Case | Result |
| --- | --- |
| 1 Password-only overlay submit/cancel | **pass** — GTK `password_only_volume_prompt_submits_and_cancels_the_original_operation`. Password field only; Connect → `Handled` + `PasswordSave::Never`; Cancel → `Aborted`. Private Xvfb screenshot of the themed overlay. |
| 2 Quiet cancel vs terminal errors | **pass** — `Cancelled`/`FailedHandled` quiet; `NotSupported` terminal; LUKS reject substring is auth retry, not in-flight. |
| 3 Pending/Busy not terminal | **pass** — `Pending`/`Busy`/`already unlocking` are in-flight, not auth, not cancelled. `NotSupported` (`Operation not supported`) stays terminal. |
| 4 External mount completion | **pass** (classification) — `AlreadyMounted` / late `get_mount()` → ready; `StillLocked` → `StartOwnedMount` then `Quiet` if already waited; `Gone` → Quiet. Wait helper itself not driven (nit). |
| 5 Overlay `Unhandled` only without a window | **skipped** — Finding C not implemented this round. Adjacent probe (not committed): overlay with no root still replies `Unhandled`. Presented-window overlay still hosts. |
| 6 Desktop-only attach prompt ownership | **skipped** — no Nautilus, no `thunar-volman`, XFCE/X11 not Omarchy/Hyprland. `udisksd` cannot bind the system bus. Not a product fail. |
| 7 Strata click with automounter present | **skipped** — no competing automounter. Did not install Nautilus. |
| 8 Cancel / retry / unlock with automounter absent | **skipped** (live LUKS) — `cryptsetup luksOpen` → `Cannot initialize device-mapper` (`dm_mod` absent; `modprobe` not present). Overlay submit/cancel covered by case 1. Retry-vs-success unlock needs device-mapper. Isolated Xvfb must not close this case. |
| 9 External mount-state while Strata is open | **skipped** — same host gap as 6–7. Classification for navigate / StartOwnedMount / Quiet is case 4. |

## Exploratory / adjacent

- Sidebar listing still does not call `volume.mount()`; only the Devices row click does. Automount GSettings / Nautilus / `thunar-volman` were not touched.
- Volume path still parents `gtk::MountOperation` and stops `ask-password` only. SFTP/SMB still use `mount_result_is_ok` / `Unable to connect`, not the volume in-flight wait.
- Adjacent tests **9 passed**: window media-release / eject, modal host (including `blurred_for` none without a window), volume-query dedupe.
- Two-window: mount still uses `overlay.root()` on the clicked window. Not launched as a second process.
- Preferences: no new setting; Settings pages not opened.
- Disposable LUKS2 128 MiB loop (`fixture-passphrase-537`) was formatted then detached. Never real credentials. `udisksctl` could not see it without `udisksd`.

## Gaps

- No live Devices-row click against a GVfs volume (no udisks volume monitor).
- No live 8s wait / `volume-changed` / poll / timeout of `wait_for_foreign_volume_mount`.
- No Wayland competing-prompt PID trace.
- No unlock-and-navigate on this VM.
- Canonical `./scripts/e2e.sh` not run (wait is local to `location.rs`, not shared VolumeMonitor/sidebar rebuild). `cargo-deny` / `typos` not installed; not treated as a CI pass.

## Commands / results

Private Xvfb; never `DISPLAY=:1`. Parent desktop was `:1`; tests used `xvfb-run -a`.

```bash
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  CC=gcc CXX=/tmp/cxx-nolibcxx/c++ PATH="/tmp/cxx-nolibcxx:$PATH" \
  RUSTFLAGS="-C link-arg=-L/usr/lib/gcc/x86_64-linux-gnu/13" \
  cargo test --all-targets --all-features -- ui::browser::location::tests
```

- `ui::browser::location::tests`: **13 passed**, 0 failed (nonzero filter)
- Adjacent window/modal/volume: **9 passed**, 0 failed
- Host GStreamer `-dev` and `/tmp/cxx-nolibcxx` (`g++`, strip `-stdlib=libc++`) required to link `unrar_sys`

Omitted: container `./scripts/quality.sh`, `./scripts/e2e.sh`.

## Stop condition

Review `approve-with-comments` + QA `pass-with-nits` with **no** product non-nits → no further code round. Cases 6–9 stay owner/QA on a LUKS-capable Wayland host.
