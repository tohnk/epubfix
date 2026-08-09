//! Fixes and diagnostics for markup that XHTML 1.1 rejects but HTML5 accepts.
//!
//! Both of these are EPUB 2 only, and both run the opposite way round from
//! `legacy-table-attrs`: there it is HTML5 that is stricter, here it is
//! XHTML 1.1.

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, NodeKind, scan};
use crate::util::{basename, re};

// ---------------------------------------------------------------------------
// img-alt
// ---------------------------------------------------------------------------

/// Filenames that read as ornament rather than content.
static DECORATIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)(^|[^a-z])(orn|ornament|rule|deco|divider|sep|separator|dingbat|flourish|swash|border|spacer|blank|line|star|asterism|fleuron|glyph)([^a-z]|$)",
    )
});

/// RSC-005: `element "img" missing required attribute "alt"`.
///
/// XHTML 1.1 requires `alt`; HTML5 does not make its absence a validation
/// error. So this is EPUB 2 only.
///
/// The mechanical fix is `alt=""`, which is correct for an ornament and a lie
/// for a photograph — and writing it onto a content image actively harms a
/// screen-reader user while making the validator happy. That is the wrong trade
/// to make silently, so `alt=""` is written only where the filename reads as
/// decorative; everything else is reported for a human to caption.
pub struct ImgAlt;

impl Fixer for ImgAlt {
    fn name(&self) -> &'static str {
        "img-alt"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "add alt=\"\" to decorative images in EPUB 2, and report the rest"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() >= 3 {
            return outcome;
        }

        let mut added = 0u32;
        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();
            let mut needs_caption = 0u32;

            for node in nodes.iter().filter(|n| n.name == "img") {
                if node.attr("alt").is_some() {
                    continue;
                }
                let src = node.attr("src").map_or("", |a| a.value.as_str());
                // A caption or title means somebody already described it.
                let described = node.attr("title").is_some() || node.attr("aria-label").is_some();
                if !described && DECORATIVE_RE.is_match(basename(src)) {
                    edits.insert(node.name_end, " alt=\"\"");
                    added += 1;
                } else {
                    needs_caption += 1;
                }
            }

            if needs_caption > 0 {
                outcome.push_finding(format!(
                    "{}: {needs_caption} image(s) have no alt text and do not look decorative; \
                     they need a real caption, or migrate the book to EPUB 3 where alt is not \
                     a validation error",
                    basename(&doc)
                ));
            }
            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if added > 0 {
            outcome.push_change(format!("added alt=\"\" to {added} decorative image(s)"));
        }
        outcome
    }
}

// ---------------------------------------------------------------------------
// version-mismatch (diagnostic only)
// ---------------------------------------------------------------------------

/// Elements whose XHTML 1.1 content model is block-level only, but whose HTML5
/// content model is flow.
const BLOCK_ONLY: &[&str] = &["blockquote", "body", "form", "noscript", "fieldset"];

/// Elements that are inline in XHTML 1.1, so illegal as a direct child above.
const INLINE: &[&str] = &[
    "a", "abbr", "acronym", "b", "bdo", "big", "br", "button", "cite", "code", "dfn", "em", "font",
    "i", "img", "input", "kbd", "label", "map", "object", "q", "s", "samp", "select", "small",
    "span", "strike", "strong", "sub", "sup", "textarea", "tt", "u", "var",
];

/// Reports an EPUB 2 book whose content only validates under HTML5 rules.
///
/// The giveaway is verse: `<blockquote>` holding bare text or a bare `<span>`,
/// which XHTML 1.1 forbids and HTML5 allows. One real book produced 28,874
/// errors as EPUB 2 and 728 as EPUB 3 — byte-identical markup, one wrong
/// attribute in the package document.
///
/// Rewriting the markup to satisfy XHTML 1.1 would mean wrapping every inline
/// run in a `<div>`: thousands of edits across hundreds of files to work around
/// one attribute. So this only ever reports, and points at `--migrate-epub3`.
pub struct VersionMismatch;

impl Fixer for VersionMismatch {
    fn name(&self) -> &'static str {
        "version-mismatch"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "report EPUB 2 books whose markup only validates as EPUB 3 (never rewrites)"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() >= 3 {
            return outcome;
        }

        let mut violations = 0u32;
        let mut documents = 0u32;
        for doc in book.markup_names() {
            let Some(text) = book.text(&doc) else {
                continue;
            };
            let Ok(nodes) = scan(text) else { continue };

            let mut here = 0u32;
            for (i, node) in nodes.iter().enumerate() {
                if node.kind != NodeKind::Start || !BLOCK_ONLY.contains(&node.name.as_str()) {
                    continue;
                }
                if node.has_text {
                    here += 1;
                }
                here += u32::try_from(
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
            if here > 0 {
                documents += 1;
                violations += here;
            }
        }

        if violations > 0 {
            outcome.push_finding(format!(
                "declares EPUB 2, but {violations} element(s) across {documents} document(s) \
                 hold inline content that only validates under EPUB 3 rules (verse in \
                 <blockquote>, typically). Repairing the markup would mean wrapping every one \
                 of them in a <div>; --migrate-epub3 fixes it by changing the declaration \
                 instead, and is checked to preserve the whole book before it lands."
            ));
        }
        outcome
    }
}
