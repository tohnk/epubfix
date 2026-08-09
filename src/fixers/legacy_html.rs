//! Fixes and diagnostics for markup that XHTML 1.1 rejects but HTML5 accepts.
//!
//! Both of these are EPUB 2 only, and both run the opposite way round from
//! `legacy-table-attrs`: there it is HTML5 that is stricter, here it is
//! XHTML 1.1.

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, scan};
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

/// Reports a book whose markup does not match its declared version, when there
/// is not enough evidence to retag it automatically.
///
/// The retagging in [`crate::version`] handles the clear-cut cases in both
/// directions. What is left is the middle: a book declaring EPUB 2 with a
/// handful of inline runs inside `<blockquote>`. Too few to justify changing the
/// book's format identity, too awkward to repair without wrapping each one in a
/// `<div>`. So it says so, and stops.
pub struct VersionMismatch;

impl Fixer for VersionMismatch {
    fn name(&self) -> &'static str {
        "version-mismatch"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "report markup that does not match the declared version (never rewrites)"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() >= 3 {
            return outcome;
        }
        let assessment = crate::version::assess(book);
        if assessment.suggestive_only() {
            outcome.push_finding(format!(
                "declares EPUB 2, but {} element(s) hold inline content that only validates \
                 under EPUB 3 rules (verse in <blockquote>, typically). That is too few to \
                 retag the whole book on, so nothing was changed; wrap them in a <div> by \
                 hand, or use --migrate-epub3 to change the declaration instead.",
                assessment.inline_in_block
            ));
        }
        outcome
    }
}
