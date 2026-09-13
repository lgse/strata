// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use super::devices::normalize_luks_uuid;
use zbus::zvariant::OwnedObjectPath;

#[cfg(test)]
mod tests;

/// GVfs stores LUKS passphrases under `gvfs-luks-uuid`; GNOME Disks uses
/// `gvfs.crypto.luks.uuid`. Search both, and both hyphenated and compact UUID forms.
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

pub(super) fn forget_cached_volume_password(uuid: &str) {
    if let Err(error) = async_io::block_on(delete_cached_luks_items(uuid)) {
        tracing::warn!(%error, "unable to forget cached volume password");
    }
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

async fn delete_cached_luks_items(uuid: &str) -> zbus::Result<()> {
    let connection = secret_connection().await?;
    for path in search_cached_luks_items_on(&connection, uuid).await? {
        let item = zbus::Proxy::new(
            &connection,
            SECRET_SERVICE,
            path.as_str(),
            SECRET_ITEM_INTERFACE,
        )
        .await?;
        let prompt: OwnedObjectPath = item.call("Delete", &()).await?;
        if prompt.as_str() != "/" {
            tracing::debug!(
                prompt = %prompt,
                "secret service asked to confirm deleting a cached volume password"
            );
        }
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
