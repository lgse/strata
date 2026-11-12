// SPDX-License-Identifier: MIT

use super::*;
use rustls::{
    ServerConfig, ServerConnection,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
};

const CERTIFICATE: &[u8] = include_bytes!("tests/localhost.cert.der");
// Test-only key for the self-signed localhost certificate; never used by production.
const PRIVATE_KEY: &[u8] = include_bytes!("tests/localhost.key.der");

struct Server {
    url: String,
    started: mpsc::Receiver<()>,
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            thread.join().expect("TLS fixture thread");
        }
    }
}

fn serve(
    handler: impl FnOnce(TcpStream, mpsc::Sender<()>, mpsc::Receiver<()>) + Send + 'static,
) -> Server {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("TLS listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let port = listener.local_addr().expect("listener address").port();
    let (started, ready) = mpsc::channel();
    let (stop, stopped) = mpsc::channel();
    let thread = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(4)))
                        .expect("read timeout");
                    stream
                        .set_write_timeout(Some(Duration::from_secs(4)))
                        .expect("write timeout");
                    handler(stream, started, stopped);
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline
                        || stopped.recv_timeout(Duration::from_millis(10)).is_ok()
                    {
                        return;
                    }
                }
                Err(error) => panic!("accept: {error}"),
            }
        }
    });
    Server {
        url: format!("https://localhost:{port}/update"),
        started: ready,
        stop,
        thread: Some(thread),
    }
}

fn trickle(
    mut stream: TcpStream,
    bytes: &[u8],
    started: mpsc::Sender<()>,
    stopped: mpsc::Receiver<()>,
) {
    if stream.write_all(&bytes[..1]).is_err() {
        return;
    }
    let _ = started.send(());
    for byte in bytes[1..].iter().take(120) {
        if stopped.recv_timeout(Duration::from_millis(50)).is_ok()
            || stream.write_all(&[*byte]).is_err()
        {
            return;
        }
    }
}

fn tls_body_server() -> Server {
    serve(|stream, started, stopped| tls_response(stream, started, stopped, true))
}

fn tls_response(
    mut stream: TcpStream,
    started: mpsc::Sender<()>,
    stopped: mpsc::Receiver<()>,
    fragmented: bool,
) {
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(CERTIFICATE)],
                PrivatePkcs8KeyDer::from(PRIVATE_KEY).into(),
            )
            .expect("test certificate");
    let mut connection = ServerConnection::new(Arc::new(config)).expect("server connection");
    let mut request = Vec::new();
    let mut byte = [0];
    {
        let mut tls = rustls::Stream::new(&mut connection, &mut stream);
        while !request.ends_with(b"\r\n\r\n") {
            if tls.read_exact(&mut byte).is_err() {
                return;
            }
            request.push(byte[0]);
        }
        tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n")
            .expect("response headers");
        tls.flush().expect("flush headers");
    }
    connection
        .writer()
        .write_all(&[b'x'; 4096])
        .expect("response body");
    let mut encrypted = Vec::new();
    while connection.wants_write() {
        connection
            .write_tls(&mut encrypted)
            .expect("encrypted record");
    }
    if fragmented {
        trickle(stream, &encrypted, started, stopped);
    } else {
        stream.write_all(&encrypted).expect("complete TLS response");
    }
}

fn client_config(
    connect: Duration,
    trust_fixture: bool,
    proxy: Option<ureq::Proxy>,
) -> ureq::config::Config {
    let mut config = ureq::Agent::config_builder()
        .proxy(proxy)
        .timeout_connect(Some(connect))
        .timeout_recv_response(Some(Duration::from_secs(4)));
    if trust_fixture {
        config = config.tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs([ureq::tls::Certificate::from_der(CERTIFICATE)].into())
                .build(),
        );
    }
    config.build()
}

fn client(
    cancel: &InstallCancel,
    idle: Duration,
    connect: Duration,
    trust_fixture: bool,
) -> ureq::Agent {
    agent_with_config(client_config(connect, trust_fixture, None), idle, cancel)
}

#[test]
fn trusted_https_downloads_work_directly_and_through_a_connect_proxy() {
    for proxied in [false, true] {
        let server = serve(move |mut stream, started, stopped| {
            if proxied {
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).expect("CONNECT request");
                    request.push(byte[0]);
                }
                assert!(request.starts_with(b"CONNECT localhost:443 HTTP/1.1\r\n"));
                stream
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .expect("proxy tunnel");
            }
            tls_response(stream, started, stopped, false);
        });
        let cancel = InstallCancel::new();
        let proxy = proxied.then(|| {
            ureq::Proxy::new(
                server
                    .url
                    .replace("https://localhost", "http://127.0.0.1")
                    .trim_end_matches("/update"),
            )
            .expect("proxy URL")
        });
        let config = client_config(Duration::from_secs(4), true, proxy);
        let agent = agent_with_config(config, Duration::from_secs(4), &cancel);
        let url = if proxied {
            "https://localhost:443/update"
        } else {
            &server.url
        };
        let mut response = agent.get(url).call().expect("trusted HTTPS response");
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .read_to_end(&mut body)
            .expect("complete body");
        assert_eq!(body, vec![b'x'; 4096]);
    }
}

#[test]
fn cancellation_interrupts_a_trickled_tls_body_record() {
    let server = tls_body_server();
    let cancel = InstallCancel::new();
    let agent = client(
        &cancel,
        Duration::from_secs(10),
        Duration::from_secs(4),
        true,
    );
    let mut response = agent
        .get(&server.url)
        .call()
        .expect("trusted HTTPS response");
    let canceller = cancel.clone();
    server
        .started
        .recv_timeout(Duration::from_secs(4))
        .expect("partial TLS record");
    thread::scope(|scope| {
        let cancelled = scope.spawn(|| {
            thread::sleep(Duration::from_millis(200));
            canceller.cancel();
            Instant::now()
        });
        let result = response.body_mut().as_reader().read_to_end(&mut Vec::new());
        let finished = Instant::now();
        let cancelled = cancelled.join().expect("canceller");
        assert!(result.is_err(), "cancelled TLS read succeeded");
        assert!(finished.saturating_duration_since(cancelled) < Duration::from_secs(3));
    });
}

#[test]
fn a_partial_tls_record_cannot_extend_the_idle_deadline() {
    let server = tls_body_server();
    let cancel = InstallCancel::new();
    let agent = client(
        &cancel,
        Duration::from_millis(500),
        Duration::from_secs(4),
        true,
    );
    let mut response = agent
        .get(&server.url)
        .call()
        .expect("trusted HTTPS response");
    let started = Instant::now();
    let error = response
        .body_mut()
        .as_reader()
        .read_to_end(&mut Vec::new())
        .expect_err("incomplete record must time out");
    assert_eq!(
        describe_read_error(&error),
        "The download stalled — check your connection and try again"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_partial_tls_handshake_cannot_extend_the_connect_deadline() {
    let server = serve(|stream, started, stopped| {
        let mut record = vec![0x16, 0x03, 0x03, 0x10, 0x00];
        record.extend([0; 4096]);
        trickle(stream, &record, started, stopped);
    });
    let cancel = InstallCancel::new();
    let agent = client(
        &cancel,
        Duration::from_secs(10),
        Duration::from_millis(500),
        true,
    );
    let started = Instant::now();
    let error = agent
        .get(&server.url)
        .call()
        .expect_err("incomplete handshake must time out");
    assert!(
        matches!(error, ureq::Error::Timeout(ureq::Timeout::Connect)),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn custom_transport_still_rejects_untrusted_tls_certificates() {
    let server = tls_body_server();
    let cancel = InstallCancel::new();
    let agent = client(
        &cancel,
        Duration::from_secs(2),
        Duration::from_secs(2),
        false,
    );
    let error = agent
        .get(&server.url)
        .call()
        .expect_err("untrusted certificate must be rejected");
    assert!(
        error.to_string().contains("invalid peer certificate"),
        "{error:?}"
    );
}
