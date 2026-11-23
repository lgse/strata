// SPDX-License-Identifier: MIT

#[cfg(feature = "foreign-parent")]
fn main() {
    probe::main();
}

#[cfg(not(feature = "foreign-parent"))]
fn main() {
    eprintln!("run with --features foreign-parent");
}

#[cfg(feature = "foreign-parent")]
mod probe {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, Ordering};

    use rune::{App, AppHandle, AppSpec, Cx, El, Tx, WindowHandle, WindowOptions, WindowSpec};
    use rune::{col, text};
    use serde_json::{Value, json};
    use strata_platform::foreign_parent::{self, Options, Outcome, Parent};
    use strata_platform::transfer::Transfer;

    static AS_IF_OLD: AtomicBool = AtomicBool::new(false);

    thread_local! {
        static CHOOSERS: RefCell<HashMap<String, (WindowHandle, AppHandle<Chooser>)>> =
            RefCell::new(HashMap::new());
        /// Choosers whose `on_close` skips `release`.
        static NO_RELEASE: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    }

    fn log(event: &str, fields: Value) {
        let mut line = json!({ "ev": event });
        if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
            obj.extend(fields);
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    #[derive(Default)]
    struct Host;

    struct Chooser {
        key: String,
    }

    fn host_view(_: &Host, _: &mut Cx<Host>) -> El {
        col()
            .w_full()
            .h_full()
            .center()
            .child(text("foreign-parent host"))
    }

    fn chooser_view(s: &Chooser, _: &mut Cx<Chooser>) -> El {
        col()
            .w_full()
            .h_full()
            .center()
            .child(text(format!("chooser {}", s.key)))
    }

    fn p0_windows() -> Option<usize> {
        Transfer::current().map(|t| t.attached_windows())
    }

    /// The outcome's variant names, for the tests to match on.
    fn outcome_json(key: &str, o: &Outcome) -> Value {
        let name = |d: String| {
            d.split(|c: char| !c.is_alphanumeric())
                .next()
                .unwrap_or_default()
                .to_owned()
        };
        match o {
            Outcome::Parented { system, late } => json!({
                "key": key, "parented": true, "system": format!("{system:?}"), "late": late,
                "text": o.to_string(),
            }),
            Outcome::Parentless { reason, fallback } => json!({
                "key": key, "parented": false, "reason": name(format!("{reason:?}")),
                "fallback": format!("{fallback:?}"), "text": o.to_string(),
            }),
            other => json!({ "key": key, "parented": false, "text": other.to_string() }),
        }
    }

    fn on_lost(s: &mut Chooser, _: &mut Tx) {
        log("lost", json!({ "key": s.key }));
    }

    fn on_close(s: &mut Chooser, _: &mut Tx) {
        let skip = NO_RELEASE.with(|n| n.borrow_mut().remove(&s.key));
        let released = !skip && foreign_parent::release(&s.key);
        CHOOSERS.with(|c| c.borrow_mut().remove(&s.key));
        log(
            "closed",
            json!({ "key": s.key, "released": released, "p0_windows": p0_windows() }),
        );
    }

    fn applied(key: &str, outcome: &Outcome) {
        let mut v = outcome_json(key, outcome);
        v["p0_windows"] = json!(p0_windows());
        log("applied", v);
    }

    fn parse(key: &str, parent: &str) -> Parent {
        Parent::parse(parent).unwrap_or_else(|e| {
            log("bad-parent", json!({ "key": key, "error": e.to_string() }));
            Parent::None
        })
    }

    fn options(modal: bool) -> Options<Chooser> {
        let options = Options::new().modal(modal).on_lost(on_lost);
        if AS_IF_OLD.load(Ordering::Relaxed) {
            return options.as_if_libwayland_below_1_23();
        }
        options
    }

    /// Applies `parent` again to a chooser that is shown by now.
    fn reapply(key: &str, parent: &str, modal: bool) {
        let Some((window, app)) = CHOOSERS.with(|c| c.borrow().get(key).cloned()) else {
            return log("error", json!({ "error": format!("no chooser {key}") }));
        };
        let outcome = foreign_parent::apply(&window, &app, &parse(key, parent), options(modal));
        applied(key, &outcome);
    }

    fn open(key: &str, parent: &str, modal: bool) {
        if CHOOSERS.with(|c| c.borrow().contains_key(key)) {
            log(
                "error",
                json!({ "error": format!("chooser {key} is open") }),
            );
            return;
        }
        let parent = parse(key, parent);
        let title: &'static str = Box::leak(format!("chooser {key}").into_boxed_str());
        let app = App::new(AppSpec::new(
            title,
            (420.0, 300.0),
            || Chooser { key: String::new() },
            chooser_view,
        ))
        .window_options(WindowOptions {
            visible: false,
            ..WindowOptions::default()
        })
        .on_close(on_close);
        let handle = app.handle();
        let kept = handle.clone();
        let options = options(modal);
        let k = key.to_owned();
        let app = app
            .on_window(move |w| {
                let outcome = foreign_parent::apply(w, &handle, &parent, options);
                applied(&k, &outcome);
                w.show();
            })
            .on_ready({
                let k = key.to_owned();
                move |_| log("ready", json!({ "key": k }))
            });
        let spec = WindowSpec::from_app(key, app).state(Chooser {
            key: key.to_owned(),
        });
        let window = rune::open_window(spec);
        CHOOSERS.with(|c| c.borrow_mut().insert(key.to_owned(), (window, kept)));
    }

    fn command(line: &str, tx: &mut Tx) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            log("error", json!({ "error": format!("bad command {line:?}") }));
            return;
        };
        let key = v["key"].as_str().unwrap_or("c1");
        match v["cmd"].as_str() {
            Some("open") => open(
                key,
                v["parent"].as_str().unwrap_or_default(),
                v["modal"].as_bool().unwrap_or(true),
            ),
            Some("reapply") => reapply(
                key,
                v["parent"].as_str().unwrap_or_default(),
                v["modal"].as_bool().unwrap_or(true),
            ),
            Some("close") => match CHOOSERS.with(|c| c.borrow().get(key).cloned()) {
                Some((w, _)) => {
                    if v["release"] == false {
                        NO_RELEASE.with(|n| n.borrow_mut().insert(key.to_owned()));
                    }
                    w.close();
                }
                None => log("error", json!({ "error": format!("no chooser {key}") })),
            },
            Some("quit") => {
                if v["shutdown"].as_bool().unwrap_or(true) {
                    let result = foreign_parent::shutdown();
                    log("shutdown", json!({ "ok": result.is_ok() }));
                }
                tx.quit();
            }
            _ => log(
                "error",
                json!({ "error": format!("unknown command {line:?}") }),
            ),
        }
    }

    pub(super) fn main() {
        let args = rune::tooling::extra_args();
        AS_IF_OLD.store(
            args.iter().any(|a| a == "--as-if-libwayland-1-22"),
            Ordering::Relaxed,
        );
        let parent = args
            .iter()
            .position(|a| a == "--parent")
            .and_then(|i| args.get(i + 1))
            .cloned();
        rune::tooling::set_app_id("strata-foreign-parent");
        let app = App::new(AppSpec::new(
            "foreign-parent host",
            (320.0, 200.0),
            Host::default,
            host_view,
        ));
        let handle = app.handle();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                handle.update(move |_, tx| command(&line, tx));
            }
        });
        rune::orderly_exit();
        let started = AtomicBool::new(false);
        let result = app
            .on_ready(move |_| {
                log("host-ready", json!({}));
                if !started.swap(true, Ordering::Relaxed)
                    && let Some(parent) = &parent
                {
                    open("c1", parent, true);
                }
            })
            .try_run();
        match result {
            Ok(()) => log("exit", json!({ "code": 0 })),
            Err(error) => {
                log("exit-error", json!({ "error": error }));
                std::process::exit(1);
            }
        }
    }
}
