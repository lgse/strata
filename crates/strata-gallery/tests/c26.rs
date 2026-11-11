//! C26's prompts in process: what scripts can't drive. End process answers as the backend does
//! (a shorter list, in place), a withdrawn request closes its prompt unanswered, and a fake
//! `gio::MountOperation` round-trips through P9's GLib bridge into the real prompts.
//!
//! The binding below is test-only: the owner ruled that the real one lives in Strata (see
//! `strata_components::mount_prompts`). It is the reference Strata's follows. `show-processes`
//! isn't emitted here: its pids come in a `GArray`, which safe Rust can't build or read, so
//! that mapping is covered as plain data (`mount_prompts::tests`).

mod c26 {
    use std::cell::RefCell;
    use std::io::{ErrorKind, Read};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::mpsc::{self, Receiver};
    use std::time::{Duration, Instant};
    use std::{fs, thread};

    use gio::glib;
    use gio::prelude::*;
    use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, Tx};
    use strata_components::mount_prompts::{
        Choice, MountPromptState, MountPrompts, MountRequest, MountResponse, PasswordForm,
        PasswordRequest, QuestionRequest, Remember, Secret,
    };
    use strata_gallery::{Gallery, scenes};
    use strata_kit::look::Look;
    use strata_platform::glib_bridge::{
        Asker, Attachment, Emitter, Glib, GlibThread, Inbox, Keep, LocalTask, Request, Requests,
        Task, spawn_local,
    };

    const FRAME: f64 = 1.0 / 60.0;

    fn mount(name: &str) -> Runtime<Gallery> {
        let scene = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
        let mut g = Gallery::default();
        scene(&mut g);
        let env = Env::new(g.dark, 1.0);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        run(&mut rt, 0.4);
        rt
    }

    fn run<S>(rt: &mut Runtime<S>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    fn keys<S>(rt: &Runtime<S>) -> Vec<String> {
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .filter_map(|e| e.key)
            .collect()
    }

    fn focused<S>(rt: &Runtime<S>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    fn tap<S>(rt: &mut Runtime<S>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        run(rt, 0.1);
    }

    fn press<S>(rt: &mut Runtime<S>, key: Key) {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        run(rt, 0.1);
    }

    #[test]
    fn end_process_leaves_the_prompt_open_on_the_shorter_list() {
        let mut rt = mount("c26-processes-light");
        tap(&mut rt, "c26-mount-processes-end-5120");
        let keys = keys(&rt);
        assert!(!keys.iter().any(|k| k == "c26-mount-processes-process-5120"));
        for pid in [4242, 5133] {
            assert!(
                keys.contains(&format!("c26-mount-processes-process-{pid}")),
                "{pid} is still listed"
            );
        }
        assert!(keys.iter().any(|k| k == "c26-mount-processes-dialog"));
        assert_eq!(rt.state.c26.status, "Ended 5120", "no choice was answered");
    }

    #[test]
    fn a_withdrawn_request_closes_its_prompt_unanswered() {
        let mut rt = mount("c26-sign-in-light");
        rt.update(|g, _| {
            assert_eq!(g.c26.prompt.withdraw(), Some(1));
        });
        run(&mut rt, 0.4);
        assert!(!keys(&rt).iter().any(|k| k == "c26-mount-password-dialog"));
        assert_eq!(rt.state.c26.status, "", "nothing answered");
    }

    // The GIO round trip.

    /// Set in the child copy of this binary, which then runs the test body.
    const CHILD: &str = "STRATA_C26_GIO_TEST_CHILD";
    /// How long the test waits for something the GLib thread sends.
    const DEADLINE: Duration = Duration::from_secs(5);
    const CHILD_DEADLINE: Duration = Duration::from_secs(30);

    /// What the operation replied, read off it in its `reply` handler.
    #[derive(Clone, Debug, PartialEq)]
    struct Reply {
        result: String,
        username: Option<String>,
        password: Option<String>,
        anonymous: bool,
        save: String,
        choice: i32,
    }

    type Asks = Requests<MountRequest, MountResponse>;
    type Ticket = Request<MountRequest, MountResponse>;

    thread_local! {
        /// The fake operation, on the GLib thread, for the test to emit its signals.
        static OP: RefCell<Option<gio::MountOperation>> = const { RefCell::new(None) };
    }

    /// The reference binding: each prompt signal asks the app through the bridge, and the
    /// answer sets the operation's properties and replies. `aborted` drops the pending asks,
    /// which withdraws their requests.
    fn bind(asker: Asker<MountRequest, MountResponse>, replies: Emitter<Reply>) -> Keep {
        let op = gio::MountOperation::new();
        let pending: Rc<RefCell<Vec<LocalTask>>> = Rc::default();
        let password = {
            let (asker, pending) = (asker.clone(), pending.clone());
            op.connect_ask_password(move |op, message, user, domain, flags| {
                // GMountOperation's own handler runs last and replies Unhandled from an idle on
                // the default context: stop it, as Strata stopped GTK's dialog.
                op.stop_signal_emission_by_name("ask-password");
                let has = |f| flags.contains(f);
                let request = PasswordRequest {
                    message: message.to_owned(),
                    default_user: user.to_owned(),
                    default_domain: domain.to_owned(),
                    needs_user: has(gio::AskPasswordFlags::NEED_USERNAME),
                    needs_domain: has(gio::AskPasswordFlags::NEED_DOMAIN),
                    needs_password: has(gio::AskPasswordFlags::NEED_PASSWORD),
                    anonymous_allowed: has(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED),
                    saving_allowed: has(gio::AskPasswordFlags::SAVING_SUPPORTED),
                    secret: Secret::guess(message),
                    error: None,
                };
                ask(op, &asker, &pending, MountRequest::Password(request));
            })
        };
        let question = {
            let (asker, pending) = (asker.clone(), pending.clone());
            // gio-rs binds no typed ask-question handler.
            op.connect_local("ask-question", false, move |values| {
                let op = values.first()?.get::<gio::MountOperation>().ok()?;
                op.stop_signal_emission_by_name("ask-question");
                let message = values.get(1)?.get::<String>().ok()?;
                let choices = values.get(2)?.get::<glib::StrV>().ok()?;
                if choices.is_empty() {
                    op.reply(gio::MountOperationResult::Unhandled);
                    return None;
                }
                let request = QuestionRequest {
                    message,
                    choices: choices.iter().map(|c| Choice::new(c.as_str())).collect(),
                    // Strata: a question while mounting an sftp:// location.
                    host_key: true,
                };
                ask(&op, &asker, &pending, MountRequest::Question(request));
                None
            })
        };
        let aborted = op.connect_aborted(move |_| {
            for task in pending.borrow_mut().drain(..) {
                task.abort();
            }
        });
        let reply = op.connect_reply(move |op, result| {
            replies.emit(Reply {
                result: format!("{result:?}"),
                username: op.username().map(String::from),
                password: op.password().map(String::from),
                anonymous: op.is_anonymous(),
                save: format!("{:?}", op.password_save()),
                choice: op.choice(),
            });
        });
        OP.with_borrow_mut(|o| *o = Some(op.clone()));
        Keep::new()
            .handler(&op, password)
            .handler(&op, question)
            .handler(&op, aborted)
            .handler(&op, reply)
    }

    fn ask(
        op: &gio::MountOperation,
        asker: &Asker<MountRequest, MountResponse>,
        pending: &Rc<RefCell<Vec<LocalTask>>>,
        request: MountRequest,
    ) {
        let (op, answer) = (op.clone(), asker.ask(request));
        let task = spawn_local(async move {
            use gio::MountOperationResult::{Aborted, Handled};
            match answer.await {
                Some(MountResponse::Password(c)) => {
                    // The operation is reused across retries: an earlier Anonymous answer
                    // would still stand (Strata's `apply_mount_credentials`).
                    op.set_anonymous(false);
                    if let Some(user) = &c.user {
                        op.set_username(Some(user));
                    }
                    if let Some(domain) = &c.domain {
                        op.set_domain(Some(domain));
                    }
                    if let Some(password) = &c.password {
                        op.set_password(Some(password));
                    }
                    op.set_password_save(match c.remember {
                        Remember::Never => gio::PasswordSave::Never,
                        Remember::ForSession => gio::PasswordSave::ForSession,
                        Remember::Permanently => gio::PasswordSave::Permanently,
                    });
                    op.reply(Handled);
                }
                Some(MountResponse::Anonymous) => {
                    op.set_anonymous(true);
                    op.reply(Handled);
                }
                Some(MountResponse::Choice(i)) => {
                    op.set_choice(i32::try_from(i).unwrap_or(i32::MAX));
                    op.reply(Handled);
                }
                Some(MountResponse::Abort) | None => op.reply(Aborted),
            }
        });
        pending.borrow_mut().push(task);
    }

    /// The app: the bridge, the prompt and what the operation replied.
    struct App {
        _bridge: GlibThread,
        _attachment: Attachment,
        glib: Glib,
        asks: Asks,
        replies: Inbox<Reply>,
        prompt: MountPromptState<Ticket>,
        seen: Vec<Reply>,
        emits: Vec<Task<()>>,
    }

    /// The host's side, as Strata's poll does it.
    fn poll(s: &mut App, _: f64, _: &mut Tx) -> bool {
        let mut changed = false;
        // Withdrawal first: a binding that replaces a request asks the new one before it drops
        // the old, so a withdrawal seen here comes with its replacement in the drain below,
        // and the prompt updates in place instead of closing for a frame.
        if s.prompt.ticket().is_some_and(Request::is_withdrawn) {
            let _ = s.prompt.withdraw();
            changed = true;
        }
        for request in s.asks.drain() {
            changed = true;
            let question = request.question().clone();
            if let Some(old) = s.prompt.open(question, request) {
                old.respond(MountResponse::Abort);
            }
        }
        for reply in s.replies.drain() {
            changed = true;
            s.seen.push(reply);
        }
        changed
    }

    fn view(s: &App, _: &mut Cx<App>) -> El {
        let look = Look::fallback(false);
        let mut layer = Vec::new();
        MountPrompts::new(
            "m",
            |s: &App| &s.prompt,
            |s: &mut App| &mut s.prompt,
            |_: &mut App, request: Ticket, answer| {
                request.respond(answer);
            },
        )
        .build(&look, s, &mut layer);
        rune_kit::root(rune::el(), layer)
    }

    impl App {
        /// Emits `signal` on the fake operation, on the GLib thread, as a backend would.
        ///
        /// Then runs what is ready on the global default context: GMountOperation's own
        /// handlers queue their `Unhandled` reply there (`g_idle_add`), and the bridge's loop
        /// runs a context of its own, so without this a binding that didn't stop them would
        /// pass.
        fn emit(&mut self, emit: fn(&gio::MountOperation)) {
            self.emits.push(self.glib.spawn(move || async move {
                OP.with_borrow(|op| emit(op.as_ref().expect("the operation is bound")));
                let default = glib::MainContext::default();
                while default.iteration(false) {}
            }));
        }
    }

    fn ask_password(op: &gio::MountOperation) {
        let flags = gio::AskPasswordFlags::NEED_USERNAME
            | gio::AskPasswordFlags::NEED_PASSWORD
            | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED
            | gio::AskPasswordFlags::SAVING_SUPPORTED;
        op.emit_by_name::<()>(
            "ask-password",
            &[&"Password required for share public", &"ann", &"", &flags],
        );
    }

    fn ask_question(op: &gio::MountOperation) {
        let choices = glib::StrV::from(vec!["Log In Anyway", "Cancel Login"]);
        op.emit_by_name::<()>(
            "ask-question",
            &[
                &"Identity Verification Failed\nThe identity sent is “SHA256:abc”.",
                &choices,
            ],
        );
    }

    fn aborted(op: &gio::MountOperation) {
        op.emit_by_name::<()>("aborted", &[]);
    }

    fn waker() -> (rune::Waker, Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        (
            Arc::new(move || {
                let _ = tx.send(());
            }),
            rx,
        )
    }

    /// Steps frames until `done`, sleeping on the waker between them.
    fn frames_until(
        rt: &mut Runtime<App>,
        wakes: &Receiver<()>,
        what: &str,
        done: impl Fn(&Runtime<App>) -> bool,
    ) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            rt.frame(FRAME);
            if done(rt) {
                return;
            }
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for {what}");
            let _ = wakes.recv_timeout((deadline - now).min(Duration::from_millis(20)));
        }
    }

    fn open(rt: &Runtime<App>, dialog: &str) -> bool {
        keys(rt).iter().any(|k| k == dialog)
    }

    /// Answers one `ask-password` on the operation: `form` sets the choices, then the password
    /// is typed and Return pressed, or, with none, Connect is clicked. Returns the reply.
    fn sign_in(
        rt: &mut Runtime<App>,
        wakes: &Receiver<()>,
        password: Option<&str>,
        form: impl FnOnce(&mut PasswordForm),
    ) -> Reply {
        let n = rt.state.seen.len();
        rt.state.emit(ask_password);
        frames_until(rt, wakes, "the sign-in prompt", |rt| {
            open(rt, "m-password-dialog")
        });
        run(rt, 0.3);
        assert_eq!(focused(rt).as_deref(), Some("m-password-secret"));
        rt.update(|s, _| form(s.prompt.password_form_mut().expect("a password prompt")));
        run(rt, 0.1);
        match password {
            Some(password) => {
                press(rt, Key::Text(password.into()));
                press(rt, Key::Enter);
            }
            None => tap(rt, "m-password-primary"),
        }
        frames_until(rt, wakes, "the sign-in reply", |rt| rt.state.seen.len() > n);
        assert_eq!(rt.state.seen.len(), n + 1, "one reply: {:?}", rt.state.seen);
        rt.state.seen[n].clone()
    }

    fn round_trip() {
        let (waker, wakes) = waker();
        let bridge = GlibThread::start(waker).expect("the bridge starts");
        let glib = bridge.handle();
        let (asker, asks) = glib.requests::<MountRequest, MountResponse>();
        let (emitter, replies) = glib.channel::<Reply>();
        let attachment = glib.attach(move || bind(asker, emitter));
        let app = App {
            _bridge: bridge,
            _attachment: attachment,
            glib,
            asks,
            replies,
            prompt: MountPromptState::default(),
            seen: Vec::new(),
            emits: Vec::new(),
        };
        let mut rt = Runtime::new(app, view, (800.0, 700.0), Env::new(false, 1.0));
        rt.poll = Some(poll);
        rt.frame(0.0);

        // Sign in on one operation, as GIO reuses it across retries: Anonymous, then
        // credentials with each storage choice, so a setting one answer leaves behind shows.
        let anonymous = sign_in(&mut rt, &wakes, None, |form| form.anonymous = true);
        assert_eq!(
            anonymous,
            Reply {
                result: "Handled".into(),
                username: None,
                password: None,
                anonymous: true,
                save: "Never".into(),
                choice: 0,
            }
        );
        for (remember, save) in [
            (Remember::ForSession, "ForSession"),
            (Remember::Permanently, "Permanently"),
            (Remember::Never, "Never"),
        ] {
            let reply = sign_in(&mut rt, &wakes, Some("hunter2"), |form| {
                form.remember = remember;
            });
            assert_eq!(
                reply,
                Reply {
                    result: "Handled".into(),
                    username: Some("ann".into()),
                    password: Some("hunter2".into()),
                    anonymous: false,
                    save: save.into(),
                    choice: 0,
                },
                "{remember:?} after Anonymous"
            );
        }
        let n = rt.state.seen.len();

        // The host key, declined with Return on the declining choice, which has focus.
        rt.state.emit(ask_question);
        frames_until(&mut rt, &wakes, "the host-key prompt", |rt| {
            open(rt, "m-question-dialog")
        });
        run(&mut rt, 0.3);
        press(&mut rt, Key::Enter);
        frames_until(&mut rt, &wakes, "the host-key reply", |rt| {
            rt.state.seen.len() > n
        });
        assert_eq!(rt.state.seen.len(), n + 1, "one reply: {:?}", rt.state.seen);
        assert_eq!(rt.state.seen[n].result, "Handled");
        assert_eq!(rt.state.seen[n].choice, 1, "Cancel Login, by its index");

        // Escape aborts.
        rt.state.emit(ask_password);
        frames_until(&mut rt, &wakes, "a second sign-in prompt", |rt| {
            open(rt, "m-password-dialog")
        });
        run(&mut rt, 0.3);
        press(&mut rt, Key::Escape);
        frames_until(&mut rt, &wakes, "the abort", |rt| {
            rt.state.seen.len() == n + 2
        });
        assert_eq!(rt.state.seen[n + 1].result, "Aborted");

        // The backend's `aborted` closes the prompt, and nothing replies.
        rt.state.emit(ask_password);
        frames_until(&mut rt, &wakes, "a third sign-in prompt", |rt| {
            open(rt, "m-password-dialog")
        });
        rt.state.emit(aborted);
        frames_until(&mut rt, &wakes, "the withdrawal", |rt| {
            !rt.state.prompt.is_open()
        });
        let end = Instant::now() + Duration::from_millis(200);
        while Instant::now() < end {
            rt.frame(FRAME);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            rt.state.seen.len(),
            n + 2,
            "a withdrawn request gets no reply"
        );
        assert!(!open(&rt, "m-password-dialog"));
    }

    #[test]
    fn a_mount_operation_round_trips_through_the_glib_bridge() {
        isolated(
            "a_mount_operation_round_trips_through_the_glib_bridge",
            round_trip,
        );
    }

    /// Runs `body` in a child copy of this test binary, where GIO stays local and every bus
    /// address points at a socket this process listens on, as P9's tests do. Fails if the
    /// child fails, ran no test, or connected to either bus.
    fn isolated(name: &str, body: fn()) {
        if std::env::var_os(CHILD).is_some() {
            body();
            return;
        }
        let scratch = Scratch::new();
        let session = scratch.path().join("session-bus");
        let system = scratch.path().join("system-bus");
        let buses = [("session", listen(&session)), ("system", listen(&system))];
        let runtime_dir = scratch.path().join("run");
        fs::create_dir(&runtime_dir).expect("make the runtime dir");
        fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
            .expect("make the runtime dir private");
        let test = format!("c26::{name}");
        let mut child = Command::new(std::env::current_exe().expect("the test binary's path"))
            .args([test.as_str(), "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env("GIO_USE_VFS", "local")
            .env("GSETTINGS_BACKEND", "memory")
            .env("DBUS_SESSION_BUS_ADDRESS", bus_address(&session))
            .env("DBUS_SYSTEM_BUS_ADDRESS", bus_address(&system))
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .env("TMPDIR", scratch.path())
            .env_remove("DISPLAY")
            .env("WAYLAND_DISPLAY", "strata-test-no-wayland")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the child test");
        let stdout = read_all(child.stdout.take().expect("the child's stdout"));
        let stderr = read_all(child.stderr.take().expect("the child's stderr"));
        let deadline = Instant::now() + CHILD_DEADLINE;
        let status = loop {
            if let Some(status) = child.try_wait().expect("wait for the child test") {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            thread::sleep(Duration::from_millis(20));
        };
        let stdout = stdout.join().expect("read the child's stdout");
        let stderr = stderr.join().expect("read the child's stderr");
        let Some(status) = status else {
            panic!("{test} ran past {CHILD_DEADLINE:?} in the child:\n{stdout}\n{stderr}");
        };
        assert!(
            status.success(),
            "{test} failed in the child ({status}):\n{stdout}\n{stderr}"
        );
        assert!(
            stdout.contains("test result: ok. 1 passed"),
            "the child ran no test named {test}:\n{stdout}"
        );
        for (bus, listener) in buses {
            match listener.accept() {
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Ok(_) => panic!("{test} connected to the {bus} bus"),
                Err(e) => panic!("checking the {bus} bus: {e}"),
            }
        }
    }

    fn read_all(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<String> {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    }

    fn listen(path: &Path) -> UnixListener {
        let listener = UnixListener::bind(path).expect("listen on a stand-in bus");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        listener
    }

    fn bus_address(path: &Path) -> String {
        format!("unix:path={}", path.display())
    }

    /// A directory under the target's scratch space, removed on drop. Its name is short, as
    /// the stand-in bus sockets inside it must fit a socket address (108 bytes).
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path =
                Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("c26-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("make a scratch dir");
            Scratch(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
