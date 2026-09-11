# Issue #808 review evidence

[After cancellation and a later extraction](after.png) was captured by
`test_cancelled_extract_to_does_not_hijack_later_extract_here` in the canonical
Podman E2E environment (GTK 4.14.5, private Xvfb and D-Bus).

The test cancels the encrypted archive's “Extract to…” password prompt, then
extracts `later.zip` using “Extract here”. It verifies that `leftover/` does not
exist, `later.txt` has the expected contents, and the browser stays at the origin.
The screenshot shows the resulting selected `later.txt`. This is automated GUI
exercise, not an owner-desktop capture or a separate manual test.
