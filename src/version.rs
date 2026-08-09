//! Deciding which EPUB version a book actually *is*.
//!
//! A mis-declared book is common, and it happens in both directions. The
//! declaration is one attribute; the content is thousands of elements. So when
//! the two disagree, the declaration is the thing to change — in either
//! direction. Rewriting a book's markup to satisfy a wrong attribute is the
//! tail wagging the dog, and for the EPUB 2 direction it is not even possible:
//! there is no XHTML 1.1 spelling of a nav document or of a `<section>`.
//!
//! That needs a two-sided test, because a book can carry evidence of both. The
//! Coleridge book does: verse in `<blockquote>` that only HTML5 accepts, *and*
//! Kindle-era table markup. Only the first kind is decisive, because only the
//! first kind cannot be repaired.
//!
//! * [`Assessment::epub3_only`] — constructs that are *errors* under EPUB 2. If
//!   any exist the book is EPUB 3, whatever it says, and downgrading is off.
//!
//! That distinction — errors, not features — is the whole game, and getting it
//! wrong is how this went wrong once already. An earlier version listed `svg`
//! among the decisive constructs on the reasoning that SVG is an HTML5 thing. It
//! is not: OPS 2.0.1 lists SVG among its core media types, and epubcheck accepts
//! an inline `<svg>` in an EPUB 2 document without a murmur. The trigger fired on
//! five books in a row, three of which already validated with **zero errors**,
//! and proposed dragging each through thousands of collateral entity and DOCTYPE
//! rewrites for no benefit at all.
//!
//! The lesson generalises past the one bad entry. Asking "does this book contain
//! something HTML5-ish" requires a complete and correct model of both content
//! models, which is the hard part. Asking "does this book contain something that
//! is an error where it stands" does not. Every element in [`EPUB2_ILLEGAL`] was
//! checked against EPUB Check 5.2.1 in an EPUB 2 book and observed to produce an
//! error; `svg` was checked the same way and produced none, which is why it is
//! absent.
//! * [`Assessment::epub2_markers`] — signs it was authored as EPUB 2. These are
//!   all repairable either way, so on their own they only tip the balance when
//!   nothing needs EPUB 3.

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::markup::{NodeKind, scan};
use crate::util::{basename, re};

/// Elements whose mere presence is an error in an EPUB 2 content document.
///
/// Measured, not assumed: each was placed in an EPUB 2 book on its own and run
/// through EPUB Check 5.2.1. Note what is *not* here — `svg` is legal in EPUB 2
/// and must never appear in this list. Anything added later gets the same
/// treatment before it goes in.
const EPUB2_ILLEGAL: &[&str] = &[
    "article",
    "aside",
    "audio",
    "bdi",
    "canvas",
    "datalist",
    "details",
    "dialog",
    "embed",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "main",
    "mark",
    "math",
    "meter",
    "nav",
    "output",
    "picture",
    "progress",
    "rp",
    "rt",
    "ruby",
    "section",
    "source",
    "summary",
    "template",
    "time",
    "track",
    "video",
    "wbr",
];

/// Elements whose XHTML 1.1 content model is block-level only, but whose HTML5
/// content model is flow.
const BLOCK_ONLY: &[&str] = &["blockquote", "body", "form", "noscript", "fieldset"];

/// Elements that are inline in XHTML 1.1, so illegal as a direct child above.
const INLINE: &[&str] = &[
    "a", "abbr", "acronym", "b", "bdo", "big", "br", "button", "cite", "code", "dfn", "em", "font",
    "i", "img", "input", "kbd", "label", "map", "object", "q", "s", "samp", "select", "small",
    "span", "strike", "strong", "sub", "sup", "textarea", "tt", "u", "var",
];

static NAMED_ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| re(r"&([A-Za-z][A-Za-z0-9]*);"));

/// How many inline-in-block violations it takes before retagging beats
/// repairing.
///
/// This signal is weaker than the others: inline content in a `<blockquote>`
/// means the markup is wrong under *one* of the two rulesets, not that the book
/// is definitely EPUB 3. A handful of them is a few paragraphs somebody could
/// wrap in a `<div>` and review; thousands of them, across hundreds of files, is
/// a book whose declaration is simply wrong. Below this line the book is
/// reported rather than retagged, and `--migrate-epub3` is there for anyone who
/// disagrees.
const INLINE_IN_BLOCK_THRESHOLD: u32 = 20;

/// What the book's own contents say about which version it is.
#[derive(Debug, Default)]
pub struct Assessment {
    /// Constructs that only exist in EPUB 3. Decisive: they cannot be expressed
    /// as EPUB 2 at all.
    pub epub3_only: Vec<String>,
    /// Inline content inside block-only containers. Suggestive rather than
    /// decisive, so it only forces an upgrade in quantity.
    pub inline_in_block: u32,
    /// Signs the book was authored as EPUB 2. Repairable in either direction,
    /// so only suggestive.
    pub epub2_markers: Vec<String>,
    /// EPUB 2 needs an NCX for navigation; without one it cannot be EPUB 2.
    pub has_ncx: bool,
}

impl Assessment {
    /// True if nothing in the book requires EPUB 3.
    ///
    /// Note this consults `inline_in_block` directly rather than the threshold:
    /// even one such element is a reason not to *downgrade* into a ruleset that
    /// rejects it, though it takes many to justify upgrading out of one.
    pub fn could_be_epub2(&self) -> bool {
        self.epub3_only.is_empty() && self.inline_in_block == 0 && self.has_ncx
    }

    /// Content that only HTML5 accepts, but not enough of it to retag on.
    pub fn suggestive_only(&self) -> bool {
        self.epub3_only.is_empty() && self.inline_in_block > 0
    }
}

/// What to do about the declared version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retag {
    /// Content needs EPUB 3 but the package says 2.
    Upgrade,
    /// Package says 3 but nothing in the book needs it.
    Downgrade,
    /// The declaration already matches, or the evidence is mixed.
    Keep,
}

/// Running totals while walking the content documents.
#[derive(Default)]
struct Tally {
    inline_in_block: u32,
    html5_elements: Vec<String>,
    epub_attrs: u32,
    meta_charset: u32,
    xhtml_doctypes: u32,
    named_entities: u32,
}

/// Walk one content document, adding what it shows to `tally`.
fn scan_document(text: &str, tally: &mut Tally) {
    let Ok(nodes) = scan(text) else { return };

    for (i, node) in nodes.iter().enumerate() {
        match node.kind {
            NodeKind::Other if node.name == "#doctype" => {
                if !text[node.span.clone()].eq_ignore_ascii_case("<!DOCTYPE html>") {
                    tally.xhtml_doctypes += 1;
                }
                continue;
            }
            NodeKind::End | NodeKind::Other => continue,
            _ => {}
        }

        if EPUB2_ILLEGAL.contains(&node.name.as_str()) && !tally.html5_elements.contains(&node.name)
        {
            tally.html5_elements.push(node.name.clone());
        }
        if node.attrs.iter().any(|x| x.name.starts_with("epub:")) {
            tally.epub_attrs += 1;
        }
        if node.name == "meta" && node.attr("charset").is_some() {
            tally.meta_charset += 1;
        }

        // Inline content directly inside a block-only container.
        if BLOCK_ONLY.contains(&node.name.as_str()) {
            if node.has_text {
                tally.inline_in_block += 1;
            }
            tally.inline_in_block += u32::try_from(
                nodes
                    .iter()
                    .filter(|c| {
                        c.parent == Some(i)
                            && c.kind != NodeKind::End
                            && INLINE.contains(&c.name.as_str())
                    })
                    .count(),
            )
            .unwrap_or(u32::MAX);
        }
    }

    tally.named_entities += u32::try_from(
        NAMED_ENTITY_RE
            .captures_iter(text)
            .filter(|c| !matches!(&c[1], "amp" | "lt" | "gt" | "quot" | "apos"))
            .count(),
    )
    .unwrap_or(u32::MAX);
}

/// Read the book and tally the evidence.
pub fn assess(book: &Book) -> Assessment {
    let mut a = Assessment {
        has_ncx: book.ncx_name().is_some(),
        ..Assessment::default()
    };

    let mut tally = Tally::default();
    for doc in book.markup_names() {
        if let Some(text) = book.text(&doc) {
            scan_document(text, &mut tally);
        }
    }
    let Tally {
        inline_in_block,
        mut html5_elements,
        epub_attrs,
        meta_charset,
        xhtml_doctypes,
        named_entities,
    } = tally;

    a.inline_in_block = inline_in_block;
    if inline_in_block >= INLINE_IN_BLOCK_THRESHOLD {
        a.epub3_only.push(format!(
            "{inline_in_block} element(s) hold inline content that only HTML5 permits \
             (verse in <blockquote>, typically)"
        ));
    }
    if !html5_elements.is_empty() {
        html5_elements.sort();
        a.epub3_only.push(format!(
            "element(s) that are errors under EPUB 2: {}",
            html5_elements.join(", ")
        ));
    }
    if epub_attrs > 0 {
        a.epub3_only
            .push(format!("{epub_attrs} epub: attribute(s)"));
    }
    if meta_charset > 0 {
        a.epub3_only.push(format!(
            "{meta_charset} HTML5 <meta charset> declaration(s)"
        ));
    }

    // Package-level evidence.
    if let Some(opf) = book.opf_text() {
        if opf.contains("properties=\"nav\"") || opf.contains("properties='nav'") {
            a.epub3_only.push("an EPUB 3 nav document".to_string());
        }
        if opf.contains("refines=") {
            a.epub3_only
                .push("EPUB 3 <meta refines> metadata".to_string());
        }
        if opf.contains("opf:role") || opf.contains("opf:file-as") || opf.contains("opf:scheme") {
            a.epub2_markers
                .push("legacy opf: metadata attributes".to_string());
        }
        if a.has_ncx && !opf.contains("properties=\"nav\"") && !opf.contains("properties='nav'") {
            a.epub2_markers
                .push("an NCX and no nav document".to_string());
        }
    }
    if xhtml_doctypes > 0 {
        a.epub2_markers
            .push(format!("{xhtml_doctypes} XHTML 1.1 DOCTYPE(s)"));
    }
    if named_entities > 0 {
        a.epub2_markers.push(format!(
            "{named_entities} named character entit(ies), which only the XHTML 1.1 DTD declares"
        ));
    }

    a
}

/// Decide whether the declared version should move to match the content.
///
/// Both directions require *positive evidence of a problem*. A book that shows
/// none is left exactly as it is, whichever version it declares — there is
/// nothing to fix, so every change is downside. That is the hard precondition
/// the SVG mistake violated: it retagged books that already validated clean.
pub fn decide(book: &Book, a: &Assessment) -> Retag {
    let declared = book.epub_version();
    if declared < 3 && !a.epub3_only.is_empty() {
        return Retag::Upgrade;
    }
    if declared >= 3 && a.could_be_epub2() && !a.epub2_markers.is_empty() {
        return Retag::Downgrade;
    }
    Retag::Keep
}

/// A one-line summary of why, for the change log.
pub fn reason(a: &Assessment, retag: Retag) -> String {
    match retag {
        Retag::Upgrade => format!(
            "the content requires EPUB 3 ({}), so the package declaration was wrong",
            a.epub3_only.join("; ")
        ),
        Retag::Downgrade => format!(
            "nothing in the book requires EPUB 3 and it is written as EPUB 2 ({})",
            a.epub2_markers.join("; ")
        ),
        Retag::Keep => "the declared version matches the content".to_string(),
    }
}

/// Documents carrying EPUB 2 markers, for reporting.
pub fn epub2_marked_documents(book: &Book) -> Vec<String> {
    book.markup_names()
        .into_iter()
        .filter(|d| {
            book.text(d)
                .is_some_and(|t| t.contains("XHTML 1.1") || NAMED_ENTITY_RE.is_match(t))
        })
        .map(|d| basename(&d).to_string())
        .collect()
}
