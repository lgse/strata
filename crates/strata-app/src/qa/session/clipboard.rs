// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    io::{Read as _, Write as _},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fd::{AsFd as _, OwnedFd},
    pipe::pipe,
};
use strata_platform::clipboard::TEXT_TYPES;
use strata_qa::{ClipboardContent, DEFAULT_WAIT};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy as _, QueueHandle,
    backend::ObjectId,
    delegate_noop, event_created_child,
    globals::{GlobalList, GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_seat::WlSeat},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

const APPEAR_WAIT: Duration = Duration::from_secs(1);

type Offers = Arc<Vec<(String, Vec<u8>)>>;

#[derive(Default)]
struct ClientState {
    offers: HashMap<ObjectId, Vec<String>>,
    current: Option<ZwlrDataControlOfferV1>,
    cancelled: bool,
}

impl ClientState {
    fn set_current(&mut self, next: Option<ZwlrDataControlOfferV1>) {
        if let Some(old) = self.current.take() {
            self.offers.remove(&old.id());
            old.destroy();
        }
        self.current = next;
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for ClientState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(ClientState: ignore WlSeat);
delegate_noop!(ClientState: ignore ZwlrDataControlManagerV1);

impl Dispatch<ZwlrDataControlDeviceV1, ()> for ClientState {
    event_created_child!(ClientState, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            zwlr_data_control_device_v1::Event::Selection { id } => state.set_current(id),
            zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                state.offers.remove(&offer.id());
                offer.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offers.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

impl Dispatch<ZwlrDataControlSourceV1, Offers> for ClientState {
    fn event(
        state: &mut Self,
        source: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        offers: &Offers,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                let bytes = offers
                    .iter()
                    .find(|(mime, _)| *mime == mime_type)
                    .map(|(_, data)| data.clone());
                if let Some(bytes) = bytes {
                    let _ = thread::Builder::new()
                        .name("qa-clipboard-send".to_owned())
                        .spawn(move || {
                            let mut file = std::fs::File::from(fd);
                            let _ = file.write_all(&bytes);
                        });
                }
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                state.cancelled = true;
                source.destroy();
            }
            _ => {}
        }
    }
}

fn bind<I>(
    globals: &GlobalList,
    handle: &QueueHandle<ClientState>,
    versions: std::ops::RangeInclusive<u32>,
    name: &str,
) -> Result<I, String>
where
    I: wayland_client::Proxy + 'static,
    ClientState: Dispatch<I, ()>,
{
    globals
        .bind(handle, versions, ())
        .map_err(|error| format!("the compositor has no usable {name}: {error}"))
}

fn connect() -> Result<
    (
        Connection,
        EventQueue<ClientState>,
        ZwlrDataControlManagerV1,
        ZwlrDataControlDeviceV1,
    ),
    String,
> {
    let connection = Connection::connect_to_env()
        .map_err(|error| format!("cannot connect to the Wayland compositor: {error}"))?;
    let (globals, queue) = registry_queue_init::<ClientState>(&connection)
        .map_err(|error| format!("cannot list the Wayland globals: {error}"))?;
    let handle = queue.handle();
    let seat: WlSeat = bind(&globals, &handle, 1..=7, "wl_seat")?;
    let manager: ZwlrDataControlManagerV1 =
        bind(&globals, &handle, 1..=2, "zwlr_data_control_manager_v1")?;
    let device = manager.get_data_device(&seat, &handle, ());
    Ok((connection, queue, manager, device))
}

fn offers_for(content: &ClipboardContent) -> Vec<(String, Vec<u8>)> {
    match content {
        ClipboardContent::Text(text) => ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
            .into_iter()
            .map(|mime| (mime.to_owned(), text.as_bytes().to_vec()))
            .collect(),
        ClipboardContent::Offers(list) => list
            .offers
            .iter()
            .map(|offer| (offer.mime.clone(), offer.text.as_bytes().to_vec()))
            .collect(),
    }
}

/// Owns the Wayland clipboard from a second client connection until dropped or superseded.
pub(super) struct ClipboardOwner {
    stop: Option<OwnedFd>,
    thread: Option<JoinHandle<()>>,
}

impl ClipboardOwner {
    pub(super) fn spawn(content: &ClipboardContent) -> Result<Self, String> {
        let offers = Arc::new(offers_for(content));
        let (stop_read, stop_write) =
            pipe().map_err(|error| format!("cannot open a clipboard pipe: {error}"))?;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("qa-clipboard".to_owned())
            .spawn(move || match setup_owner(offers) {
                Ok((connection, queue, state)) => {
                    let _ = ready_tx.send(Ok(()));
                    pump_owner(connection, queue, state, stop_read);
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .map_err(|error| format!("cannot start the clipboard owner: {error}"))?;
        ready_rx
            .recv()
            .map_err(|_| "the clipboard owner exited before taking the selection".to_owned())??;
        Ok(Self {
            stop: Some(stop_write),
            thread: Some(thread),
        })
    }
}

impl Drop for ClipboardOwner {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn setup_owner(
    offers: Offers,
) -> Result<(Connection, EventQueue<ClientState>, ClientState), String> {
    let (connection, mut queue, manager, device) = connect()?;
    let handle = queue.handle();
    if offers.is_empty() {
        device.set_selection(None);
    } else {
        let source = manager.create_data_source(&handle, Arc::clone(&offers));
        let mut seen = HashSet::new();
        for (mime, _) in offers.iter() {
            if seen.insert(mime.as_str()) {
                source.offer(mime.clone());
            }
        }
        device.set_selection(Some(&source));
    }
    let mut state = ClientState::default();
    queue
        .roundtrip(&mut state)
        .map_err(|error| format!("cannot own the clipboard: {error}"))?;
    Ok((connection, queue, state))
}

fn poll_owner_step(queue: &EventQueue<ClientState>, stop: &OwnedFd) -> bool {
    let Some(guard) = queue.prepare_read() else {
        return true;
    };
    let conn_fd = guard.connection_fd();
    let mut fds = [
        PollFd::new(&conn_fd, PollFlags::IN),
        PollFd::new(stop, PollFlags::IN),
    ];
    match poll(&mut fds, None) {
        Err(rustix::io::Errno::INTR) => {
            drop(guard);
            true
        }
        Ok(_) if fds[1].revents().is_empty() => {
            if fds[0].revents().contains(PollFlags::IN) {
                let _ = guard.read();
            } else {
                drop(guard);
            }
            true
        }
        _ => {
            drop(guard);
            false
        }
    }
}

fn pump_owner(
    connection: Connection,
    mut queue: EventQueue<ClientState>,
    mut state: ClientState,
    stop: OwnedFd,
) {
    while !state.cancelled {
        let _ = queue.dispatch_pending(&mut state);
        if state.cancelled {
            break;
        }
        let _ = connection.flush();
        if !poll_owner_step(&queue, &stop) {
            break;
        }
    }
    state.set_current(None);
}

fn timeout_spec(remaining: Duration) -> Timespec {
    Timespec {
        tv_sec: i64::try_from(remaining.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(remaining.subsec_nanos()),
    }
}

fn poll_offer_step(queue: &EventQueue<ClientState>, remaining: Duration) -> bool {
    let Some(guard) = queue.prepare_read() else {
        return true;
    };
    let conn_fd = guard.connection_fd();
    let mut fds = [PollFd::new(&conn_fd, PollFlags::IN)];
    let spec = timeout_spec(remaining);
    match poll(&mut fds, Some(&spec)) {
        Err(rustix::io::Errno::INTR) => {
            drop(guard);
            true
        }
        Ok(ready) if ready > 0 && fds[0].revents().contains(PollFlags::IN) => {
            let _ = guard.read();
            true
        }
        _ => {
            drop(guard);
            false
        }
    }
}

fn wait_for_offer(
    connection: &Connection,
    queue: &mut EventQueue<ClientState>,
    state: &mut ClientState,
) {
    let deadline = Instant::now() + APPEAR_WAIT;
    while state.current.is_none() {
        let _ = queue.dispatch_pending(state);
        if state.current.is_some() {
            break;
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let _ = connection.flush();
        if !poll_offer_step(queue, remaining) {
            break;
        }
    }
}

fn choose_mime(offered: &[String], requested: Option<&str>) -> Option<String> {
    match requested {
        Some(mime) => offered.iter().find(|entry| entry.as_str() == mime).cloned(),
        None => TEXT_TYPES.iter().find_map(|mime| {
            offered
                .iter()
                .find(|entry| entry.as_str() == *mime)
                .cloned()
        }),
    }
}

/// Reads `mime` (or plain text when `None`) from the Wayland clipboard via a second client.
pub(super) fn read(mime: Option<&str>) -> Result<Option<String>, String> {
    let (connection, mut queue, _manager, _device) = connect()?;
    let mut state = ClientState::default();
    queue
        .roundtrip(&mut state)
        .map_err(|error| format!("cannot query the clipboard: {error}"))?;
    wait_for_offer(&connection, &mut queue, &mut state);

    let chosen = state.current.as_ref().and_then(|offer| {
        let offered = state.offers.get(&offer.id())?;
        choose_mime(offered, mime)
    });
    let (Some(offer), Some(chosen)) = (state.current.as_ref(), chosen) else {
        state.set_current(None);
        return Ok(None);
    };

    let (reader, writer) = pipe().map_err(|error| format!("cannot open a read pipe: {error}"))?;
    offer.receive(chosen, writer.as_fd());
    drop(writer);
    let (tx, rx) = mpsc::sync_channel(1);
    let _ = thread::Builder::new()
        .name("qa-clipboard-read".to_owned())
        .spawn(move || {
            let mut file = std::fs::File::from(reader);
            let mut bytes = Vec::new();
            let result = file.read_to_end(&mut bytes).map(|_| bytes);
            let _ = tx.send(result);
        });
    connection
        .flush()
        .map_err(|error| format!("cannot request the clipboard payload: {error}"))?;
    let outcome = match rx.recv_timeout(DEFAULT_WAIT) {
        Ok(Ok(bytes)) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        _ => Ok(None),
    };
    state.set_current(None);
    outcome
}
