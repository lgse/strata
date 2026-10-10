// SPDX-License-Identifier: MIT

use super::*;
use gtk::gdk::prelude::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
};

#[test]
fn github_image_pages_resolve_to_raw_image_downloads() {
    let link =
        WebLink::parse("https://github.com/vshvedov/rhun/blob/main/assets/social/github@2x.png")
            .expect("image page");
    assert_eq!(
        download_address(&link),
        "https://raw.githubusercontent.com/vshvedov/rhun/main/assets/social/github@2x.png"
    );
}

#[test]
fn extensionless_downloads_use_mime_type_and_preserve_original_data() {
    crate::test_support::gtk_test(
        "services::web_image::tests::extensionless_downloads_use_mime_type_and_preserve_original_data",
        || {
            let pixels = [30, 80, 120, 255];
            let texture = gdk::MemoryTexture::new(
                1,
                1,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from(&pixels),
                4,
            );
            let png = texture.save_to_png_bytes();
            for (status, mime, body, succeeds) in [
                ("200 OK", "image/png", png.as_ref(), Some(true)),
                (
                    "200 OK",
                    "image/png",
                    b"<html>not an image</html>".as_slice(),
                    None,
                ),
                (
                    "200 OK",
                    "text/html",
                    b"<html>web page</html>".as_slice(),
                    Some(false),
                ),
                ("302 Found", "image/png", b"".as_slice(), Some(false)),
            ] {
                let listener = TcpListener::bind("127.0.0.1:0").expect("image server");
                listener.set_nonblocking(true).expect("nonblocking");
                let address = listener.local_addr().expect("address");
                std::thread::scope(|scope| {
                    let server = scope.spawn(move || {
                        let deadline = std::time::Instant::now() + Duration::from_secs(10);
                        let mut stream = loop {
                            match listener.accept() {
                                Ok((stream, _)) => break stream,
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                                Err(error) => panic!("request: {error}"),
                            }
                        };
                        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
                        let mut request = [0; 4096];
                        let count = stream.read(&mut request).expect("request bytes");
                        assert!(std::str::from_utf8(&request[..count]).expect("HTTP").starts_with("GET /photo?auto=format "));
                        write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/private\r\nConnection: close\r\n\r\n", body.len()).expect("headers");
                        stream.write_all(body).expect("image response");
                    });
                    let link = WebLink::parse(&format!("http://{address}/photo?auto=format"))
                        .expect("URL");
                    let result = download(&link);
                    assert_eq!(
                        result.as_ref().ok().map(|image| image.is_some()),
                        succeeds,
                        "{status} {mime}"
                    );
                    if let Ok(Some(image)) = result {
                        assert_eq!(image.bytes.as_slice(), png.as_ref());
                        assert_eq!(image.extension, "png");
                    }
                    server.join().expect("server");
                });
            }
        },
    );
}
