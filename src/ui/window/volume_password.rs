// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    fmt,
    time::Duration,
};

use super::devices::normalize_luks_uuid;
use zbus::zvariant::OwnedObjectPath;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ForgetCachedPasswordError {
    NeedsConfirmation,
    ItemLocked,
    Failed(String),
}

impl fmt::Display for ForgetCachedPasswordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedsConfirmation => f.write_str(
                "Strata cannot display the password manager's deletion confirmation. Remove this volume's saved password in your password manager, then try Lock again.",
            ),
            Self::ItemLocked => f.write_str(
                "The saved password is locked in the password manager. Unlock the keyring and try again.",
            ),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

fn forget_delete_prompt_is_complete(prompt_path: &str) -> bool {
    prompt_path == "/"
}

fn forget_failure_from_delete_prompt(prompt_path: &str) -> Result<(), ForgetCachedPasswordError> {
    if forget_delete_prompt_is_complete(prompt_path) {
        Ok(())
    } else {
        Err(ForgetCachedPasswordError::NeedsConfirmation)
    }
}

fn forget_failure_from_search(locked_item_count: usize) -> Result<(), ForgetCachedPasswordError> {
    if locked_item_count == 0 {
        Ok(())
    } else {
        Err(ForgetCachedPasswordError::ItemLocked)
    }
}

fn delete_error_is_item_locked(name: &str) -> bool {
    name == "org.freedesktop.Secret.Error.IsLocked"
}

// GVfs and GNOME Disks use different Secret Service attribute names.
const LUKS_PASSWORD_ATTRIBUTES: [&str; 2] = ["gvfs-luks-uuid", "gvfs.crypto.luks.uuid"];
const SECRET_SERVICE: &str = "org.freedesktop.secrets";
const SECRET_SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SECRET_SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const SECRET_ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const SECRET_METHOD_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) fn volume_password_is_cached(uuid: &str) -> bool {
    match async_io::block_on(search_cached_luks_items(uuid)) {
        Ok(items) => !items.is_empty(),
        Err(error) => {
            tracing::debug!(%error, "unable to search cached volume password");
            false
        }
    }
}

pub(super) fn forget_cached_volume_password(uuid: &str) -> Result<(), ForgetCachedPasswordError> {
    async_io::block_on(delete_cached_luks_items(uuid))
}

pub(super) fn luks_password_lookups(uuid: &str) -> Vec<(&'static str, String)> {
    let Some(hyphenated) = normalize_luks_uuid(uuid) else {
        return Vec::new();
    };
    let compact: String = hyphenated.chars().filter(|ch| *ch != '-').collect();
    let mut lookups = Vec::with_capacity(LUKS_PASSWORD_ATTRIBUTES.len() * 2);
    for key in LUKS_PASSWORD_ATTRIBUTES {
        lookups.push((key, hyphenated.clone()));
        if compact != hyphenated {
            lookups.push((key, compact.clone()));
        }
    }
    lookups
}

async fn search_cached_luks_items(uuid: &str) -> zbus::Result<Vec<OwnedObjectPath>> {
    if luks_password_lookups(uuid).is_empty() {
        return Ok(Vec::new());
    }
    let connection = secret_connection().await?;
    search_cached_luks_items_on(&connection, uuid).await
}

async fn search_cached_luks_items_on(
    connection: &zbus::Connection,
    uuid: &str,
) -> zbus::Result<Vec<OwnedObjectPath>> {
    let lookups = luks_password_lookups(uuid);
    if lookups.is_empty() {
        return Ok(Vec::new());
    }
    let proxy = secret_service_proxy(connection).await?;
    let mut items = Vec::new();
    for (key, value) in lookups {
        let mut attributes = HashMap::new();
        attributes.insert(key, value.as_str());
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            proxy.call("SearchItems", &(attributes,)).await?;
        items.extend(unlocked);
        items.extend(locked);
    }
    let mut seen = HashSet::new();
    items.retain(|path| seen.insert(path.as_str().to_owned()));
    Ok(items)
}

fn forget_failure_from_zbus(error: zbus::Error) -> ForgetCachedPasswordError {
    if delete_error_is_item_locked(zbus_error_name(&error)) {
        ForgetCachedPasswordError::ItemLocked
    } else {
        ForgetCachedPasswordError::Failed(error.to_string())
    }
}

fn zbus_error_name(error: &zbus::Error) -> &str {
    match error {
        zbus::Error::MethodError(name, _, _) => name.as_str(),
        _ => "",
    }
}

async fn delete_cached_luks_items(uuid: &str) -> Result<(), ForgetCachedPasswordError> {
    let connection = secret_connection()
        .await
        .map_err(forget_failure_from_zbus)?;
    let lookups = luks_password_lookups(uuid);
    if lookups.is_empty() {
        return Ok(());
    }
    let proxy = secret_service_proxy(&connection)
        .await
        .map_err(forget_failure_from_zbus)?;
    let mut unlocked = Vec::new();
    let mut locked = Vec::new();
    for (key, value) in lookups {
        let mut attributes = HashMap::new();
        attributes.insert(key, value.as_str());
        let (found_unlocked, found_locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = proxy
            .call("SearchItems", &(attributes,))
            .await
            .map_err(forget_failure_from_zbus)?;
        unlocked.extend(found_unlocked);
        locked.extend(found_locked);
    }
    forget_failure_from_search(locked.len())?;
    let mut seen = HashSet::new();
    unlocked.retain(|path| seen.insert(path.as_str().to_owned()));
    for path in unlocked {
        let item = zbus::Proxy::new(
            &connection,
            SECRET_SERVICE,
            path.as_str(),
            SECRET_ITEM_INTERFACE,
        )
        .await
        .map_err(forget_failure_from_zbus)?;
        let prompt: OwnedObjectPath = item
            .call("Delete", &())
            .await
            .map_err(forget_failure_from_zbus)?;
        forget_failure_from_delete_prompt(prompt.as_str())?;
    }
    Ok(())
}

async fn secret_connection() -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .method_timeout(SECRET_METHOD_TIMEOUT)
        .build()
        .await
}

async fn secret_service_proxy(connection: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::Proxy::new(
        connection,
        SECRET_SERVICE,
        SECRET_SERVICE_PATH,
        SECRET_SERVICE_INTERFACE,
    )
    .await
}
