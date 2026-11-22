// SPDX-License-Identifier: MIT

//! Locking encrypted volumes and mounts, forgetting cached LUKS passwords via Secret Service,
//! closing LUKS containers through UDisks2, and ejecting password-start drives
//! (`src/ui/window/device_lock.rs`, `volume_password.rs`, and `drive_ops.rs`).

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::Path,
    rc::Rc,
};

use gio::prelude::*;
use rune::Tx;
use strata_core::{i18n::tr, model::Location};
use strata_fs::adapters::{location_for_file, mount_watch};

use super::{
    crypto::{
        crypto_password_uuid_for_mount, crypto_password_uuid_for_volume, normalize_luks_uuid,
    },
    listing::unix_device,
    release::{
        self, ReleaseKey, ReleaseKind, ReleaseTarget, key_for_drive, key_for_mount, key_for_volume,
    },
};
use crate::{
    App,
    error_dialog::{self, Error},
    mount_prompts, shell,
    slots::WeakLater,
};

// GVfs and GNOME Disks use different Secret Service attribute names.
const LUKS_PASSWORD_ATTRIBUTES: [&str; 2] = ["gvfs-luks-uuid", "gvfs.crypto.luks.uuid"];
const SECRET_SERVICE: &str = "org.freedesktop.secrets";
const SECRET_SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SECRET_SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const SECRET_ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const SECRET_METHOD_TIMEOUT_MSEC: i32 = 2_000;

thread_local! {
    static LOCKING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// How an encrypted device is locked once its cleartext filesystem is unmounted.
pub(super) enum LockTarget {
    /// GVfs locks removable encrypted drives by stopping them.
    StopDrive(gio::Drive),
    /// Internal disks have no stoppable drive, so UDisks locks the container.
    Container {
        unix_device: Option<String>,
        luks_uuid: Option<String>,
    },
}

struct LockRequest {
    lock_id: String,
    volume_name: String,
    luks_uuid: Option<String>,
    mount: Option<gio::Mount>,
    target: Option<LockTarget>,
    key: ReleaseKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ForgetCachedPasswordError {
    NeedsConfirmation,
    ItemLocked,
    Failed(String),
}

impl ForgetCachedPasswordError {
    pub(super) fn user_message(&self) -> String {
        match self {
            Self::NeedsConfirmation => tr(
                "Strata cannot display the password manager's deletion confirmation. Remove this volume's saved password in your password manager, then try Lock again.",
            ),
            Self::ItemLocked => tr(
                "The saved password is locked in the password manager. Unlock the keyring and try again.",
            ),
            Self::Failed(message) => message.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum LockDeviceError {
    CommandFailed(String),
    MissingInterface(String),
    DeviceNotFound,
    Cancelled,
}

impl LockDeviceError {
    fn user_message(&self) -> String {
        match self {
            Self::CommandFailed(message) | Self::MissingInterface(message) => message.clone(),
            Self::DeviceNotFound => tr("Could not identify the drive's block device"),
            Self::Cancelled => String::new(),
        }
    }
}

/// Starts locking an encrypted volume: checks Secret Service for a cached passphrase, asks to
/// forget it if saved, then unmounts and locks the container or stops the drive.
pub(super) fn lock_encrypted_volume(
    app: &mut App,
    id: String,
    volume: &gio::Volume,
    label_id: Option<&str>,
) {
    let labels = shell::preferences(app).device_labels.clone();
    let key = key_for_volume(volume, label_id, &labels);
    let luks_uuid = crypto_password_uuid_for_volume(volume);
    let target = lock_target(
        volume.drive(),
        unix_device(volume).map(String::from),
        luks_uuid.clone(),
    );
    let request = LockRequest {
        lock_id: id,
        volume_name: key.display_name.clone(),
        luks_uuid,
        mount: volume.get_mount(),
        target,
        key,
    };
    request_encrypted_lock(app, request);
}

/// Starts locking an encrypted mount without a listed volume row.
pub(super) fn lock_encrypted_mount(
    app: &mut App,
    id: String,
    mount: &gio::Mount,
    label_id: Option<&str>,
) {
    let labels = shell::preferences(app).device_labels.clone();
    let key = key_for_mount(mount, label_id, &labels);
    let luks_uuid = crypto_password_uuid_for_mount(mount);
    let device = mount
        .volume()
        .and_then(|volume| unix_device(&volume))
        .map(String::from)
        .or_else(|| {
            mount
                .root()
                .path()
                .and_then(|root| block_device_for_path(&root))
        });
    let target = lock_target(mount.drive(), device, luks_uuid.clone());
    let request = LockRequest {
        lock_id: id,
        volume_name: key.display_name.clone(),
        luks_uuid,
        mount: Some(mount.clone()),
        target,
        key,
    };
    request_encrypted_lock(app, request);
}

/// Ejects an orphaned password-start drive through the unified release flow.
pub(super) fn eject_password_drive(app: &mut App, drive: gio::Drive, tx: &mut Tx) {
    if !drive.can_eject() {
        return;
    }
    let target = ReleaseTarget {
        key: key_for_drive(&drive),
        kind: ReleaseKind::Eject,
        unplug: true,
        sync_root: None,
        away_root: None,
    };
    release::begin_release(app, target, Some(tx), move |operation| async move {
        drive
            .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
            .await
    });
}

fn lock_target(
    drive: Option<gio::Drive>,
    unix_device: Option<String>,
    luks_uuid: Option<String>,
) -> Option<LockTarget> {
    if let Some(drive) = password_stop_drive(drive) {
        return Some(LockTarget::StopDrive(drive));
    }
    (unix_device.is_some() || luks_uuid.is_some()).then_some(LockTarget::Container {
        unix_device,
        luks_uuid,
    })
}

fn password_stop_drive(drive: Option<gio::Drive>) -> Option<gio::Drive> {
    drive.filter(|drive| {
        drive.start_stop_type() == gio::DriveStartStopType::Password && drive.can_stop()
    })
}

fn block_device_for_path(path: &Path) -> Option<String> {
    mount_watch::latest()
        .source_for(path)
        .filter(|source| source.starts_with("/dev"))
        .map(|source| source.to_string_lossy().into_owned())
}

/// RAII guard preventing duplicate lock attempts for the same device; releases on drop even
/// when the initiating window closes or an async step is abandoned.
#[derive(Debug)]
pub(super) struct LockGuard(String);

impl LockGuard {
    pub(super) fn acquire(id: impl Into<String>) -> Option<Self> {
        let id = id.into();
        LOCKING
            .with(|set| set.borrow_mut().insert(id.clone()))
            .then_some(Self(id))
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        LOCKING.with(|set| set.borrow_mut().remove(&self.0));
    }
}

fn request_encrypted_lock(app: &mut App, request: LockRequest) {
    if request.mount.is_none() && request.target.is_none() {
        error_dialog::show(
            app,
            Error::new(
                tr("Unable to lock device"),
                tr(
                    "This volume has no supported lock operation while unmounted. Mount it in Strata, then try Lock again.",
                ),
            ),
        );
        return;
    }
    let Some(guard) = LockGuard::acquire(request.lock_id.clone()) else {
        return;
    };
    let window_later = shell::window_later(app).downgrade();
    let uuid_for_check = request.luks_uuid.clone();
    glib::MainContext::default().spawn_local(async move {
        let cached = password_is_cached(uuid_for_check).await;
        let Some(later) = window_later.upgrade() else {
            return;
        };
        if cached {
            let volume_name = request.volume_name.clone();
            let confirm_later = window_later.clone();
            let shared_guard = Rc::new(RefCell::new(Some(guard)));
            let confirm_guard = Rc::clone(&shared_guard);
            let cancel_guard = Rc::clone(&shared_guard);
            later.then(move |app: &mut App, _: &mut Tx| {
                mount_prompts::show_forget_password(
                    app,
                    volume_name,
                    move || {
                        if let Some(guard) = confirm_guard.borrow_mut().take() {
                            spawn_forget_then_lock(request, guard, confirm_later);
                        }
                    },
                    move || {
                        let _ = cancel_guard.borrow_mut().take();
                    },
                );
                true
            });
        } else {
            later.then(move |app: &mut App, _: &mut Tx| {
                start_lock_release(app, request, guard);
                true
            });
        }
    });
}

fn spawn_forget_then_lock(request: LockRequest, guard: LockGuard, window_later: WeakLater<App>) {
    let Some(uuid) = request.luks_uuid.clone() else {
        if let Some(later) = window_later.upgrade() {
            later.then(move |app: &mut App, _: &mut Tx| {
                start_lock_release(app, request, guard);
                true
            });
        }
        return;
    };
    glib::MainContext::default().spawn_local(async move {
        let forget = gio::spawn_blocking(move || forget_cached_volume_password(&uuid)).await;
        let error_message = match forget {
            Ok(Ok(())) => {
                if let Some(later) = window_later.upgrade() {
                    later.then(move |app: &mut App, _: &mut Tx| {
                        start_lock_release(app, request, guard);
                        true
                    });
                }
                return;
            }
            Ok(Err(error)) => error.user_message(),
            Err(_) => tr("The saved password could not be deleted."),
        };
        drop(guard);
        if let Some(later) = window_later.upgrade() {
            later.then(move |app: &mut App, _: &mut Tx| {
                error_dialog::show(
                    app,
                    Error::new(tr("Couldn't forget the saved password"), error_message),
                );
                true
            });
        }
    });
}

fn start_lock_release(app: &mut App, request: LockRequest, guard: LockGuard) {
    let LockRequest {
        mount, target, key, ..
    } = request;
    let away_root: Option<Location> = mount
        .as_ref()
        .and_then(|mount| location_for_file(&mount.root()));
    let sync_root = mount.as_ref().and_then(|mount| mount.root().path());
    let release_target = ReleaseTarget {
        key,
        kind: ReleaseKind::Lock,
        unplug: false,
        sync_root,
        away_root,
    };
    // If `begin_release_with` declines a duplicate release, it drops the closure and `guard`.
    let _ = release::begin_release_with(
        app,
        release_target,
        (None, Box::new(move || drop(guard))),
        move |operation| async move {
            if let Some(mount) = mount {
                mount
                    .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                    .await?;
            }
            lock_target_device(target, &operation).await
        },
    );
}

async fn lock_target_device(
    target: Option<LockTarget>,
    operation: &gio::MountOperation,
) -> Result<(), glib::Error> {
    match target {
        Some(LockTarget::StopDrive(drive)) => {
            drive
                .stop_future(gio::MountUnmountFlags::NONE, Some(operation))
                .await
        }
        Some(LockTarget::Container {
            unix_device,
            luks_uuid,
        }) => lock_encrypted_device_on_system_bus(unix_device, luks_uuid).await,
        None => Ok(()),
    }
}

/// Lookup pairs for Secret Service: both attribute names and both hyphenated and compact UUIDs.
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

pub(super) fn forget_failure_from_delete_prompt(
    prompt_path: &str,
) -> Result<(), ForgetCachedPasswordError> {
    if prompt_path == "/" {
        Ok(())
    } else {
        Err(ForgetCachedPasswordError::NeedsConfirmation)
    }
}

pub(super) fn forget_failure_from_search(
    locked_item_count: usize,
) -> Result<(), ForgetCachedPasswordError> {
    if locked_item_count == 0 {
        Ok(())
    } else {
        Err(ForgetCachedPasswordError::ItemLocked)
    }
}

async fn password_is_cached(luks_uuid: Option<String>) -> bool {
    let Some(uuid) = luks_uuid else {
        return false;
    };
    gio::spawn_blocking(move || volume_password_is_cached(&uuid))
        .await
        .unwrap_or(false)
}

fn volume_password_is_cached(uuid: &str) -> bool {
    match search_cached_luks_items(uuid) {
        Ok((unlocked, locked)) => !unlocked.is_empty() || !locked.is_empty(),
        Err(error) => {
            tracing::debug!(%error, "unable to search cached volume password");
            false
        }
    }
}

fn search_cached_luks_items(uuid: &str) -> Result<(Vec<String>, Vec<String>), glib::Error> {
    let lookups = luks_password_lookups(uuid);
    if lookups.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
    search_items_on(&connection, &lookups)
}

fn search_items_on(
    connection: &gio::DBusConnection,
    lookups: &[(&'static str, String)],
) -> Result<(Vec<String>, Vec<String>), glib::Error> {
    let mut unlocked = Vec::new();
    let mut locked = Vec::new();
    for (key, value) in lookups {
        let mut attrs = HashMap::new();
        attrs.insert(*key, value.as_str());
        let reply = connection.call_sync(
            Some(SECRET_SERVICE),
            SECRET_SERVICE_PATH,
            SECRET_SERVICE_INTERFACE,
            "SearchItems",
            Some(&(attrs,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            SECRET_METHOD_TIMEOUT_MSEC,
            gio::Cancellable::NONE,
        )?;
        if let Some((u, l)) = reply.get::<(
            Vec<glib::variant::ObjectPath>,
            Vec<glib::variant::ObjectPath>,
        )>() {
            unlocked.extend(u.into_iter().map(|p| p.as_str().to_owned()));
            locked.extend(l.into_iter().map(|p| p.as_str().to_owned()));
        }
    }
    let mut seen = HashSet::new();
    unlocked.retain(|path| seen.insert(path.clone()));
    let mut seen_locked = HashSet::new();
    locked.retain(|path| seen_locked.insert(path.clone()));
    Ok((unlocked, locked))
}

fn forget_cached_volume_password(uuid: &str) -> Result<(), ForgetCachedPasswordError> {
    let lookups = luks_password_lookups(uuid);
    if lookups.is_empty() {
        return Ok(());
    }
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(forget_error_from_glib)?;
    let (unlocked, locked) =
        search_items_on(&connection, &lookups).map_err(forget_error_from_glib)?;
    forget_failure_from_search(locked.len())?;
    for path in unlocked {
        let reply = connection
            .call_sync(
                Some(SECRET_SERVICE),
                &path,
                SECRET_ITEM_INTERFACE,
                "Delete",
                None,
                None,
                gio::DBusCallFlags::NONE,
                SECRET_METHOD_TIMEOUT_MSEC,
                gio::Cancellable::NONE,
            )
            .map_err(forget_error_from_glib)?;
        let prompt = reply
            .get::<(glib::variant::ObjectPath,)>()
            .map(|(p,)| p.as_str().to_owned())
            .unwrap_or_default();
        forget_failure_from_delete_prompt(&prompt)?;
    }
    Ok(())
}

fn forget_error_from_glib(mut error: glib::Error) -> ForgetCachedPasswordError {
    if gio::DBusError::remote_error(&error)
        .is_some_and(|name| name == "org.freedesktop.Secret.Error.IsLocked")
    {
        return ForgetCachedPasswordError::ItemLocked;
    }
    gio::DBusError::strip_remote_error(&mut error);
    ForgetCachedPasswordError::Failed(error.message().to_owned())
}

/// Closes the LUKS container behind a volume via UDisks2 (`lock_encrypted_device` in `drive_ops.rs`).
pub(super) fn lock_encrypted_device(
    device: Option<&str>,
    luks_uuid: Option<&str>,
    mut call: impl FnMut(&str, &str, &str, &glib::Variant) -> Result<glib::Variant, LockDeviceError>,
) -> Result<(), LockDeviceError> {
    let mut blocks = Vec::new();
    if let Some(device) = device {
        blocks.extend(resolve_udisks_block(&mut call, "path", device)?);
    }
    if let Some(uuid) = luks_uuid {
        blocks.extend(resolve_udisks_block(&mut call, "uuid", uuid)?);
    }
    for block in blocks {
        let backing = udisks_object_property(
            &mut call,
            &block,
            "org.freedesktop.UDisks2.Block",
            "CryptoBackingDevice",
        )?
        .filter(|backing| backing != "/");
        let container = backing.unwrap_or(block);
        let cleartext = match udisks_object_property(
            &mut call,
            &container,
            "org.freedesktop.UDisks2.Encrypted",
            "CleartextDevice",
        ) {
            Ok(Some(cleartext)) => cleartext,
            Ok(None) | Err(LockDeviceError::MissingInterface(_)) => continue,
            Err(error) => return Err(error),
        };
        if cleartext != "/" {
            call(
                &container,
                "org.freedesktop.UDisks2.Encrypted",
                "Lock",
                &glib::Variant::tuple_from_iter([glib::VariantDict::new(None).end()]),
            )?;
        }
        return Ok(());
    }
    Err(LockDeviceError::DeviceNotFound)
}

fn resolve_udisks_block(
    call: &mut impl FnMut(&str, &str, &str, &glib::Variant) -> Result<glib::Variant, LockDeviceError>,
    key: &str,
    value: &str,
) -> Result<Option<String>, LockDeviceError> {
    let spec = glib::VariantDict::new(None);
    spec.insert(key, value);
    let resolved = call(
        "/org/freedesktop/UDisks2/Manager",
        "org.freedesktop.UDisks2.Manager",
        "ResolveDevice",
        &glib::Variant::tuple_from_iter([spec.end(), glib::VariantDict::new(None).end()]),
    )?;
    let (paths,) = resolved
        .get::<(Vec<glib::variant::ObjectPath>,)>()
        .ok_or(LockDeviceError::DeviceNotFound)?;
    Ok(match paths.as_slice() {
        [path] => Some(path.as_str().to_owned()),
        _ => None,
    })
}

fn udisks_object_property(
    call: &mut impl FnMut(&str, &str, &str, &glib::Variant) -> Result<glib::Variant, LockDeviceError>,
    object: &str,
    interface: &str,
    property: &str,
) -> Result<Option<String>, LockDeviceError> {
    let value = call(
        object,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(interface, property).to_variant(),
    )?;
    Ok(value
        .child_value(0)
        .as_variant()
        .and_then(|value| value.get::<glib::variant::ObjectPath>())
        .map(|path| path.as_str().to_owned()))
}

async fn lock_encrypted_device_on_system_bus(
    device: Option<String>,
    luks_uuid: Option<String>,
) -> Result<(), glib::Error> {
    let result = gio::spawn_blocking(move || {
        let connection =
            gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).map_err(lock_error)?;
        lock_encrypted_device(
            device.as_deref(),
            luks_uuid.as_deref(),
            |path, interface, method, parameters| {
                let timeout_msec = if method == "Lock" { i32::MAX } else { -1 };
                connection
                    .call_sync(
                        Some("org.freedesktop.UDisks2"),
                        path,
                        interface,
                        method,
                        Some(parameters),
                        None,
                        gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                        timeout_msec,
                        gio::Cancellable::NONE,
                    )
                    .map_err(lock_error)
            },
        )
    })
    .await
    .map_err(|_| LockDeviceError::CommandFailed(tr("Locking task did not complete")))
    .and_then(|result| result);

    result.map_err(|error| match error {
        LockDeviceError::Cancelled => glib::Error::new(gio::IOErrorEnum::FailedHandled, ""),
        other => glib::Error::new(gio::IOErrorEnum::Failed, &other.user_message()),
    })
}

fn lock_error(mut error: glib::Error) -> LockDeviceError {
    if gio::DBusError::remote_error(&error)
        .is_some_and(|name| name == "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed")
        || error.matches(gio::IOErrorEnum::Cancelled)
        || error.matches(gio::IOErrorEnum::FailedHandled)
    {
        return LockDeviceError::Cancelled;
    }
    let missing_interface = gio::DBusError::remote_error(&error).is_some_and(|name| {
        matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.InvalidArgs"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownProperty"
        )
    });
    gio::DBusError::strip_remote_error(&mut error);
    let message = error.message().to_owned();
    if missing_interface {
        LockDeviceError::MissingInterface(message)
    } else {
        LockDeviceError::CommandFailed(message)
    }
}
