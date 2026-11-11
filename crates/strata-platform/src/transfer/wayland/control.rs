//! Data control: the selection heard and written without focus or a serial.
//! `ext_data_control_v1` (the standard) and `zwlr_data_control_v1` (wlroots before 0.19) carry
//! the same events, so one macro writes the handlers for both.

use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

use super::super::data::SourceSpec;
use super::super::error::Error;
use super::super::event::{Event, Route, Selection};
use super::offer::AnyOffer;
use super::{Prefer, State};

pub(super) enum Control {
    Ext(ExtDataControlManagerV1, ExtDataControlDeviceV1),
    Wlr(ZwlrDataControlManagerV1, ZwlrDataControlDeviceV1),
}

pub(super) enum Source {
    Ext(ExtDataControlSourceV1),
    Wlr(ZwlrDataControlSourceV1),
}

impl Source {
    pub(super) fn destroy(&self) {
        match self {
            Source::Ext(s) => s.destroy(),
            Source::Wlr(s) => s.destroy(),
        }
    }
}

impl Control {
    /// The preferred protocol, else the other.
    pub(super) fn bind(
        globals: &GlobalList,
        qh: &QueueHandle<State>,
        seat: &WlSeat,
        prefer: Prefer,
    ) -> Option<Control> {
        let ext = || {
            let m = globals
                .bind::<ExtDataControlManagerV1, _, _>(qh, 1..=1, ())
                .ok()?;
            let d = m.get_data_device(seat, qh, ());
            Some(Control::Ext(m, d))
        };
        let wlr = || {
            let m = globals
                .bind::<ZwlrDataControlManagerV1, _, _>(qh, 1..=2, ())
                .ok()?;
            let d = m.get_data_device(seat, qh, ());
            Some(Control::Wlr(m, d))
        };
        match prefer {
            Prefer::Ext => ext().or_else(wlr),
            Prefer::Wlr => wlr().or_else(ext),
        }
    }

    pub(super) fn name(&self) -> &'static str {
        match self {
            Control::Ext(..) => "ext",
            Control::Wlr(..) => "wlr",
        }
    }

    pub(super) fn set_selection(
        &self,
        qh: &QueueHandle<State>,
        id: u64,
        selection: Selection,
        spec: &SourceSpec,
    ) -> Result<Source, Error> {
        match self {
            Control::Ext(m, d) => {
                let s = m.create_data_source(qh, id);
                for mime in spec.mimes() {
                    s.offer(mime.to_owned());
                }
                match selection {
                    Selection::Clipboard => d.set_selection(Some(&s)),
                    Selection::Primary => d.set_primary_selection(Some(&s)),
                }
                Ok(Source::Ext(s))
            }
            Control::Wlr(m, d) => {
                if selection == Selection::Primary && d.version() < 2 {
                    return Err(Error::Refused(
                        "zwlr_data_control v1 has no primary selection",
                    ));
                }
                let s = m.create_data_source(qh, id);
                for mime in spec.mimes() {
                    s.offer(mime.to_owned());
                }
                match selection {
                    Selection::Clipboard => d.set_selection(Some(&s)),
                    Selection::Primary => d.set_primary_selection(Some(&s)),
                }
                Ok(Source::Wlr(s))
            }
        }
    }

    /// A test switch: commits a protocol error (`invalid_offer`, an offer after
    /// `set_selection`), which ends the connection.
    pub(super) fn provoke_protocol_error(&self, qh: &QueueHandle<State>) {
        match self {
            Control::Ext(m, d) => {
                let s = m.create_data_source(qh, 0);
                d.set_selection(Some(&s));
                s.offer("text/plain".to_owned());
            }
            Control::Wlr(m, d) => {
                let s = m.create_data_source(qh, 0);
                d.set_selection(Some(&s));
                s.offer("text/plain".to_owned());
            }
        }
    }

    pub(super) fn destroy(self) {
        match self {
            Control::Ext(m, d) => {
                d.destroy();
                m.destroy();
            }
            Control::Wlr(m, d) => {
                d.destroy();
                m.destroy();
            }
        }
    }
}

macro_rules! handlers {
    ($manager:ty, $device:ty, $device_mod:ident, $offer:ty, $offer_mod:ident, $source:ty, $source_mod:ident, $any:ident) => {
        impl Dispatch<$manager, ()> for State {
            fn event(_: &mut Self, _: &$manager, _: <$manager as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }

        impl Dispatch<$device, ()> for State {
            event_created_child!(State, $device, [$device_mod::EVT_DATA_OFFER_OPCODE => ($offer, ())]);

            fn event(st: &mut Self, _: &$device, e: $device_mod::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
                match e {
                    $device_mod::Event::DataOffer { id } => {
                        st.announced.insert(id.id(), Vec::new());
                    }
                    $device_mod::Event::Selection { id } => {
                        let oid = id.as_ref().map(Proxy::id);
                        st.set_selection_offer(Selection::Clipboard, Route::DataControl, id.map(AnyOffer::$any), oid);
                    }
                    $device_mod::Event::PrimarySelection { id } => {
                        let oid = id.as_ref().map(Proxy::id);
                        st.set_selection_offer(Selection::Primary, Route::DataControl, id.map(AnyOffer::$any), oid);
                    }
                    $device_mod::Event::Finished => {
                        // The device is no longer valid (its seat went away).
                        if let Some(control) = st.control.take() {
                            control.destroy();
                        }
                        st.events.push(Event::Failed {
                            message: "the data-control device finished".to_owned(),
                        });
                    }
                    _ => {}
                }
            }
        }

        impl Dispatch<$offer, ()> for State {
            fn event(st: &mut Self, o: &$offer, e: $offer_mod::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
                if let $offer_mod::Event::Offer { mime_type } = e
                    && let Some(mimes) = st.announced.get_mut(&o.id())
                {
                    mimes.push(mime_type);
                }
            }
        }

        impl Dispatch<$source, u64> for State {
            fn event(st: &mut Self, _: &$source, e: $source_mod::Event, id: &u64, _: &Connection, _: &QueueHandle<Self>) {
                match e {
                    $source_mod::Event::Send { mime_type, fd } => st.source_send(*id, mime_type, fd),
                    $source_mod::Event::Cancelled => st.source_cancelled(*id),
                    _ => {}
                }
            }
        }
    };
}

handlers!(
    ExtDataControlManagerV1,
    ExtDataControlDeviceV1,
    ext_data_control_device_v1,
    ExtDataControlOfferV1,
    ext_data_control_offer_v1,
    ExtDataControlSourceV1,
    ext_data_control_source_v1,
    Ext
);
handlers!(
    ZwlrDataControlManagerV1,
    ZwlrDataControlDeviceV1,
    zwlr_data_control_device_v1,
    ZwlrDataControlOfferV1,
    zwlr_data_control_offer_v1,
    ZwlrDataControlSourceV1,
    zwlr_data_control_source_v1,
    Wlr
);
