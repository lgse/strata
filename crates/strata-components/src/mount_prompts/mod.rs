//! MountPrompts (C26): the dialogs a mount operation asks for, kept apart from GIO so they are
//! testable without it. Strata's replacement for `gtk::MountOperation`'s dialogs.
//!
//! Work package: C26, `docs/work/phase-2-composites/C26-mount-prompts.md`.
//!
//! Three C15 dialogs, each driven by a plain [`MountRequest`] and answered with a plain
//! [`MountResponse`]:
//!
//! - **ask-password** ([`PasswordRequest`]): Username, Domain, the password, "Connect as"
//!   Registered user or Anonymous, and "Password storage" (Don't remember, Until logout,
//!   Forever), each only when the request allows it. [`Secret::Passphrase`] is the variant for
//!   an SSH key's or a LUKS volume's passphrase: "Passphrase required", a Passphrase field, and
//!   the same storage choice.
//! - **ask-question** ([`QuestionRequest`]): the backend's message and its choices. With
//!   `host_key` it is Strata's "Verify server identity" dialog for an SSH host key.
//! - **show-processes** ([`ProcessesRequest`]): the processes keeping a volume busy, each with
//!   an End process button when the host takes [`MountPrompts::on_end_process`], and the
//!   choices. The backend sends it again as the list changes; the prompt updates in place.
//!
//! The host keeps a [`MountPromptState`] (one prompt at a time; queueing is the host's) and
//! builds [`MountPrompts`] over it. The component owns no I/O and knows no GIO.
//!
//! # The GIO binding lives in Strata
//!
//! **Decided by the owner (C26 planning): the `gio::MountOperation` binding lives in Strata,
//! not here.** strata-components depends only on rune, rune-kit and strata-kit: never on
//! strata-platform (`docs/work/README.md`, rule 1) or GIO. The binding is tangled with
//! Strata's policy (credentials typed in a URI answer the first ask unprompted, a second
//! ask-password is a retry, a declined host key changes the error, which questions are trust
//! questions), and `show-processes` needs `unsafe` to read its `GArray` of pids and `/proc` to
//! name them, neither of which belongs here.
//!
//! The binding, on P9's GLib bridge (`strata_platform::glib_bridge`):
//!
//! - **On the GLib thread**, each handler builds the request and awaits
//!   `Asker::ask(request)` in a `spawn_local` future, keeping its `LocalTask`:
//!   `connect_ask_password` (flags to the `needs_*`, `anonymous_allowed` and `saving_allowed`
//!   switches), `connect_local("ask-question", ..)` (gio-rs binds no typed handler; choices
//!   from the `StrV`; none at all: reply `Unhandled`), and `show-processes` (pids from the
//!   `GArray`, named by the host). Each handler first calls `stop_signal_emission_by_name`:
//!   `GMountOperation`'s own handlers run last and reply `Unhandled` from an idle on the
//!   global default main context, which would race the prompt's answer wherever that context
//!   runs.
//! - **The answer:** [`MountResponse::Password`] first clears `anonymous`
//!   (`set_anonymous(false)`: GIO reuses one operation across the retries of a mount, so an
//!   earlier Anonymous answer would still stand, and Strata's `apply_mount_credentials`
//!   always sets it), then sets `username`, `domain`, `password` (each when `Some`) and
//!   `password-save` from [`Remember`] (`Never`, `ForSession`, `Permanently`), and replies
//!   `Handled`; [`MountResponse::Anonymous`] sets `anonymous`; [`MountResponse::Choice`] sets
//!   `choice`; [`MountResponse::Abort`], or the request dropped unanswered (`ask` resolving to
//!   `None`), replies `Aborted`.
//! - **`aborted`** aborts the pending `LocalTask`s: the bridge then marks their requests
//!   withdrawn and wakes the app, whose `poll` calls [`MountPromptState::withdraw`], so the
//!   prompt closes without a reply (GTK's `GtkMountOperation` does the same).
//! - **A newer `show-processes`** replaces the pending one: in the same synchronous handler,
//!   `ask` the new request first, then abort the old task. `is_withdrawn` turns true the
//!   moment the old task is aborted, so the other order lets a `poll` in between close the
//!   prompt and the next one reopen it (focus reset, the dialog animating in again). The host's
//!   `respond(Abort)` on the replaced ticket then goes nowhere: the old future is dropped before
//!   the GLib loop runs it again.
//! - **On Rune's loop**, the app's `poll` first withdraws the prompt if its ticket
//!   `is_withdrawn`, then drains `Requests`, opening each with
//!   `state.open(request.question().clone(), request)` (answering any ticket it replaces with
//!   `respond(MountResponse::Abort)`); `on_answer` calls `request.respond(answer)`. Withdrawal
//!   comes first so that a withdrawal it sees comes with its replacement, which the binding
//!   queued before it, in the same drain, and the prompt updates in place. Draining first
//!   leaves a window: an empty drain, then the binding's ask and abort, then the withdrawal.
//! - **The host's, past the prompt:** the host-key message is shown as given, so taking
//!   credentials out of a URI in it (Strata's `sanitize_failure_message`) is the binding's; and
//!   [`Credentials::password`](Credentials) is a plain `String`, not zeroized on drop, from the
//!   answer on.
//! - `show-unmount-progress` isn't a prompt: it goes to Strata's release overlay through the
//!   bridge's `listen`.
//!
//! `crates/strata-gallery/tests/c26.rs` holds a test-only reference binding that round-trips a
//! fake `MountOperation`'s `ask-password`, `ask-question` and `aborted` through the real bridge
//! into these prompts. `show-processes` can't be emitted from safe Rust (its `GArray`), so its
//! mapping is covered as plain data only.
//!
//! # Keyboard and focus
//!
//! C15's rules hold (Escape and the close button answer [`MountResponse::Abort`]; Enter in a
//! field presses Connect). Past them:
//!
//! - The password prompt opens on Username when it shows empty, else on the secret field, else
//!   on Connect (Strata's `focus_first`). Backdrop clicks are ignored once a field holds text,
//!   a default username included, as in Strata.
//! - Choosing Anonymous disables the fields, the show/hide toggle and Password storage.
//! - A question or a process list ignores backdrop clicks, has **no default button**
//!   (`StrataDialog::default_button(false)`), so Return with nothing focused never accepts a
//!   host key or ends processes, and opens on its first declining choice (the secondary), or
//!   on the close button, which aborts, when no choice declines (Strata's
//!   `focus_button(rejection.unwrap_or(&layout.close))`). Each choice answers with its index
//!   in the request. With two choices they sit as Strata orders them, the declining one
//!   first. With more, the footer reads: the other choices, declining ones first, then the
//!   first declining choice, then the first accepting one, last as the primary; Strata put
//!   every declining choice before every accepting one, which C15's footer (extra actions,
//!   the secondary, the primary) can't. GIO doesn't say which choice declines, so the host
//!   marks it ([`Choice::rejects`]; [`looks_like_rejection`] is Strata's English heuristic).
//! - The show/hide toggle is no Tab stop of its own, as GTK's peek icon isn't.
//!
//! # What assistive technology hears
//!
//! The dialog, named by its title and described by its subtitle; the message; a refusal, as a
//! polite live region (each one a new node, so a second is heard too); Username and Domain as
//! text fields and the secret as a `password-input` whose value is never exposed, each named by
//! its visible label; "Connect as" and "Password storage" as radio groups; the process list as
//! a list of items named by program, described by pid and command line; the buttons, named by
//! their choices.
//!
//! # Coverage
//!
//! What checks each feature-map Behavior bullet. Scripts are `tests/scripts/c26/<name>.json`;
//! "golden" is a scene's accessibility or PNG golden; "unit" is in `mount_prompts/tests.rs`;
//! "test" is in `crates/strata-gallery/tests/c26.rs`. **Partial**: the prompt's half is here,
//! the rest (navigation, error dialogs, retry detection) is Strata's.
//!
//! | Bullet (node: opening words) | Checked by |
//! | --- | --- |
//! | network-locations: When the backend asks for credentials… only the fields the backend requested | unit `a_password_prompt_shows_only_the_fields_and_choices_its_request_allows`; scripts `sign-in-submit`, `sign-in-focus`; goldens `c26-sign-in`, `c26-luks` |
//! | network-locations: An SMB sign-in can show Username, Domain, Password… Password storage | golden `c26-sign-in`; scripts `sign-in-remember`, `sign-in-anonymous-submit` |
//! | network-locations: Choosing Anonymous disables the credential and Password storage fields | script `sign-in-anonymous`; unit `choosing_anonymous_disables_the_fields_and_the_storage_choice` |
//! | network-locations: Wrong credentials reopen the dialog with "Those credentials weren't accepted…" | **partial**: golden `c26-sign-in-retry`, units `the_retry_message_names_only_the_fields_shown`, `each_refusal_is_a_new_node_so_a_second_one_is_heard_too`; that a request is a retry is Strata's |
//! | network-locations: Cancel or Escape on the sign-in dialog returns… | **partial**: scripts `sign-in-cancel`, `sign-in-escape` (Abort); the navigation is Strata's |
//! | network-locations: When GVfs asks Strata for a key passphrase… "Passphrase required"… "Passphrase" | script `key-passphrase` (review scene `c26-key-passphrase`); golden `c26-luks` |
//! | network-locations: An unknown or changed SFTP host key opens a "Verify server identity" dialog… Strata never answers it | script `host-key-accept`; golden `c26-host-key` (no default button) |
//! | network-locations: The host-key dialog warns that a missing fingerprint… | golden `c26-host-key` |
//! | network-locations: The host-key dialog opens with the rejecting choice… focused | script `host-key-decline`; golden `c26-host-key`; unit `with_no_choice_marked_declining_a_question_opens_on_close_and_enter_aborts` (none marked: close) |
//! | network-locations: Declining the host key by its rejecting choice, Escape, or close… | **partial**: scripts `host-key-decline`, `host-key-escape`, `host-key-close`; the rest is Strata's |
//! | network-locations: A connection that fails on host-key verification shows… | **Strata** (its error text) |
//! | network-locations: Trust questions from other schemes and from devices use GTK's native question dialog | **changed** (D5): they use the general question (review scene `c26-question`) |
//! | location-bar: The sign-in dialog's fields are named after their visible labels | goldens `c26-sign-in`, `c26-luks` |
//! | connect-to-server: A sign-in prompt raised while connecting stacks above the dialog | **partial**: [`MountPrompts::stacked`]; the stacking is C15's |
//! | app/dialogs: The mount authentication dialog ignores backdrop clicks once… has text | scripts `sign-in-backdrop` (typed: it stays), `sign-in-backdrop-untyped` (Abort) |
//! | app/dialogs: Enter in a single-line text or password field… | script `sign-in-submit` |
//! | encrypted: Clicking a locked row, its lock button, or Unlock opens the themed "Passphrase required" prompt… | **partial**: script `luks-unlock`, golden `c26-luks`; the row and its menu are C8's and Strata's |
//! | encrypted: Cancel in the prompt closes it with no error… | **partial**: script `luks-cancel` |
//! | encrypted: A rejected passphrase reopens the prompt with a retry message… | **partial**: as the sign-in retry above |
//! | encrypted: A correct passphrase unlocks and mounts the volume… | **partial**: script `luks-unlock` (the answer); the unlock is GIO's |
//! | encrypted: the Unlocking and Locking overlays, Forget saved password | **not applicable**: Strata and C15 |
//! | release: Cancelling GVfs's own busy dialog during Disconnect shows no "Unable to disconnect" dialog | **partial**, **changed** (D5): script `processes-cancel` answers Cancel by its index; the busy dialog is this one now; the error handling is Strata's |
//! | release: A GIO unmount-progress message replaces the overlay body… | **not applicable**: Strata's release overlay |
//! | volumes, release: Cancelling a mount's (a release's) authentication prompt shows no error | **partial**: scripts `sign-in-cancel`, `luks-cancel` |
//!
//! Also: end process and the live list (test `end_process_leaves_the_prompt_open_on_the_shorter_list`,
//! unit `a_process_list_over_a_process_list_updates_in_place`), withdrawal (test
//! `a_withdrawn_request_closes_its_prompt_unanswered`), and the GIO round trip (test
//! `a_mount_operation_round_trips_through_the_glib_bridge`: Anonymous, then credentials with
//! each storage choice on one operation, and GIO's own handlers stopped). The replacement
//! order for a newer `show-processes` is guidance only: no test emits it.
//!
//! # Deviations
//!
//! - **D2** No TCRYPT options (PIM, hidden or system volume), as Strata: GTK's own dialog had
//!   them.
//! - **D3** The password prompt's icon is the lock: strata-kit has no key icon yet (an
//!   `IconName::Key` addition is a K3/W2 follow-up).
//! - **D5** Every question and process list uses these dialogs; Strata left non-SFTP questions
//!   and the busy dialog to GTK's.
//! - **D6** The host marks declining choices.
//! - **D7** Credentials carry only the fields the request asked for; Strata always set the
//!   username.
//! - **D8** Process names come from the host; End process is an intent the host acts on.
//! - **D9** The fields are the core `text_input` in C18's frame, not rune-kit's `TextInput`,
//!   which takes no right-click handler (Rune draft U-C21-2).
//!
//! Rune 2a44156: **partly**. It covers secure single-line fields (role `password-input`,
//! the value never exposed, no copy, cut, undo or input method; `El::revealed` to show it),
//! modal dialogs, a dialog's first focus (`El::autofocus`, through C15) and radio groups. It
//! lacks a rune-kit text field that takes a right-click menu (U-C21-2), and a GLib loop (P9).

mod labels;
mod request;
mod state;
mod view;

#[cfg(test)]
mod tests;

pub use labels::MountLabels;
pub use request::{
    BlockingProcess, Choice, Credentials, MountRequest, MountResponse, PasswordRequest,
    ProcessesRequest, PromptField, QuestionRequest, Remember, Secret, looks_like_rejection,
};
pub use state::{MountPromptState, PasswordForm};
pub use view::{MountPrompts, PASSWORD_WIDTH};
