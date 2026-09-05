# FileChooser rework

Before: [original PR demo](https://github.com/user-attachments/assets/e781721f-4d0e-4800-8a1b-125d1020afbb).

After, captured using disposable sample files on a private D-Bus session:

- `test-page.png`: recreated five-case Chromium test page.
- `browser-results.png`: all five browser cases completed successfully through the portal frontend.
- `open-explorer.png`, `open-multiple.png`: current Explorer chrome, metadata, and multiple selection.
- `preview.png`: native image preview and application-supplied image filter.
- `save.png`, `overwrite.png`: suggested filename, compact options, toolbar New Folder icon, and shared themed overwrite modal.
- `grid.png`: multiple selection across type groups in Grid.
- `focus-chooser-explorer-before.png`: missing keyboard cursor and stale hover treatment after pointer input.
- `focus-main-{columns,grid,explorer}.png`, `focus-chooser-{columns,grid,explorer}.png`: matching shared selection fills and keyboard cursors after switching from pointer to keyboard navigation.
- `focus-main-explorer-light.png`, `focus-chooser-explorer-light.png`: the same comparison in Classic Light.
- `context-menu-before.png`, `context-menu.png`, `context-menu-light.png`: right-click behavior before and after adding the chooser-only Rename/Properties menu.
- `folder-context-menu.png`, `chooser-rename.png`, `chooser-properties.png`: empty-space New Folder, inline rename, and shared themed Properties in the chooser.
- `save-light.png`: Filter, Encoding, and Compress files on one compact row in Classic Light.
- `option-focus-before.png`, `option-focus.png`, `option-focus-light.png`, `filter-focus.png`: keyboard focus before/after removing the extra layout-wrapper ring; the select controls retain their own focus indicators.
- `savefiles-columns.png`: FileChooser v4 SaveFiles with Columns and choices.
- `opt-in.png`, `opt-in-light.png`: the one-time, consent-only offer in Azure Glow and Classic Light, with matching paragraph insets.
- `setup-success.png`, `setup-success-light.png`: the success state replaces the explanation with a theme-colored Lucide circle-check above the result.
- `settings-integration.png`: permanent enable/restore access in Settings → General.
- `settings-restore-before.png`, `settings-restore.png`, `settings-restore-light.png`: the configured integration dialog before and after applying the shared message-width wrapping, including Classic Light.

The opt-in captures use isolated XDG directories and a private bus; service restart/reload commands were stubbed during installation/removal testing, so the installed desktop portal was never replaced.

The browser-page captures use Chromium's File System Access API. The updated chooser captures use the dedicated client, including Classic Light, grouped Grid, and SaveFiles. See [local test instructions](../../portal-file-chooser.md#local-test-tools) to reproduce them without modifying the installed desktop portal.
