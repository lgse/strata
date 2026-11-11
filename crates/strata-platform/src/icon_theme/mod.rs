//! Freedesktop icon theme lookup, delivered as `rune::Pixels`: application and MIME icons by
//! name, at a size and scale, the way GTK 4's `GtkIconTheme` finds them.
//!
//! Rune lacks, at the pinned rev (`2a44156`):
//!
//! - **The user's icon theme.** `ImageCache::app_icon` on Linux resolves a `.desktop` entry's
//!   `Icon=` from hicolor only: no current theme and no `Inherits`. It reads sizes from
//!   directory names, not from `index.theme` (`Size`, `Type`, `Threshold`, `MinSize`,
//!   `MaxSize`, `Scale`), and lists every size directory again on each call.
//! - **Lookups by icon name**, so MIME icons and a GIO icon's names, with fallback names.
//! - **"Not found" apart from a broken file.** `app_icon` fails both with `Image::error`.
//!
//! **Rune 2a44156: partly.** `ImageCache` covers the rest: decoding PNG and rasterizing SVG
//! with resvg on its worker threads, mips, a decoder panic failing one image, sharing handles and
//! waking the app. This module looks the file up and hands it to `ImageCache::file`. Rune's GIF,
//! WebP, BMP, ICO and AVIF decoding (`d7946ee3`) adds nothing for icon themes, which use PNG,
//! SVG and XPM (no XPM decoder yet), and nothing in Rune looks up an icon theme.
//!
//! Rune issue: upstream [#21](https://github.com/anomalyco/rune/issues/21) (open; PR #29, a
//! theme-chain lookup, was closed unmerged). Drafts, not filed:
//! [U-P6-1](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u-p6-1)
//! (a comment on #21 with what the pin lacks) and
//! [U-P6-2](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u-p6-2)
//! (drawing `Pixels` in the element's colour, for symbolic icons).
//!
//! Work package: P6, `docs/work/phase-1-leaves/P6-icon-theme.md`.
//!
//! # The shape
//!
//! [`IconLoader::start`] runs one thread ([`THREAD_NAME`]) that owns an [`IconResolver`].
//! [`IconLoader::load`] never blocks: it returns a [`LoadedIcon`] at once, shared by every
//! request for the same icon. The thread resolves it, calls the waker, and hands the file to
//! the `ImageCache`, which wakes the app again when the pixels land. [`IconLoader::poll`] says
//! whether to run the view again:
//!
//! ```ignore
//! struct State { icons: Option<IconLoader>, /* … */ }
//!
//! fn setup(s: &mut State, waker: rune::Waker) {
//!     s.icons = IconLoader::start(waker, Options::default()).ok();
//!     // The theme name comes from the app: its settings, or the desktop's.
//!     if let Some(icons) = &s.icons { icons.set_theme("Adwaita") }
//! }
//!
//! fn poll(s: &mut State, _: f64, _: &mut rune::Tx) -> bool {
//!     s.icons.as_ref().is_some_and(IconLoader::poll)
//! }
//!
//! // In the view: `load` hands back the same handle each run (the recent icons keep it).
//! fn app_icon(s: &State, icon_value: &str, scale: f32) -> El {
//!     let icon = IconRequest::from_desktop_icon(icon_value, 22, scale)
//!         .zip(s.icons.as_ref())
//!         .map(|(request, icons)| icons.load(&request));
//!     let failed = |i: &LoadedIcon| matches!(i.state(), IconState::NotFound | IconState::Failed(_));
//!     match icon.as_ref().filter(|i| !failed(i)).and_then(LoadedIcon::pixels) {
//!         Some(pixels) => el().size(22.0, 22.0).pixels_fit(pixels, SurfaceFit::Fit),
//!         None => own_fallback_icon(),
//!     }
//! }
//! ```
//!
//! Pass the app's own `ImageCache` in [`Options::cache`] to share its decoder threads; by
//! default the loader makes one (up to 4 threads that sleep while idle). Dropping the last
//! clone of the loader stops its thread after the lookup it is running, with the queued ones
//! left pending; handles keep what they found.
//!
//! A pending request stays among the recent icons (see Caching) like a settled one, since a
//! view that asks each run drops its handle at once and still wants the icon. So dropping
//! the handles doesn't cancel a request: it is skipped only if, at its turn, no handle holds it
//! and it has left the recent icons. A view that scrolls past many icons queues lookups for at
//! most [`Options::lru_entries`] it no longer shows.
//!
//! # The lookup
//!
//! As GTK 4 (`gtkicontheme.c`), with the differences listed below:
//!
//! - **Search path** ([`SearchPath::from_env`]): `$XDG_DATA_HOME/icons`, `~/.icons`, then
//!   `icons` under each `$XDG_DATA_DIRS`; unthemed icons also in `pixmaps` under each of those
//!   and in `/usr/share/pixmaps`.
//! - **Theme chain**: the theme, then what its `Inherits` names, depth-first in order, then
//!   hicolor. A theme without an `index.theme` in any root is left out, with what it would
//!   inherit; a cycle stops at the first repeat. A hicolor without an `index.theme` gets one
//!   derived from its directory names (GTK has a built-in copy).
//! - **Directories**: `Directories` then `ScaledDirectories`, with the spec's defaults (`Type`
//!   Threshold, `Threshold` 2, `MinSize` and `MaxSize` the `Size`, `Scale` 1). Directories with
//!   the same parameters merge into one size class; within it a name keeps the first
//!   directory that has it, in `Directories` order and then root order, so a user root
//!   overrides a system one. `Context` is ignored. In one directory PNG beats SVG.
//! - **Order**: leading `-symbolic` names through the whole chain first, then every theme in
//!   order with each name in order (themes outside names: a fallback name in the theme beats
//!   the first name in hicolor), then unthemed icons (PNG before SVG).
//! - **Size**: GTK's comparison, rule for rule: an exact match, else always an icon to scale
//!   down over one to scale up, the closest size, the requested scale, a bitmap over a scalable
//!   directory, then the closest scaled size; a full tie goes to the later directory. Like
//!   GTK, a Scalable directory's `MaxSize` is compared with the unscaled size, and a larger
//!   directory listed after an exact match beats it (GTK checks "scale down" first).
//! - **Scale**: directories match at the scale rounded up ([`IconRequest::theme_scale`]); the
//!   file is drawn at `ceil(size * scale)` device pixels ([`IconRequest::pixels`]). An SVG is
//!   rasterized at that size, a PNG decoded at its own and drawn through Rune's mips.
//! - **Names**: [`IconRequest::from_desktop_icon`] reads `Icon=` as `GDesktopAppInfo` does and
//!   adds the `-symbolic` counterpart as `GThemedIcon` does; [`themed_names`] builds the same
//!   lists, with GIO's optional dash fallbacks (`text-x-python`, `text-x`, `text`).
//!
//! # Differences from GTK
//!
//! - **Symbolic icons draw as their files paint them**, usually grey, not recoloured to the
//!   foreground: Rune can't tint `Pixels`, and `rune::Icon` needs a `'static` body.
//!   [`LoadedIcon::is_symbolic`] says which they are. Draft U-P6-2.
//! - **No `.xpm`.** Rune has no XPM decoder, so XPM files are left out of listings and the
//!   lookup moves on to a file it can draw. `.symbolic.png` (GTK's own encoding) never matches.
//! - **No `icon-theme.cache`.** Directories are listed instead: a few milliseconds a theme, on
//!   the thread, once.
//! - **Nothing found is [`IconState::NotFound`]**, where GTK shows `image-missing`, so the app
//!   shows its own fallback. A file that fails to decode is [`IconState::Failed`]; neither
//!   GTK nor this module tries another file then.
//! - No `-rtl` and `-ltr` variants and no forced regular or symbolic lookups.
//! - **Each theme lists its own directories only.** GTK keeps one array of theme directories
//!   (`dir_mtimes`) for the whole chain and looks for each subdirectory under every theme
//!   directory read so far, so an inherited theme also picks up the files of the themes before
//!   it under its own size parameters. That isn't copied.
//! - **`index.theme` is read more leniently.** Where GTK would drop something, this module
//!   keeps going:
//!   - A group that appears twice is read from its first appearance; `GKeyFile` merges them.
//!   - A `MinSize`, `MaxSize`, `Threshold` or `Scale` that isn't a whole number takes the
//!     spec's default; GTK reads 0.
//!   - An `index.theme` without an `[Icon Theme]` group or its `Directories` key counts as
//!     missing: the next root's is read, and a hicolor gets one derived from its directories.
//!     GTK leaves the theme out, hicolor too.
//!   - A `Directories` entry that is absolute or climbs out with `..` is left out, where GTK
//!     appends it to the theme directory as a string (keeping an absolute one inside the theme,
//!     and following `..` out of it).
//!
//! # Changes on disk
//!
//! A [`IconLoader::load`] at least [`Options::revalidate_after`] (5 s, as GTK) after the last
//! check has the thread check the modification time of every root, theme directory and
//! `index.theme` read so far, and read again the themes where one changed. That also catches
//! an icon installed into an existing size directory, which GTK only sees once
//! `icon-theme.cache` is rebuilt. [`IconLoader::rescan`] checks at once. After a change
//! [`IconLoader::poll`] turns true, so the view asks again, and each icon it asks for is looked
//! up again. Until that lookup has settled and decoded, [`IconLoader::load`] keeps returning
//! the old handle of an icon that was drawn, so nothing flashes to the fallback (a package
//! install that runs `gtk-update-icon-cache` changes `hicolor`'s directory each time); then it
//! returns the new one. Handles kept from before keep their icon.
//!
//! A file rewritten in place, at the same path and size, keeps its old pixels while any
//! handle to them lives: Rune's `ImageCache` shares decoded files by path, size and scale, and
//! the handle drawn until the new lookup lands is one of them.
//!
//! # Measured
//!
//! The example (`examples/icon-theme.rs`, `--time`) on a desktop with Yaru, Adwaita, Breeze and
//! hicolor installed, warm file cache, this crate at opt-level 3: the first lookup in Yaru
//! (reading Yaru's and hicolor's indexes and listing their directories) 25 ms, a warm lookup
//! 7 µs, a revalidation with nothing changed 0.6 ms, and [`IconLoader::load`] returning a
//! shared handle 0.5 µs. With `--icon-theme HighContrast`, Firefox's icon comes from
//! HighContrast and Nautilus's from hicolor's SVG; in Yaru, Nautilus's from Yaru's
//! `24x24@2x` PNG. That desktop has no Firefox installed, so Firefox's own hicolor icon is
//! untested.
//!
//! # Caching
//!
//! Handles are shared while one lives, by theme, source, size and scale. Besides those, the
//! [`Options::lru_entries`] most recent icons drawn at up to [`Options::lru_max_px`] device
//! pixels stay loaded (128 and 256: GTK keeps 100, up to 128 pixels). A large PNG decodes at
//! its own size (a 256 px PNG for a 22 pt row is 256 KiB, plus a third for its mips), so the
//! recent icons are counted by their decoded pixels too: together at most what 128 icons of
//! 256 px hold, 8 Mpx (32 MiB, 43 MiB with mips), plus the most recent icon whatever its size.
//! A 1024 px PNG for a 22 pt row counts as 16 icons.
//!
//! A view that asks for its icons each run and drops the handles relies on the recent icons:
//! one that shows more than they hold has the oldest looked up again on each run, and each
//! lookup runs the view again. Such a view keeps its handles in its state instead.

mod index;
mod loader;
mod matching;
mod request;
mod resolver;
#[cfg(test)]
mod test_support;

pub use loader::{IconLoader, IconState, LoadedIcon, Options, THREAD_NAME};
pub use request::{IconRequest, IconSource, themed_names};
pub use resolver::{FALLBACK_THEME, IconResolver, Resolved, SearchPath};
