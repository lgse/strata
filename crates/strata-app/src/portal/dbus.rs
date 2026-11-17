// SPDX-License-Identifier: MIT

//! D-Bus service for `org.freedesktop.impl.portal.FileChooser` and
//! `org.freedesktop.impl.portal.Request` on `gio::DBusConnection`.

use std::{
    cell::RefCell,
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt as _,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
};

use glib::prelude::*;

use super::{
    BACKEND_NAME, ChooserKind, ChooserRequest, FILE_CHOOSER_VERSION, FilterRule, MAX_STRING_BYTES,
    PortalCallbacks, PortalChoice, PortalError, PortalFileFilter, RequestTracker, SelectedResult,
    TrackedRequest, accessible_folder, default_accept_label, invalid_argument, request_title,
    save_file_suggestion, validate_common_request, validate_filename, validate_filters,
    validate_path, validate_save_filenames, window_geometry,
};

const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const REQUEST_PREFIX: &str = "/org/freedesktop/portal/desktop/request/";
const REQUEST_NAME_DO_NOT_QUEUE: u32 = 0x4;
const REQUEST_NAME_PRIMARY_OWNER: u32 = 1;
const REQUEST_NAME_ALREADY_OWNER: u32 = 4;

const FILE_CHOOSER_XML: &str = r#"
<node>
  <interface name="org.freedesktop.impl.portal.FileChooser">
    <property name="version" type="u" access="read"/>
    <method name="OpenFile">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFile">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFiles">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
  </interface>
</node>
"#;

const REQUEST_XML: &str = r#"
<node>
  <interface name="org.freedesktop.impl.portal.Request">
    <method name="Close"/>
  </interface>
</node>
"#;

type RawFilter = (String, Vec<(u32, String)>);
type RawChoice = (String, String, Vec<(String, String)>, String);

pub(crate) struct PortalService {
    connection: gio::DBusConnection,
    registration: Option<gio::RegistrationId>,
    name_lost: Option<gio::SignalSubscription>,
    pub(crate) requests: Arc<RequestTracker>,
}

impl PortalService {
    pub(crate) fn start(callbacks: PortalCallbacks) -> Result<Self, String> {
        let connection = connect_session_bus()?;
        let requests = Arc::new(RequestTracker::default());
        let chooser_info = gio::DBusNodeInfo::for_xml(FILE_CHOOSER_XML)
            .map_err(|error| format!("portal XML error: {error}"))?;
        let request_info = Rc::new(
            gio::DBusNodeInfo::for_xml(REQUEST_XML)
                .map_err(|error| format!("request XML error: {error}"))?,
        );
        let iface = chooser_info
            .lookup_interface("org.freedesktop.impl.portal.FileChooser")
            .ok_or_else(|| "missing FileChooser interface".to_owned())?;

        let tracker = requests.clone();
        let cb = Rc::new(callbacks);
        let cb_for_calls = cb.clone();
        let registration = connection
            .register_object(DESKTOP_PATH, &iface)
            .property(|_, _, _, _, _| FILE_CHOOSER_VERSION.to_variant())
            .method_call(move |conn, _, _, _, method, params, invocation| {
                let call = IncomingCall {
                    connection: conn,
                    method,
                    params,
                    invocation,
                };
                handle_chooser_call(call, (&tracker, &request_info, &cb_for_calls));
            })
            .build()
            .map_err(|error| format!("cannot register FileChooser on D-Bus: {error}"))?;

        claim_bus_name(&connection)?;
        let quit = cb.quit.clone();
        let name_lost = Some(connection.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameLost"),
            Some("/org/freedesktop/DBus"),
            Some(BACKEND_NAME),
            gio::DBusSignalFlags::NONE,
            move |_| quit(),
        ));

        Ok(Self {
            connection,
            registration: Some(registration),
            name_lost,
            requests,
        })
    }
}

impl Drop for PortalService {
    fn drop(&mut self) {
        self.name_lost.take();
        if let Some(id) = self.registration.take() {
            let _ = self.connection.unregister_object(id);
        }
    }
}

fn connect_session_bus() -> Result<gio::DBusConnection, String> {
    if let Ok(address) = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        && !address.trim().is_empty()
    {
        return gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .map_err(|error| format!("cannot connect to session bus: {error}"));
    }
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|error| format!("cannot connect to session bus: {error}"))
}

fn claim_bus_name(connection: &gio::DBusConnection) -> Result<(), String> {
    let reply = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(BACKEND_NAME, REQUEST_NAME_DO_NOT_QUEUE).to_variant()),
            Some(glib::VariantTy::new("(u)").expect("valid reply type")),
            gio::DBusCallFlags::NONE,
            5_000,
            gio::Cancellable::NONE,
        )
        .map_err(|error| format!("cannot request {BACKEND_NAME}: {error}"))?;
    let (code,) = reply
        .get::<(u32,)>()
        .ok_or_else(|| "invalid RequestName reply".to_owned())?;
    if matches!(
        code,
        REQUEST_NAME_PRIMARY_OWNER | REQUEST_NAME_ALREADY_OWNER
    ) {
        Ok(())
    } else {
        Err(format!("{BACKEND_NAME} is already owned (code {code})"))
    }
}

struct PendingCall {
    connection: gio::DBusConnection,
    request_reg: Option<gio::RegistrationId>,
    tracked: TrackedRequest,
    invocation: Option<gio::DBusMethodInvocation>,
}

impl PendingCall {
    fn complete(&mut self, result: Result<SelectedResult, PortalError>) {
        if let Some(reg) = self.request_reg.take() {
            let _ = self.connection.unregister_object(reg);
        }
        let Some(invocation) = self.invocation.take() else {
            return;
        };
        if self.tracked.cancelled.load(Ordering::SeqCst) {
            invocation.return_value(Some(&cancelled_reply()));
            return;
        }
        match result {
            Ok(files) => invocation.return_value(Some(&ok_reply(files))),
            Err(PortalError::Cancelled(_)) => invocation.return_value(Some(&cancelled_reply())),
            Err(error) => invocation.return_dbus_error(error.dbus_name(), error.message()),
        }
    }
}

impl Drop for PendingCall {
    fn drop(&mut self) {
        if self.invocation.is_some() {
            self.complete(Err(PortalError::Cancelled("request dropped".into())));
        }
    }
}

struct CallParams {
    token: String,
    app_id: String,
    parent: Option<String>,
    title: String,
    options: glib::Variant,
}

fn unpack_call(params: &glib::Variant) -> Result<CallParams, PortalError> {
    if params.n_children() != 5 {
        return invalid_argument("expected 5 FileChooser arguments");
    }
    let handle = params
        .child_value(0)
        .get::<glib::variant::ObjectPath>()
        .ok_or_else(|| PortalError::InvalidArgument("invalid request handle".into()))?;
    let token = handle.as_str().to_owned();
    if !token.starts_with(REQUEST_PREFIX) {
        return invalid_argument("invalid file chooser request path");
    }
    let app_id = params.child_value(1).get::<String>().unwrap_or_default();
    let parent = params
        .child_value(2)
        .get::<String>()
        .filter(|s| !s.is_empty());
    let title = params
        .child_value(3)
        .get::<String>()
        .ok_or_else(|| PortalError::InvalidArgument("invalid title".into()))?;
    let options = params.child_value(4);
    Ok(CallParams {
        token,
        app_id,
        parent,
        title,
        options,
    })
}

/// A FileChooser method call as GIO hands it over.
struct IncomingCall<'a> {
    connection: gio::DBusConnection,
    method: &'a str,
    params: glib::Variant,
    invocation: gio::DBusMethodInvocation,
}

fn handle_chooser_call(
    IncomingCall {
        connection,
        method,
        params,
        invocation,
    }: IncomingCall<'_>,
    (tracker, request_info, callbacks): (
        &Arc<RequestTracker>,
        &Rc<gio::DBusNodeInfo>,
        &Rc<PortalCallbacks>,
    ),
) {
    let parsed = match unpack_call(&params) {
        Ok(parsed) => parsed,
        Err(error) => {
            invocation.return_dbus_error(error.dbus_name(), error.message());
            return;
        }
    };
    let tracked = match tracker.begin(parsed.token.clone()) {
        Ok(tracked) => tracked,
        Err(error) => {
            invocation.return_dbus_error(error.dbus_name(), error.message());
            return;
        }
    };
    let request_reg = match register_request_object(
        &connection,
        &parsed.token,
        request_info,
        (tracker, callbacks),
    ) {
        Ok(reg) => reg,
        Err(error) => {
            invocation.return_dbus_error("org.freedesktop.portal.Error.Failed", &error);
            return;
        }
    };
    let pending = Rc::new(RefCell::new(PendingCall {
        connection,
        request_reg: Some(request_reg),
        tracked,
        invocation: Some(invocation),
    }));
    let method = method.to_owned();
    let callbacks = callbacks.clone();
    glib::MainContext::default().spawn_local(async move {
        dispatch_method(&method, parsed, pending, callbacks).await;
    });
}

fn register_request_object(
    connection: &gio::DBusConnection,
    token: &str,
    request_info: &gio::DBusNodeInfo,
    (tracker, callbacks): (&Arc<RequestTracker>, &Rc<PortalCallbacks>),
) -> Result<gio::RegistrationId, String> {
    let iface = request_info
        .lookup_interface("org.freedesktop.impl.portal.Request")
        .ok_or_else(|| "missing Request interface".to_owned())?;
    let token_owned = token.to_owned();
    let tracker = tracker.clone();
    let cancel_cb = callbacks.cancel.clone();
    connection
        .register_object(token, &iface)
        .method_call(move |_, _, _, _, method, _, invocation| {
            if method == "Close" {
                if tracker.cancel(&token_owned) {
                    cancel_cb(&token_owned);
                }
                invocation.return_value(None);
            } else {
                invocation.return_dbus_error(
                    "org.freedesktop.DBus.Error.UnknownMethod",
                    "unknown Request method",
                );
            }
        })
        .build()
        .map_err(|error| error.to_string())
}

async fn dispatch_method(
    method: &str,
    params: CallParams,
    pending: Rc<RefCell<PendingCall>>,
    callbacks: Rc<PortalCallbacks>,
) {
    let size_hint =
        window_geometry::parent_size_hint(&params.app_id, params.parent.as_deref()).await;
    let built = match method {
        "OpenFile" => build_open_request(params).await,
        "SaveFile" => build_save_file_request(params).await,
        "SaveFiles" => build_save_files_request(params).await,
        _ => Err(PortalError::InvalidArgument(
            "unknown FileChooser method".into(),
        )),
    };
    let mut request = match built {
        Ok(req) => req,
        Err(error) => {
            pending.borrow_mut().complete(Err(error));
            return;
        }
    };
    request.parent_size_hint = size_hint;
    let cancelled = pending.borrow().tracked.cancelled.clone();
    if !cancelled.load(Ordering::SeqCst) {
        window_geometry::prepare_chooser_placement().await;
    }
    let done = pending.clone();
    (callbacks.present)(
        request,
        cancelled,
        Box::new(move |result| done.borrow_mut().complete(result)),
    );
}

async fn build_open_request(params: CallParams) -> Result<ChooserRequest, PortalError> {
    let dict = glib::VariantDict::new(Some(&params.options));
    let accept_label = lookup_string(&dict, "accept_label")?;
    let modal = lookup_bool(&dict, "modal")?.unwrap_or(true);
    let multiple = lookup_bool(&dict, "multiple")?.unwrap_or(false);
    let directory = lookup_bool(&dict, "directory")?.unwrap_or(false);
    let filters = lookup_filters(&dict)?;
    let current_filter = lookup_current_filter(&dict)?;
    let choices = lookup_choices(&dict)?;
    let current_folder = lookup_bytestring_path(&dict, "current_folder")?;

    validate_common_request(
        &params.title,
        accept_label.as_deref(),
        params.parent.as_deref(),
        &choices,
    )?;
    validate_filters(&filters, current_filter.as_ref())?;
    validate_path(current_folder.as_deref(), "current folder")?;
    let initial_directory = accessible_folder(current_folder).await;
    Ok(ChooserRequest {
        token: params.token,
        title: request_title(&params.title, "Open Files"),
        accept_label: default_accept_label(accept_label.as_deref(), "Open"),
        modal,
        parent: params.parent,
        parent_size_hint: None,
        initial_directory,
        kind: ChooserKind::Open {
            directory,
            multiple,
        },
        filters,
        current_filter,
        choices,
    })
}

async fn build_save_file_request(params: CallParams) -> Result<ChooserRequest, PortalError> {
    let dict = glib::VariantDict::new(Some(&params.options));
    let accept_label = lookup_string(&dict, "accept_label")?;
    let modal = lookup_bool(&dict, "modal")?.unwrap_or(true);
    let filters = lookup_filters(&dict)?;
    let current_filter = lookup_current_filter(&dict)?;
    let choices = lookup_choices(&dict)?;
    let current_name = lookup_string(&dict, "current_name")?;
    let current_folder = lookup_bytestring_path(&dict, "current_folder")?;
    let current_file = lookup_bytestring_path(&dict, "current_file")?;

    validate_common_request(
        &params.title,
        accept_label.as_deref(),
        params.parent.as_deref(),
        &choices,
    )?;
    validate_filters(&filters, current_filter.as_ref())?;
    validate_path(current_file.as_deref(), "current file")?;
    validate_path(current_folder.as_deref(), "current folder")?;
    if let Some(name) = current_name.as_deref() {
        validate_filename(name)?;
    }
    let (initial_directory, current_name) =
        save_file_suggestion(current_file, current_folder, current_name).await;
    Ok(ChooserRequest {
        token: params.token,
        title: request_title(&params.title, "Save File"),
        accept_label: default_accept_label(accept_label.as_deref(), "Save"),
        modal,
        parent: params.parent,
        parent_size_hint: None,
        initial_directory,
        kind: ChooserKind::SaveFile { current_name },
        filters,
        current_filter,
        choices,
    })
}

async fn build_save_files_request(params: CallParams) -> Result<ChooserRequest, PortalError> {
    let dict = glib::VariantDict::new(Some(&params.options));
    let accept_label = lookup_string(&dict, "accept_label")?;
    let modal = lookup_bool(&dict, "modal")?.unwrap_or(true);
    let choices = lookup_choices(&dict)?;
    let current_folder = lookup_bytestring_path(&dict, "current_folder")?;
    let names = lookup_bytestring_list(&dict, "files")?;

    validate_common_request(
        &params.title,
        accept_label.as_deref(),
        params.parent.as_deref(),
        &choices,
    )?;
    validate_path(current_folder.as_deref(), "current folder")?;
    validate_save_filenames(&names)?;
    let initial_directory = accessible_folder(current_folder).await;
    Ok(ChooserRequest {
        token: params.token,
        title: request_title(&params.title, "Save Files"),
        accept_label: default_accept_label(accept_label.as_deref(), "Save"),
        modal,
        parent: params.parent,
        parent_size_hint: None,
        initial_directory,
        kind: ChooserKind::SaveFiles { names },
        filters: Vec::new(),
        current_filter: None,
        choices,
    })
}

fn lookup_string(dict: &glib::VariantDict, key: &str) -> Result<Option<String>, PortalError> {
    let Some(value) = dict.lookup_value(key, None) else {
        return Ok(None);
    };
    value
        .get::<String>()
        .map(Some)
        .ok_or_else(|| PortalError::InvalidArgument(format!("invalid {key} type")))
}

fn lookup_bool(dict: &glib::VariantDict, key: &str) -> Result<Option<bool>, PortalError> {
    let Some(value) = dict.lookup_value(key, None) else {
        return Ok(None);
    };
    value
        .get::<bool>()
        .map(Some)
        .ok_or_else(|| PortalError::InvalidArgument(format!("invalid {key} type")))
}

fn decode_c_bytes(variant: &glib::Variant, field: &str) -> Result<OsString, PortalError> {
    let bytes = variant
        .fixed_array::<u8>()
        .map_err(|_| PortalError::InvalidArgument(format!("invalid {field} byte array")))?;
    let trimmed = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if trimmed.len() > MAX_STRING_BYTES || trimmed.contains(&0) {
        return invalid_argument(format!("{field} is too long or contains a NUL byte"));
    }
    Ok(OsStr::from_bytes(trimmed).to_os_string())
}

fn lookup_bytestring_path(
    dict: &glib::VariantDict,
    key: &str,
) -> Result<Option<PathBuf>, PortalError> {
    let Some(value) = dict.lookup_value(key, None) else {
        return Ok(None);
    };
    let raw = decode_c_bytes(&value, key)?;
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(raw)))
}

fn lookup_bytestring_list(
    dict: &glib::VariantDict,
    key: &str,
) -> Result<Vec<OsString>, PortalError> {
    let Some(value) = dict.lookup_value(key, None) else {
        return Ok(Vec::new());
    };
    if !value.is_type(glib::VariantTy::BYTE_STRING_ARRAY) {
        return invalid_argument(format!("invalid {key} array type"));
    }
    (0..value.n_children())
        .map(|index| decode_c_bytes(&value.child_value(index), key))
        .collect()
}

fn parse_raw_filter((label, rules): RawFilter) -> Result<PortalFileFilter, PortalError> {
    let rules = rules
        .into_iter()
        .map(|(kind, rule)| match kind {
            0 => Ok(FilterRule::Pattern(rule)),
            1 => Ok(FilterRule::MimeType(rule)),
            _ => invalid_argument("unknown file filter rule kind"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PortalFileFilter { label, rules })
}

fn lookup_filters(dict: &glib::VariantDict) -> Result<Vec<PortalFileFilter>, PortalError> {
    let Some(value) = dict.lookup_value("filters", None) else {
        return Ok(Vec::new());
    };
    let raw = value
        .get::<Vec<RawFilter>>()
        .ok_or_else(|| PortalError::InvalidArgument("invalid filters type".into()))?;
    raw.into_iter().map(parse_raw_filter).collect()
}

fn lookup_current_filter(
    dict: &glib::VariantDict,
) -> Result<Option<PortalFileFilter>, PortalError> {
    let Some(value) = dict.lookup_value("current_filter", None) else {
        return Ok(None);
    };
    let raw = value
        .get::<RawFilter>()
        .ok_or_else(|| PortalError::InvalidArgument("invalid current_filter type".into()))?;
    parse_raw_filter(raw).map(Some)
}

fn lookup_choices(dict: &glib::VariantDict) -> Result<Vec<PortalChoice>, PortalError> {
    let Some(value) = dict.lookup_value("choices", None) else {
        return Ok(Vec::new());
    };
    let raw = value
        .get::<Vec<RawChoice>>()
        .ok_or_else(|| PortalError::InvalidArgument("invalid choices type".into()))?;
    Ok(raw
        .into_iter()
        .map(|(id, label, pairs, initial_selection)| PortalChoice {
            id,
            label,
            pairs,
            initial_selection,
        })
        .collect())
}

fn encode_raw_filter(filter: PortalFileFilter) -> RawFilter {
    let rules = filter
        .rules
        .into_iter()
        .map(|rule| match rule {
            FilterRule::Pattern(pattern) => (0u32, pattern),
            FilterRule::MimeType(mime) => (1u32, mime),
        })
        .collect();
    (filter.label, rules)
}

fn cancelled_reply() -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    glib::Variant::tuple_from_iter([1u32.to_variant(), dict.end()])
}

fn ok_reply(result: SelectedResult) -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    dict.insert_value("uris", &result.uris.to_variant());
    if !result.choices.is_empty() {
        dict.insert_value("choices", &result.choices.to_variant());
    }
    if let Some(filter) = result.current_filter {
        dict.insert_value("current_filter", &encode_raw_filter(filter).to_variant());
    }
    if let Some(writable) = result.writable {
        dict.insert_value("writable", &writable.to_variant());
    }
    glib::Variant::tuple_from_iter([0u32.to_variant(), dict.end()])
}

#[cfg(test)]
mod tests;
