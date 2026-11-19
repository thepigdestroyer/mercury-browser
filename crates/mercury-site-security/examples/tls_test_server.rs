// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A loopback HTTPS server with a self-signed certificate made at start, for checking
//! Mercury's certificate page and certificate exceptions. Nothing is added to any trust store:
//! the browser must refuse the certificate.
//!
//! `cargo run --release -p mercury-site-security --example tls_test_server -- --port <port>`
//!
//! Prints `listening on https://127.0.0.1:<port>/` once ready and serves until it is ended.
//! Every path answers with a small page whose title is "PASS: loaded over HTTPS"; `/cookie`
//! also sets a cookie, for the site-data checks.

use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

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
    let mut reader = BufReader::new(&mut tls);
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
    let body = "<!DOCTYPE html><html><head><title>PASS: loaded over HTTPS</title></head>\
<body><h1>PASS: loaded over HTTPS</h1><p>The self-signed test server answered.</p></body></html>";
    write!(
        tls,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
Cache-Control: no-store\r\n{cookie}Connection: close\r\n\r\n{body}",
        body.len()
    )?;
    tls.conn.send_close_notify();
    tls.flush()?;
    Ok(())
}
