//! Shows icons from the system's icon themes (the `icon-theme` feature), and prints what each
//! lookup found.
//!
//! Each `--icon VALUE` is a `.desktop` file (its `Icon=` is read), an absolute image path, or
//! an `Icon=` value such as `firefox`; by default `firefox` and `org.gnome.Nautilus`. The
//! lookup starts in `--icon-theme NAME` (default hicolor) at `--icon-size N` points (default
//! 48) and `--icon-scale S` (default 1). Rune's own flags pass through, so render without a
//! window with
//!
//! ```sh
//! cargo run -p strata-platform --example icon-theme --features icon-theme -- \
//!     --icon-theme HighContrast --snap icons.png
//! ```
//!
//! For each icon it prints `value: state path WxH (theme, symbolic)` before the first frame;
//! setup waits for the lookups (up to 5 s) so a `--snap` shows them decoded.
//!
//! `--time` prints lookup timings against the system themes instead, and opens nothing: the
//! first lookup in the theme (reading its chain's indexes and directories), a warm lookup, a
//! revalidation with nothing changed, and `IconLoader::load` returning a shared handle.

#[cfg(feature = "icon-theme")]
mod demo {
    use std::sync::{Arc, OnceLock};
    use std::time::{Duration, Instant};

    use rune::{
        App, AppSpec, Color, Cx, El, SurfaceFit, TextStyle, Waker, col, el, oklch, row, text,
    };
    use strata_platform::icon_theme::{
        IconLoader, IconRequest, IconResolver, IconSource, IconState, LoadedIcon, Options,
        SearchPath,
    };

    struct Config {
        theme: String,
        size: u32,
        scale: f32,
        icons: Vec<String>,
    }

    static CONFIG: OnceLock<Config> = OnceLock::new();

    #[derive(Default)]
    pub struct State {
        loader: Option<IconLoader>,
        shown: Vec<(String, Option<LoadedIcon>)>,
    }

    /// An `--icon` value as a request: a `.desktop` file's `Icon=`, or the value itself.
    fn request(value: &str, config: &Config) -> Option<IconRequest> {
        let icon = if value.ends_with(".desktop") {
            let entry = std::fs::read_to_string(value).ok()?;
            desktop_icon(&entry)?.to_owned()
        } else {
            value.to_owned()
        };
        IconRequest::from_desktop_icon(&icon, config.size, config.scale)
    }

    /// The `Icon=` key of an entry's `[Desktop Entry]` group.
    fn desktop_icon(entry: &str) -> Option<&str> {
        let mut group = "";
        for line in entry.lines().map(str::trim) {
            if line.starts_with('[') {
                group = line;
            } else if group == "[Desktop Entry]"
                && let Some(value) = line.strip_prefix("Icon=")
            {
                return Some(value.trim());
            }
        }
        None
    }

    fn describe(value: &str, icon: Option<&LoadedIcon>) -> String {
        let Some(icon) = icon else {
            return format!("{value}: no Icon= to look up");
        };
        let state = match icon.state() {
            IconState::Failed(e) => format!("failed ({e})"),
            other => format!("{other:?}").to_lowercase(),
        };
        let path = icon
            .path()
            .map(|p| format!(" {}", p.display()))
            .unwrap_or_default();
        let size = icon.image().and_then(rune::Image::size);
        let size = size.map(|[w, h]| format!(" {w}x{h}")).unwrap_or_default();
        let symbolic = if icon.is_symbolic() { ", symbolic" } else { "" };
        format!(
            "{value}: {state}{path}{size} (theme {}{symbolic})",
            icon.theme()
        )
    }

    fn setup(s: &mut State, waker: Waker) {
        let Some(config) = CONFIG.get() else { return };
        let options = Options {
            theme: config.theme.clone(),
            ..Options::default()
        };
        let loader = match IconLoader::start(waker, options) {
            Ok(loader) => loader,
            Err(e) => {
                println!("no loader: {e}");
                return;
            }
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        for value in &config.icons {
            let icon = request(value, config).map(|r| loader.load(&r));
            if let Some(icon) = &icon {
                icon.wait(deadline.saturating_duration_since(Instant::now()));
            }
            println!("{}", describe(value, icon.as_ref()));
            s.shown.push((value.clone(), icon));
        }
        s.loader = Some(loader);
    }

    fn poll(s: &mut State, _: f64, _: &mut rune::Tx) -> bool {
        s.loader.as_ref().is_some_and(IconLoader::poll)
    }

    fn view(s: &State, _: &mut Cx<State>) -> El {
        let size = CONFIG.get().map_or(48, |c| c.size) as f32;
        let label = |t: &str| {
            text(t.to_owned())
                .font(TextStyle::new(12.0, 400.0))
                .color(Color::WHITE)
        };
        let cells: Vec<El> = s
            .shown
            .iter()
            .map(|(value, icon)| {
                let picture = match icon.as_ref().and_then(LoadedIcon::pixels) {
                    Some(pixels) => el().size(size, size).pixels_fit(pixels, SurfaceFit::Fit),
                    None => el()
                        .size(size, size)
                        .radius(6.0)
                        .fill(oklch(0.35, 0.02, 250.0)),
                };
                col().gap(8.0).center().child(picture).child(label(value))
            })
            .collect();
        col()
            .w_full()
            .h_full()
            .center()
            .fill(oklch(0.18, 0.01, 250.0))
            .child(row().gap(32.0).padding(24.0).children(cells))
    }

    fn time(config: &Config) {
        let names: Vec<Vec<String>> = config
            .icons
            .iter()
            .filter_map(|v| match request(v, config)?.source {
                IconSource::Names(names) => Some(names),
                IconSource::File(_) => None,
            })
            .collect();
        let scale = config.scale.ceil() as u32;
        let mut resolver = IconResolver::new(SearchPath::from_env());
        let start = Instant::now();
        for n in &names {
            let found = resolver.resolve(&config.theme, n, config.size, scale);
            println!("{}: {:?}", n[0], found.map(|f| f.path));
        }
        println!(
            "first lookups in {} (chain read): {:?}",
            config.theme,
            start.elapsed()
        );
        let rounds = 1000;
        let start = Instant::now();
        for _ in 0..rounds {
            for n in &names {
                std::hint::black_box(resolver.resolve(&config.theme, n, config.size, scale));
            }
        }
        let per = start.elapsed() / (rounds * names.len().max(1) as u32);
        println!("warm lookup, per request: {per:?}");
        let start = Instant::now();
        let changed = resolver.revalidate();
        println!("revalidation (changed: {changed}): {:?}", start.elapsed());

        let waker: Waker = Arc::new(|| {});
        let options = Options {
            theme: config.theme.clone(),
            ..Options::default()
        };
        let Ok(loader) = IconLoader::start(waker, options) else {
            return;
        };
        let Some(request) = config.icons.first().and_then(|v| request(v, config)) else {
            return;
        };
        let icon = loader.load(&request);
        icon.wait(Duration::from_secs(5));
        let hits = 10_000;
        let start = Instant::now();
        for _ in 0..hits {
            std::hint::black_box(loader.load(&request));
        }
        println!(
            "IconLoader::load, shared handle: {:?}",
            start.elapsed() / hits
        );
    }

    pub fn main() {
        let mut config = Config {
            theme: "hicolor".into(),
            size: 48,
            scale: 1.0,
            icons: Vec::new(),
        };
        let mut timing = false;
        let mut rune_args = Vec::new();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
            match arg.as_str() {
                "--icon-theme" => config.theme = value(),
                "--icon-size" => {
                    config.size = value()
                        .parse()
                        .unwrap_or_else(|e| panic!("--icon-size: {e}"))
                }
                "--icon-scale" => {
                    config.scale = value()
                        .parse()
                        .unwrap_or_else(|e| panic!("--icon-scale: {e}"))
                }
                "--icon" => config.icons.push(value()),
                "--time" => timing = true,
                _ => rune_args.push(arg),
            }
        }
        if config.icons.is_empty() {
            config.icons = vec!["firefox".into(), "org.gnome.Nautilus".into()];
        }
        if timing {
            time(&config);
            return;
        }
        let _ = CONFIG.set(config);
        App::new(AppSpec {
            setup: Some(setup),
            poll: Some(poll),
            ..AppSpec::new("Icon theme", (520.0, 200.0), State::default, view)
        })
        .args(rune_args)
        .run();
    }
}

#[cfg(feature = "icon-theme")]
fn main() {
    demo::main();
}

#[cfg(not(feature = "icon-theme"))]
fn main() {
    eprintln!("run with --features icon-theme");
}
