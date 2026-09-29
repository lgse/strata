// SPDX-License-Identifier: MIT

use std::{
    ffi::OsStr,
    fs::{self, DirBuilder},
    io,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::model::{ActionDefinition, ActionError, ActionRuntime, InterpreterFamily};
use crate::services::actions::{
    ActionAvailability, ActionCatalog, ActionHandle, ActionLoadFailure, ActionProgram,
    ActionScript, ActionStore, ActionStoreError, ActionWriteRequest,
};

#[cfg(test)]
mod tests;

pub(crate) const MANIFEST_FILE: &str = "action.toml";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;
const MAX_SHEBANG_PREFIX_BYTES: u64 = 4096;

pub(crate) fn actions_directory() -> PathBuf {
    crate::storage::config_directory().join("actions")
}

pub(crate) struct LocalActionStore {
    root: PathBuf,
}

impl LocalActionStore {
    pub(crate) fn new() -> Rc<Self> {
        Self::at(actions_directory())
    }

    pub(crate) fn at(root: PathBuf) -> Rc<Self> {
        Rc::new(Self { root })
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    fn action_directory(&self, id: &str) -> Result<PathBuf, ActionStoreError> {
        if !crate::model::valid_action_id(id) {
            return Err(ActionStoreError::Invalid(ActionError::InvalidId(
                id.to_owned(),
            )));
        }
        Ok(self.root.join(id))
    }

    fn load_action(&self, id: &str, directory: &Path) -> Result<ActionHandle, ActionStoreError> {
        ensure_plain_directory(directory)?;
        let manifest = directory.join(MANIFEST_FILE);
        let source = match read_regular_text(&manifest, MAX_MANIFEST_BYTES) {
            Ok(source) => source,
            Err(ActionStoreError::NotFound(_)) => {
                return Err(ActionStoreError::NotAnActionDirectory(
                    directory.display().to_string(),
                ));
            }
            Err(error) => return Err(error),
        };
        let definition = ActionDefinition::parse(&source)?;
        if definition.id != id {
            return Err(ActionStoreError::IdMismatch {
                declared: definition.id.clone(),
                directory: id.to_owned(),
            });
        }
        let availability = resolve_availability(directory, &definition)?;
        Ok(ActionHandle {
            definition,
            directory: directory.to_path_buf(),
            availability,
        })
    }

    fn unique_id(&self, preferred: &str) -> Result<String, ActionStoreError> {
        for suffix in 0..1000 {
            let candidate = if suffix == 0 {
                preferred.to_owned()
            } else {
                format!("{preferred}-{suffix}")
            };
            if candidate.chars().count() > crate::model::MAX_ACTION_ID_CHARS {
                continue;
            }
            if !self.root.join(&candidate).exists() {
                return Ok(candidate);
            }
        }
        Err(ActionStoreError::Io(format!(
            "No free action name is available for “{preferred}”"
        )))
    }

    fn prepare_import(
        &self,
        source: &Path,
    ) -> Result<(ActionDefinition, Option<ActionScript>), ActionStoreError> {
        ensure_plain_directory(source)?;
        let source_manifest = source.join(MANIFEST_FILE);
        let text = match read_regular_text(&source_manifest, MAX_MANIFEST_BYTES) {
            Ok(text) => text,
            Err(ActionStoreError::NotFound(_)) => {
                return Err(ActionStoreError::NotAnActionDirectory(
                    source.display().to_string(),
                ));
            }
            Err(error) => return Err(error),
        };
        let definition = ActionDefinition::parse(&text)?;
        let script = match definition.run.script_entrypoint() {
            Some(entrypoint) => {
                let script_path = source.join(entrypoint);
                let contents = match read_regular_text(&script_path, MAX_SCRIPT_BYTES) {
                    Ok(contents) => contents,
                    Err(ActionStoreError::NotFound(_)) => {
                        return Err(ActionStoreError::MissingEntrypoint(entrypoint.to_owned()));
                    }
                    Err(error) => return Err(error),
                };
                definition.interpreter_for_source(&contents)?;
                Some(ActionScript {
                    file_name: entrypoint.to_owned(),
                    contents,
                })
            }
            None => None,
        };
        Ok((definition, script))
    }
}

impl ActionStore for LocalActionStore {
    fn load(&self) -> ActionCatalog {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return ActionCatalog::default();
            }
            Err(error) => {
                tracing::warn!(%error, root = %self.root.display(), "unable to read the actions directory");
                return ActionCatalog::default();
            }
        };
        let mut candidates: Vec<(String, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                // Ignore editor temporaries and notes, but report linked directories.
                if name.starts_with('.') || entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    return None;
                }
                Some((name, entry.path()))
            })
            .collect();
        candidates.sort_by(|left, right| left.0.cmp(&right.0));

        let mut actions = Vec::new();
        let mut failures = Vec::new();
        for (name, path) in candidates {
            match self.load_action(&name, &path) {
                Ok(handle) => actions.push(Rc::new(handle)),
                Err(error) => {
                    tracing::warn!(
                        action = %name,
                        %error,
                        "unable to load a custom action"
                    );
                    failures.push(ActionLoadFailure {
                        directory: name,
                        error,
                    });
                }
            }
        }
        ActionCatalog::new(actions, failures)
    }

    fn create(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError> {
        request.definition.validate()?;
        let directory = self.action_directory(&request.definition.id)?;
        ensure_private_directory(&self.root)?;
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    ActionStoreError::AlreadyExists(request.definition.id.clone())
                } else {
                    io_error(error)
                }
            })?;
        let result = self.write(request);
        if result.is_err() {
            let _removed = fs::remove_dir_all(&directory);
        }
        result
    }

    fn write(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError> {
        let definition = &request.definition;
        definition.validate()?;
        let directory = self.action_directory(&definition.id)?;
        ensure_private_directory(&self.root)?;
        ensure_private_directory(&directory)?;

        match (request.script.as_ref(), definition.run.script_entrypoint()) {
            (Some(script), Some(entrypoint)) => {
                if script.file_name != entrypoint {
                    return Err(ActionStoreError::Invalid(ActionError::InvalidEntrypoint(
                        script.file_name.clone(),
                    )));
                }
                if script.contents.len() as u64 > MAX_SCRIPT_BYTES {
                    return Err(ActionStoreError::Io(format!(
                        "“{entrypoint}” is larger than {} KiB",
                        MAX_SCRIPT_BYTES / 1024
                    )));
                }
                definition.interpreter_for_source(&script.contents)?;
                crate::storage::atomic_write(
                    &directory.join(entrypoint),
                    script.contents.as_bytes(),
                )
                .map_err(io_error)?;
            }
            (None, Some(entrypoint)) => {
                let script = directory.join(entrypoint);
                let metadata = fs::symlink_metadata(&script).map_err(|error| {
                    if error.kind() == io::ErrorKind::NotFound {
                        ActionStoreError::MissingEntrypoint(entrypoint.to_owned())
                    } else {
                        io_error(error)
                    }
                })?;
                if !metadata.file_type().is_file() {
                    return Err(ActionStoreError::NotARegularFile(entrypoint.to_owned()));
                }
            }
            (Some(_script), None) => {
                return Err(ActionStoreError::Invalid(ActionError::UnexpectedEntrypoint));
            }
            (None, None) => {}
        }

        let manifest = definition.to_manifest()?;
        crate::storage::atomic_write(&directory.join(MANIFEST_FILE), manifest.as_bytes())
            .map_err(io_error)
    }

    fn delete(&self, id: &str) -> Result<(), ActionStoreError> {
        let directory = self.action_directory(id)?;
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ActionStoreError::NotFound(id.to_owned()));
            }
            Err(error) => return Err(io_error(error)),
        };
        if !metadata.file_type().is_dir() {
            return Err(ActionStoreError::NotARegularFile(id.to_owned()));
        }
        ensure_plain_directory(&directory)?;
        fs::remove_dir_all(&directory).map_err(io_error)
    }

    fn read_script(&self, id: &str) -> Result<Option<ActionScript>, ActionStoreError> {
        let catalog = self.load();
        let handle = catalog
            .get(id)
            .ok_or_else(|| ActionStoreError::NotFound(id.to_owned()))?;
        let Some(entrypoint) = handle.definition.run.script_entrypoint() else {
            return Ok(None);
        };
        let contents = read_regular_text(&handle.directory.join(entrypoint), MAX_SCRIPT_BYTES)
            .map_err(|error| match error {
                ActionStoreError::NotFound(_) => {
                    ActionStoreError::MissingEntrypoint(entrypoint.to_owned())
                }
                other => other,
            })?;
        Ok(Some(ActionScript {
            file_name: entrypoint.to_owned(),
            contents,
        }))
    }

    fn import(&self, source: &Path) -> Result<String, ActionStoreError> {
        let (mut definition, script) = self.prepare_import(source)?;
        definition.enabled = false;
        let id = self.unique_id(&definition.id)?;
        definition.id = id.clone();
        ensure_private_directory(&self.root)?;
        let directory = self.action_directory(&id)?;
        if directory.exists() {
            return Err(ActionStoreError::AlreadyExists(id));
        }
        self.create(&ActionWriteRequest { definition, script })?;
        Ok(id)
    }

    fn export(&self, id: &str, destination: &Path) -> Result<PathBuf, ActionStoreError> {
        let catalog = self.load();
        let handle = catalog
            .get(id)
            .ok_or_else(|| ActionStoreError::NotFound(id.to_owned()))?;
        ensure_plain_directory(destination)?;
        let target = destination.join(id);
        DirBuilder::new()
            .mode(0o700)
            .create(&target)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    ActionStoreError::AlreadyExists(id.to_owned())
                } else {
                    io_error(error)
                }
            })?;

        if let Some(entrypoint) = handle.definition.run.script_entrypoint() {
            let contents = read_regular_text(&handle.directory.join(entrypoint), MAX_SCRIPT_BYTES)
                .map_err(|error| match error {
                    ActionStoreError::NotFound(_) => {
                        ActionStoreError::MissingEntrypoint(entrypoint.to_owned())
                    }
                    other => other,
                })?;
            crate::storage::atomic_write(&target.join(entrypoint), contents.as_bytes())
                .map_err(io_error)?;
        }
        let manifest = handle.definition.to_manifest()?;
        crate::storage::atomic_write(&target.join(MANIFEST_FILE), manifest.as_bytes())
            .map_err(io_error)?;
        Ok(target)
    }
}

fn resolve_availability(
    directory: &Path,
    definition: &ActionDefinition,
) -> Result<ActionAvailability, ActionStoreError> {
    match definition.run.runtime {
        ActionRuntime::Command => {
            let program = definition
                .run
                .program
                .as_deref()
                .expect("validated command actions have a program");
            let arguments = definition.run.argument_tokens()?;
            Ok(match resolve_executable(program) {
                Some(path) => ActionAvailability::Available(ActionProgram::Command {
                    program: path.into_os_string(),
                    arguments,
                }),
                None => ActionAvailability::Unavailable {
                    reason: ActionStoreError::MissingProgram(program.to_owned()).to_string(),
                },
            })
        }
        ActionRuntime::Python | ActionRuntime::Bash => {
            let entrypoint = definition
                .run
                .entrypoint
                .as_deref()
                .expect("validated script actions have an entrypoint");
            let script = directory.join(entrypoint);
            let metadata = match fs::symlink_metadata(&script) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(ActionStoreError::MissingEntrypoint(entrypoint.to_owned()));
                }
                Err(error) => return Err(io_error(error)),
            };
            if !metadata.file_type().is_file() {
                return Err(ActionStoreError::NotARegularFile(entrypoint.to_owned()));
            }
            let prefix = read_regular_prefix(&script, MAX_SHEBANG_PREFIX_BYTES)?;
            let declared = definition.interpreter_for_source(&prefix)?;
            let (program, interpreter_arguments, family) = match declared {
                Some(interpreter) => {
                    let family = crate::model::interpreter_family(&interpreter.program);
                    (interpreter.program, interpreter.arguments, family)
                }
                None => {
                    let program = definition
                        .run
                        .runtime
                        .default_interpreter()
                        .expect("script runtimes have a default interpreter");
                    let family = match definition.run.runtime {
                        ActionRuntime::Python => InterpreterFamily::Python,
                        _ => InterpreterFamily::Shell,
                    };
                    (program.to_owned(), Vec::new(), family)
                }
            };
            Ok(match resolve_executable(&program) {
                Some(path) => ActionAvailability::Available(ActionProgram::Script {
                    interpreter: path.into_os_string(),
                    interpreter_arguments: interpreter_arguments
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    script,
                    family,
                }),
                None => ActionAvailability::Unavailable {
                    reason: ActionStoreError::MissingInterpreter(program).to_string(),
                },
            })
        }
    }
}

/// Unlike sandbox helpers, trusted actions need the user's PATH, including shims.
pub(crate) fn resolve_executable(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        if !Path::new(program).is_absolute() {
            return None;
        }
        return is_executable_file(Path::new(program)).then(|| PathBuf::from(program));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|entry| !entry.as_os_str().is_empty())
        .map(|entry| entry.join(program))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn read_regular_text(path: &Path, max_bytes: u64) -> Result<String, ActionStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ActionStoreError::NotFound(file_label(path))
        } else {
            io_error(error)
        }
    })?;
    if !metadata.file_type().is_file() {
        return Err(ActionStoreError::NotARegularFile(file_label(path)));
    }
    if metadata.len() > max_bytes {
        return Err(ActionStoreError::Io(format!(
            "“{}” is larger than {} KiB",
            file_label(path),
            max_bytes / 1024
        )));
    }
    fs::read_to_string(path).map_err(io_error)
}

fn read_regular_prefix(path: &Path, max_bytes: u64) -> Result<String, ActionStoreError> {
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(io_error)?;
    let mut buffer = Vec::new();
    file.by_ref()
        .take(max_bytes)
        .read_to_end(&mut buffer)
        .map_err(io_error)?;
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .unwrap_or_else(|| OsStr::new("file"))
        .to_string_lossy()
        .into_owned()
}

fn ensure_private_directory(path: &Path) -> Result<(), ActionStoreError> {
    if path.exists() {
        return ensure_plain_directory(path);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        ensure_private_directory(parent)?;
    }
    DirBuilder::new().mode(0o700).create(path).map_err(io_error)
}

fn ensure_plain_directory(path: &Path) -> Result<(), ActionStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_dir() {
        return Err(ActionStoreError::Io(format!(
            "“{}” is not a folder",
            path.display()
        )));
    }
    Ok(())
}

fn io_error(error: io::Error) -> ActionStoreError {
    ActionStoreError::Io(error.to_string())
}
