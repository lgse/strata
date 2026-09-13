# Plan: verify unlock prompt ownership alongside desktop automounters

Issue: [#537](https://github.com/lgse/strata/issues/537)
Labels: `bug`, `UX`, `P2` (unchanged)
Assignee: `wmfeht`
No linked PR.

## Goal

A **Strata-initiated** encrypted-volume mount (Devices sidebar click) must present Strata’s themed overlay and finish unlock, mount, and navigation without requiring Nautilus or another automounter. Independent desktop attach-time prompts may still exist. Identify which prompts the desktop starts on its own versus any duplicate Strata causes, then apply the **smallest** matching product change. Do not globally disable the user’s automounter.

## Research

### Reported gap

Follow-up to [#302](https://github.com/lgse/strata/issues/302) / merged [#493](https://github.com/lgse/strata/pull/493). Isolated X11 verification of a disposable LUKS2 loop already passed (`PROMPT FOUND` / `CANCEL PASS` / `RETRY PASS` / `UNLOCK AND NAVIGATION PASS`). On the reporter’s Omarchy/Hyprland Wayland session, unlock popups on the **active desktop** looked like Nautilus. Process ownership and a simultaneous-prompt race were never traced. Nautilus attribution is an observation, not a confirmed root cause.

[#233](https://github.com/lgse/strata/issues/233) only shares authentication for SFTP; it does not cover this volume path.

### Cloud Agent pass (2026-09-10, comment on the issue)

PARTIAL on `main` @ `7fd0e64` (Strata 0.14.0). XFCE/X11, no Nautilus, no `thunar-volman`, **no device-mapper** (`cryptsetup luksOpen` → `Cannot initialize device-mapper`). Isolated Strata still shows the in-window themed overlay (`WM_CLASS=strata`); Cancel stays quiet. Wrong/correct passphrase both fail with `Failed to activate device: Operation not supported` (missing `dm_mod`), so retry-vs-success unlock cannot be shown here. Attaching the loop to the live XFCE session raised **no** desktop unlock dialog. That pass must not close #537.

This Cloud VM still cannot prove competing-prompt ownership. Do not install Nautilus on XFCE as a substitute for the reporter’s Wayland stack, and do not treat isolated Xvfb success as desktop verification.

### Isolated contract that already works (keep)

Sidebar click on an unmounted `gio::Volume` goes `SidebarState::append_volume` → `BrowserView::mount_volume` → `ViewState::mount_device_volume` → `mount_target` (`src/ui/browser/location.rs`).

- `gtk::MountOperation` is parented to the window so GTK can still handle `ask-question` (host-key / cert).
- `ask-password` stops the default emission and shows Strata’s overlay (`Authentication required` / password-only when flags say so).
- Cancel / Escape / close reply `Aborted`; `mount_failure_message` maps `Cancelled` and `FailedHandled` to no error dialog.
- Rejected LUKS passphrase (`no key available with this passphrase`, etc.) retries via `show_mount_retry_prompt`.
- Success or `AlreadyMounted` navigates to `mount.root()`.

GTK coverage lives in `src/ui/browser/location/tests.rs` (`password_only_volume_prompt_submits_and_cancels_the_original_operation`, quiet-cancel / passphrase-failure classification). There is **no** E2E LUKS scenario.

### What Strata does not do today

- Listing Devices (`VolumeMonitor::volumes()`, `volume.name()`, `get_mount()`, eject capability) does not call `volume.mount()`.
- Startup `gio::VolumeMonitor::get()` (`src/main.rs` GVfs probe, sidebar construction) does not automount.
- Sidebar rebuilds on `volume-added` / `volume-changed` / `mount-added` only refresh the list; they do not start a mount.

So an attach-time Nautilus (or udiskie / gnome-shell / gvfs automount) prompt is **expected desktop policy**, independent of Strata, unless a Strata click **also** produces a second prompt or Strata never gets `ask-password` and falls through to a session helper.

### Likely mechanisms (confirm before coding a specific one)

1. **Independent automount (Finding A).** `volume-added` → Nautilus/`should_automount` / udiskie Unlock. Strata overlay still appears on sidebar click and can finish without the other app. Not a Strata defect. Do not kill automount GSettings or the automounter process.
2. **In-flight foreign mount (Finding B).** Automount already called `g_volume_mount`. Strata’s second `volume.mount_future` either shares that job (user must finish the Nautilus prompt), starts a second Unlock (duplicate dialogs), or returns pending/busy which Strata currently treats as a terminal `Unable to mount volume` (only `Cancelled` / `FailedHandled` are quiet; `AlreadyMounted` is success). Smallest fix: wait for `volume-changed` / `get_mount()`, then navigate; if still locked after the foreign attempt ends, start Strata’s own mount with Strata’s `MountOperation`.
3. **Strata `ask-password` never fires / Unhandled (Finding C).** `show_authentication_dialog` replies `Unhandled` when `ModalHost::blurred_for` fails (`src/ui/browser/location.rs`). GVfs/GTK may then show a native/session dialog that looks like Nautilus. Isolated X11 has a presented window + overlay, so this is a Wayland/parenting hypothesis. Fix: do not reply `Unhandled` for a sidebar-initiated mount when the window exists; keep `stop_signal_emission` so GTK’s native password dialog does not appear beside the overlay.
4. **External unlock while Strata is prompting (Finding D).** Nautilus completes Unlock; Strata’s in-flight `mount_future` should complete as success/`AlreadyMounted` and already dismisses `active_prompt`. If it instead errors, classify that as non-terminal and navigate from `volume.get_mount()`.

### Constraints

- Disposable LUKS fixtures only. Passphrase in tests/docs: a published fixture string (existing comment used `fixture-passphrase-537`). Never real credentials, never core dumps, never format an existing drive.
- GTK 4 baseline; private Xvfb for GTK tests (`STRATA_REQUIRE_GTK_TESTS=1`). Never `DISPLAY=:1`.
- Tests: module tests in `src/ui/browser/location/tests.rs` (and sidebar tests only if the wait/subscribe lives there). No inline `#[cfg(test)]` in production functions.
- No preference for “disable automount”. No new icons or theme tokens.
- Do not log passphrases. If adding `tracing` around mount start/result/`ask-password`, log volume name/device id and reply kind only.

## Approach

Investigation first, then one of the gated changes below. Do not implement all of them.

### Investigation (code stage, before a product patch)

On a host that can actually Unlock LUKS **and** has an automounter (reporter-class: Wayland + Nautilus or equivalent). Disposable 128 MiB LUKS2+ext4 loop only.

1. Attach/relock with Strata **not** focused on Devices; record every unlock dialog’s PID / process name (`_NET_WM_PID` / Wayland equivalent). That is the desktop-only baseline.
2. Locked fixture, click the Devices row. Record every dialog owner, whether they overlap, and whether Strata’s overlay appears.
3. Cancel, wrong passphrase + retry, correct passphrase + navigate. Repeat with the automounter **absent** (isolated session / private HOME, not by globally turning off the user’s automount setting as the product workaround).
4. If the Cloud Agent still lacks `dm_mod` and Nautilus/Wayland, record PARTIAL and implement only the code-justified defensive path in Finding B/D plus GTK tests. Do not claim Wayland competing-prompt verification.

### Product change (pick the matching smallest one)

| Finding | Change | Non-change |
| --- | --- | --- |
| A only | No mount-policy change. Optional: a short comment in the mount path that sidebar listing does not automount. Close or convert only with traced evidence; do not invent a workaround. | Do not disable automount. |
| B | Before or during `mount_device_volume`, if mount is pending/busy or `get_mount()` appears from outside, wait and navigate; if still locked, Strata mounts with its `MountOperation`. | Do not cancel another app’s mount job. Do not skip passing `MountOperation`. |
| C | Keep overlay ownership: `ask-password` always stopped; `Unhandled` only when there is genuinely no window. | Do not switch sidebar mounts to a bare `gio::MountOperation` (loses `ask-question`). |
| D | Treat external `AlreadyMounted` / late `get_mount()` as success: dismiss overlay, navigate, no error dialog. | Do not unmount the foreign mount to “take over”. |

Shared implementation notes if B or D:

- Reuse existing `mount_result_is_ok` (`AlreadyMounted`) and quiet-cancel classification.
- Add pending/busy (and message matches such as “already unlocking”) next to that classification; they are not auth failures and must not open `Unable to mount volume` while the desktop prompt is still up.
- Subscribe with the existing sidebar `VolumeMonitor` (or a one-shot handler on the same `gio::Volume`) rather than a new global daemon.
- Keep retry/overlay behavior for **Strata-owned** `ask-password` unchanged.

### Validation scope (code stage)

Bounded to volume mount auth + Devices click. Targeted GTK tests in `location/tests.rs` (nonzero filter). Do **not** add a canonical E2E LUKS job: the pinned e2e image and this Cloud VM cannot Unlock. Manual Wayland cases stay in `test-cases.md` for owner/QA.

Pre-push: `./scripts/quality.sh fmt` and `clippy` on the final code; `./scripts/test-headless.py` with a filter that includes the new/changed location tests. Full `quality.sh` / `e2e.sh` only if the wait/subscribe grows into shared `VolumeMonitor` / sidebar rebuild behavior.

## Risks

- Waiting forever on a foreign mount that never completes. Bound the wait or fall through to Strata’s own `mount_future` on timeout/error; never hang the sidebar.
- Treating a real Unlock failure (`Operation not supported`, I/O error) as pending. Keep `dm_mod` / `NotSupported` as terminal errors (existing `Unable to mount volume` path).
- `Unhandled` is the escape hatch when the overlay cannot be shown (no window). Tightening it must not deadlock GVfs waiting for a reply.
- Two windows: mount must stay on the window whose sidebar was clicked (existing `overlay.root()`).
- Passphrase retry currently matches message substrings. Do not fold pending/busy into `volume_error_is_authentication_failure`.

## Non-goals

- Globally disabling or killing Nautilus, udiskie, gvfs automount, or `org.gnome.desktop.media-handling`.
- Changing SFTP/SMB authentication (#233) except where they share `mount_target` helpers that the volume path must keep working.
- Making Strata the session secret agent / polkit agent.
- Real LUKS E2E in `./scripts/e2e.sh`.
- Formatting or unlocking the user’s real disks.
- Theme/icon work; the overlay already exists.

## Recommended verification (code stage)

See `test-cases.md`. Isolated GTK + classification tests are the CI gate. Competing-prompt ownership is a manual Wayland case and remains unverified until a LUKS-capable automounter host runs it.
