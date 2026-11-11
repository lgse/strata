# C26 MountPrompts

| | |
| --- | --- |
| Phase / lane | 2 · Chrome |
| Plan | §4 C26, `remote/file-providers/network-locations`, `devices/volumes/encrypted` |
| Depends on | C15, P9 (the request and response path) |
| Unblocks | M4. Strata's replacement for `gtk::MountOperation` (11 files). |
| Owns | `crates/strata-components/src/mount_prompts/`, `scenes/c26.rs` |
| Branch | `feat/<issue>-c26-mount-prompts` |

## Goal

The dialogs that a mount operation asks for, kept apart from GIO, so they are testable
without it.

## Scope

- Three C15 dialogs, each driven by a plain request type and answered with a plain response:
  - **ask-password:** user, domain, password, anonymous login, and the remember choice (only
    the options the request allows);
  - **ask-question:** a message and its choices, which also covers the SSH host key;
  - **show-processes:** the processes blocking an unmount, with their choices.
- A LUKS passphrase variant of ask-password.
- **The GIO binding:** a `gio::MountOperation` whose signals become these requests, through
  P9's request and response path. strata-components owns no I/O, and the binding only
  forwards signals, so it can sit here behind a `gio` cargo feature, or in Strata. Decide
  with the owner, and record the choice in the module docs.
  **Decided by the owner (option B): the binding lives in Strata.** It is tangled with
  Strata's policy, and `show-processes` needs `unsafe` and `/proc`. The module docs
  (`mount_prompts/mod.rs`, "The GIO binding lives in Strata") give the mapping it must
  follow, and a test-only reference binding in `crates/strata-gallery/tests/c26.rs` takes the
  place of the `gio` feature's round trip below.

## Tests

Scripts per dialog: fill in and submit, cancel, and the remember choice. A11y golden: the
password field marked secure, and names. With the `gio` feature: a fake `MountOperation`
signal round-trips through P9 in a headless runtime.

## Done when

The prompts cover sign-in, the host key and LUKS. The binding location is decided: in Strata.
