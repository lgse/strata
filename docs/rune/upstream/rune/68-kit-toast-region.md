# 68. rune-kit `ToastRegion` and `Toast` are fixed

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C16-1 |
| Affects | C16 (progress dock, built on Rune primitives instead) |
| Retires | C16's own dock |
| Checked | Re-checked at `2a44156` by source. `ToastRegion::build` still keys itself `toast-region`, names itself "Notifications" and pins 32 and 48 pt in (`rune-kit/src/toast.rs:128–131`), with no height cap or scroll. `Toast` is still 320 pt wide, with 12 pt padding and an 8 pt radius (`toast.rs:98–101`). The new `ToastStack` (`1633f8d6`, #454; timers held while hidden, `6bf91246`, #497) is a state machine that the app renders where `ToastStack::place` says (`toast_stack.rs:16`). So the app owns the container's key, name and insets there, but the stack shows at most three toasts (`VISIBLE`, `toast_stack.rs:31`), collapsed or spread 12 pt apart, with no list that scrolls, and `Toast`'s width doesn't change. Gallery: C16's own dock stands (`progress_dock/mod.rs:314`); `progress_dock::` and `c16::` pass. Earlier: by source at `5d421fd` and `04b38ae` (`toast.rs:128–130`). |
| Status | Ready to file, trimmed: dropped "(or the same on the new `ToastStack`)". `ToastStack` leaves the container to the app, so a key, name and insets there are the app's own; the body now says what it lacks for a dock (more than three toasts, a list that scrolls). |

Paste from here.

---

**Title:** rune-kit `ToastRegion` and `Toast`: no key, name, insets, height cap or width

At `2a44156`, `ToastRegion` keys itself `toast-region`, names itself "Notifications"
(`rune-kit/src/toast.rs:128–131`), pins 32 and 48 points in, and has no height cap or scroll;
`Toast` is 320 points wide with a fixed radius and padding. Strata's (lgse/strata, a file manager on
Rune) progress dock (several long-running copies, each a card) needs its own name, insets and a list
that scrolls. The new `ToastStack` (#454) leaves the container to the app, but shows at most three
toasts (`VISIBLE`) and has no list that scrolls.

**Ask.** A key, a name, insets, a maximum height that scrolls, and the width from the caller.
