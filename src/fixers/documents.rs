//! RSC-005: content documents that are not documents.
//!
//! A Penguin build of *The Complete Poems of John Keats* ships a `cover.html`
//! of 45 bytes, in its entirety:
//!
//! ```text
//! <img alt="Image" src="../images/Cover.jpg" />
//! ```
//!
//! No XML declaration, no `<html>`, no namespace, no `<head>`, no `<body>` — a
//! markup fragment saved with an extension and listed in both the manifest and
//! the spine. epubcheck says `elements from namespace "" are not allowed`,
//! which is oblique but exact: with no `xmlns` in scope that `<img>` is in no
//! namespace at all, and a content document may only hold XHTML-namespace
//! elements.
//!
//! # Wrapping is not enough, and that is the whole point of this fixer
//!
//! Measured, on the real book. Wrapping the fragment in `html`/`head`/`body`
//! and stopping there:
//!
//! ```text
//! element "img" not allowed here; expected element "address", "blockquote", …
//! element "body" incomplete; expected element "address", "blockquote", …
//! ```
//!
//! Two errors where there was one. XHTML 1.1 wants block-level content directly
//! inside `<body>` and an `<img>` is inline, so the wrapper has to supply a
//! block container as well. With `<div><img/></div>` the book validates clean.
//! `<div>` rather than `<p>` because it carries no default margins, so nothing
//! moves on the page.
//!
//! HTML5 accepts flow content in `<body>`, so the container is only strictly
//! required under EPUB 2 — but it is valid in both and one code path is worth
//! more than the saved element.

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, Node, NodeKind, scan};
use crate::refs::resolve_href;
use crate::util::{basename, dirname};

/// Elements that may sit directly inside `<body>` under XHTML 1.1.
///
/// Only used to ask "does this body have *any* block content", so it needs to
/// be complete in the direction that matters: a name missing from here makes a
/// real document look like a fragment. Script and template are included for the
/// same reason, though they are not block content as such — an over-long list
/// only makes this fixer quieter, and a short one makes it wrong.
///
/// It was wrong once, and the omission is worth keeping visible. Both cover
/// documents of a Harper Collins *Hobbit* hold an `<svg>` directly in `<body>`,
/// which epubcheck accepts and this fixer wrapped in a pointless `<div>`. The
/// permitted set is quoted verbatim in epubcheck's own message, `svg` included:
///
/// ```text
/// expected element "address", "blockquote", "del", "div", "dl", "h1", "h2",
/// "h3", "h4", "h5", "h6", "hr", "ins", "noscript", "ns:svg", "ol", "p",
/// "pre", "script", "table" or "ul"
/// ```
const BLOCK: &[&str] = &[
    "address",
    "blockquote",
    "del",
    "div",
    "dl",
    "fieldset",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "ins",
    "noscript",
    "ol",
    "p",
    "pre",
    "script",
    "table",
    "template",
    "ul",
    // Foreign-namespace roots, which a body may hold directly in both
    // rulesets. The scanner matches on the local name, so `<svg:svg>` and a
    // default-namespaced `<svg>` both land here.
    "svg",
    "math",
    // HTML5 sectioning and grouping, so an EPUB 3 document is never mistaken
    // for a fragment.
    "article",
    "aside",
    "details",
    "figure",
    "footer",
    "header",
    "main",
    "nav",
    "section",
];

pub struct FragmentDocuments;

/// The `<head>` of another document in the same directory, to copy the house
/// style from — stylesheet links above all, so a wrapped cover is still styled.
fn sibling_head(book: &Book, doc: &str) -> Option<(String, String)> {
    let dir = dirname(doc);
    for other in book.markup_names() {
        if other == doc || dirname(&other) != dir {
            continue;
        }
        let Some(text) = book.text(&other) else {
            continue;
        };
        let Ok(nodes) = scan(text) else { continue };
        let html = nodes
            .iter()
            .find(|n| n.name == "html" && n.kind == NodeKind::Start)?;
        let head = nodes
            .iter()
            .position(|n| n.name == "head" && n.kind == NodeKind::Start)?;
        let close = nodes[head].close?;

        // Everything the <head> holds except its title, which belongs to this
        // document rather than to its neighbour.
        let inner = &text[nodes[head].span.end..nodes[close].span.start];
        let without_title = match (inner.find("<title"), inner.find("</title>")) {
            (Some(a), Some(b)) => format!("{}{}", &inner[..a], &inner[b + "</title>".len()..]),
            _ => inner.to_string(),
        };
        return Some((
            text[html.span.clone()].to_string(),
            without_title.trim().to_string(),
        ));
    }
    None
}

/// What the NCX calls this document, for the `<title>`.
fn ncx_label(book: &Book, doc: &str) -> Option<String> {
    let ncx_name = book.ncx_name()?;
    let src = book.text(ncx_name)?;
    let nodes = scan(src).ok()?;

    let point = nodes.iter().enumerate().find(|(_, n)| {
        n.name == "content"
            && n.attr("src")
                .and_then(|a| resolve_href(ncx_name, &a.value))
                .is_some_and(|(t, _)| t == doc)
    })?;
    let parent = nodes[point.0].parent?;
    let label = nodes
        .iter()
        .position(|n| n.parent == Some(parent) && n.name == "navlabel")?;
    let text = nodes
        .iter()
        .position(|n| n.parent == Some(label) && n.name == "text")?;
    let close = nodes[text].close?;
    let inner = src[nodes[text].span.end..nodes[close].span.start].trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

/// Is this element one a `<body>` may hold directly?
///
/// Shared with the missing-image fixer, which must not remove a wrapper when
/// doing so would leave the body with nothing in it — measured, that trades
/// RSC-007 for `element "body" incomplete`.
pub fn is_block(name: &str) -> bool {
    BLOCK.contains(&name)
}

/// Does this body hold any block-level element at all?
///
/// The question is deliberately "any", not "only". A chapter with five hundred
/// paragraphs and one stray `<span>` directly in the body is the *other*
/// problem — the version mismatch of section 5a — and rewriting a third of a
/// book's markup to paper over a wrong package attribute is exactly what this
/// tool must not do. A body with no block content anywhere is a different
/// animal: the whole document is one fragment, and one container fixes it.
fn has_block_content(nodes: &[Node], body: usize) -> bool {
    nodes
        .iter()
        .any(|n| n.parent == Some(body) && n.kind != NodeKind::End && is_block(&n.name))
}

impl Fixer for FragmentDocuments {
    fn name(&self) -> &'static str {
        "fragment-documents"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "give a bare markup fragment the document and block container it needs"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let (mut wrapped, mut boxed) = (0u32, 0u32);

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };

            if !nodes.iter().any(|n| n.name == "html" && n.kind != NodeKind::End) {
                // No root element: the file is a fragment, not a document.
                let (open, head) = sibling_head(book, &doc).unwrap_or_else(|| {
                    (
                        r#"<html xmlns="http://www.w3.org/1999/xhtml">"#.to_string(),
                        String::new(),
                    )
                });
                let title = ncx_label(book, &doc).unwrap_or_else(|| {
                    basename(&doc)
                        .rsplit_once('.')
                        .map_or_else(|| basename(&doc).to_string(), |(stem, _)| stem.to_string())
                });
                let head = if head.is_empty() {
                    String::new()
                } else {
                    format!("\n{head}")
                };
                book.set_text(
                    &doc,
                    format!(
                        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
                         {open}\n<head>\n<title>{title}</title>{head}\n</head>\n\
                         <body>\n<div>{}</div>\n</body>\n</html>\n",
                        text.trim()
                    ),
                );
                wrapped += 1;
                continue;
            }

            // A real document, but one whose body holds nothing a body may
            // directly hold. Wrapping the fragment above without this step
            // trades one error for two, measured.
            let Some(body) = nodes
                .iter()
                .position(|n| n.name == "body" && n.kind == NodeKind::Start)
            else {
                continue;
            };
            let Some(close) = nodes[body].close else {
                continue;
            };
            let inner = &text[nodes[body].span.end..nodes[close].span.start];
            if inner.trim().is_empty() || has_block_content(&nodes, body) {
                continue;
            }

            let mut edits = Edits::new();
            edits.insert(nodes[body].span.end, "\n<div>".to_string());
            edits.insert(nodes[close].span.start, "</div>\n".to_string());
            book.set_text(&doc, edits.apply(&text));
            boxed += 1;
        }

        if wrapped > 0 {
            outcome.push_change(format!(
                "wrapped {wrapped} bare markup fragment(s) in a document"
            ));
        }
        if boxed > 0 {
            outcome.push_change(format!(
                "gave {boxed} document(s) the block container XHTML 1.1 requires inside <body>"
            ));
        }
        outcome
    }
}
