// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The wire protocol between the browser and the PDF helper process.
//!
//! The helper (`mercury-browser --pdf-worker`) reads **requests** from its standard input and
//! writes **replies** to its standard output. Both directions are streams of frames:
//!
//! ```text
//! [u32 LE header length][u32 LE blob length][header: JSON][blob: bytes]
//! ```
//!
//! A header is at most [`MAX_HEADER_BYTES`] (64 KiB) and a blob at most [`MAX_BLOB_BYTES`]
//! (64 MiB). Both lengths are checked **before** anything is allocated, and the blob is read in
//! chunks, so a length that lies cannot make the reader allocate more than it receives. A frame
//! that is malformed, truncated, too long, or that holds an unknown message is a
//! [`FrameError`]; the helper answers it with one [`Reply::Error`] and ends in an orderly way
//! (see [`crate::serve`]), and the client ends the helper and reports the loss.
//!
//! What travels where:
//!
//! | Reply | Header (JSON) | Blob |
//! |---|---|---|
//! | [`Reply::Ready`] | protocol version and engine name | empty |
//! | [`Reply::Opened`] | title, page count, error | page sizes: 8 bytes each (`f32` width, `f32` height, little endian) |
//! | [`Reply::Tile`] | width, height, error | RGBA pixels, rows top to bottom, no padding |
//! | [`Reply::PageText`] | page, character count, error | 20-byte records: `u32` character, then `f32` left, top, right, bottom |
//! | [`Reply::Links`], [`Reply::Outline`], [`Reply::Found`] | ids, error (and `finished`) | the list as JSON |
//!
//! Lists that can be large (a page's links, the outline, the hits on one page) are JSON in the
//! blob, because a header never exceeds 64 KiB. Requests carry no blob.
//!
//! A request that breaks a rule of the document (a page that does not exist, a tile outside its
//! page) is **not** a frame error: it gets an ordinary reply whose `error` field says what was
//! wrong, and the helper carries on.

use std::fmt;
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::PdfError;
use crate::types::{FindOptions, PageSize, PageText, Rect, TextChar, TileRequest};

/// The protocol version the helper announces in [`Reply::Ready`]; the client refuses any other.
pub const PROTOCOL_VERSION: u32 = 1;
/// The longest header, in bytes.
pub const MAX_HEADER_BYTES: usize = 64 * 1024;
/// The longest blob, in bytes.
pub const MAX_BLOB_BYTES: usize = 64 * 1024 * 1024;
/// The size of one character record of a page's text in a blob.
pub const TEXT_RECORD_BYTES: usize = 20;
/// The size of one page size record of an `Opened` blob.
pub const PAGE_RECORD_BYTES: usize = 8;

/// What the browser asks of the helper. Ids are chosen by the browser and echoed in the reply.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Opens the document at `path`; the password, if any, is used once and never logged.
    Open {
        id: u64,
        path: String,
        password: Option<String>,
    },
    /// Renders one tile. Higher `priority` runs sooner.
    Tile {
        id: u64,
        priority: u8,
        request: TileRequest,
    },
    /// Drops a tile request that has not started; it is answered with `Cancelled`.
    Cancel {
        id: u64,
    },
    PageText {
        id: u64,
        page: u32,
    },
    Links {
        id: u64,
        page: u32,
    },
    Outline {
        id: u64,
    },
    /// Searches page by page, starting at `from_page` and wrapping round.
    Find {
        id: u64,
        query: String,
        options: FindOptions,
        from_page: u32,
    },
    /// Stops a find; nothing is answered.
    CancelFind {
        id: u64,
    },
    /// Closes the document (a barrier: everything asked before it is answered first).
    Close {
        id: u64,
    },
}

impl fmt::Debug for Request {
    /// Like the derived output, except that a password is never shown and a query is only
    /// counted.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { id, path, password } => formatter
                .debug_struct("Open")
                .field("id", id)
                .field("path", path)
                .field("password", &password.as_ref().map(|_| "<hidden>"))
                .finish(),
            Self::Tile {
                id,
                priority,
                request,
            } => formatter
                .debug_struct("Tile")
                .field("id", id)
                .field("priority", priority)
                .field("request", request)
                .finish(),
            Self::Cancel { id } => formatter.debug_struct("Cancel").field("id", id).finish(),
            Self::PageText { id, page } => formatter
                .debug_struct("PageText")
                .field("id", id)
                .field("page", page)
                .finish(),
            Self::Links { id, page } => formatter
                .debug_struct("Links")
                .field("id", id)
                .field("page", page)
                .finish(),
            Self::Outline { id } => formatter.debug_struct("Outline").field("id", id).finish(),
            Self::Find {
                id,
                query,
                options,
                from_page,
            } => formatter
                .debug_struct("Find")
                .field("id", id)
                .field("query_chars", &query.chars().count())
                .field("options", options)
                .field("from_page", from_page)
                .finish(),
            Self::CancelFind { id } => formatter
                .debug_struct("CancelFind")
                .field("id", id)
                .finish(),
            Self::Close { id } => formatter.debug_struct("Close").field("id", id).finish(),
        }
    }
}

/// What the helper answers. `error` is `None` when the request worked and its data is in the
/// fields and the blob (see the module documentation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    /// The first frame of every helper: the engine is loaded and requests are read.
    Ready {
        protocol_version: u32,
        engine: String,
    },
    Opened {
        id: u64,
        title: Option<String>,
        page_count: u32,
        error: Option<PdfError>,
    },
    Tile {
        id: u64,
        width: u32,
        height: u32,
        error: Option<PdfError>,
    },
    PageText {
        id: u64,
        page: u32,
        chars: u32,
        error: Option<PdfError>,
    },
    Links {
        id: u64,
        page: u32,
        error: Option<PdfError>,
    },
    Outline {
        id: u64,
        error: Option<PdfError>,
    },
    /// The hits on one page of a find; `finished` on the last page.
    Found {
        find: u64,
        page: u32,
        finished: bool,
        error: Option<PdfError>,
    },
    Closed {
        id: u64,
    },
    /// A failure that belongs to no request, or to a request that cannot be told apart (a
    /// panic). The helper ends after it.
    Error {
        id: Option<u64>,
        error: PdfError,
    },
}

/// Why a frame could not be read or written.
#[derive(Debug)]
pub enum FrameError {
    /// The stream failed.
    Io(io::Error),
    /// The stream ended in the middle of a frame.
    Truncated,
    /// The header length is zero.
    EmptyHeader,
    /// The header is longer than [`MAX_HEADER_BYTES`].
    HeaderTooLong(u32),
    /// The blob is longer than [`MAX_BLOB_BYTES`].
    BlobTooLong(u32),
    /// The header is not a message of this protocol (bad JSON, an unknown type, a wrong field).
    BadHeader(String),
    /// A blob that does not belong to the message, or does not fit it.
    BadBlob(String),
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "stream error: {error}"),
            Self::Truncated => formatter.write_str("the stream ended inside a frame"),
            Self::EmptyHeader => formatter.write_str("a frame with an empty header"),
            Self::HeaderTooLong(length) => {
                write!(formatter, "header of {length} bytes is too long")
            }
            Self::BlobTooLong(length) => write!(formatter, "blob of {length} bytes is too long"),
            Self::BadHeader(detail) => write!(formatter, "bad header: {detail}"),
            Self::BadBlob(detail) => write!(formatter, "bad blob: {detail}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// One frame as read: the raw header and the blob, before the header is decoded.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub header: Vec<u8>,
    pub blob: Vec<u8>,
}

/// Writes one frame and flushes. The caller keeps `header` within [`MAX_HEADER_BYTES`] and
/// `blob` within [`MAX_BLOB_BYTES`]; a larger one is refused, never sent.
pub fn write_frame<W: Write>(output: &mut W, header: &[u8], blob: &[u8]) -> Result<(), FrameError> {
    if header.is_empty() {
        return Err(FrameError::EmptyHeader);
    }
    let too_long = |length: usize| u32::try_from(length).unwrap_or(u32::MAX);
    if header.len() > MAX_HEADER_BYTES {
        return Err(FrameError::HeaderTooLong(too_long(header.len())));
    }
    if blob.len() > MAX_BLOB_BYTES {
        return Err(FrameError::BlobTooLong(too_long(blob.len())));
    }
    let mut prefix = [0_u8; 8];
    prefix[..4].copy_from_slice(&too_long(header.len()).to_le_bytes());
    prefix[4..].copy_from_slice(&too_long(blob.len()).to_le_bytes());
    output.write_all(&prefix)?;
    output.write_all(header)?;
    output.write_all(blob)?;
    output.flush()?;
    Ok(())
}

/// Reads exactly `buffer.len()` bytes; `Ok(false)` when the stream ended before the first byte
/// (a clean end), [`FrameError::Truncated`] when it ended after some.
fn read_full<R: Read>(input: &mut R, buffer: &mut [u8]) -> Result<bool, FrameError> {
    let mut filled = 0;
    while filled < buffer.len() {
        match input.read(&mut buffer[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(FrameError::Io(error)),
        }
    }
    Ok(true)
}

/// Reads one frame; `Ok(None)` when the stream ends cleanly between frames.
pub fn read_frame<R: Read>(input: &mut R) -> Result<Option<Frame>, FrameError> {
    let mut prefix = [0_u8; 8];
    if !read_full(input, &mut prefix)? {
        return Ok(None);
    }
    let header_length = u32::from_le_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]);
    let blob_length = u32::from_le_bytes([prefix[4], prefix[5], prefix[6], prefix[7]]);
    if header_length == 0 {
        return Err(FrameError::EmptyHeader);
    }
    if header_length as usize > MAX_HEADER_BYTES {
        return Err(FrameError::HeaderTooLong(header_length));
    }
    if blob_length as usize > MAX_BLOB_BYTES {
        return Err(FrameError::BlobTooLong(blob_length));
    }
    // The header length is within its limit (64 KiB), so this allocation is bounded.
    let mut header = vec![0_u8; header_length as usize];
    if !read_full(input, &mut header)? {
        return Err(FrameError::Truncated);
    }
    // The blob grows with the bytes that arrive, so a length that lies allocates only what was
    // really sent.
    let mut blob = Vec::new();
    let wanted = u64::from(blob_length);
    let received = input.by_ref().take(wanted).read_to_end(&mut blob)?;
    if received as u64 != wanted {
        return Err(FrameError::Truncated);
    }
    Ok(Some(Frame { header, blob }))
}

fn encode_header<T: Serialize>(message: &T) -> Result<Vec<u8>, FrameError> {
    serde_json::to_vec(message).map_err(|error| FrameError::BadHeader(error.to_string()))
}

fn decode_header<T: DeserializeOwned>(header: &[u8]) -> Result<T, FrameError> {
    serde_json::from_slice(header).map_err(|error| FrameError::BadHeader(error.to_string()))
}

/// Writes a request frame (no blob). The serialised bytes, which hold a password for an open
/// request, are wiped before they are freed.
pub fn write_request<W: Write>(output: &mut W, request: &Request) -> Result<(), FrameError> {
    let mut header = encode_header(request)?;
    let result = write_frame(output, &header, &[]);
    header.zeroize();
    result
}

/// Reads a request; `Ok(None)` at a clean end of the stream. A frame that carries a blob is
/// malformed: requests have none. The raw header, which holds a password for an open request,
/// is wiped before this returns.
pub fn read_request<R: Read>(input: &mut R) -> Result<Option<Request>, FrameError> {
    let Some(mut frame) = read_frame(input)? else {
        return Ok(None);
    };
    let decoded = decode_header::<Request>(&frame.header);
    frame.header.zeroize();
    let request = decoded?;
    if !frame.blob.is_empty() {
        return Err(FrameError::BadBlob(
            "a request cannot carry a blob".to_owned(),
        ));
    }
    Ok(Some(request))
}

/// Writes a reply frame with its blob.
pub fn write_reply<W: Write>(output: &mut W, reply: &Reply, blob: &[u8]) -> Result<(), FrameError> {
    let header = encode_header(reply)?;
    write_frame(output, &header, blob)
}

/// Reads a reply and its blob; `Ok(None)` at a clean end of the stream.
pub fn read_reply<R: Read>(input: &mut R) -> Result<Option<(Reply, Vec<u8>)>, FrameError> {
    let Some(frame) = read_frame(input)? else {
        return Ok(None);
    };
    let reply = decode_header::<Reply>(&frame.header)?;
    Ok(Some((reply, frame.blob)))
}

/// The blob of an `Opened` reply: 8 bytes per page.
pub fn encode_page_sizes(pages: &[PageSize]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(pages.len() * PAGE_RECORD_BYTES);
    for page in pages {
        blob.extend_from_slice(&page.width.to_le_bytes());
        blob.extend_from_slice(&page.height.to_le_bytes());
    }
    blob
}

/// Decodes the blob of an `Opened` reply; every size must be finite and positive.
pub fn decode_page_sizes(blob: &[u8], count: u32) -> Result<Vec<PageSize>, FrameError> {
    let count = count as usize;
    if blob.len() != count.saturating_mul(PAGE_RECORD_BYTES) {
        return Err(FrameError::BadBlob(
            "the page sizes do not match the page count".to_owned(),
        ));
    }
    let mut pages = Vec::with_capacity(count);
    for record in blob.as_chunks::<PAGE_RECORD_BYTES>().0 {
        let width = read_f32(record, 0);
        let height = read_f32(record, 4);
        if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
            return Err(FrameError::BadBlob("a page has no usable size".to_owned()));
        }
        pages.push(PageSize::new(width, height));
    }
    Ok(pages)
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_f32(bytes: &[u8], at: usize) -> f32 {
    f32::from_bits(read_u32(bytes, at))
}

/// The blob of a `PageText` reply: [`TEXT_RECORD_BYTES`] per character.
pub fn encode_page_text(text: &PageText) -> Vec<u8> {
    let mut blob = Vec::with_capacity(text.chars.len() * TEXT_RECORD_BYTES);
    for character in &text.chars {
        blob.extend_from_slice(&(character.ch as u32).to_le_bytes());
        for side in [
            character.bounds.left,
            character.bounds.top,
            character.bounds.right,
            character.bounds.bottom,
        ] {
            blob.extend_from_slice(&side.to_le_bytes());
        }
    }
    blob
}

/// Decodes the blob of a `PageText` reply. A value that is not a character becomes U+FFFD and a
/// box with a side that is not finite becomes an empty box, so nothing a helper sends can
/// poison the view model.
pub fn decode_page_text(blob: &[u8], count: u32) -> Result<PageText, FrameError> {
    let count = count as usize;
    if blob.len() != count.saturating_mul(TEXT_RECORD_BYTES) {
        return Err(FrameError::BadBlob(
            "the text does not match the character count".to_owned(),
        ));
    }
    let mut chars = Vec::with_capacity(count);
    for record in blob.as_chunks::<TEXT_RECORD_BYTES>().0 {
        let ch = char::from_u32(read_u32(record, 0)).unwrap_or('\u{FFFD}');
        let sides = [
            read_f32(record, 4),
            read_f32(record, 8),
            read_f32(record, 12),
            read_f32(record, 16),
        ];
        let bounds = if sides.iter().all(|side| side.is_finite()) {
            Rect::new(sides[0], sides[1], sides[2], sides[3])
        } else {
            Rect::default()
        };
        chars.push(TextChar { ch, bounds });
    }
    Ok(PageText { chars })
}

/// Encodes a list as the JSON blob of a `Links`, `Outline` or `Found` reply.
pub fn encode_list<T: Serialize>(list: &[T]) -> Result<Vec<u8>, FrameError> {
    serde_json::to_vec(list).map_err(|error| FrameError::BadBlob(error.to_string()))
}

/// Decodes the JSON blob of a `Links`, `Outline` or `Found` reply.
pub fn decode_list<T: DeserializeOwned>(blob: &[u8]) -> Result<Vec<T>, FrameError> {
    serde_json::from_slice(blob).map_err(|error| FrameError::BadBlob(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Link, LinkTarget, OutlineItem, Rotation, SearchHit};

    fn frame_bytes(header: &[u8], blob: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, header, blob).expect("frame");
        bytes
    }

    fn raw_prefix(header_length: u32, blob_length: u32) -> Vec<u8> {
        let mut bytes = header_length.to_le_bytes().to_vec();
        bytes.extend_from_slice(&blob_length.to_le_bytes());
        bytes
    }

    fn tile_request() -> TileRequest {
        TileRequest {
            page: 3,
            scale: 1.5,
            rotation: Rotation::Deg90,
            x: 512,
            y: 0,
            width: 512,
            height: 300,
        }
    }

    #[test]
    fn a_frame_round_trips_with_and_without_a_blob() {
        let bytes = frame_bytes(b"{}", b"");
        let frame = read_frame(&mut bytes.as_slice())
            .expect("read")
            .expect("frame");
        assert_eq!(frame.header, b"{}");
        assert!(frame.blob.is_empty());

        let bytes = frame_bytes(b"{\"a\":1}", &[1, 2, 3, 4, 5]);
        assert_eq!(&bytes[..8], &[7, 0, 0, 0, 5, 0, 0, 0]);
        let frame = read_frame(&mut bytes.as_slice())
            .expect("read")
            .expect("frame");
        assert_eq!(frame.header, b"{\"a\":1}");
        assert_eq!(frame.blob, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn two_frames_in_a_row_and_then_a_clean_end() {
        let mut bytes = frame_bytes(b"{1}", b"ab");
        bytes.extend(frame_bytes(b"{2}", b""));
        let mut input = bytes.as_slice();
        assert_eq!(read_frame(&mut input).unwrap().unwrap().header, b"{1}");
        assert_eq!(read_frame(&mut input).unwrap().unwrap().header, b"{2}");
        assert!(read_frame(&mut input).unwrap().is_none());
    }

    #[test]
    fn the_longest_allowed_header_is_accepted() {
        let header = vec![b' '; MAX_HEADER_BYTES];
        let bytes = frame_bytes(&header, &vec![7_u8; 1 << 20]);
        let frame = read_frame(&mut bytes.as_slice()).unwrap().unwrap();
        assert_eq!(frame.header.len(), MAX_HEADER_BYTES);
        assert_eq!(frame.blob.len(), 1 << 20);
    }

    #[test]
    fn the_longest_allowed_blob_is_accepted() {
        let blob = vec![3_u8; MAX_BLOB_BYTES];
        let bytes = frame_bytes(b"{}", &blob);
        let frame = read_frame(&mut bytes.as_slice()).unwrap().unwrap();
        assert_eq!(frame.blob.len(), MAX_BLOB_BYTES);
    }

    #[test]
    fn writing_refuses_an_empty_or_oversized_frame() {
        let mut sink = Vec::new();
        assert!(matches!(
            write_frame(&mut sink, b"", b""),
            Err(FrameError::EmptyHeader)
        ));
        assert!(matches!(
            write_frame(&mut sink, &vec![b'x'; MAX_HEADER_BYTES + 1], b""),
            Err(FrameError::HeaderTooLong(_))
        ));
        let blob = vec![0_u8; MAX_BLOB_BYTES + 1];
        assert!(matches!(
            write_frame(&mut sink, b"{}", &blob),
            Err(FrameError::BlobTooLong(_))
        ));
        assert!(sink.is_empty(), "nothing is written for a refused frame");
    }

    #[test]
    fn every_malformed_frame_is_an_error_and_never_a_panic() {
        // Zero-length header.
        let mut bytes = raw_prefix(0, 0);
        bytes.push(b'x');
        assert!(matches!(
            read_frame(&mut bytes.as_slice()),
            Err(FrameError::EmptyHeader)
        ));
        // A header longer than the limit: refused from the lengths alone, nothing follows.
        for length in [MAX_HEADER_BYTES as u32 + 1, u32::MAX] {
            assert!(matches!(
                read_frame(&mut raw_prefix(length, 0).as_slice()),
                Err(FrameError::HeaderTooLong(found)) if found == length
            ));
        }
        // A blob longer than the limit.
        for length in [MAX_BLOB_BYTES as u32 + 1, u32::MAX] {
            assert!(matches!(
                read_frame(&mut raw_prefix(2, length).as_slice()),
                Err(FrameError::BlobTooLong(found)) if found == length
            ));
        }
        // A stream that ends inside the prefix, the header and the blob.
        let whole = frame_bytes(b"{\"k\":1}", &[9; 16]);
        for cut in [1, 7, 9, 12, 15, 16, 23, whole.len() - 1] {
            assert!(
                matches!(read_frame(&mut &whole[..cut]), Err(FrameError::Truncated)),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn a_blob_length_that_lies_allocates_only_what_arrives() {
        // 60 MiB are promised and 3 bytes arrive.
        let mut bytes = raw_prefix(2, 60 * 1024 * 1024);
        bytes.extend_from_slice(b"{}abc");
        assert!(matches!(
            read_frame(&mut bytes.as_slice()),
            Err(FrameError::Truncated)
        ));
    }

    #[test]
    fn a_failing_stream_is_an_io_error() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("boom"))
            }
        }
        assert!(matches!(read_frame(&mut Broken), Err(FrameError::Io(_))));
    }

    #[test]
    fn every_request_round_trips() {
        let requests = vec![
            Request::Open {
                id: 1,
                path: "/tmp/a b.pdf".to_owned(),
                password: Some("p\"w".to_owned()),
            },
            Request::Open {
                id: 2,
                path: "x.pdf".to_owned(),
                password: None,
            },
            Request::Tile {
                id: 3,
                priority: 200,
                request: tile_request(),
            },
            Request::Cancel { id: 3 },
            Request::PageText { id: 4, page: 9 },
            Request::Links { id: 5, page: 0 },
            Request::Outline { id: 6 },
            Request::Find {
                id: u64::MAX,
                query: "fox ünï".to_owned(),
                options: FindOptions {
                    match_case: true,
                    whole_word: false,
                },
                from_page: 2,
            },
            Request::CancelFind { id: 7 },
            Request::Close { id: 8 },
        ];
        let mut wire = Vec::new();
        for request in &requests {
            write_request(&mut wire, request).unwrap();
        }
        let mut input = wire.as_slice();
        for request in &requests {
            assert_eq!(&read_request(&mut input).unwrap().unwrap(), request);
        }
        assert!(read_request(&mut input).unwrap().is_none());
    }

    #[test]
    fn a_password_never_shows_in_debug_output() {
        let request = Request::Open {
            id: 1,
            path: "a.pdf".to_owned(),
            password: Some("hunter2".to_owned()),
        };
        let shown = format!("{request:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("<hidden>"));
        let find = Request::Find {
            id: 1,
            query: "a secret query".to_owned(),
            options: FindOptions::default(),
            from_page: 0,
        };
        assert!(!format!("{find:?}").contains("secret"));
    }

    #[test]
    fn every_reply_round_trips_with_its_blob() {
        let replies = vec![
            (
                Reply::Ready {
                    protocol_version: PROTOCOL_VERSION,
                    engine: "pdfium".to_owned(),
                },
                vec![],
            ),
            (
                Reply::Opened {
                    id: 1,
                    title: Some("T".to_owned()),
                    page_count: 2,
                    error: None,
                },
                encode_page_sizes(&[PageSize::new(612.0, 792.0), PageSize::new(1.0, 2.5)]),
            ),
            (
                Reply::Opened {
                    id: 1,
                    title: None,
                    page_count: 0,
                    error: Some(PdfError::PasswordRequired),
                },
                vec![],
            ),
            (
                Reply::Tile {
                    id: 2,
                    width: 1,
                    height: 1,
                    error: None,
                },
                vec![1, 2, 3, 255],
            ),
            (
                Reply::Tile {
                    id: 2,
                    width: 0,
                    height: 0,
                    error: Some(PdfError::Cancelled),
                },
                vec![],
            ),
            (
                Reply::PageText {
                    id: 3,
                    page: 4,
                    chars: 0,
                    error: None,
                },
                vec![],
            ),
            (
                Reply::Links {
                    id: 4,
                    page: 1,
                    error: None,
                },
                b"[]".to_vec(),
            ),
            (
                Reply::Outline {
                    id: 5,
                    error: Some(PdfError::NoDocument),
                },
                vec![],
            ),
            (
                Reply::Found {
                    find: 6,
                    page: 7,
                    finished: true,
                    error: None,
                },
                b"[]".to_vec(),
            ),
            (Reply::Closed { id: 9 }, vec![]),
            (
                Reply::Error {
                    id: None,
                    error: PdfError::InvalidRequest("bad".to_owned()),
                },
                vec![],
            ),
        ];
        let mut wire = Vec::new();
        for (reply, blob) in &replies {
            write_reply(&mut wire, reply, blob).unwrap();
        }
        let mut input = wire.as_slice();
        for (reply, blob) in &replies {
            let (read, read_blob) = read_reply(&mut input).unwrap().unwrap();
            assert_eq!(&read, reply);
            assert_eq!(&read_blob, blob);
        }
        assert!(read_reply(&mut input).unwrap().is_none());
    }

    #[test]
    fn an_unknown_message_is_a_bad_header() {
        for header in [
            &b"not json"[..],
            b"[]",
            b"{}",
            b"{\"type\":\"explode\"}",
            b"{\"type\":\"tile\",\"id\":\"one\"}",
            b"{\"type\":\"open\",\"id\":1}",
            b"{\"type\":\"cancel\",\"id\":-1}",
            b"\xff\xfe",
        ] {
            let bytes = frame_bytes(header, b"");
            assert!(
                matches!(
                    read_request(&mut bytes.as_slice()),
                    Err(FrameError::BadHeader(_))
                ),
                "{}",
                String::from_utf8_lossy(header)
            );
            assert!(
                matches!(
                    read_reply(&mut bytes.as_slice()),
                    Err(FrameError::BadHeader(_))
                ),
                "{}",
                String::from_utf8_lossy(header)
            );
        }
    }

    #[test]
    fn a_request_with_a_blob_is_malformed() {
        let header = serde_json::to_vec(&Request::Outline { id: 1 }).unwrap();
        let wire = frame_bytes(&header, b"surprise");
        assert!(matches!(
            read_request(&mut wire.as_slice()),
            Err(FrameError::BadBlob(_))
        ));
    }

    #[test]
    fn page_sizes_round_trip_and_reject_nonsense() {
        let pages = [PageSize::new(612.0, 792.0), PageSize::new(0.5, 14400.0)];
        let blob = encode_page_sizes(&pages);
        assert_eq!(blob.len(), 16);
        assert_eq!(decode_page_sizes(&blob, 2).unwrap(), pages);
        assert!(decode_page_sizes(&blob, 3).is_err(), "count mismatch");
        assert!(decode_page_sizes(&blob[..15], 2).is_err(), "short blob");
        for bad in [f32::NAN, f32::INFINITY, 0.0, -3.0] {
            let blob = encode_page_sizes(&[PageSize::new(bad, 10.0)]);
            assert!(decode_page_sizes(&blob, 1).is_err(), "{bad}");
        }
    }

    #[test]
    fn page_text_round_trips_with_twenty_byte_records() {
        let text = PageText {
            chars: vec![
                TextChar {
                    ch: 'H',
                    bounds: Rect::new(1.0, 2.0, 7.5, 14.0),
                },
                TextChar {
                    ch: '\r',
                    bounds: Rect::default(),
                },
                TextChar {
                    ch: '😀',
                    bounds: Rect::new(-3.0, 0.0, 9.0, 9.0),
                },
            ],
        };
        let blob = encode_page_text(&text);
        assert_eq!(blob.len(), 3 * TEXT_RECORD_BYTES);
        assert_eq!(&blob[..4], &72_u32.to_le_bytes());
        assert_eq!(decode_page_text(&blob, 3).unwrap(), text);
        assert!(decode_page_text(&blob, 2).is_err());
        assert!(decode_page_text(&blob[..59], 3).is_err());
    }

    #[test]
    fn hostile_text_records_are_made_harmless() {
        let mut record = Vec::new();
        record.extend_from_slice(&0xD800_u32.to_le_bytes()); // a surrogate: not a char
        for side in [f32::NAN, 0.0, f32::INFINITY, 1.0] {
            record.extend_from_slice(&side.to_le_bytes());
        }
        let text = decode_page_text(&record, 1).unwrap();
        assert_eq!(text.chars[0].ch, '\u{FFFD}');
        assert!(text.chars[0].bounds.is_empty());
    }

    #[test]
    fn lists_round_trip_as_json_blobs() {
        let links = vec![
            Link {
                rect: Rect::new(1.0, 2.0, 3.0, 4.0),
                target: LinkTarget::Uri("https://example.com/?q=1".to_owned()),
            },
            Link {
                rect: Rect::new(0.0, 0.0, 9.0, 9.0),
                target: LinkTarget::Page {
                    page: 2,
                    y: Some(100.0),
                },
            },
        ];
        assert_eq!(
            decode_list::<Link>(&encode_list(&links).unwrap()).unwrap(),
            links
        );
        let outline = vec![OutlineItem {
            title: "One".to_owned(),
            target: Some(LinkTarget::Page { page: 0, y: None }),
            children: vec![OutlineItem {
                title: "Child".to_owned(),
                target: None,
                children: vec![],
            }],
        }];
        assert_eq!(
            decode_list::<OutlineItem>(&encode_list(&outline).unwrap()).unwrap(),
            outline
        );
        let hits = vec![SearchHit {
            page: 1,
            char_start: 5,
            char_len: 3,
            rects: vec![Rect::new(1.0, 2.0, 3.0, 4.0)],
        }];
        assert_eq!(
            decode_list::<SearchHit>(&encode_list(&hits).unwrap()).unwrap(),
            hits
        );
        assert!(decode_list::<Link>(b"{}").is_err());
        assert!(decode_list::<Link>(b"[1]").is_err());
    }
}
