// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Test support for the PDF viewer: a generator for small valid PDF files, in-memory pipes, and a
//! harness that runs the helper's request loop in a thread.
//!
//! **The generator.** [`minimal_pdf`] and [`document_pdf`] write a valid PDF 1.4 file with a
//! correct cross-reference table: Helvetica lines of text, optional `/CropBox` and `/Rotate`,
//! URI and GoTo links and an outline. The output is a pure function of the spec, so the fixtures
//! in `tests/pages/pdf/` (three-pages.pdf, rotated.pdf) are reproduced byte for byte by
//! [`three_pages_spec`] and [`rotated_spec`], and a test checks that.
//!
//! A page is described the way a reader **sees** it: lines of text and link rectangles are given
//! in the page's displayed coordinates (points, origin at the top-left of the shown page, y down,
//! `/Rotate` applied), and the generator maps them back to the PDF's own user space, so a page
//! with `/Rotate 90` and a crop box shows upright text. What a backend reports for such a page
//! is therefore checkable against the same numbers.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::backend::PdfBackend;
use crate::protocol::{Reply, Request, read_reply, write_request};
use crate::serve::{ServeExit, serve};

// ---------------------------------------------------------------------------------------------
// The PDF generator.
// ---------------------------------------------------------------------------------------------

/// The left margin of the first text line, and its top, in displayed points.
pub const LINE_LEFT: f32 = 36.0;
/// The distance between the tops of two lines, in displayed points.
pub const LINE_PITCH: f32 = 14.0;
/// The top of the first line, in displayed points.
pub const LINE_TOP: f32 = 36.0;
/// The font size of the generated text.
pub const FONT_SIZE: f32 = 12.0;

/// A link on a generated page. Rectangles are `[left, top, right, bottom]` in displayed points.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkSpec {
    /// A link to an address.
    Uri { rect: [f32; 4], uri: String },
    /// A link to a page (counted from 0), at `y` displayed points from its top if given.
    GoTo {
        rect: [f32; 4],
        page: usize,
        y: Option<f32>,
    },
}

/// One generated page.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSpec {
    /// The media box is `[0 0 width height]`, in points.
    pub width: f32,
    pub height: f32,
    /// `[left, bottom, right, top]` in the PDF's own user space (y up), if the page has one.
    pub crop_box: Option<[f32; 4]>,
    /// The page's `/Rotate`: 0, 90, 180 or 270.
    pub rotate: u16,
    /// Lines of Helvetica text, from the top-left of the shown page.
    pub lines: Vec<String>,
    pub links: Vec<LinkSpec>,
    /// Filled rectangles drawn under the text: `[left, top, right, bottom]` in displayed points
    /// and an RGB colour, each channel from 0 to 1.
    pub fills: Vec<([f32; 4], [f32; 3])>,
}

impl PageSpec {
    /// A US Letter page with the given lines.
    pub fn letter<S: Into<String>>(lines: impl IntoIterator<Item = S>) -> Self {
        Self {
            width: 612.0,
            height: 792.0,
            crop_box: None,
            rotate: 0,
            lines: lines.into_iter().map(Into::into).collect(),
            links: Vec::new(),
            fills: Vec::new(),
        }
    }

    /// The size of the page as shown: the crop box (or media box) with `/Rotate` applied.
    pub fn shown_size(&self) -> (f32, f32) {
        let (width, height) = self.visible_size();
        if self.rotate == 90 || self.rotate == 270 {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// The visible box before `/Rotate`: `[left, bottom, right, top]`.
    fn visible_box(&self) -> [f32; 4] {
        self.crop_box.unwrap_or([0.0, 0.0, self.width, self.height])
    }

    fn visible_size(&self) -> (f32, f32) {
        let [left, bottom, right, top] = self.visible_box();
        (right - left, top - bottom)
    }

    /// Maps a shown point (origin top-left, y down) to the PDF's user space (y up).
    pub fn user_point(&self, shown_x: f32, shown_y: f32) -> (f32, f32) {
        let [left, bottom, _, _] = self.visible_box();
        let (width, height) = self.visible_size();
        // The position in the unrotated visible box, y down.
        let (x, y) = match self.rotate {
            90 => (shown_y, height - shown_x),
            180 => (width - shown_x, height - shown_y),
            270 => (width - shown_y, shown_x),
            _ => (shown_x, shown_y),
        };
        (left + x, bottom + (height - y))
    }

    fn user_rect(&self, rect: [f32; 4]) -> [f32; 4] {
        let (x1, y1) = self.user_point(rect[0], rect[1]);
        let (x2, y2) = self.user_point(rect[2], rect[3]);
        [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)]
    }

    /// The text matrix that makes text run left to right on the shown page.
    fn text_matrix(&self) -> [f32; 4] {
        match self.rotate {
            90 => [0.0, 1.0, -1.0, 0.0],
            180 => [-1.0, 0.0, 0.0, -1.0],
            270 => [0.0, -1.0, 1.0, 0.0],
            _ => [1.0, 0.0, 0.0, 1.0],
        }
    }
}

/// One entry of a generated outline.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineSpec {
    pub title: String,
    /// The page it leads to, counted from 0.
    pub page: usize,
    pub children: Vec<OutlineSpec>,
}

impl OutlineSpec {
    pub fn new(title: &str, page: usize, children: Vec<OutlineSpec>) -> Self {
        Self {
            title: title.to_owned(),
            page,
            children,
        }
    }
}

/// A whole generated document.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentSpec {
    pub title: Option<String>,
    pub pages: Vec<PageSpec>,
    pub outline: Vec<OutlineSpec>,
}

/// A valid PDF of the given pages, with no title and no outline.
pub fn minimal_pdf(pages: &[PageSpec]) -> Vec<u8> {
    document_pdf(&DocumentSpec {
        title: None,
        pages: pages.to_vec(),
        outline: Vec::new(),
    })
}

/// A PDF string: a literal for ASCII, UTF-16BE in hex (with a byte order mark) otherwise.
fn pdf_string(text: &str) -> String {
    if text.is_ascii() {
        let mut escaped = String::from("(");
        for ch in text.chars() {
            match ch {
                '(' | ')' | '\\' => {
                    escaped.push('\\');
                    escaped.push(ch);
                }
                '\n' | '\r' => escaped.push(' '),
                _ => escaped.push(ch),
            }
        }
        escaped.push(')');
        escaped
    } else {
        let mut hex = String::from("<FEFF");
        for unit in text.encode_utf16() {
            let _ = write!(hex, "{unit:04X}");
        }
        hex.push('>');
        hex
    }
}

/// A line of page text: ASCII only (the generator uses a standard font without an encoding).
fn text_literal(text: &str) -> String {
    let ascii: String = text
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { '?' })
        .collect();
    pdf_string(&ascii)
}

fn number(value: f32) -> String {
    format!("{value}")
}

fn rect_array(rect: [f32; 4]) -> String {
    format!(
        "[{} {} {} {}]",
        number(rect[0]),
        number(rect[1]),
        number(rect[2]),
        number(rect[3])
    )
}

struct Objects {
    bodies: Vec<(u32, Vec<u8>)>,
}

impl Objects {
    fn add(&mut self, id: u32, body: impl Into<Vec<u8>>) {
        self.bodies.push((id, body.into()));
    }

    fn add_stream(&mut self, id: u32, content: &str) {
        let mut body = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        body.extend_from_slice(content.as_bytes());
        body.extend_from_slice(b"endstream");
        self.add(id, body);
    }
}

/// Adds the outline items of one level (and, recursively, their children) and returns the ids of
/// the first and last of them and the number of items below and including them.
fn add_outline_level(
    objects: &mut Objects,
    items: &[OutlineSpec],
    parent: u32,
    next_id: &mut u32,
    page_ids: &[u32],
) -> (u32, u32, usize) {
    let ids: Vec<u32> = items
        .iter()
        .map(|_| {
            let id = *next_id;
            *next_id += 1;
            id
        })
        .collect();
    let mut total = 0;
    for (index, item) in items.iter().enumerate() {
        let (first_child, last_child, below) =
            add_outline_level(objects, &item.children, ids[index], next_id, page_ids);
        total += 1 + below;
        let mut body = format!("<< /Title {} /Parent {parent} 0 R", pdf_string(&item.title));
        if index > 0 {
            let _ = write!(body, " /Prev {} 0 R", ids[index - 1]);
        }
        if index + 1 < ids.len() {
            let _ = write!(body, " /Next {} 0 R", ids[index + 1]);
        }
        if below > 0 {
            let _ = write!(
                body,
                " /First {first_child} 0 R /Last {last_child} 0 R /Count {below}"
            );
        }
        let page_id = page_ids.get(item.page).copied().unwrap_or(page_ids[0]);
        let _ = write!(body, " /Dest [{page_id} 0 R /Fit] >>");
        objects.add(ids[index], body);
    }
    (
        ids.first().copied().unwrap_or(0),
        ids.last().copied().unwrap_or(0),
        total,
    )
}

/// A valid PDF of the document: a pure function of `spec` (see the module documentation).
///
/// Object numbers: 1 the catalog, 2 the page tree, 3 the font, then the information dictionary if
/// there is a title, then for each page its object, its content stream and its link
/// annotations, then the outline.
pub fn document_pdf(spec: &DocumentSpec) -> Vec<u8> {
    assert!(!spec.pages.is_empty(), "a PDF needs at least one page");
    let catalog = 1_u32;
    let tree = 2_u32;
    let font = 3_u32;
    let mut next_id = 4_u32;
    let info = spec.title.as_ref().map(|_| {
        let id = next_id;
        next_id += 1;
        id
    });
    struct PageIds {
        page: u32,
        content: u32,
        annotations: Vec<u32>,
    }
    let ids: Vec<PageIds> = spec
        .pages
        .iter()
        .map(|page| {
            let page_id = next_id;
            let content = next_id + 1;
            next_id += 2;
            let annotations = page
                .links
                .iter()
                .map(|_| {
                    let id = next_id;
                    next_id += 1;
                    id
                })
                .collect();
            PageIds {
                page: page_id,
                content,
                annotations,
            }
        })
        .collect();
    let page_ids: Vec<u32> = ids.iter().map(|ids| ids.page).collect();

    let mut objects = Objects { bodies: Vec::new() };
    let outlines = (!spec.outline.is_empty()).then(|| {
        let id = next_id;
        next_id += 1;
        id
    });
    let mut catalog_body = format!("<< /Type /Catalog /Pages {tree} 0 R");
    if let Some(outlines) = outlines {
        let _ = write!(
            catalog_body,
            " /Outlines {outlines} 0 R /PageMode /UseOutlines"
        );
    }
    catalog_body.push_str(" >>");
    objects.add(catalog, catalog_body);

    let kids: Vec<String> = page_ids.iter().map(|id| format!("{id} 0 R")).collect();
    objects.add(
        tree,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            kids.len()
        ),
    );
    objects.add(
        font,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    if let (Some(id), Some(title)) = (info, &spec.title) {
        objects.add(
            id,
            format!(
                "<< /Title {} /Producer (Mercury test generator) >>",
                pdf_string(title)
            ),
        );
    }

    for (page, ids) in spec.pages.iter().zip(&ids) {
        let mut body = format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox {} /Resources << /Font << /F1 {font} 0 R >> >> /Contents {} 0 R",
            rect_array([0.0, 0.0, page.width, page.height]),
            ids.content
        );
        if let Some(crop) = page.crop_box {
            let _ = write!(body, " /CropBox {}", rect_array(crop));
        }
        if page.rotate != 0 {
            let _ = write!(body, " /Rotate {}", page.rotate);
        }
        if !ids.annotations.is_empty() {
            let refs: Vec<String> = ids
                .annotations
                .iter()
                .map(|id| format!("{id} 0 R"))
                .collect();
            let _ = write!(body, " /Annots [{}]", refs.join(" "));
        }
        body.push_str(" >>");
        objects.add(ids.page, body);

        let [a, b, c, d] = page.text_matrix();
        let mut content = String::new();
        for (rect, rgb) in &page.fills {
            let [left, bottom, right, top] = page.user_rect(*rect);
            let _ = writeln!(
                content,
                "{} {} {} rg {} {} {} {} re f",
                number(rgb[0]),
                number(rgb[1]),
                number(rgb[2]),
                number(left),
                number(bottom),
                number(right - left),
                number(top - bottom)
            );
        }
        for (index, line) in page.lines.iter().enumerate() {
            let baseline = LINE_TOP + index as f32 * LINE_PITCH + 10.0;
            let (x, y) = page.user_point(LINE_LEFT, baseline);
            let _ = writeln!(
                content,
                "BT /F1 {} Tf {} {} {} {} {} {} Tm {} Tj ET",
                number(FONT_SIZE),
                number(a),
                number(b),
                number(c),
                number(d),
                number(x),
                number(y),
                text_literal(line)
            );
        }
        objects.add_stream(ids.content, &content);

        for (link, id) in page.links.iter().zip(&ids.annotations) {
            let annotation = match link {
                LinkSpec::Uri { rect, uri } => format!(
                    "<< /Type /Annot /Subtype /Link /Rect {} /Border [0 0 0] /A << /S /URI /URI {} >> >>",
                    rect_array(page.user_rect(*rect)),
                    pdf_string(uri)
                ),
                LinkSpec::GoTo {
                    rect,
                    page: target,
                    y,
                } => {
                    let target_spec = &spec.pages[(*target).min(spec.pages.len() - 1)];
                    let target_id = page_ids[(*target).min(page_ids.len() - 1)];
                    let destination = match y {
                        Some(y) => {
                            let (left, top) = target_spec.user_point(0.0, *y);
                            format!("[{target_id} 0 R /XYZ {} {} 0]", number(left), number(top))
                        }
                        None => format!("[{target_id} 0 R /Fit]"),
                    };
                    format!(
                        "<< /Type /Annot /Subtype /Link /Rect {} /Border [0 0 0] /Dest {destination} >>",
                        rect_array(page.user_rect(*rect))
                    )
                }
            };
            objects.add(*id, annotation);
        }
    }

    if let Some(outlines) = outlines {
        let mut id_counter = next_id;
        let (first, last, total) = add_outline_level(
            &mut objects,
            &spec.outline,
            outlines,
            &mut id_counter,
            &page_ids,
        );
        objects.add(
            outlines,
            format!("<< /Type /Outlines /First {first} 0 R /Last {last} 0 R /Count {total} >>"),
        );
    }

    objects.bodies.sort_by_key(|(id, _)| *id);
    let mut file = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.bodies.len());
    for (id, body) in &objects.bodies {
        offsets.push(file.len());
        file.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        file.extend_from_slice(body);
        file.extend_from_slice(b"\nendobj\n");
    }
    let xref_offset = file.len();
    let size = objects.bodies.last().map_or(1, |(id, _)| id + 1);
    file.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    file.extend_from_slice(b"0000000000 65535 f \n");
    for entry in 1..size {
        match objects.bodies.iter().position(|(id, _)| *id == entry) {
            Some(index) => {
                file.extend_from_slice(format!("{:010} 00000 n \n", offsets[index]).as_bytes());
            }
            None => file.extend_from_slice(b"0000000000 00000 f \n"),
        }
    }
    let mut trailer = format!("trailer\n<< /Size {size} /Root {catalog} 0 R");
    if let Some(info) = info {
        let _ = write!(trailer, " /Info {info} 0 R");
    }
    let _ = write!(trailer, " >>\nstartxref\n{xref_offset}\n%%EOF\n");
    file.extend_from_slice(trailer.as_bytes());
    file
}

/// The text a three-pages fixture page shows, line by line.
pub fn three_pages_lines(page: usize) -> Vec<String> {
    let number = page + 1;
    vec![
        format!("Mercury PDF test page {number}"),
        "The quick brown fox jumps over the lazy dog.".to_owned(),
        format!("The word fox appears on page {number}."),
        "Visit the example site.".to_owned(),
        "Jump to the last page.".to_owned(),
    ]
}

/// The document behind `tests/pages/pdf/three-pages.pdf`: three Letter pages with the same few
/// lines of text, a URI link and a GoTo link on the first page, and a two-level outline.
pub fn three_pages_spec() -> DocumentSpec {
    let mut pages: Vec<PageSpec> = (0..3)
        .map(|index| PageSpec::letter(three_pages_lines(index)))
        .collect();
    // Line 3 (index 3) is "Visit the example site." and line 4 "Jump to the last page.".
    let line_rect = |line: usize, width: f32| {
        let top = LINE_TOP + line as f32 * LINE_PITCH;
        [LINE_LEFT, top, LINE_LEFT + width, top + FONT_SIZE]
    };
    pages[0].links = vec![
        LinkSpec::Uri {
            rect: line_rect(3, 130.0),
            uri: "https://example.com/mercury".to_owned(),
        },
        LinkSpec::GoTo {
            rect: line_rect(4, 130.0),
            page: 2,
            y: Some(100.0),
        },
    ];
    pages[2].links = vec![LinkSpec::GoTo {
        rect: line_rect(4, 130.0),
        page: 0,
        y: None,
    }];
    DocumentSpec {
        title: Some("Mercury three-page test".to_owned()),
        pages,
        outline: vec![
            OutlineSpec::new("Chapter 1", 0, Vec::new()),
            OutlineSpec::new(
                "Chapter 2",
                1,
                vec![OutlineSpec::new("Section 2.1", 2, Vec::new())],
            ),
        ],
    }
}

/// The document behind `tests/pages/pdf/rotated.pdf`: four pages that each have a crop box that
/// does not start at the origin, with `/Rotate` 90, 270, 180 and 0. Their text is written so that
/// it reads upright on the shown page.
pub fn rotated_spec() -> DocumentSpec {
    let page = |width: f32, height: f32, crop: [f32; 4], rotate: u16, number: usize| {
        let mut spec = PageSpec::letter([
            format!("Rotated page {number}"),
            "Upright text here".to_owned(),
            format!("FindMe {number}"),
        ]);
        spec.width = width;
        spec.height = height;
        spec.crop_box = Some(crop);
        spec.rotate = rotate;
        spec
    };
    let mut pages = vec![
        page(612.0, 792.0, [50.0, 60.0, 450.0, 360.0], 90, 1),
        page(595.0, 842.0, [20.0, 30.0, 575.0, 800.0], 270, 2),
        page(612.0, 792.0, [10.0, 20.0, 410.0, 320.0], 180, 3),
        page(612.0, 792.0, [36.0, 72.0, 576.0, 720.0], 0, 4),
    ];
    let top = LINE_TOP + LINE_PITCH;
    pages[0].links = vec![LinkSpec::Uri {
        rect: [LINE_LEFT, top, LINE_LEFT + 90.0, top + FONT_SIZE],
        uri: "https://example.com/rotated".to_owned(),
    }];
    DocumentSpec {
        title: Some("Mercury rotated test".to_owned()),
        pages,
        outline: vec![OutlineSpec::new("First", 0, Vec::new())],
    }
}

// ---------------------------------------------------------------------------------------------
// In-memory pipes and a harness for the request loop.
// ---------------------------------------------------------------------------------------------

/// The writing end of [`pipe`]. Dropping it ends the stream for the reader.
#[derive(Debug)]
pub struct PipeWriter {
    sender: Sender<Vec<u8>>,
}

impl Write for PipeWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.sender
            .send(buffer.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the pipe is closed"))?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The reading end of [`pipe`]: blocks until bytes arrive and reads zero bytes once the writer is
/// dropped and everything sent was read. After [`PipeReader::with_timeout`] it fails with
/// `TimedOut` instead of waiting forever; after [`PipeReader::with_cancel`] it reads zero bytes
/// (the end of the stream) as soon as the flag is set.
#[derive(Debug)]
pub struct PipeReader {
    receiver: Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
    timeout: Option<Duration>,
    cancel: Option<Arc<AtomicBool>>,
}

/// How often a reader with a timeout or a cancel flag looks at them.
const POLL: Duration = Duration::from_millis(20);

impl PipeReader {
    /// A reader that gives up after `timeout` without data.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// A reader that ends the stream when `flag` becomes true.
    pub fn with_cancel(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }

    /// Waits for the next chunk; `Ok(false)` at the end of the stream.
    fn fill(&mut self) -> io::Result<bool> {
        let started = Instant::now();
        loop {
            if self
                .cancel
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
            {
                return Ok(false);
            }
            let received = if self.cancel.is_none() && self.timeout.is_none() {
                self.receiver
                    .recv()
                    .map_err(|_| RecvTimeoutError::Disconnected)
            } else {
                self.receiver.recv_timeout(POLL)
            };
            match received {
                Ok(chunk) => {
                    self.pending.extend(chunk);
                    return Ok(true);
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(false),
                Err(RecvTimeoutError::Timeout) => {
                    if self.timeout.is_some_and(|limit| started.elapsed() >= limit) {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "pipe read timed out",
                        ));
                    }
                }
            }
        }
    }
}

impl Read for PipeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.pending.is_empty() && !self.fill()? {
            return Ok(0);
        }
        let count = buffer.len().min(self.pending.len());
        for slot in buffer.iter_mut().take(count) {
            *slot = self.pending.pop_front().unwrap_or(0);
        }
        Ok(count)
    }
}

/// An in-memory pipe.
pub fn pipe() -> (PipeWriter, PipeReader) {
    let (sender, receiver) = mpsc::channel();
    (
        PipeWriter { sender },
        PipeReader {
            receiver,
            pending: VecDeque::new(),
            timeout: None,
            cancel: None,
        },
    )
}

/// The default time [`ServeHarness`] waits for a reply before it fails the test.
pub const HARNESS_TIMEOUT: Duration = Duration::from_secs(20);

/// Runs [`serve`] on a thread with a backend and talks to it like the client does.
pub struct ServeHarness {
    requests: Option<PipeWriter>,
    replies: PipeReader,
    thread: Option<JoinHandle<ServeExit>>,
}

impl ServeHarness {
    /// Starts the loop. The first reply to read is `Ready`.
    pub fn start<B: PdfBackend + Send + 'static>(backend: B) -> Self {
        let (request_writer, request_reader) = pipe();
        let (reply_writer, reply_reader) = pipe();
        let thread = std::thread::Builder::new()
            .name("serve-harness".to_owned())
            .spawn(move || serve(backend, request_reader, reply_writer))
            .expect("the harness thread starts");
        Self {
            requests: Some(request_writer),
            replies: reply_reader.with_timeout(HARNESS_TIMEOUT),
            thread: Some(thread),
        }
    }

    /// Sends a request.
    pub fn send(&mut self, request: &Request) {
        let writer = self.requests.as_mut().expect("the input is still open");
        write_request(writer, request).expect("the request is sent");
    }

    /// Sends raw bytes (a malformed frame, for instance).
    pub fn send_raw(&mut self, bytes: &[u8]) {
        let writer = self.requests.as_mut().expect("the input is still open");
        writer.write_all(bytes).expect("the bytes are sent");
    }

    /// Reads the next reply and its blob; panics on a timeout or a malformed frame, and returns
    /// `None` when the loop has ended and everything was read.
    pub fn recv_opt(&mut self) -> Option<(Reply, Vec<u8>)> {
        read_reply(&mut self.replies).expect("a well-formed reply")
    }

    /// Reads the next reply; panics if the loop ended.
    pub fn recv(&mut self) -> (Reply, Vec<u8>) {
        self.recv_opt()
            .expect("a reply before the end of the stream")
    }

    /// Reads replies until `matches` accepts one, returning the ones skipped and the match.
    pub fn recv_until(
        &mut self,
        mut matches: impl FnMut(&Reply) -> bool,
    ) -> (Vec<Reply>, (Reply, Vec<u8>)) {
        let mut skipped = Vec::new();
        loop {
            let (reply, blob) = self.recv();
            if matches(&reply) {
                return (skipped, (reply, blob));
            }
            skipped.push(reply);
        }
    }

    /// Ends the input (the browser went away) without waiting.
    pub fn close_input(&mut self) {
        self.requests = None;
    }

    /// Waits for the loop to end and says how. Closes the input first.
    pub fn finish(mut self) -> ServeExit {
        self.requests = None;
        self.thread
            .take()
            .expect("the thread is joined once")
            .join()
            .expect("the serve thread does not panic")
    }

    /// Reads every remaining reply until the stream ends.
    pub fn drain(&mut self) -> Vec<(Reply, Vec<u8>)> {
        let mut rest = Vec::new();
        while let Some(item) = self.recv_opt() {
            rest.push(item);
        }
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_carries_bytes_and_ends_when_the_writer_drops() {
        let (mut writer, mut reader) = pipe();
        writer.write_all(b"hello").unwrap();
        drop(writer);
        let mut text = String::new();
        reader.read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello");
    }

    #[test]
    fn a_timeout_pipe_fails_instead_of_waiting() {
        let (_writer, reader) = pipe();
        let mut reader = reader.with_timeout(Duration::from_millis(20));
        let error = reader.read(&mut [0; 4]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn writing_to_a_dropped_reader_is_a_broken_pipe() {
        let (mut writer, reader) = pipe();
        drop(reader);
        assert_eq!(
            writer.write(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn the_generator_is_deterministic_and_structurally_valid() {
        let spec = three_pages_spec();
        let first = document_pdf(&spec);
        assert_eq!(first, document_pdf(&spec));
        let text = String::from_utf8_lossy(&first);
        assert!(text.starts_with("%PDF-1.4\n"));
        assert!(text.ends_with("%%EOF\n"));
        assert!(text.contains("/Count 3"));
        assert!(text.contains("/Outlines"));
        assert!(text.contains("/URI (https://example.com/mercury)"));
    }

    #[test]
    fn every_cross_reference_entry_points_at_its_object() {
        for spec in [three_pages_spec(), rotated_spec()] {
            let file = document_pdf(&spec);
            // The binary comment after the header is not text; every byte becomes one character
            // so that offsets in the string are offsets in the file.
            let text: String = file
                .iter()
                .map(|byte| if byte.is_ascii() { *byte as char } else { '?' })
                .collect();
            let start = text.rfind("startxref\n").expect("startxref") + "startxref\n".len();
            let xref_offset: usize = text[start..]
                .lines()
                .next()
                .unwrap()
                .parse()
                .expect("an offset");
            assert!(text[xref_offset..].starts_with("xref\n0 "));
            let table: Vec<&str> = text[xref_offset..].lines().collect();
            let count: usize = table[1].split(' ').nth(1).unwrap().parse().unwrap();
            for number in 1..count {
                let entry = table[2 + number];
                assert_eq!(entry.len(), 19, "entries are 20 bytes with their newline");
                let offset: usize = entry[..10].parse().unwrap();
                assert!(
                    text[offset..].starts_with(&format!("{number} 0 obj\n")),
                    "object {number} is not at {offset}"
                );
            }
        }
    }

    #[test]
    fn text_is_laid_out_in_the_shown_page_and_mapped_back() {
        let mut page = PageSpec::letter(["x"]);
        page.crop_box = Some([50.0, 60.0, 450.0, 360.0]);
        // Without a rotation the top-left of the shown page is the top-left of the crop box.
        assert_eq!(page.user_point(0.0, 0.0), (50.0, 360.0));
        page.rotate = 90;
        assert_eq!(page.shown_size(), (300.0, 400.0));
        // The shown top-left corner is the bottom-left of the box before the turn.
        assert_eq!(page.user_point(0.0, 0.0), (50.0, 60.0));
        // The shown top-right corner is the top-left of the box.
        assert_eq!(page.user_point(300.0, 0.0), (50.0, 360.0));
        page.rotate = 270;
        assert_eq!(page.user_point(0.0, 0.0), (450.0, 360.0));
        page.rotate = 180;
        assert_eq!(page.user_point(0.0, 0.0), (450.0, 60.0));
    }

    #[test]
    fn titles_outside_ascii_are_written_as_utf16() {
        assert_eq!(pdf_string("a(b)"), "(a\\(b\\))");
        assert_eq!(pdf_string("é"), "<FEFF00E9>");
    }
}
