// SPDX-License-Identifier: MIT

//! Peers a scenario runs beside the application: the file-chooser portal client, an
//! image server, `tests/qa/fixtures/file_drag_source.py`, and a ZipCrypto writer.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

use gtk::{gio, gio::prelude::*, glib};
use serde::Deserialize;

const BACKEND: &str = "org.freedesktop.impl.portal.desktop.strata";
const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.impl.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.impl.portal.Request";
const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(20);
const OPEN_FILE_TIMEOUT_MS: i32 = 60_000;
const CLOSE_TIMEOUT_MS: i32 = 2_000;
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenOptions {
    current_folder: Option<String>,
    multiple: Option<bool>,
    #[serde(default)]
    filters: Vec<(String, Vec<(u32, String)>)>,
}

impl OpenOptions {
    fn variant(&self) -> glib::Variant {
        let dictionary = glib::VariantDict::new(None);
        if let Some(folder) = &self.current_folder {
            let mut bytes = folder.as_bytes().to_vec();
            bytes.push(0);
            dictionary.insert_value(
                "current_folder",
                &glib::Variant::array_from_fixed_array(&bytes),
            );
        }
        if let Some(multiple) = self.multiple {
            dictionary.insert_value("multiple", &multiple.to_variant());
        }
        if !self.filters.is_empty() {
            dictionary.insert_value("filters", &self.filters.to_variant());
        }
        dictionary.end()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Answer {
    pub(crate) status: u32,
    pub(crate) paths: Vec<PathBuf>,
}

pub(crate) struct PortalRequest {
    connection: gio::DBusConnection,
    handle: String,
    pending: Receiver<Result<Answer, String>>,
    answer: Option<Result<Answer, String>>,
}

impl PortalRequest {
    pub(crate) fn send(bus: &str, title: &str, options: &OpenOptions) -> Result<Self, String> {
        let connection = gio::DBusConnection::for_address_sync(
            bus,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .map_err(|error| format!("cannot connect to the session bus: {error}"))?;
        let deadline = Instant::now() + REGISTRATION_TIMEOUT;
        loop {
            let owned = connection
                .call_sync(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "NameHasOwner",
                    Some(&(BACKEND,).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    1000,
                    gio::Cancellable::NONE,
                )
                .ok()
                .and_then(|reply| reply.get::<(bool,)>())
                .is_some_and(|(owned,)| owned);
            if owned {
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out after {}s waiting for portal backend registration",
                    REGISTRATION_TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        let token: String = (0..16).map(|_| format!("{:02x}", rand_byte())).collect();
        let handle = format!("{DESKTOP}/request/strata_test/request_{token}");
        let parameters = glib::Variant::tuple_from_iter([
            glib::variant::ObjectPath::try_from(handle.clone())
                .map_err(|error| error.to_string())?
                .to_variant(),
            "".to_variant(),
            "".to_variant(),
            title.to_variant(),
            options.variant(),
        ]);
        let (sender, pending) = mpsc::channel();
        let caller = connection.clone();
        std::thread::spawn(move || {
            let reply = caller
                .call_sync(
                    Some(BACKEND),
                    DESKTOP,
                    FILE_CHOOSER,
                    "OpenFile",
                    Some(&parameters),
                    Some(glib::VariantTy::new("(ua{sv})").expect("a valid type")),
                    gio::DBusCallFlags::NONE,
                    OPEN_FILE_TIMEOUT_MS,
                    gio::Cancellable::NONE,
                )
                .map_err(|error| format!("OpenFile failed: {error}"))
                .and_then(|reply| answer(&reply));
            let _ = sender.send(reply);
        });
        Ok(Self {
            connection,
            handle,
            pending,
            answer: None,
        })
    }

    pub(crate) fn poll(&mut self) -> Option<&Result<Answer, String>> {
        if self.answer.is_none() {
            match self.pending.try_recv() {
                Ok(answer) => self.answer = Some(answer),
                Err(TryRecvError::Disconnected) => {
                    self.answer = Some(Err("the request's worker went away".to_owned()));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        self.answer.as_ref()
    }
}

impl Drop for PortalRequest {
    fn drop(&mut self) {
        if self.poll().is_none() {
            let _ = self.connection.call_sync(
                Some(BACKEND),
                &self.handle,
                REQUEST,
                "Close",
                None,
                None,
                gio::DBusCallFlags::NONE,
                CLOSE_TIMEOUT_MS,
                gio::Cancellable::NONE,
            );
        }
    }
}

fn answer(reply: &glib::Variant) -> Result<Answer, String> {
    let status = reply
        .child_value(0)
        .get::<u32>()
        .ok_or("the response has no status")?;
    let results = glib::VariantDict::new(Some(&reply.child_value(1)));
    let uris: Vec<String> = results
        .lookup_value("uris", None)
        .and_then(|uris| uris.get::<Vec<String>>())
        .unwrap_or_default();
    let paths = uris
        .iter()
        .map(|uri| {
            gio::File::for_uri(uri)
                .path()
                .ok_or_else(|| format!("{uri} is not a local file"))
        })
        .collect::<Result<_, _>>()?;
    Ok(Answer { status, paths })
}

fn rand_byte() -> u8 {
    let mut byte = [0];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut byte));
    byte[0]
}

pub(crate) struct ImageServer {
    pub(crate) url: String,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl ImageServer {
    pub(crate) fn start(body: Vec<u8>) -> Result<Self, String> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|error| format!("cannot listen: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (counted, stopped) = (requests.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let counted = counted.clone();
                        let body = body.clone();
                        std::thread::spawn(move || serve(stream, &body, &counted));
                    }
                    Err(_) => std::thread::sleep(POLL_INTERVAL),
                }
            }
        });
        Ok(Self {
            url: format!("http://127.0.0.1:{port}/download.png"),
            requests,
            stop,
        })
    }

    pub(crate) fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for ImageServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn serve(stream: std::net::TcpStream, body: &[u8], requests: &AtomicUsize) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || !line.starts_with("GET ") {
        return;
    }
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => break,
            Ok(_) if header == "\r\n" || header == "\n" => break,
            Ok(_) => {}
        }
    }
    requests.fetch_add(1, Ordering::SeqCst);
    let mut stream = stream;
    let head = format!(
        "HTTP/1.0 200 OK\r\nServer: BaseHTTP/0.6 Python\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.flush());
}

#[derive(Default)]
struct Reported {
    bounds: Option<[i64; 4]>,
    label: Option<String>,
}

pub(crate) struct ForeignSource {
    child: Child,
    reported: Arc<Mutex<Reported>>,
}

impl ForeignSource {
    pub(crate) fn start(
        python: &str,
        repository: &Path,
        arguments: &[String],
        environment: &[(String, String)],
        log: &Path,
    ) -> Result<Self, String> {
        let errors = std::fs::File::create(log).map_err(|error| error.to_string())?;
        let mut command = Command::new(python);
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command
            .arg(repository.join("tests/qa/foreign_drag_source.py"))
            .arg(repository.join("tests/qa/fixtures/file_drag_source.py"))
            .args(arguments)
            .env_clear()
            .envs(environment.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(errors);
        let mut child = super::spawn(&mut command)
            .map_err(|error| format!("cannot start the foreign drag source: {error}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let reported = Arc::new(Mutex::new(Reported::default()));
        let shared = reported.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let mut reported = shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(rest) = line.strip_prefix("bounds ") {
                    let numbers: Vec<i64> = rest
                        .split(' ')
                        .filter_map(|part| part.parse().ok())
                        .collect();
                    if let [x, y, width, height] = numbers[..] {
                        reported.bounds = Some([x, y, width, height]);
                    }
                } else if let Some(label) = line.strip_prefix("label ") {
                    reported.label = Some(label.to_owned());
                }
            }
        });
        Ok(Self { child, reported })
    }

    pub(crate) fn bounds(&self) -> Option<[i64; 4]> {
        self.lock().bounds
    }

    pub(crate) fn label(&self) -> Option<String> {
        self.lock().label.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Reported> {
        self.reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn stop(&mut self) {
        let group = rustix::process::Pid::from_child(&self.child);
        for signal in [rustix::process::Signal::TERM, rustix::process::Signal::KILL] {
            let _ = rustix::process::kill_process_group(group, signal);
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
        let _ = self.child.wait();
    }
}

impl Drop for ForeignSource {
    fn drop(&mut self) {
        self.stop();
    }
}

fn crc32_byte(byte: u8, crc: u32) -> u32 {
    let mut value = (crc ^ u32::from(byte)) & 0xFF;
    for _ in 0..8 {
        value = if value & 1 == 1 {
            (value >> 1) ^ 0xEDB8_8320
        } else {
            value >> 1
        };
    }
    (crc >> 8) ^ value
}

struct Keys([u32; 3]);

impl Keys {
    fn new(password: &[u8]) -> Self {
        let mut keys = Self([305_419_896, 591_751_049, 878_082_192]);
        for byte in password {
            keys.update(*byte);
        }
        keys
    }

    fn update(&mut self, byte: u8) {
        let [key0, key1, key2] = &mut self.0;
        *key0 = crc32_byte(byte, *key0);
        *key1 = key1.wrapping_add(*key0 & 0xFF);
        *key1 = key1.wrapping_mul(134_775_813).wrapping_add(1);
        *key2 = crc32_byte((*key1 >> 24) as u8, *key2);
    }

    fn stream_byte(&self) -> u8 {
        let key = self.0[2] | 2;
        ((key.wrapping_mul(key ^ 1) >> 8) & 0xFF) as u8
    }

    fn encrypt(&mut self, plain: u8) -> u8 {
        let cipher = plain ^ self.stream_byte();
        self.update(plain);
        cipher
    }

    fn decrypt(&mut self, cipher: u8) -> u8 {
        let plain = cipher ^ self.stream_byte();
        self.update(plain);
        plain
    }
}

pub(crate) fn write_zipcrypto(
    path: &Path,
    password: &str,
    name: &str,
    contents: &[u8],
    deflated: bool,
) -> Result<(), String> {
    let mut crc = flate2::Crc::new();
    crc.update(contents);
    let crc = crc.sum();
    let payload = if deflated {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(9));
        encoder
            .write_all(contents)
            .map_err(|error| error.to_string())?;
        encoder.finish().map_err(|error| error.to_string())?
    } else {
        contents.to_vec()
    };
    let mut header = [0u8; 12];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut header[..11]))
        .map_err(|error| error.to_string())?;
    header[11] = (crc >> 24) as u8;
    let mut keys = Keys::new(password.as_bytes());
    let encrypted: Vec<u8> = header
        .iter()
        .chain(&payload)
        .map(|byte| keys.encrypt(*byte))
        .collect();
    let name = name.as_bytes();
    let method: u16 = if deflated { 8 } else { 0 };
    let flags: u16 = 0x0001;
    let sizes = |data: &mut Vec<u8>| {
        data.extend_from_slice(&crc.to_le_bytes());
        data.extend_from_slice(
            &u32::try_from(encrypted.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        data.extend_from_slice(
            &u32::try_from(contents.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
    };
    let name_length = u16::try_from(name.len()).map_err(|error| error.to_string())?;
    let mut local = Vec::new();
    local.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
    for field in [20u16, flags, method, 0, 0] {
        local.extend_from_slice(&field.to_le_bytes());
    }
    sizes(&mut local);
    local.extend_from_slice(&name_length.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(name);
    local.extend_from_slice(&encrypted);
    let mut central = Vec::new();
    central.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
    for field in [20u16, 20, flags, method, 0, 0] {
        central.extend_from_slice(&field.to_le_bytes());
    }
    sizes(&mut central);
    for field in [name_length, 0, 0, 0, 0] {
        central.extend_from_slice(&field.to_le_bytes());
    }
    central.extend_from_slice(&0u32.to_le_bytes());
    central.extend_from_slice(&0u32.to_le_bytes());
    central.extend_from_slice(name);
    let mut end = Vec::new();
    end.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    for field in [0u16, 0, 1, 1] {
        end.extend_from_slice(&field.to_le_bytes());
    }
    end.extend_from_slice(
        &u32::try_from(central.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    end.extend_from_slice(&u32::try_from(local.len()).unwrap_or(u32::MAX).to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    let mut archive = local;
    archive.extend_from_slice(&central);
    archive.extend_from_slice(&end);
    std::fs::write(path, archive).map_err(|error| format!("{}: {error}", path.display()))
}

/// The first numeric password whose decrypted check byte matches the CRC's top byte,
/// which Python's `zipfile` accepts as right before the member fails its CRC.
pub(crate) fn zipcrypto_collision(path: &Path) -> Result<String, String> {
    let archive = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let field = |at: usize| -> Result<u16, String> {
        archive
            .get(at..at + 2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .ok_or_else(|| "the archive is too short".to_owned())
    };
    let crc = archive
        .get(14..18)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or("the archive is too short")?;
    let start = 30 + usize::from(field(26)?) + usize::from(field(28)?);
    let header = archive
        .get(start..start + 12)
        .ok_or("the archive has no encryption header")?;
    for candidate in 0..4096 {
        let password = candidate.to_string();
        let mut keys = Keys::new(password.as_bytes());
        let check = header.iter().map(|byte| keys.decrypt(*byte)).last();
        if check == Some((crc >> 24) as u8) {
            return Ok(password);
        }
    }
    Err("no ZipCrypto CRC collision in 0..4096".to_owned())
}
