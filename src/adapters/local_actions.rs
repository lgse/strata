// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs::{self, DirBuilder},
    io,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::adapters::mount_lanes::{self, Class, Unanswered};
use crate::model::{ActionDefinition, ActionError, ActionRuntime, InterpreterFamily};
use crate::services::actions::{
    ActionAvailability, ActionHandle, ActionLoadFailure, ActionProgram, ActionScript, ActionStore,
    ActionStoreError, ActionWriteRequest, StoredActions,
};
use crate::util::ui_thread;

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
    pub(crate) fn new() -> Arc<Self> {
        Self::at(actions_directory())
    }

    pub(crate) fn at(root: PathBuf) -> Arc<Self> {
        Arc::new(Self { root })
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

    fn existing_action_directory(&self, id: &str) -> Result<PathBuf, ActionStoreError> {
        let directory = self.action_directory(id)?;
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_dir() => Ok(directory),
            Ok(_) => Err(ActionStoreError::NotARegularFile(id.to_owned())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(ActionStoreError::NotFound(id.to_owned()))
            }
            Err(error) => Err(io_error(error)),
        }
    }

    fn load_action(&self, id: &str, directory: &Path) -> Result<ActionHandle, ActionStoreError> {
        let definition = definition_in(id, directory)?;
        let program = program_as_written(directory, &definition)?;
        Ok(ActionHandle {
            definition,
            directory: directory.to_path_buf(),
            availability: ActionAvailability::Available(program),
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
    fn load(&self) -> StoredActions {
        ui_thread::assert_off();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return StoredActions::default();
            }
            Err(error) => {
                tracing::warn!(%error, root = %self.root.display(), "unable to read the actions directory");
                return StoredActions::default();
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
                Ok(handle) => actions.push(handle),
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
        StoredActions { actions, failures }
    }

    fn locate(&self, mut stored: StoredActions) -> StoredActions {
        ui_thread::assert_off();
        let mut found: HashMap<OsString, Option<PathBuf>> = HashMap::new();
        for action in &mut stored.actions {
            let ActionAvailability::Available(program) = &mut action.availability else {
                continue;
            };
            let (written, missing): (&mut OsString, fn(String) -> ActionStoreError) = match program
            {
                ActionProgram::Command { program, .. } => {
                    (program, ActionStoreError::MissingProgram)
                }
                ActionProgram::Script { interpreter, .. } => {
                    (interpreter, ActionStoreError::MissingInterpreter)
                }
            };
            let name = written.to_string_lossy().into_owned();
            let located = found
                .entry(written.clone())
                .or_insert_with(|| resolve_executable(&name));
            match located {
                Some(path) => *written = path.clone().into_os_string(),
                None => {
                    action.availability = ActionAvailability::Unavailable {
                        reason: missing(name).user_message(),
                    };
                }
            }
        }
        stored
    }

    fn definition(&self, id: &str) -> Result<ActionDefinition, ActionStoreError> {
        ui_thread::assert_off();
        definition_in(id, &self.existing_action_directory(id)?)
    }

    fn read(&self, id: &str) -> Result<ActionWriteRequest, ActionStoreError> {
        ui_thread::assert_off();
        let directory = self.existing_action_directory(id)?;
        let definition = definition_in(id, &directory)?;
        let script = match definition.run.script_entrypoint() {
            Some(entrypoint) => Some(ActionScript {
                file_name: entrypoint.to_owned(),
                contents: read_entrypoint(&directory, entrypoint)?,
            }),
            None => None,
        };
        Ok(ActionWriteRequest { definition, script })
    }

    fn create(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError> {
        ui_thread::assert_off();
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
        ui_thread::assert_off();
        let definition = &request.definition;
        definition.validate()?;
        let directory = self.existing_action_directory(&definition.id)?;

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
        ui_thread::assert_off();
        fs::remove_dir_all(self.existing_action_directory(id)?).map_err(io_error)
    }

    fn read_import(&self, source: &Path) -> Result<ActionWriteRequest, ActionStoreError> {
        ui_thread::assert_off();
        let admitted = mount_lanes::admit(source, Class::Interactive).map_err(unanswered)?;
        let read = self.prepare_import(source);
        if read.is_ok() {
            admitted.progress().beat();
        }
        let (definition, script) = read?;
        Ok(ActionWriteRequest { definition, script })
    }

    fn import(&self, request: ActionWriteRequest) -> Result<String, ActionStoreError> {
        ui_thread::assert_off();
        let ActionWriteRequest {
            mut definition,
            script,
        } = request;
        definition.enabled = false;
        let id = self.unique_id(&definition.id)?;
        definition.id = id.clone();
        self.create(&ActionWriteRequest { definition, script })?;
        Ok(id)
    }

    fn export(
        &self,
        request: &ActionWriteRequest,
        destination: &Path,
    ) -> Result<PathBuf, ActionStoreError> {
        ui_thread::assert_off();
        let admitted = mount_lanes::admit(destination, Class::Interactive).map_err(unanswered)?;
        let exported = export_to(request, destination);
        if exported.is_ok() {
            admitted.progress().beat();
        }
        exported
    }
}

fn definition_in(id: &str, directory: &Path) -> Result<ActionDefinition, ActionStoreError> {
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
    Ok(definition)
}

fn read_entrypoint(directory: &Path, entrypoint: &str) -> Result<String, ActionStoreError> {
    read_regular_text(&directory.join(entrypoint), MAX_SCRIPT_BYTES).map_err(|error| match error {
        ActionStoreError::NotFound(_) => ActionStoreError::MissingEntrypoint(entrypoint.to_owned()),
        other => other,
    })
}

fn export_to(
    request: &ActionWriteRequest,
    destination: &Path,
) -> Result<PathBuf, ActionStoreError> {
    let id = &request.definition.id;
    ensure_plain_directory(destination)?;
    let target = destination.join(id);
    DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                ActionStoreError::AlreadyExists(id.clone())
            } else {
                io_error(error)
            }
        })?;
    if let Some(script) = &request.script {
        crate::storage::atomic_write(&target.join(&script.file_name), script.contents.as_bytes())
            .map_err(io_error)?;
    }
    let manifest = request.definition.to_manifest()?;
    crate::storage::atomic_write(&target.join(MANIFEST_FILE), manifest.as_bytes())
        .map_err(io_error)?;
    Ok(target)
}

fn unanswered(_: Unanswered) -> ActionStoreError {
    ActionStoreError::Io(crate::services::io_error_message(
        &io::ErrorKind::TimedOut.into(),
    ))
}

fn program_as_written(
    directory: &Path,
    definition: &ActionDefinition,
) -> Result<ActionProgram, ActionStoreError> {
    match definition.run.runtime {
        ActionRuntime::Command => {
            let program = definition
                .run
                .program
                .as_deref()
                .expect("validated command actions have a program");
            Ok(ActionProgram::Command {
                program: program.into(),
                arguments: definition.run.argument_tokens()?,
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
            Ok(ActionProgram::Script {
                interpreter: program.into(),
                interpreter_arguments: interpreter_arguments.into_iter().map(Into::into).collect(),
                script,
                family,
            })
        }
    }
}

/// Unlike sandbox helpers, trusted actions need the user's PATH, including shims.
pub(crate) fn resolve_executable(program: &str) -> Option<PathBuf> {
    ui_thread::assert_off();
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
    let Ok(admitted) = mount_lanes::admit(path.parent().unwrap_or(path), Class::Interactive) else {
        return false;
    };
    let metadata = fs::metadata(path);
    admitted.progress().answered(&metadata);
    metadata.is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
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
