// SPDX-License-Identifier: MIT

use futures_channel::oneshot;
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};

#[cfg(test)]
mod tests;

pub(super) const PREFERRED_LAUNCHER: &str = "xdg-terminal-exec";

const GHOSTTY: &str = "ghostty";

const OMARCHY_PRESENTATION_LAUNCHER: &str = "omarchy-launch-floating-terminal-with-presentation";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryStyle {
    Joined(&'static str),
    Separate(&'static str),
    Inherited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecStyle {
    Separator,
    Flag(&'static str),
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KnownTerminal {
    program: &'static str,
    leading: &'static [&'static str],
    directory: DirectoryStyle,
    exec: ExecStyle,
}

const KNOWN_TERMINALS: &[KnownTerminal] = &[
    KnownTerminal {
        program: PREFERRED_LAUNCHER,
        leading: &[],
        directory: DirectoryStyle::Joined("--dir="),
        exec: ExecStyle::Separator,
    },
    KnownTerminal {
        program: "konsole",
        leading: &[],
        directory: DirectoryStyle::Separate("--workdir"),
        exec: ExecStyle::Flag("-e"),
    },
    KnownTerminal {
        program: "gnome-terminal",
        leading: &[],
        directory: DirectoryStyle::Joined("--working-directory="),
        exec: ExecStyle::Separator,
    },
    KnownTerminal {
        program: "xfce4-terminal",
        leading: &[],
        directory: DirectoryStyle::Separate("--working-directory"),
        exec: ExecStyle::Flag("-x"),
    },
    KnownTerminal {
        program: "kitty",
        leading: &[],
        directory: DirectoryStyle::Separate("--working-directory"),
        exec: ExecStyle::Separator,
    },
    KnownTerminal {
        program: GHOSTTY,
        leading: &[],
        directory: DirectoryStyle::Joined("--working-directory="),
        exec: ExecStyle::Flag("-e"),
    },
    KnownTerminal {
        program: "alacritty",
        leading: &[],
        directory: DirectoryStyle::Separate("--working-directory"),
        exec: ExecStyle::Flag("-e"),
    },
    KnownTerminal {
        program: "foot",
        leading: &[],
        directory: DirectoryStyle::Separate("--working-directory"),
        exec: ExecStyle::Direct,
    },
    KnownTerminal {
        program: "wezterm",
        leading: &["start"],
        directory: DirectoryStyle::Separate("--cwd"),
        exec: ExecStyle::Separator,
    },
    KnownTerminal {
        program: "tilix",
        leading: &[],
        directory: DirectoryStyle::Separate("--working-directory"),
        exec: ExecStyle::Flag("-e"),
    },
    KnownTerminal {
        program: "terminator",
        leading: &[],
        directory: DirectoryStyle::Separate("--working-directory"),
        exec: ExecStyle::Flag("-x"),
    },
    KnownTerminal {
        program: "xterm",
        leading: &[],
        directory: DirectoryStyle::Inherited,
        exec: ExecStyle::Flag("-e"),
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Terminal {
    program: OsString,
    leading: Vec<OsString>,
    directory: DirectoryStyle,
    exec: ExecStyle,
}

impl Terminal {
    pub(super) fn resolve() -> Option<Terminal> {
        Self::resolve_with(
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("TERMINAL").as_deref(),
        )
    }

    pub(super) fn resolve_with(
        path_var: Option<&OsStr>,
        terminal_var: Option<&OsStr>,
    ) -> Option<Terminal> {
        if let Some(explicit) = explicit_terminal(path_var, terminal_var) {
            return Some(explicit);
        }
        KNOWN_TERMINALS.iter().find_map(|known| {
            find_on_path(path_var, known.program).map(|program| Terminal::known(program, known))
        })
    }

    fn known(program: OsString, known: &KnownTerminal) -> Terminal {
        Terminal {
            program,
            leading: known.leading.iter().map(OsString::from).collect(),
            directory: known.directory,
            exec: known.exec,
        }
    }

    pub(super) fn program(&self) -> &OsStr {
        &self.program
    }

    fn base(&self, action: Option<&str>) -> Command {
        let mut command = Command::new(&self.program);
        command.args(action);
        command.args(&self.leading);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::child_environment::restore_command(&mut command);
        command
    }

    pub(super) fn directory_command(&self, path: &Path) -> Command {
        // A single-instance Ghostty hands a plain launch to the running instance,
        // which ignores the directory; `+new-window` forwards it from 1.3 onward.
        let action =
            (self.is_ghostty() && ghostty_has_new_window(&self.program)).then_some("+new-window");
        let mut command = self.base(action);
        command.current_dir(path);
        match self.directory {
            DirectoryStyle::Joined(prefix) => {
                let mut argument = OsString::from(prefix);
                argument.push(path);
                command.arg(argument);
            }
            DirectoryStyle::Separate(flag) => {
                command.arg(flag).arg(path);
            }
            DirectoryStyle::Inherited => {}
        }
        command
    }

    pub(super) fn exec_command<S: AsRef<OsStr>>(&self, argv: &[S]) -> Command {
        let mut command = self.base(None);
        match self.exec {
            ExecStyle::Separator => {
                command.arg("--");
            }
            ExecStyle::Flag(flag) => {
                command.arg(flag);
            }
            ExecStyle::Direct => {}
        }
        command.args(argv);
        command
    }

    fn is_ghostty(&self) -> bool {
        Path::new(&self.program).file_name() == Some(OsStr::new(GHOSTTY))
    }

    pub(super) fn launch_failure(&self, error: &std::io::Error) -> String {
        let program = self.program.to_string_lossy();
        if error.kind() == ErrorKind::NotFound {
            return rust_i18n::t!(
                "Terminal “%{program}” was not found on your PATH",
                program = program
            )
            .into_owned();
        }
        rust_i18n::t!(
            "Terminal “%{program}” could not be started: %{error}",
            program = program,
            error = crate::services::io_error_detail(error)
        )
        .into_owned()
    }
}

/// Omarchy's own launcher for its maintenance commands: a floating terminal
/// that keeps the output open until a key is pressed, even after a failure.
pub(super) fn omarchy_presentation_command(
    path_var: Option<&OsStr>,
    command: &str,
) -> Option<Command> {
    let launcher = find_on_path(path_var, OMARCHY_PRESENTATION_LAUNCHER)?;
    let mut launch = Command::new(launcher);
    launch
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::child_environment::restore_command(&mut launch);
    Some(launch)
}

pub(super) fn no_terminal_message() -> String {
    // Program names are identifiers, so they keep ASCII separators that wrap between names
    // even in languages whose list separator has no space; they start on their own line.
    let fallbacks = KNOWN_TERMINALS
        .iter()
        .skip(1)
        .map(|known| known.program)
        .collect::<Vec<_>>()
        .join(", ");
    let fallbacks = format!("\n{fallbacks}");
    rust_i18n::t!("No terminal emulator was found. Install “%{PREFERRED_LAUNCHER}” with a configured ~/.config/xdg-terminals.list, set $TERMINAL, or install one of: %{fallbacks}", PREFERRED_LAUNCHER = PREFERRED_LAUNCHER, fallbacks = fallbacks).into_owned()
}

/// Starts and reaps the child off the UI thread, so a launcher neither blocks
/// it nor lingers as defunct.
pub(super) fn spawn_reaped(
    start: impl FnOnce() -> Result<Child, String> + Send + 'static,
) -> impl Future<Output = Result<(), String>> + 'static {
    let (started, answer) = oneshot::channel();
    let reaper = std::thread::Builder::new()
        .name("process-reaper".into())
        .spawn(move || match start() {
            Ok(mut child) => {
                let _ = started.send(Ok(()));
                if let Err(error) = child.wait() {
                    tracing::warn!(%error, pid = child.id(), "unable to reap child process");
                }
            }
            Err(error) => {
                let _ = started.send(Err(error));
            }
        });
    if let Err(error) = reaper {
        tracing::warn!(%error, "unable to start reaper thread");
    }
    async move {
        answer
            .await
            .unwrap_or_else(|_| Err(crate::i18n::tr(crate::services::UNEXPECTED_SYSTEM_ERROR)))
    }
}

fn explicit_terminal(path_var: Option<&OsStr>, terminal_var: Option<&OsStr>) -> Option<Terminal> {
    let text = terminal_var.and_then(OsStr::to_str)?;
    let mut words = glib::shell_parse_argv(text).ok()?.into_iter();
    let program = words.next()?;
    let file_name = Path::new(&program).file_name()?.to_str()?;
    let known = KNOWN_TERMINALS
        .iter()
        .find(|known| known.program == file_name);
    let leading = known
        .into_iter()
        .flat_map(|known| known.leading.iter().map(OsString::from))
        .chain(words)
        .collect();
    Some(Terminal {
        program: find_on_path(path_var, program.to_str()?).unwrap_or(program),
        leading,
        directory: known
            .map(|known| known.directory)
            .unwrap_or(DirectoryStyle::Inherited),
        exec: known
            .map(|known| known.exec)
            .unwrap_or(ExecStyle::Flag("-e")),
    })
}

fn ghostty_has_new_window(program: &OsStr) -> bool {
    static PROBED: Mutex<Vec<(OsString, bool)>> = Mutex::new(Vec::new());
    let mut probed = PROBED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, supported)) = probed.iter().find(|(probed, _)| probed == program) {
        return *supported;
    }
    let supported = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| {
            ghostty_version_has_new_window(&String::from_utf8_lossy(&output.stdout))
        });
    probed.push((program.to_owned(), supported));
    supported
}

fn ghostty_version_has_new_window(version_output: &str) -> bool {
    let Some(version) = version_output
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Ghostty "))
    else {
        return false;
    };
    let mut numbers = version
        .split(['.', '-', '+'])
        .map(|part| part.parse::<u32>().ok());
    match (numbers.next().flatten(), numbers.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) >= (1, 3),
        _ => false,
    }
}

fn find_on_path(path_var: Option<&OsStr>, program: &str) -> Option<OsString> {
    if program.contains('/') {
        return is_executable(Path::new(program))
            .then(|| std::path::absolute(program).ok())
            .flatten()
            .map(|path| path.into_os_string());
    }
    let path_var = path_var?;
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if is_executable(&dir.join(program)) {
            return std::path::absolute(dir.join(program))
                .ok()
                .map(|path| path.into_os_string());
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    crate::util::ui_thread::assert_off();
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
