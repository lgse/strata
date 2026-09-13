# Round 1 review: #537 unlock prompt ownership

- **verdict:** `approve-with-comments`
- **staging_pr:** https://github.com/lgse/strata/pull/935 (draft, left draft)
- **head_sha reviewed:** `f8b050e3d5fd94dfe62e141a42a66d279a1ae199`
- **agent_id:** `bc-9ac1570a-aef6-54c9-891f-07f3ae084792`
- **n:** 537 (`working-docs/537/`)

## Blockers

None.

Competing-prompt ownership was not traced (no `dm_mod`, no Nautilus/Wayland). That is **not** a round-1 blocker. The plan’s investigation step 4 already gates this: record PARTIAL and implement only the code-justified Finding B/D path plus GTK tests; do not claim Wayland verification. Isolated overlay already works; Finding C is correctly left alone.

## Nits / suggestions

- `wait_for_foreign_volume_mount` completes on any `volume-changed`, not only mount-present or job-end. An unrelated changed signal can `StartOwnedMount` while the automounter still holds Unlock (duplicate-prompt residual, or a second Pending wait that then goes Quiet).
- The 8s bound is shorter than a typical passphrase. After timeout the first follow-up starts a Strata-owned mount; if that is also Pending, `already_waited` then Quiet can swallow the click until the user tries again.
- Test 4 drives classification / follow-up enums only. It does not run `wait_for_foreign_volume_mount` (changed / poll / timeout / `get_mount()` appeared). Optional GTK wait test; not required to approve this round.
- `poll_id.remove()` / `timeout_id.remove()` after those sources already fired can log GLib “source ID not found”.
- PR body still says `Closes #537`. Issue acceptance is still the untraced Wayland ownership cases (test-cases 6–9). Isolated Xvfb plus this wait path must not close that gap.
- `working-docs/537/status.md` at the reviewed tip recorded `deff9ede…`, one docs commit behind the branch head.

## Notes

Product change matches gated Finding B/D:

- Pending / Busy / “already unlocking” / “already in progress” wait up to 8s, then navigate, start a Strata-owned `MountOperation`, or stay quiet.
- Late `get_mount()` / `AlreadyMounted` navigate; overlay dismiss stays in `mount_target`.
- Overlay submit/cancel/retry unchanged. `Unhandled` still only when there is no window host.
- Automounter not disabled. Foreign jobs not cancelled. `NotSupported` stays terminal. Passphrase rejects still retry. Volume path only; SFTP/SMB still use `mount_result_is_ok`.

Residual risk for QA/owner: cases 6–9 on a LUKS-capable Wayland host. This round does not claim that.

## CI / process

- GitHub format/lint/test and E2E skipped (draft). Metadata policy failed (agent attribution). William will resign/sign; history was not rewritten.
- `mergeable_state: blocked`. Did not rebase, amend, undraft, merge, squash, or run strata-cleanup.
- Did not re-run full local CI. Code notes already recorded native fmt, clippy `-D warnings`, and `ui::browser::location::tests` **13 passed**.

Did not implement a fix.
