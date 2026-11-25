/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: limits how deeply an SVG image may nest (decision D8).
//!
//! **What goes wrong.** `usvg` walks and converts a document with functions that call themselves once per
//! level of nesting, and Servo runs it on a rayon worker, whose stack is Rust's default 2 MiB. usvg 0.48.1
//! refuses a document deeper than 1024 levels on its own (`NodesLimitReached`; a `<use>` counts as one more
//! level of the tree it expands), so a tiny file nested 100,000 deep does not get far. The crash is below
//! that limit. Measured on a 2 MiB thread with this usvg: nested `<svg>` elements overflow the stack from
//! about 700 levels (2 to 3 MiB at 1024), and the other elements tried (`g`, `a`, `switch`, opacity,
//! transform, clip path, mask, filter) need at most 2 MiB at 1024. The XML depth is also not the depth usvg
//! recurses over: a `<use>` puts the whole tree it links to under itself, so four groups of 200 nested
//! `<svg>` elements, each ending in a `<use>` of the one before, are 202 levels deep in the file and about
//! 800 deep for usvg, and overflow the stack too. An `<image>` whose `href` is a `data:` SVG is parsed by the
//! same functions, on the same stack, inside the outer conversion.
//!
//! Servo fixed this in servo/servo#49835 by setting `usvg::Options::max_depth` to 500, an option that
//! exists from usvg 0.50. This tree has usvg 0.48.1, and moving resvg also touches `servo-malloc-size-of`,
//! which Mercury does not carry. This file does the job with what 0.48.1 offers, in three parts:
//!
//! 1. **The depth of the parsed document.** [`parse_tree`] does what `usvg::Tree::from_data` does (unpack
//!    SVGZ, read UTF-8, parse with roxmltree, `allow_dtd` on), counts the nesting of the elements on the
//!    parsed document, so entities declared in a `<!DOCTYPE>` that expand to elements are counted, and
//!    refuses more than [`MAX_SVG_DEPTH`] (500, upstream's constant). It then hands the parsed document to
//!    `usvg::Tree::from_xmltree`. The count walks the nodes in order and never recurses, and roxmltree's
//!    parser does not recurse either.
//! 2. **Nested `data:` images.** [`data_resolver`] wraps usvg's own resolver for `data:` URLs. An SVG
//!    inside an SVG is counted the same way, against the same budget: the depths of the documents that are
//!    being converted at once add up to at most 500, and there are at most [`MAX_NESTED_DOCUMENTS`] of them.
//!    A nested image over budget is left out (the `<image>` draws nothing); the outer image still loads.
//! 3. **A large stack.** [`parse_tree`] runs on a thread of its own with [`PARSE_STACK_BYTES`] of stack,
//!    because the depth that is counted is the file's, and the depth usvg recurses over is up to 1024
//!    (`<use>`). 32 MiB is ten times what 1024 levels of nested `<svg>` need, for each of the four nested
//!    documents. Rendering (`resvg::render`) needs less than 512 KiB for the same trees (measured at 1024
//!    levels), and dropping a tree is a loop over a vector per level, so they stay on the worker.
//! 4. **A size limit on SVGZ.** usvg 0.48.1 unpacks gzip with `read_to_end` and no limit, so a few kilobytes of
//!    gzip in `<img src="x.svgz">` (or in a nested `data:` image) could fill the memory before the depth count
//!    starts. [`unpack`] reads through `flate2`'s `GzDecoder` limited to [`MAX_SVG_BYTES`] + 1 bytes and refuses
//!    anything larger; a nested image over the limit is left out before usvg's own resolver can unpack it.
//!
//! This is a Mercury file. Drop it (with the two calls in `image_cache.rs` and the `mod mercury_svg;` line
//! in `lib.rs`) at the Servo bump that takes resvg 0.50, and set `max_depth: 500` there as upstream does.

use std::borrow::Cow;
use std::cell::Cell;
use std::io::Read;
use std::sync::Arc;

use log::warn;
use resvg::usvg::{self, ImageHrefDataResolverFn, roxmltree};

/// The deepest nesting of elements Mercury accepts: the root is depth 1. Upstream's constant (servo#49835).
/// For SVG images that nest through `data:` URLs this is the total over the documents being converted at once.
pub(crate) const MAX_SVG_DEPTH: u32 = 500;

/// How many SVG documents may be converted inside one another: the image itself and three `data:` images.
pub(crate) const MAX_NESTED_DOCUMENTS: u32 = 4;

/// The stack of the thread that parses an SVG: ten times what 1024 levels of nested `<svg>` elements
/// (the most expensive element measured, 2 to 3 MiB) need, for each of [`MAX_NESTED_DOCUMENTS`].
const PARSE_STACK_BYTES: usize = 32 << 20;

/// The error `image_cache.rs` reports for a document that is too deeply nested.
pub(crate) const TOO_DEEP: &str = "SVG is nested too deeply";

/// The error for any other document usvg or roxmltree cannot read.
pub(crate) const INVALID: &str = "Not a valid SVG document";

/// The most an SVG may be once unpacked: 32 MiB. An SVGZ can be a thousand times smaller than its content, and
/// usvg 0.48.1 unpacks without a limit, so one `<img src="x.svgz">` could otherwise take the memory of the process.
pub(crate) const MAX_SVG_BYTES: usize = 32 << 20;

/// The two magic bytes at the start of a gzip stream (an `.svgz` file).
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// The SVG documents being converted on this thread, outermost first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Chain {
    /// How many.
    documents: u32,
    /// The sum of their depths.
    depth: u32,
}

thread_local! {
    static CHAIN: Cell<Chain> = const { Cell::new(Chain { documents: 0, depth: 0 }) };
}

/// A document of some depth entered into the chain; it leaves the chain when this is dropped.
struct Entered {
    previous: Chain,
}

impl Entered {
    /// `None` when the document would take the chain over [`MAX_SVG_DEPTH`] levels or
    /// [`MAX_NESTED_DOCUMENTS`] documents.
    fn new(depth: u32) -> Option<Entered> {
        let previous = CHAIN.get();
        if previous.documents >= MAX_NESTED_DOCUMENTS ||
            previous.depth.saturating_add(depth) > MAX_SVG_DEPTH
        {
            return None;
        }
        CHAIN.set(Chain {
            documents: previous.documents + 1,
            depth: previous.depth + depth,
        });
        Some(Entered { previous })
    }
}

impl Drop for Entered {
    fn drop(&mut self) {
        CHAIN.set(self.previous);
    }
}

/// What `usvg::Tree::from_str` passes to roxmltree.
fn xml_options<'a>() -> roxmltree::ParsingOptions<'a> {
    roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    }
}

/// Why an SVGZ stream was refused by [`unpack`].
#[derive(Debug)]
enum UnpackError {
    /// Not a valid gzip stream.
    Gzip(std::io::Error),
    /// More than [`MAX_SVG_BYTES`] come out of it.
    TooLarge,
}

impl std::fmt::Display for UnpackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnpackError::Gzip(error) => write!(formatter, "{error}"),
            UnpackError::TooLarge => write!(
                formatter,
                "the SVGZ file unpacks to more than {MAX_SVG_BYTES} bytes"
            ),
        }
    }
}

/// The bytes of an SVG, unpacked when they are gzip. `Err` for malformed gzip, or when more than
/// [`MAX_SVG_BYTES`] come out of it: the reader stops one byte past the limit, so a few kilobytes of gzip
/// cannot be made to fill the memory (`usvg::decompress_svgz` reads to the end without a limit). A gzip stream
/// inside a gzip stream comes out as gzip, which is not UTF-8, so it is never read as XML afterwards.
fn unpack(bytes: &[u8]) -> Result<Cow<'_, [u8]>, UnpackError> {
    if !bytes.starts_with(&GZIP_MAGIC) {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut unpacked = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_SVG_BYTES as u64 + 1)
        .read_to_end(&mut unpacked)
        .map_err(UnpackError::Gzip)?;
    if unpacked.len() > MAX_SVG_BYTES {
        return Err(UnpackError::TooLarge);
    }
    Ok(Cow::Owned(unpacked))
}

/// The nesting of the elements of `document`: the root element is 1. Stops counting when the depth is over
/// [`MAX_SVG_DEPTH`] and returns the first depth over it, so a document that goes deep early is not walked to
/// the end. One pass over the nodes in document order; a stack of the open elements stands in for recursion.
fn element_depth(document: &roxmltree::Document<'_>) -> u32 {
    let mut open: Vec<roxmltree::NodeId> = Vec::new();
    let mut deepest = 0usize;
    for node in document
        .root()
        .descendants()
        .filter(|node| node.is_element())
    {
        let parent = node.parent_element().map(|parent| parent.id());
        while let Some(&last) = open.last() {
            if Some(last) == parent {
                break;
            }
            open.pop();
        }
        open.push(node.id());
        deepest = deepest.max(open.len());
        if deepest > MAX_SVG_DEPTH as usize {
            break;
        }
    }
    u32::try_from(deepest).unwrap_or(u32::MAX)
}

/// Parses SVG `bytes` as `usvg::Tree::from_data` does, refusing a document that is nested too deeply (see the
/// module documentation), on a thread with a large stack. The error is [`TOO_DEEP`] or [`INVALID`]; the
/// reason is logged.
pub(crate) fn parse_tree(
    bytes: &[u8],
    options: &usvg::Options,
) -> Result<usvg::Tree, &'static str> {
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("MercurySvgParse".to_owned())
            .stack_size(PARSE_STACK_BYTES)
            .spawn_scoped(scope, || parse_tree_here(bytes, options));
        match spawned {
            Ok(handle) => handle.join().unwrap_or_else(|_| {
                warn!("The SVG parser panicked; the image is refused");
                Err(INVALID)
            }),
            Err(error) => {
                warn!(
                    "Could not start a thread with a large stack for an SVG ({error}); parsing on this thread"
                );
                parse_tree_here(bytes, options)
            },
        }
    })
}

/// [`parse_tree`] on the current thread.
fn parse_tree_here(bytes: &[u8], options: &usvg::Options) -> Result<usvg::Tree, &'static str> {
    let invalid = |error: &dyn std::fmt::Display| {
        warn!("Error when parsing SVG data: {error}");
        INVALID
    };
    let data = unpack(bytes).map_err(|error| invalid(&error))?;
    let text = std::str::from_utf8(&data).map_err(|error| invalid(&error))?;
    let document = roxmltree::Document::parse_with_options(text, xml_options())
        .map_err(|error| invalid(&error))?;
    let depth = element_depth(&document);
    let Some(_entered) = Entered::new(depth) else {
        warn!("Refusing an SVG image: {TOO_DEEP} (more than {MAX_SVG_DEPTH} levels)");
        return Err(TOO_DEEP);
    };
    usvg::Tree::from_xmltree(&document, options).map_err(|error| invalid(&error))
}

/// usvg's resolver for `data:` URLs in `<image href>`, with the nesting limits applied to an SVG inside an SVG.
/// Replaces `usvg::ImageHrefResolver::default_data_resolver()` in the options Servo builds.
pub(crate) fn data_resolver() -> ImageHrefDataResolverFn<'static> {
    let default = usvg::ImageHrefResolver::default_data_resolver();
    Box::new(
        move |mime: &str, data: Arc<Vec<u8>>, options: &usvg::Options| {
            // The default resolver reads a `data:` URL as an SVG only for these two types (for `text/plain`
            // after it has ruled out the raster formats by their first bytes); every other type is an
            // image format that is not parsed here, or nothing.
            if !matches!(mime, "image/svg+xml" | "text/plain") {
                return default(mime, data, options);
            }
            let _entered = match nested_depth(&data) {
                // Bad gzip, or more than MAX_SVG_BYTES once unpacked: usvg's resolver would unpack it
                // again without a limit, so it never sees it. The `<image>` draws nothing.
                Err(error) => {
                    warn!("Leaving out an SVG image inside an SVG image: {error}");
                    return None;
                },
                // Not XML at all (a PNG sent as `text/plain`, say): usvg decides what it is.
                Ok(None) => None,
                Ok(Some(depth)) => {
                    let entered = Entered::new(depth);
                    if entered.is_none() {
                        warn!(
                            "Leaving out an SVG image inside an SVG image: {TOO_DEEP} \
                             (it is {depth} levels deep; at most {MAX_SVG_DEPTH} levels and \
                             {MAX_NESTED_DOCUMENTS} documents in all)"
                        );
                        return None;
                    }
                    entered
                },
            };
            default(mime, data, options)
        },
    )
}

/// The nesting of the XML in `data` (unpacked if it is gzip): `Ok(None)` if it is not XML that roxmltree reads,
/// `Err` if it is gzip that cannot be unpacked within [`MAX_SVG_BYTES`].
fn nested_depth(data: &[u8]) -> Result<Option<u32>, UnpackError> {
    let data = unpack(data)?;
    let Ok(text) = std::str::from_utf8(&data) else {
        return Ok(None);
    };
    Ok(roxmltree::Document::parse_with_options(text, xml_options())
        .ok()
        .map(|document| element_depth(&document)))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use base64::Engine;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use usvg::ImageKind;

    use super::*;

    const HEAD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">"#;

    /// `levels` elements deep: the `<svg>` root, `levels - 1` nested `<g>`, and a `<rect>` beside them.
    fn nested(levels: usize) -> Vec<u8> {
        let mut svg =
            format!(r#"<?xml version="1.0"?>{HEAD}<rect width="10" height="10" fill="green"/>"#);
        svg.push_str(&"<g>".repeat(levels - 1));
        svg.push_str(&"</g>".repeat(levels - 1));
        svg.push_str("</svg>");
        svg.into_bytes()
    }

    /// `levels` nested `<svg>` elements, the root included, and a `<rect>` beside them: the most expensive
    /// element for usvg's stack.
    fn nested_svg(levels: usize) -> Vec<u8> {
        let mut svg = format!(r#"{HEAD}<rect width="10" height="10" fill="green"/>"#);
        svg.push_str(&r#"<svg width="10" height="10">"#.repeat(levels - 1));
        svg.push_str(&"</svg>".repeat(levels - 1));
        svg.push_str("</svg>");
        svg.into_bytes()
    }

    fn depth_of(text: &str) -> u32 {
        element_depth(&roxmltree::Document::parse_with_options(text, xml_options()).unwrap())
    }

    /// Options as Servo builds them: the guard's resolver, nothing from files.
    fn options() -> usvg::Options<'static> {
        usvg::Options {
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: data_resolver(),
                resolve_string: Box::new(|_, _| None),
            },
            ..usvg::Options::default()
        }
    }

    fn parse(bytes: &[u8]) -> Result<usvg::Tree, &'static str> {
        parse_tree(bytes, &options())
    }

    /// Runs `body` on a thread with the 2 MiB stack of a rayon worker.
    fn on_a_worker_stack<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(body)
            .unwrap()
            .join()
            .unwrap()
    }

    /// The `<image>` nodes of a tree: (SVG images, other images).
    fn images(tree: &usvg::Tree) -> (usize, usize) {
        fn walk(group: &usvg::Group, counts: &mut (usize, usize)) {
            for node in group.children() {
                match node {
                    usvg::Node::Group(inner) => walk(inner, counts),
                    usvg::Node::Image(image) => match image.kind() {
                        ImageKind::SVG(_) => counts.0 += 1,
                        _ => counts.1 += 1,
                    },
                    _ => {},
                }
            }
        }
        let mut counts = (0, 0);
        walk(tree.root(), &mut counts);
        counts
    }

    /// How many SVG images sit inside one another below `tree`.
    fn chain_length(tree: &usvg::Tree) -> usize {
        fn find(group: &usvg::Group) -> Option<&usvg::Tree> {
            group.children().iter().find_map(|node| match node {
                usvg::Node::Group(inner) => find(inner),
                usvg::Node::Image(image) => match image.kind() {
                    ImageKind::SVG(inner) => Some(inner),
                    _ => None,
                },
                _ => None,
            })
        }
        find(tree.root()).map_or(0, |inner| 1 + chain_length(inner))
    }

    fn data_url(mime: &str, bytes: &[u8]) -> String {
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    /// An SVG that holds one `<image>` with `href`.
    fn with_image(href: &str) -> Vec<u8> {
        format!(r#"{HEAD}<image width="10" height="10" xlink:href="{href}"/></svg>"#).into_bytes()
    }

    /// An SVG of `levels` nested `<g>` with one `<image>` inside the innermost.
    fn with_image_at_depth(levels: usize, href: &str) -> Vec<u8> {
        format!(
            r#"{HEAD}{}<image width="10" height="10" xlink:href="{href}"/>{}</svg>"#,
            "<g>".repeat(levels - 1),
            "</g>".repeat(levels - 1)
        )
        .into_bytes()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn the_limits_are_the_upstream_constant_and_a_few_documents() {
        assert_eq!(MAX_SVG_DEPTH, 500);
        assert_eq!(MAX_NESTED_DOCUMENTS, 4);
    }

    #[test]
    fn the_depth_is_counted_on_the_parsed_document() {
        assert_eq!(depth_of("<svg/>"), 1);
        assert_eq!(depth_of("<svg><g><g/></g><g/></svg>"), 3);
        assert_eq!(depth_of(&String::from_utf8(nested(499)).unwrap()), 499);
        assert_eq!(depth_of(&String::from_utf8(nested(500)).unwrap()), 500);
        // Counting stops at the first depth over the limit.
        assert_eq!(depth_of(&String::from_utf8(nested(501)).unwrap()), 501);
        assert_eq!(depth_of(&String::from_utf8(nested(5000)).unwrap()), 501);
    }

    #[test]
    fn the_boundary_is_500_levels() {
        assert!(parse(&nested(499)).is_ok());
        assert!(parse(&nested(500)).is_ok());
        assert_eq!(parse(&nested(501)).unwrap_err(), TOO_DEEP);
        assert_eq!(parse(&nested(502)).unwrap_err(), TOO_DEEP);
        assert!(parse(&nested_svg(500)).is_ok());
    }

    #[test]
    fn the_range_between_the_limit_and_usvgs_own_is_refused() {
        // usvg alone accepts all of these (it refuses from 1025 levels). 600 is the case the first version of
        // this guard could not tell from usvg and no test covered; 1000 and 1024 are the edge of usvg's limit.
        for levels in [501, 600, 700, 800, 1000, 1024] {
            assert_eq!(
                parse(&nested(levels)).unwrap_err(),
                TOO_DEEP,
                "{levels} nested g"
            );
            assert_eq!(
                parse(&nested_svg(levels)).unwrap_err(),
                TOO_DEEP,
                "{levels} nested svg"
            );
        }
        let usvg_alone = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let options = usvg::Options::default();
                [600, 1000, 1024]
                    .iter()
                    .map(|&levels| usvg::Tree::from_data(&nested(levels), &options).is_ok())
                    .collect::<Vec<_>>()
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(
            usvg_alone,
            [true, true, true],
            "usvg 0.48.1 accepts them, so it is the guard that refuses"
        );
    }

    #[test]
    fn a_hundred_thousand_levels_are_refused_and_the_count_does_not_recurse() {
        // A recursive count, or a recursive parser, would overflow a 2 MiB stack long before 100,000 levels.
        let counted = on_a_worker_stack(|| {
            let text = String::from_utf8(nested(100_000)).unwrap();
            let document = roxmltree::Document::parse_with_options(&text, xml_options()).unwrap();
            element_depth(&document)
        });
        assert_eq!(counted, MAX_SVG_DEPTH + 1);
        assert_eq!(parse(&nested(100_000)).unwrap_err(), TOO_DEEP);
        // Opened and never closed: not well formed, refused as invalid, again without recursing.
        assert_eq!(
            parse("<g>".repeat(100_000).as_bytes()).unwrap_err(),
            INVALID
        );
    }

    #[test]
    fn a_wide_document_is_not_a_deep_one() {
        let flat = format!("{HEAD}{}</svg>", "<rect/>".repeat(100_000));
        assert_eq!(depth_of(&flat), 2);
        let siblings = format!("{HEAD}{}</svg>", "<g></g>".repeat(100_000));
        assert_eq!(depth_of(&siblings), 2);
        assert!(parse(flat.as_bytes()).is_ok());
    }

    #[test]
    fn a_self_closing_element_does_not_deepen_what_follows() {
        let doc = format!(
            "{HEAD}{}{}{}</svg>",
            "<g>".repeat(497),
            "<rect/>".repeat(100),
            "</g>".repeat(497)
        );
        assert_eq!(depth_of(&doc), 499);
        assert_eq!(
            depth_of(&format!("<svg>{}</svg>", "<g/>".repeat(100_000))),
            2
        );
    }

    #[test]
    fn comments_cdata_and_processing_instructions_hide_markup() {
        let hidden = format!(
            "{HEAD}<!-- {} --><style><![CDATA[ {} ]]></style><?x {} ?></svg>",
            "<g>".repeat(10_000),
            "<g>".repeat(10_000),
            "<g>".repeat(10_000)
        );
        assert_eq!(
            depth_of(&hidden),
            2,
            "svg and style: the rest is text, comment and PI"
        );
        assert!(parse(hidden.as_bytes()).is_ok());
    }

    #[test]
    fn quoted_attribute_values_may_hold_angle_brackets() {
        let doc = format!(
            r#"{HEAD}<g a="&lt;g&gt;&lt;g&gt;" b='>' c="/>" d='"&lt;g&gt;"'>{}{}</g></svg>"#,
            "<g>".repeat(10),
            "</g>".repeat(10)
        );
        assert_eq!(depth_of(&doc), 12);
    }

    #[test]
    fn an_entity_that_expands_to_elements_is_counted() {
        // The first guard skipped the whole DOCTYPE and said an entity adds at most 10 levels. roxmltree's limit
        // of 10 is on how deep entity references nest, not on how deep the markup inside one value is.
        let value = format!("{}{}", "<g>".repeat(600), "</g>".repeat(600));
        let doc = format!(r#"<!DOCTYPE svg [<!ENTITY x "{value}">]>{HEAD}&x;</svg>"#);
        assert_eq!(
            depth_of(&doc),
            MAX_SVG_DEPTH + 1,
            "the entity is expanded in the tree"
        );
        assert_eq!(parse(doc.as_bytes()).unwrap_err(), TOO_DEEP);

        // 400 levels in an entity is fine, and so is the entity used twice side by side.
        let value = format!("{}{}", "<g>".repeat(400), "</g>".repeat(400));
        let doc = format!(r#"<!DOCTYPE svg [<!ENTITY x "{value}">]>{HEAD}&x;&x;</svg>"#);
        assert_eq!(depth_of(&doc), 401);
        assert!(parse(doc.as_bytes()).is_ok());

        // Entities inside entities add their depths: 5 x 120 = 600 levels.
        let mut dtd = String::from(r#"<!ENTITY e0 "<g><rect/></g>">"#);
        for step in 1..=5 {
            let inner = format!("{}&e{};{}", "<g>".repeat(120), step - 1, "</g>".repeat(120));
            dtd.push_str(&format!(r#"<!ENTITY e{step} "{inner}">"#));
        }
        let doc = format!("<!DOCTYPE svg [{dtd}]>{HEAD}&e5;</svg>");
        assert_eq!(parse(doc.as_bytes()).unwrap_err(), TOO_DEEP);

        // A DOCTYPE whose internal subset is full of look-alikes is only a DOCTYPE.
        let doc = format!(
            r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
                <!ENTITY b 'x > y ] z'>
                <!-- <g><g> '"' ] > -->
            ]>{HEAD}<g/></svg>"#
        );
        assert_eq!(depth_of(&doc), 2);
    }

    #[test]
    fn what_is_not_xml_is_refused_as_invalid_not_as_deep() {
        for bytes in [
            &b""[..],
            b"plain text, no markup at all",
            b"<",
            b"< <3 <1 <. <- <= < g>",
            b"<svg><!-- never ends <g><g><g>",
            b"<svg><![CDATA[ never ends",
            b"<svg a=\"never ends>",
            b"<svg></",
            &[0x89, b'P', b'N', b'G', b'<', 0x00, 0xff, b'<', 0x1a, b'>'],
        ] {
            assert_eq!(parse(bytes).unwrap_err(), INVALID, "{bytes:?}");
        }
    }

    #[test]
    fn names_with_namespaces_and_non_ascii_letters_count() {
        let namespaced = format!(
            r#"<s:svg xmlns:s="http://www.w3.org/2000/svg">{}{}</s:svg>"#,
            "<s:g>".repeat(600),
            "</s:g>".repeat(600)
        );
        assert_eq!(parse(namespaced.as_bytes()).unwrap_err(), TOO_DEEP);
        let unicode = format!(
            "<r>{}{}</r>",
            "<\u{e9}l>".repeat(600),
            "</\u{e9}l>".repeat(600)
        );
        assert_eq!(depth_of(&unicode), MAX_SVG_DEPTH + 1);
    }

    #[test]
    fn an_svgz_file_is_unpacked_before_it_is_counted() {
        // Compressed, 100,000 levels are a few hundred bytes of gzip.
        let packed = gzip(&nested(100_000));
        assert!(packed.len() < 4096, "{} bytes", packed.len());
        assert_eq!(parse(&packed).unwrap_err(), TOO_DEEP);
        assert_eq!(parse(&gzip(&nested(600))).unwrap_err(), TOO_DEEP);
        assert!(parse(&gzip(&nested(20))).is_ok());
    }

    #[test]
    fn a_broken_or_doubled_gzip_stream_is_invalid() {
        let mut packed = gzip(&nested(20));
        packed.truncate(packed.len() / 2);
        assert_eq!(parse(&packed).unwrap_err(), INVALID);
        assert_eq!(parse(&gzip(&gzip(&nested(20)))).unwrap_err(), INVALID);
    }

    /// A gzip stream of `len` zero bytes, written in pieces so that the test never holds them.
    fn gzip_of_zeros(len: usize) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        let piece = vec![0u8; 1 << 20];
        let mut left = len;
        while left > 0 {
            let now = left.min(piece.len());
            encoder.write_all(&piece[..now]).unwrap();
            left -= now;
        }
        encoder.finish().unwrap()
    }

    /// An SVG document of exactly `len` bytes (blanks after the root element).
    fn svg_of_length(len: usize) -> Vec<u8> {
        let mut svg = format!("{HEAD}</svg>").into_bytes();
        svg.resize(len, b' ');
        svg
    }

    #[test]
    fn a_gzip_bomb_is_refused_before_it_is_unpacked() {
        // A gigabyte of zeros is a few megabytes of gzip. Unbounded, `unpack` returns the whole gigabyte.
        let bomb = gzip_of_zeros(1 << 30);
        assert!(bomb.len() < 8 << 20, "{} bytes", bomb.len());
        assert!(matches!(unpack(&bomb), Err(UnpackError::TooLarge)));
        assert_eq!(parse(&bomb).unwrap_err(), INVALID);
    }

    #[test]
    fn the_unpacked_size_limit_is_32_mib_exactly() {
        assert_eq!(MAX_SVG_BYTES, 32 << 20);
        let at_the_limit = unpack_len(&gzip(&svg_of_length(MAX_SVG_BYTES)));
        assert_eq!(at_the_limit.unwrap(), MAX_SVG_BYTES);
        let over = unpack_len(&gzip(&svg_of_length(MAX_SVG_BYTES + 1)));
        assert!(matches!(over, Err(UnpackError::TooLarge)));
        // Plain (not packed) bytes are not this function's business: the caller's own size limits apply.
        assert_eq!(unpack_len(b"<svg/>").unwrap(), 6);
    }

    fn unpack_len(bytes: &[u8]) -> Result<usize, UnpackError> {
        unpack(bytes).map(|unpacked| unpacked.len())
    }

    #[test]
    fn an_svgz_at_the_limit_still_loads() {
        assert!(parse(&gzip(&svg_of_length(MAX_SVG_BYTES))).is_ok());
    }

    #[test]
    fn a_gzip_bomb_in_a_nested_data_url_leaves_the_image_out() {
        // Over the limit, so refused here and never handed to usvg's own (unbounded) resolver: the outer
        // document loads and the `<image>` draws nothing.
        let bomb = gzip_of_zeros(MAX_SVG_BYTES + (16 << 20));
        for mime in ["image/svg+xml", "text/plain"] {
            let tree = parse(&with_image(&data_url(mime, &bomb))).expect("the outer one loads");
            assert_eq!(images(&tree), (0, 0), "{mime}");
        }
        // Just under the limit, a nested SVGZ is as before.
        let tree = parse(&with_image(&data_url("image/svg+xml", &gzip(&nested(20))))).unwrap();
        assert_eq!(images(&tree), (1, 0));
    }

    #[test]
    fn a_nested_svg_in_a_data_url_is_counted_too() {
        // The outer document is 2 levels deep and loads; what it embeds is 600 deep (usvg alone loads it).
        for mime in ["image/svg+xml", "text/plain"] {
            let tree =
                parse(&with_image(&data_url(mime, &nested(600)))).expect("the outer one loads");
            assert_eq!(
                images(&tree),
                (0, 0),
                "{mime}: the 600-level image is left out"
            );
        }
        // Packed, and 100,000 levels packed into a few hundred bytes.
        let tree = parse(&with_image(&data_url("image/svg+xml", &gzip(&nested(600))))).unwrap();
        assert_eq!(images(&tree), (0, 0), "an SVGZ data URL is unpacked first");
        let tree = parse(&with_image(&data_url(
            "image/svg+xml",
            &gzip(&nested(100_000)),
        )))
        .unwrap();
        assert_eq!(images(&tree), (0, 0));
        // A shallow image is kept, as an SVG image.
        for mime in ["image/svg+xml", "text/plain"] {
            let tree = parse(&with_image(&data_url(mime, &nested(20)))).unwrap();
            assert_eq!(images(&tree), (1, 0), "{mime}: a shallow image still loads");
        }
        // Raster data is not touched: a PNG under its own type and under text/plain.
        let png: &[u8] = &[
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R',
            0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
        ];
        for mime in ["image/png", "text/plain"] {
            let tree = parse(&with_image(&data_url(mime, png))).unwrap();
            assert_eq!(images(&tree), (0, 1), "{mime}");
        }
    }

    #[test]
    fn the_depths_of_nested_documents_add_up() {
        // 299 levels in the outer one and 300 in the inner one are over 500 in all: the inner one is left out.
        let outer = with_image_at_depth(299, &data_url("image/svg+xml", &nested(300)));
        let tree = parse(&outer).expect("299 levels is fine on its own");
        assert_eq!(images(&tree), (0, 0));
        // 100 and 100 are not.
        let outer = with_image_at_depth(100, &data_url("image/svg+xml", &nested(100)));
        let tree = parse(&outer).unwrap();
        assert_eq!(images(&tree), (1, 0));
        // 250 and 250 is exactly the budget: the outer one is 251 deep with its image, so 250 + 250 + 1 is over.
        let outer = with_image_at_depth(250, &data_url("image/svg+xml", &nested(250)));
        let tree = parse(&outer).unwrap();
        assert_eq!(images(&tree), (0, 0));
    }

    #[test]
    fn at_most_four_documents_nest_in_one_another() {
        // Five tiny documents, each holding the next as a data: image.
        let mut five = nested(2);
        for _ in 0..4 {
            five = with_image(&data_url("image/svg+xml", &five));
        }
        let tree = parse(&five).expect("the outer one loads");
        assert_eq!(
            chain_length(&tree),
            3,
            "documents 2, 3 and 4 load; the fifth is left out"
        );
        // Four are fine.
        let mut four = nested(2);
        for _ in 0..3 {
            four = with_image(&data_url("image/svg+xml", &four));
        }
        assert_eq!(chain_length(&parse(&four).unwrap()), 3);
        // Three too, of course.
        let mut three = nested(2);
        for _ in 0..2 {
            three = with_image(&data_url("image/svg+xml", &three));
        }
        assert_eq!(chain_length(&parse(&three).unwrap()), 2);
    }

    #[test]
    fn the_chain_is_clean_after_every_parse() {
        // `parse_tree` runs on a thread of its own, whose chain dies with it; `parse_tree_here` is the same
        // code on this thread (it is also what runs when the large thread cannot be started), so the chain
        // of this thread shows whether a parse leaves anything behind. Shallow documents only: this is a small stack.
        let options = options();
        assert!(parse_tree_here(&nested(10), &options).is_ok());
        assert_eq!(CHAIN.get(), Chain::default());
        assert_eq!(
            parse_tree_here(&nested(501), &options).unwrap_err(),
            TOO_DEEP
        );
        assert_eq!(CHAIN.get(), Chain::default());
        assert_eq!(parse_tree_here(b"<svg", &options).unwrap_err(), INVALID);
        assert_eq!(CHAIN.get(), Chain::default());
        let nested_image = with_image(&data_url("image/svg+xml", &nested(20)));
        assert!(parse_tree_here(&nested_image, &options).is_ok());
        assert_eq!(CHAIN.get(), Chain::default());
        let over_budget = with_image_at_depth(299, &data_url("image/svg+xml", &nested(300)));
        assert!(parse_tree_here(&over_budget, &options).is_ok());
        assert_eq!(CHAIN.get(), Chain::default());
        let entered = Entered::new(100).unwrap();
        assert_eq!(
            CHAIN.get(),
            Chain {
                documents: 1,
                depth: 100
            }
        );
        drop(entered);
        assert_eq!(CHAIN.get(), Chain::default());
        assert!(Entered::new(501).is_none());
        let outer = Entered::new(300).unwrap();
        assert!(Entered::new(201).is_none());
        let inner = Entered::new(200).unwrap();
        drop(inner);
        drop(outer);
        assert_eq!(CHAIN.get(), Chain::default());
    }

    /// `groups` groups of `levels` nested `<svg>` elements, each ending in a `<use>` of the one before.
    fn use_amplified_document(groups: usize, levels: usize) -> Vec<u8> {
        let mut svg =
            format!(r#"{HEAD}<defs><g id="g0"><rect width="10" height="10" fill="green"/></g>"#);
        for group in 1..=groups {
            svg.push_str(&format!(r#"<g id="g{group}">"#));
            svg.push_str(&r#"<svg width="10" height="10">"#.repeat(levels));
            svg.push_str(&format!(r##"<use xlink:href="#g{}"/>"##, group - 1));
            svg.push_str(&"</svg>".repeat(levels));
            svg.push_str("</g>");
        }
        svg.push_str(&format!(r##"</defs><use xlink:href="#g{groups}"/></svg>"##));
        svg.into_bytes()
    }

    #[test]
    fn a_use_chain_over_a_shallow_file_does_not_overflow_the_stack() {
        // Four groups of 200 nested <svg>: 202 levels in the file, about 800 for usvg. Measured: usvg alone on a
        // 2 MiB stack overflows on it (the process aborts with "stack overflow") and on a 3 MiB stack it parses.
        // `parse_tree` must survive it on its own large stack, so a failure here is the test process aborting.
        let doc = use_amplified_document(4, 200);
        assert!(
            depth_of(std::str::from_utf8(&doc).unwrap()) < 210,
            "the file is shallow"
        );
        assert!(parse(&doc).is_ok());
        // Called from a worker's own 2 MiB stack, which is where Servo calls it.
        assert!(on_a_worker_stack(
            || parse(&use_amplified_document(4, 200)).is_ok()
        ));
    }

    #[test]
    fn the_test_documents_are_real_svg_that_usvg_reads() {
        // The verdicts above are about documents usvg accepts. Parse on a thread with a large stack so this
        // test does not depend on how much stack usvg takes per level.
        let parsed = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let options = usvg::Options::default();
                usvg::Tree::from_data(&nested(MAX_SVG_DEPTH as usize), &options).is_ok() &&
                    usvg::Tree::from_data(&nested_svg(MAX_SVG_DEPTH as usize), &options).is_ok() &&
                    usvg::Tree::from_data(&gzip(&nested(20)), &options).is_ok() &&
                    usvg::Tree::from_data(&use_amplified_document(4, 200), &options).is_ok()
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(parsed);
    }
}
