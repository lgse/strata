# Drag browser content into Strata

Drag content from a web browser onto a folder row, a directory's empty area, a
parent breadcrumb, a sidebar folder, or a browser tab. Columns, Icons, and List
views share the same handling.

| Content offered by the browser | Saved result |
| --- | --- |
| An existing file or folder | The usual copy/move operation |
| Pixel-only image data | A PNG file, such as `image.png` |
| Selected plain text | A UTF-8 `.txt` file, such as `Dropped Text.txt` |
| Browser-supplied image file contents | The original image format and bytes |
| A URL returning image content | The downloaded image in its original format, including extensionless URLs |
| Another HTTP or HTTPS URL | A `.desktop` web-link shortcut named after the host |

The default image/text names follow Strata's language. Existing names are never
overwritten: a collision creates a numbered name such as `image (1).png`.
Content is published only after its file has been written. Symbolic links and
folders occupying a candidate name are also preserved.

New image, text, and shortcut files can be saved to **local folders**, including
locally mounted storage. Existing file transfers retain their remote-location
support. Trash and Recent are not destinations for new content.

**Open folder after drop** applies to new content as well as file transfers.
When enabled, Strata opens the destination and selects the saved files. When
disabled, it keeps your current location. Navigating away during the save is
respected. Write failures are shown in a dialog.

## Images versus image links

Browsers offer different drag formats. Strata prefers image payloads when they
are available, even if the browser also supplies an image URL. Pixel-only
payloads are saved as PNG; their original encoding and animation are unavailable.
Browser-supplied file contents take precedence over an accompanying URL. This
includes Chromium's named `application/octet-stream` format on Wayland and
unnamed binary contents on X11. Image files retain their original bytes and
format; only pixel-only payloads are encoded as PNG.

If the browser supplies only a URL, Strata checks the HTTP response's content
type in the background, not the URL's filename extension. Image responses are
downloaded and validated before publication. GitHub `blob` resource URLs are
checked through their raw-file endpoints. Non-image responses become shortcuts;
webpage bodies are not downloaded. URLs that cannot be reached, including offline,
can still be saved as shortcuts. Invalid image bodies produce an error.

Requests have time and size limits, do not follow arbitrary redirects, and never
use the browser's cookies or authenticated session.

Selected text is saved literally, without interpreting HTML or commands. A
selection containing only a valid HTTP(S) URL is treated as a web link.

## Video files and links

Existing video files use the usual file copy/move handling. Browser-supplied
binary contents currently support images only, not raw video data. Video URLs and
YouTube page links become shortcuts; Strata does not extract or download streaming
videos from webpage players.

## Opening web-link shortcuts

Activate a saved shortcut in Strata to open its address with your default web
browser. These are Linux desktop entries with `Type=Link` and `URL=`, not
application launchers. They contain no `Exec` command and are not executable.
Strata accepts only HTTP(S) addresses without embedded credentials and rejects
invalid link entries or link entries containing an `Exec` key. No website is
opened merely by dropping its link.

## Regression coverage

`tests/e2e/scenarios/test_browser_content_drops.py` uses an external GTK process
that offers browser-style MIME formats, including image data together with its
URL. Real pointer drags verify saved contents, each destination surface, the
open-folder preference, and default-browser activation without launching a real
browser. Synthetic MIME offers do not establish interoperability with every
browser or its native drag transport. Existing file-transfer, sidebar-reordering,
and tab tests retain their
separate routing and lifecycle coverage.

The adjacent `content_drop` tests cover exact text contents, safe shortcut
creation, naming collisions, symlinks, concurrent publication, and I/O failures.
`services::web_link` and `browser::desktop` tests cover address validation,
command rejection, special-file rejection, and bounded shortcut loading. The
existing drop-open-preference regression in `browser::transfer` covers startup,
live changes across two views, mode rebuilds, and navigation during saving for
new content as well as file transfers.
