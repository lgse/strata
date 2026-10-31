# Dropping content from other applications

Besides files, Strata accepts content dragged from a web browser or another
application: an image, selected text, or a web address. Drop it on a folder row,
a pane's background, a breadcrumb, a sidebar folder, or a tab, in any view. It
is saved as a new file in that folder:

| Dropped content | Saved as |
| --- | --- |
| Image data, such as an image dragged from Firefox | A PNG, `image.png` |
| An image's own file contents, as Chromium-based browsers offer them | The original bytes, named as the browser names them, with the extension of the image's actual format |
| Selected text | A UTF-8 text file, `dropped text.txt` |
| An HTTP or HTTPS address | A link, `example.com.desktop` |

A file dragged from another file manager is still copied or moved as usual.
When a browser offers an image together with its address, the image wins.
Names in the table follow the interface language. A name already in use gets a
number, such as `image (1).png`: nothing existing is ever replaced, including a
dangling symbolic link, and a file appears only once it is complete. New content
is saved only to local folders, so Trash, Recent, and remote locations do not
accept it. A save that fails, for example in a read-only folder, shows an error.
[Open folder after dropping files](preferences.md#drag-and-drop-destination)
applies to these drops as to file drops.

## Web links

Dropping an address never contacts it: Strata writes a link file holding the
address and nothing is downloaded, even when the address is an image. Use the
browser itself to save what an address points to.

The link is a freedesktop `Type=Link` desktop entry with no `Exec` key, so it can
never run a program, and it is not executable. Opening it in Strata hands the
address to the default web browser. Strata only treats a `.desktop` file this
way when it is such a link to a credential-free HTTP or HTTPS address; any
other desktop entry, including an application launcher, opens as before.

## Testing

`src/services/dropped_content/tests.rs` covers address and name validation,
link round trips, image recognition, and no-clobber saving;
`src/ui/browser/content_drop/tests.rs` covers how dropped values are classified.
The QA scenarios in `tests/qa/scenarios/content_drops/` drag real content from
a separate process, using the `offers` of the `foreign` step, and count the
requests a local HTTP server receives to prove that dropping and opening a link
fetch nothing.
