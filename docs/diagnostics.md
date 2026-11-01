# Diagnostics

**Settings → Diagnostics** checks the runtime components Strata relies on but
does not link against, so users can find why previews, locations, devices, or
desktop integration fail. Nothing runs until the user selects **Run**, and no
check changes or installs anything.

## What a run does

Where possible a check exercises the feature itself rather than looking for a
binary:

- The **preview sandbox** renders a generated PNG through a one-shot bubblewrap
  sandbox (`sandbox::parse`), which exercises bubblewrap, unprivileged user
  namespaces, `prlimit`, and the preview helper. **Sandboxed workers** render it
  again through the pooled browser workers, unless Landlock ABI 3 is missing.
- **PDF thumbnails**, **video thumbnails** (a one-second Y4M clip), and **video
  and audio previews** (FFprobe metadata for a short WAV file) go through the
  same `sandbox::browser` paths as the file list.
- Helper tools such as `ffmpeg`, `ffprobe`, `ffmpegthumbnailer`, the RAW
  converters, and `unsquashfs` are looked up with `sandbox::sandbox_tool`, which
  searches the configured `STRATA_SANDBOX_PATH`, not the user's `PATH`, because
  helpers run inside the sandbox.
- **Audio playback** looks up the GStreamer elements the preview audio pipeline
  links and the highest-ranked audio sink.
- **GVfs** checks that GIO's default VFS is GVfs, and each backend looks for
  its URI scheme among the schemes GVfs supports. `GIO_USE_VFS=local` means
  the startup probe found GVfs unresponsive and restarted Strata without it.
- **Removable drives**, the **desktop portal**, and **password storage** look
  for owned or activatable names on the session and system D-Bus buses.
- The **update server** check requests `https://api.github.com/rate_limit`,
  which GitHub does not count against the API rate limit.
- Syntax highlighting, folder change monitoring, the terminal, and custom
  actions run on the GTK thread; everything else runs on worker threads and
  streams results to the page as they finish.

Libraries Strata links against, such as GTK, Poppler, and GStreamer core,
cannot be diagnosed in-app: the dynamic loader fails before Strata starts.

Each result is passed, warning, or problem. A failed check names the package
that fixes it, with an install command for the distribution detected from
`os-release` (`services::package_manager`), or the Arch package name as a
reference when no command is known. The command has a play button that runs it
in the user's terminal, passing each word as an argument rather than through a
shell, and keeps the window open until Enter is pressed; without a terminal
emulator, the button reports that none was found. **Copy report** copies the results with the
version, installation method, toolkit, kernel, desktop, display server, and
renderer. It names custom actions that cannot run, so users should review it
before posting.

## Adding a check

1. Add a `CheckId` and place it in `services::diagnostics::GROUPS`; a check that
   is not in a group never gets a row and is reported as not run.
2. Probe in `preview_checks`, `system_checks`, or `network_checks`, or in
   `main_thread_checks` in `ui/settings/diagnostics.rs` when the probe touches
   GTK objects. Keep the decision in a pure function that takes the probe's
   result, and cover it with a table test in `services/diagnostics/tests.rs`.
3. Return language-neutral data: a `Finding`, the `packages` that fix it, and
   untranslated library text in `detail`. Add the title and message in
   `ui/settings/diagnostics.rs` and translate them in every catalog (see
   [Internationalization](internationalization.md)).

Tests inject the background checks through `diagnostics_page_with`, so the
page's tests never start sandboxes or contact the network. Probes that need the
real Strata executable, such as the sandbox checks, fail inside the test binary
because the sandbox re-executes the current executable as its helper.
