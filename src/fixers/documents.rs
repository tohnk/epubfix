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

use std::fmt::Write as _;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, Node, NodeKind, line_span, scan, well_formed};
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
            //
            // EPUB 2 only, and that is not a simplification. Measured, on a
            // body that is empty and on a body holding only inline content:
            //
            //     body            EPUB 2                    EPUB 3
            //     empty           element "body" incomplete  clean
            //     inline only     element "p" not allowed …  clean
            //     <div></div>     clean                      clean
            //
            // HTML5 accepts both, so under EPUB 3 the container repairs
            // nothing and the edit would be pure noise on a valid book.
            if book.epub_version() >= 3 {
                continue;
            }
            let Some(body) = nodes
                .iter()
                .position(|n| n.name == "body" && n.kind == NodeKind::Start)
            else {
                continue;
            };
            let Some(close) = nodes[body].close else {
                continue;
            };
            // An empty body is not "nothing to do": it is `element "body"
            // incomplete`, and skipping it here is why a Kobo build of
            // *Essays and Aphorisms* kept one error through a whole run.
            if has_block_content(&nodes, body) {
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

/// Elements that never have an end tag, so one left open is not truncation.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// FATAL RSC-016: `XML document structures must start and end within the same
/// entity` — a file that simply stops.
///
/// An abbyy-to-epub build of *True Hallucinations* ends its only content
/// document mid-air: 78 lines, the last one a complete `</p>`, and then nothing.
/// No `</div>`, no `</body>`, no `</html>`. epubcheck reports the position as
/// one byte past the end of the file, which is exactly what it is.
///
/// This matters more than one error suggests. A FATAL stops epubcheck reading
/// the file at all, so every other defect in the book's only document is
/// invisible behind it — and the same is true of this tool's own rollback guard,
/// which protects a file that parsed *before* a pass ran. A document that
/// arrives malformed has no such protection, so every later fixer edits it
/// unguarded. That is why this runs first.
///
/// The repair adds no content: the missing end tags, innermost first, using each
/// element's name exactly as written since XML is case-sensitive. Every reading
/// system already renders the document this way — closing at EOF is what a
/// recovering parser does — so nothing moves on the page.
///
/// Two things keep it honest. A void element left open is not truncation but a
/// different defect (`<br>` where XHTML wants `<br/>`), and closing it would
/// swallow the rest of the document, so that is reported rather than repaired.
/// And the result is handed to [`well_formed`] before it is kept: if appending
/// the tags does not actually produce a parseable document, the guess was wrong
/// and nothing is written.
pub struct TruncatedDocuments;

impl Fixer for TruncatedDocuments {
    fn name(&self) -> &'static str {
        "truncated-documents"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-016"]
    }
    fn description(&self) -> &'static str {
        "close the elements a document that stops mid-air left open"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let mut closed = 0u32;

        for name in book.names().to_vec() {
            if !crate::util::ends_with_any(&name, crate::util::XML) {
                continue;
            }
            let Some(text) = book.text(&name).map(str::to_owned) else {
                continue;
            };
            // Fire on the observed error, not on a scan that happens to find an
            // open element — the lenient scanner finds those in valid files too.
            if well_formed(&text).is_ok() {
                continue;
            }
            let Ok(nodes) = scan(&text) else { continue };

            let unclosed: Vec<&Node> = nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Start && n.close.is_none())
                .collect();
            if unclosed.is_empty() {
                continue;
            }
            if let Some(void) = unclosed.iter().find(|n| VOID.contains(&n.name.as_str())) {
                outcome.push_finding(format!(
                    "{}: <{}> is left open, which is a void element written without its slash \
                     rather than a document that stops early; closing it would swallow \
                     everything after it",
                    basename(&name),
                    void.name
                ));
                continue;
            }

            let mut fixed = text.clone();
            if !fixed.ends_with('\n') {
                fixed.push('\n');
            }
            for node in unclosed.iter().rev() {
                let _ = writeln!(fixed, "</{}>", node.raw_name(&text));
            }
            // The guess has to parse, or it was the wrong guess.
            if well_formed(&fixed).is_err() {
                outcome.push_finding(format!(
                    "{}: the document does not parse and closing the {} element(s) left open \
                     does not fix it, so the damage is something else",
                    basename(&name),
                    unclosed.len()
                ));
                continue;
            }
            outcome.push_change(format!(
                "{}: closed {} element(s) the document left open when it stopped [{}]",
                basename(&name),
                unclosed.len(),
                unclosed
                    .iter()
                    .rev()
                    .map(|n| n.raw_name(&text))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            book.set_text(&name, fixed);
            closed += 1;
        }

        let _ = closed;
        outcome
    }
}

/// What HTML5 requires of a `<meta http-equiv="content-type">`.
const CONTENT_TYPE: &str = "text/html; charset=utf-8";

/// RSC-005: `The meta element in encoding declaration state (http-equiv=
/// 'content-type') must have the value "text/html; charset=utf-8"`.
///
/// HTML5 does not treat this `<meta>` as a general header declaration. It is the
/// *encoding declaration*, and the one string above is the only value it may
/// hold — the media type is fixed at `text/html` even in an XHTML document,
/// which reads as wrong and is what the spec says.
///
/// A Doubleday build of *Robot Dreams* writes, in all 24 of its documents:
///
/// ```html
/// <meta content="http://www.w3.org/1999/xhtml; charset=utf-8" http-equiv="Content-Type"/>
/// ```
///
/// A namespace URI where a media type belongs — some converter reached for the
/// wrong variable. The charset is right, which is the part that has ever
/// mattered to a reading system, so nothing about the book renders differently.
///
/// EPUB 3 only, and that is the whole reason this exists as a fixer: XHTML 1.1
/// does not check the value, so the book was quiet until this tool retagged it.
/// Measured, on the real markup: 0 errors as EPUB 2, `1 ERROR(RSC-005)` per
/// document as EPUB 3, and clean once the value is corrected.
///
/// The charset is the one part not rewritten blind. `charset=utf-8` is a claim
/// about the bytes of the file, and a document declaring something else may
/// genuinely be in that encoding — relabelling it would turn a wrong declaration
/// into a wrong document. Those are reported.
pub struct ContentTypeMeta;

impl Fixer for ContentTypeMeta {
    fn name(&self) -> &'static str {
        "content-type-meta"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "correct the <meta> encoding declaration HTML5 fixes the value of"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() < 3 {
            return outcome;
        }
        let mut fixed = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            // Start and Empty only: an end tag carries no attributes, and the
            // "absent content" branch below writes one.
            for node in nodes.iter().filter(|n| {
                n.name == "meta" && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
            }) {
                let Some(equiv) = node.attr("http-equiv") else {
                    continue;
                };
                if !equiv.value.trim().eq_ignore_ascii_case("content-type") {
                    continue;
                }
                let Some(content) = node.attr("content") else {
                    edits.insert(node.name_end, format!(" content=\"{CONTENT_TYPE}\""));
                    fixed += 1;
                    continue;
                };
                let value = content.value.to_ascii_lowercase();
                if value.split_whitespace().collect::<Vec<_>>().join(" ") == CONTENT_TYPE {
                    continue;
                }
                // The declared charset, if it declares one.
                let charset = value
                    .split(';')
                    .skip(1)
                    .filter_map(|p| p.trim().strip_prefix("charset="))
                    .map(|c| c.trim().trim_matches('"').to_string())
                    .next();
                match charset.as_deref() {
                    None | Some("utf-8" | "utf8") => {
                        edits.replace(content.span.clone(), format!("content=\"{CONTENT_TYPE}\""));
                        fixed += 1;
                    }
                    Some(other) => outcome.push_finding(format!(
                        "{}: the encoding declaration says charset={other}, and EPUB requires \
                         UTF-8; rewriting the label would leave a document whose declaration \
                         and bytes disagree, so it needs converting first",
                        basename(&doc)
                    )),
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if fixed > 0 {
            outcome.push_change(format!(
                "corrected {fixed} <meta> encoding declaration(s) to \"{CONTENT_TYPE}\""
            ));
        }
        outcome
    }
}

/// Elements a `<head>` may hold.
///
/// Deletion is driven by *absence* from this list, so the same rule as
/// [`BLOCK`] applies in the same direction: an over-long list only makes the
/// fixer quieter, a short one makes it destroy markup. `object` is here because
/// XHTML 1.1 permits it in a head even though epubcheck's message does not
/// bother to mention it.
const HEAD_CONTENT: &[&str] = &[
    "base", "link", "meta", "noscript", "object", "script", "style", "title",
];

/// RSC-005: `element "p" not allowed here; expected … "base", "link", "meta",
/// "script" or "style"`.
///
/// A Calibre conversion of *Girl With Curious Hair* leaves three empty
/// paragraphs in the `<head>` of every one of its twelve documents:
///
/// ```html
/// <head>
///   <meta name="author" content="me"/>
///   <p> </p>
///   <meta name="creation-time" content="2004-5-26"/>
/// ```
///
/// Thirty-six errors from one conversion bug. They hold a single space, they
/// are in a part of the document nothing renders, and a `<head>` is the one
/// place in an EPUB where an element's meaning is entirely structural — so an
/// empty one there is removable with nothing lost.
///
/// One that is *not* empty is reported instead. Text in a `<head>` is text
/// nobody can see, and where it should go — hoisted into the body, or deleted
/// as the artefact it probably is — is a judgement about the book, not about
/// the markup.
pub struct HeadContent;

impl Fixer for HeadContent {
    fn name(&self) -> &'static str {
        "head-content"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "remove empty elements a <head> may not hold, and report any carrying text"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let mut removed = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let Some(head) = nodes
                .iter()
                .position(|n| n.name == "head" && n.kind == NodeKind::Start)
            else {
                continue;
            };
            let mut edits = Edits::new();

            // Start and Empty only. A comment is not an element, and treating
            // one as "not permitted here" would delete it.
            for node in nodes.iter().filter(|n| {
                n.parent == Some(head) && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
            }) {
                if HEAD_CONTENT.contains(&node.name.as_str()) {
                    continue;
                }
                let inner = match (node.kind, node.close) {
                    (NodeKind::Empty, _) => "",
                    (_, Some(close)) => &text[node.span.end..nodes[close].span.start],
                    _ => continue,
                };
                if inner.trim().is_empty() {
                    edits.delete(line_span(&text, node.element_span(&nodes)));
                    removed += 1;
                } else {
                    outcome.push_finding(format!(
                        "{}: <{}> in the <head> carries text, which nothing renders there; \
                         it needs a person to say whether it belongs in the body or nowhere",
                        basename(&doc),
                        node.name
                    ));
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if removed > 0 {
            outcome.push_change(format!(
                "removed {removed} empty element(s) a <head> may not hold"
            ));
        }
        outcome
    }
}
