// SPDX-License-Identifier: MIT

use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};

pub(super) const PREFERRED_LAUNCHER: &str = "xdg-terminal-exec";

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
        program: "ghostty",
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

    fn base(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.leading);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    pub(super) fn directory_command(&self, path: &Path) -> Command {
        let mut command = self.base();
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
        let mut command = self.base();
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

    pub(super) fn launch_failure(&self, error: &std::io::Error) -> String {
        let program = self.program.to_string_lossy();
        if error.kind() == ErrorKind::NotFound {
            return format!("Terminal “{program}” was not found on your PATH");
        }
        format!("Terminal “{program}” could not be started: {error}")
    }
}

pub(super) fn no_terminal_message() -> String {
    let fallbacks = KNOWN_TERMINALS
        .iter()
        .skip(1)
        .map(|known| known.program)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "No terminal emulator was found. Install “{PREFERRED_LAUNCHER}” with a configured ~/.config/xdg-terminals.list, set $TERMINAL, or install one of: {fallbacks}"
    )
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

#[cfg(test)]
mod tests;
