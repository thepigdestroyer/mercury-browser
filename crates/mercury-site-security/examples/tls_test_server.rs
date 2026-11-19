// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A loopback HTTPS server with a self-signed certificate made at start, for checking
//! Mercury's certificate page and certificate exceptions. Nothing is added to any trust store:
//! the browser must refuse the certificate.
//!
//! `cargo run --release -p mercury-site-security --example tls_test_server -- --port <port>`
//!
//! Prints `listening on https://127.0.0.1:<port>/` once ready (with `--port 0`, the port the
//! system chose) and serves until it is ended. The certificate has two names, `127.0.0.1` and
//! `localhost`; both reach this server.
//!
//! Every path answers with a small page whose title is "PASS: loaded over HTTPS", except:
//! - `/cookie` also sets a cookie, for the site-data checks;
//! - `/pixel.png` is a 1x1 PNG (`image/png`);
//! - `/host-binding` is the page of `scripts/macos/test-certificate-exceptions.sh`. Opened over
//!   `https://127.0.0.1:<port>/host-binding`, it loads `/pixel.png` from the same name ("same")
//!   and from `https://localhost:<port>/pixel.png`, the same certificate under the other name
//!   ("other"; opened over `localhost` the two names swap), and sets its title to
//!   `CERTEXC same=<loaded|failed>; other=<loaded|failed>` as each image loads or fails
//!   (`pending` until then). A browser that binds a certificate exception to the host it was
//!   accepted for loads "same" and fails "other"; one that accepts the certificate on every
//!   host loads both.
//!
//! A connection that ends before a request (the browser refusing the certificate) is reported
//! on standard error with the name the browser asked for.

use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

/// A 1x1 transparent PNG.
const PIXEL_PNG: [u8; 70] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0x60, 0xf8, 0x5f,
    0x0f, 0x00, 0x02, 0x87, 0x01, 0x80, 0xeb, 0x47, 0xba, 0x92, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The page of `/host-binding`. The images are loaded by name from the page's own port.
const HOST_BINDING_PAGE: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>CERTEXC same=pending; other=pending</title></head>\
<body><h1>Certificate exception host binding</h1><p id=\"result\"></p>\
<script>\
var results = { same: 'pending', other: 'pending' };\
function report(which, state) {\
  results[which] = state;\
  document.title = 'CERTEXC same=' + results.same + '; other=' + results.other;\
  document.getElementById('result').textContent = document.title;\
}\
function probe(which, host) {\
  var image = new Image();\
  image.onload = function () { report(which, 'loaded'); };\
  image.onerror = function () { report(which, 'failed'); };\
  image.src = 'https://' + host + ':' + location.port + '/pixel.png';\
}\
var here = location.hostname;\
probe('same', here);\
probe('other', here === 'localhost' ? '127.0.0.1' : 'localhost');\
</script></body></html>";

const PASS_PAGE: &str = "<!DOCTYPE html><html><head><title>PASS: loaded over HTTPS</title></head>\
<body><h1>PASS: loaded over HTTPS</h1><p>The self-signed test server answered.</p></body></html>";

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let mut port = 0_u16;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => {
                port = arguments.next().ok_or("--port needs a value")?.parse()?;
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }

    let certified =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned(), "localhost".to_owned()])?;
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified.signing_key.serialize_der(),
    ));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![certified.cert.der().clone()], key)?;
    let config = Arc::new(config);

    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("listening on https://{}/", listener.local_addr()?);
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let config = Arc::clone(&config);
        std::thread::spawn(move || {
            if let Err(error) = serve(stream, config) {
                eprintln!("connection ended: {error}");
            }
        });
    }
    Ok(())
}

fn serve(stream: TcpStream, config: Arc<ServerConfig>) -> Result<(), Box<dyn Error>> {
    let connection = ServerConnection::new(config)?;
    let mut tls = StreamOwned::new(connection, stream);
    answer(&mut tls).map_err(|error| {
        let name = tls.conn.server_name().unwrap_or("none");
        format!("{error} (name asked for: {name})").into()
    })
}

/// Reads one request from `tls` and answers it.
fn answer(tls: &mut StreamOwned<ServerConnection, TcpStream>) -> Result<(), Box<dyn Error>> {
    let mut reader = BufReader::new(&mut *tls);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let cookie = if path == "/cookie" {
        "Set-Cookie: mercury_site_data=1; Path=/; Max-Age=600\r\n"
    } else {
        ""
    };
    let (content_type, body): (&str, &[u8]) = match path {
        "/pixel.png" => ("image/png", PIXEL_PNG.as_slice()),
        "/host-binding" => ("text/html; charset=utf-8", HOST_BINDING_PAGE.as_bytes()),
        _ => ("text/html; charset=utf-8", PASS_PAGE.as_bytes()),
    };
    write!(
        tls,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
Cache-Control: no-store\r\n{cookie}Connection: close\r\n\r\n",
        body.len()
    )?;
    tls.write_all(body)?;
    tls.conn.send_close_notify();
    tls.flush()?;
    Ok(())
}
