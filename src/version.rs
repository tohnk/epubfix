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
use crate::util::re;

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

/// The subset of [`BLOCK_ONLY`] a `<div>` can actually rescue.
///
/// Measured, under EPUB 2. A `<blockquote>` holding bare verse is 4 errors and
/// `<blockquote><div>…</div></blockquote>` is 0. A `<form>` or `<fieldset>`
/// holding the same content reports the *container* as not allowed and wrapping
/// its children changes nothing — 3 errors either way — so those stay a report.
/// `<body>` is not here because a body with no block content at all is
/// [`crate::fixers::documents::FragmentDocuments`]' job and it gets there first.
pub(crate) const WRAPPABLE: &[&str] = &["blockquote", "noscript"];

/// Block-only containers where wrapping cannot help at all, so inline content
/// inside one cannot be repaired as EPUB 2 by anything this tool does.
///
/// This is [`BLOCK_ONLY`] minus [`WRAPPABLE`], minus the `<body>` cases that
/// get repaired in place: a body holding only inline content is
/// [`crate::fixers::documents::FragmentDocuments`]' job. A body that has block
/// content but loose inline runs between it — Calibre pagebreak spans,
/// typically — is repaired by no one, and counts as unwrappable through the
/// per-body test in [`scan_document`].
pub(crate) const UNWRAPPABLE: &[&str] = &["form", "fieldset"];

/// Elements that are inline in XHTML 1.1, so illegal as a direct child above.
pub(crate) const INLINE: &[&str] = &[
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
/// a book whose declaration is simply wrong. Below this line the wrappable runs
/// are repaired in place — and the unwrappable ones retag on their own, through
/// [`Assessment::unwrappable_inline`].
const INLINE_IN_BLOCK_THRESHOLD: u32 = 20;

/// Is this book's inline-in-block count small enough to repair in place?
///
/// Above the line the answer is to move the declaration, which the retagger has
/// already done by the time any fixer runs; below it, wrapping a handful of
/// runs in a `<div>` is the smaller change and the message used to ask the
/// reader to do it by hand.
pub fn few_enough_to_wrap(count: u32) -> bool {
    count > 0 && count < INLINE_IN_BLOCK_THRESHOLD
}

/// What the book's own contents say about which version it is.
#[derive(Debug, Default)]
pub struct Assessment {
    /// Constructs that only exist in EPUB 3. Decisive: they cannot be expressed
    /// as EPUB 2 at all.
    pub epub3_only: Vec<String>,
    /// Inline content inside block-only containers. Suggestive rather than
    /// decisive, so it only forces an upgrade in quantity.
    pub inline_in_block: u32,
    /// Inline content inside the block-only containers no fixer can rescue.
    ///
    /// [`crate::version::WRAPPABLE`] covers `<blockquote>` and `<noscript>`,
    /// where wrapping each run in a `<div>` repairs the document, and a body
    /// holding only inline content is repaired in place too. `<form>` and
    /// `<fieldset>` reject the wrapper (measured: 3 errors either way), and a
    /// body that already has block content but keeps loose inline runs between
    /// it is repaired by no one. These runs cannot be repaired as EPUB 2 at
    /// all, so even one of them is decisive: the declaration is what must
    /// move.
    pub unwrappable_inline: u32,
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

    /// Content that only HTML5 accepts, and not enough of it — or only in
    /// wrappable containers — to retag on.
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
    unwrappable_inline: u32,
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
            let mut count = u32::from(node.has_text);
            let mut has_block_child = false;
            for c in nodes.iter().filter(|c| {
                c.parent == Some(i) && c.kind != NodeKind::End
            }) {
                if INLINE.contains(&c.name.as_str()) {
                    count = count.saturating_add(1);
                } else {
                    has_block_child = true;
                }
            }
            tally.inline_in_block += count;
            // A body with block content already is not boxed by
            // FragmentDocuments, so its stray runs are unwrappable too.
            if UNWRAPPABLE.contains(&node.name.as_str())
                || (node.name == "body" && has_block_child)
            {
                tally.unwrappable_inline += count;
            }
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
        unwrappable_inline,
        mut html5_elements,
        epub_attrs,
        meta_charset,
        xhtml_doctypes,
        named_entities,
    } = tally;

    a.inline_in_block = inline_in_block;
    a.unwrappable_inline = unwrappable_inline;
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
    // Unwrappable runs are decisive on their own: a run inside <form>,
    // <fieldset>, or a body with block content cannot be repaired as EPUB 2,
    // so the declaration must move even for a single one, where a
    // <blockquote> handful would be wrapped instead. (Above the threshold
    // `epub3_only` already carries them.)
    if declared < 3 && (!a.epub3_only.is_empty() || a.unwrappable_inline > 0) {
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
        Retag::Upgrade => {
            let evidence = if a.epub3_only.is_empty() {
                format!(
                    "{} element(s) hold inline content that only EPUB 3 accepts and \
                     cannot be repaired as EPUB 2 (inside <body>, <form> or <fieldset>, \
                     which wrapping does not help)",
                    a.unwrappable_inline
                )
            } else {
                a.epub3_only.join("; ")
            };
            format!("the content requires EPUB 3 ({evidence}), so the package declaration was wrong")
        }
        Retag::Downgrade => format!(
            "nothing in the book requires EPUB 3 and it is written as EPUB 2 ({})",
            a.epub2_markers.join("; ")
        ),
        Retag::Keep => "the declared version matches the content".to_string(),
    }
}
