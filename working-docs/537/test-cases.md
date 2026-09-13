# Test cases: #537 unlock prompt ownership

Disposable LUKS only. Fixture passphrase in docs/tests: `fixture-passphrase-537`. Do not collect real credentials or core dumps. Do not format an existing drive.

## Automated (code stage)

### 1. Password-only overlay submits and cancels the original operation

- **Purpose:** Isolated Strata overlay still owns `gio::MountOperation` (no native GTK password dialog).
- **Setup:** Existing GTK test pattern in `src/ui/browser/location/tests.rs`.
- **Steps:** Present overlay + `show_authentication_dialog` with `NEED_PASSWORD` only; Connect with a fixture passphrase; repeat with Cancel.
- **Expected:** Password field visible, no username `gtk::Entry`; Connect → `Handled` + password set + `PasswordSave::Never`; Cancel → `Aborted`. No second toplevel.

Keep `password_only_volume_prompt_submits_and_cancels_the_original_operation` passing; extend only if the overlay API changes.

### 2. Quiet cancel vs terminal volume errors

- **Purpose:** Cancel stays quiet; real failures still surface.
- **Setup:** `mount_failure_message` / `volume_error_is_authentication_failure` unit tests (no GTK required).
- **Steps:** `Cancelled` and `FailedHandled` → no message. `NotSupported` / generic Failed → message. LUKS reject substring → authentication failure (retry), not quiet cancel.
- **Expected:** Existing `volume_cancellation_is_quiet_and_terminal_errors_are_preserved` stays true.

### 3. Pending or busy mount is not a terminal error (if Finding B/D is implemented)

- **Purpose:** A second Strata `volume.mount` while automount is in flight must not show `Unable to mount volume`.
- **Setup:** Unit test the new classification helper with `gio::IOErrorEnum::Pending` / `Busy` and a representative “already unlocking” message. No real udisks.
- **Steps:** Feed those errors through the same path `mount_device_volume` uses to decide dialog vs wait/retry.
- **Expected:** Pending/busy are not auth failures and not shown as terminal mount errors. `NotSupported` (“Operation not supported”) remains terminal.

### 4. External mount completion is success (if Finding B/D is implemented)

- **Purpose:** If `get_mount()` appears (or result is `AlreadyMounted`) while Strata is waiting, navigate and dismiss any overlay.
- **Setup:** GTK test with a stub/callback around the wait helper, or drive `mount_result_is_ok` plus a fake “mount appeared” callback. Do not require `dm_mod`.
- **Steps:** Simulate in-flight wait, then a mount becoming available; also simulate `AlreadyMounted`.
- **Expected:** Treated like a successful Strata mount: no error dialog, prompt dismissed. Foreign cancel (still no mount) proceeds to Strata-owned `mount_device_volume`.

### 5. Overlay `Unhandled` only without a window host (if Finding C is implemented)

- **Purpose:** Sidebar mounts in a presented window must not fall through to a session/native password dialog.
- **Setup:** GTK test: overlay inside a presented window vs a widget with no root window.
- **Steps:** Call `show_authentication_dialog` in both configurations with a `MountOperation`.
- **Expected:** With a window: overlay shown, no `Unhandled`. Without a host: existing `Unhandled` (or the tightened reply the patch documents) so GVfs does not hang.

## Manual (owner / LUKS-capable Wayland host)

These do **not** gate the Cloud Agent CI filter. They are the acceptance the issue named. Isolated Xvfb alone must not be used to declare them passed.

### 6. Desktop-only attach prompt ownership

- **Purpose:** Separate independent automount from Strata.
- **Setup:** Omarchy/Hyprland (or equivalent Wayland + Nautilus/udiskie). Disposable 128 MiB LUKS2+ext4 loop. Strata may be running but do not click Devices yet.
- **Steps:** Attach or relock the fixture. Record PID/process of every unlock dialog. Repeat with Strata not running if needed.
- **Expected:** Any prompt on attach is attributed. If none appear, record that (Cloud XFCE already saw none). Do not disable the automounter to force this case.

### 7. Strata-initiated click with automounter present

- **Purpose:** Strata-initiated mount uses Strata’s overlay and can finish without Nautilus.
- **Setup:** Fixture locked. Automounter still running (do not kill it).
- **Steps:** Click the Devices entry. Record overlay vs any second dialog. Cancel (quiet, prior listing kept). Wrong passphrase → retry in Strata. Correct passphrase → mount + navigate into the filesystem.
- **Expected:** Strata overlay is the one the user completes. No requirement to type into Nautilus. Overlap: wait-or-Strata-owned path from the plan, not a terminal error. Automounter still running afterward.

### 8. Same cancel / retry / unlock with automounter absent

- **Purpose:** Isolated contract still holds (regression vs #302/#493).
- **Setup:** Private HOME/XDG and session bus, or a session without Nautilus/udiskie automount. Same disposable fixture. Do **not** persist a user-wide automount=false as the product fix.
- **Steps:** Repeat Cancel, wrong passphrase, correct unlock + navigation.
- **Expected:** Themed overlay only; quiet cancel; retry; unlock and navigate. Matches the original isolated `PROMPT/CANCEL/RETRY/UNLOCK AND NAVIGATION PASS` log.

### 9. External mount-state while Strata is open

- **Purpose:** Desktop completes or cancels Unlock while Strata is showing Connecting/overlay.
- **Setup:** Finding B/D host. Locked fixture. Start Strata click, then complete or cancel the desktop prompt if one exists (or unlock via a second tool on the same loop).
- **Steps:** Observe Strata overlay/error/navigation.
- **Expected:** Success path navigates and drops the overlay. Foreign cancel does not show a scary error; Strata can then prompt itself. Still no automounter kill-switch.

## Commands

Cloud / isolated GTK (private Xvfb, never `DISPLAY=:1`):

```bash
./scripts/test-headless.py ui::browser::location::tests
```

If sidebar wait/subscribe is added, include that module in the same filter and confirm collection is nonzero.

Pre-push (after code exists): `./scripts/quality.sh fmt` and `./scripts/quality.sh clippy`.

Do not add `./scripts/e2e.sh` LUKS coverage. Full e2e only if the change becomes cross-cutting VolumeMonitor/sidebar infrastructure.

## Intentionally omitted

- Real device-mapper Unlock on this Cloud VM.
- Installing Nautilus on the Cloud XFCE session as a fake competing-prompt host.
- Canonical container E2E for encrypted volumes.
