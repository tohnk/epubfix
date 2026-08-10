//! Fixes that apply to the package document (`.opf`).

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, scan};
use crate::refs::resolve_href;
use crate::util::{ends_with_any, re};

/// Read the OPF, hand it to `f`, and store the result if it changed.
fn edit_opf(book: &mut Book, f: impl FnOnce(&str) -> String) -> bool {
    let Some(name) = book.opf_name().map(str::to_owned) else {
        return false;
    };
    let Some(text) = book.text(&name).map(str::to_owned) else {
        return false;
    };
    let new = f(&text);
    if new == text {
        return false;
    }
    book.set_text(&name, new);
    true
}

/// OPF-001: an OEBPS 1.0 package declaration in a file that is otherwise EPUB 2.
pub struct PackageVersion;

static VERSION_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"(<package\b[^>]*?)version="1\.0""#));

impl Fixer for PackageVersion {
    fn name(&self) -> &'static str {
        "opf-version"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-001"]
    }
    fn description(&self) -> &'static str {
        r#"package version="1.0" -> "2.0""#
    }
    fn apply(&self, book: &mut Book) -> Outcome {
        if edit_opf(book, |t| {
            VERSION_RE
                .replacen(t, 1, r#"${1}version="2.0""#)
                .into_owned()
        }) {
            Outcome::change("package version 1.0 -> 2.0")
        } else {
            Outcome::none()
        }
    }
}

/// RSC-005: `<spine page-map="...">`, an Adobe extension with no place in EPUB 2.
pub struct SpinePageMap;

static PAGE_MAP_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(<spine\b[^>]*?)\s*page-map="[^"]*""#));

impl Fixer for SpinePageMap {
    fn name(&self) -> &'static str {
        "spine-page-map"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "remove the Adobe page-map attribute from <spine>"
    }
    fn apply(&self, book: &mut Book) -> Outcome {
        if edit_opf(book, |t| PAGE_MAP_RE.replacen(t, 1, "${1}").into_owned()) {
            Outcome::change("removed spine/@page-map")
        } else {
            Outcome::none()
        }
    }
}

/// CSS-007: a doubled `application/` prefix on the TrueType font media type.
pub struct FontMediaType;

const BAD_FONT_TYPE: &str = "application/application/x-font-ttf";
const GOOD_FONT_TYPE: &str = "application/vnd.ms-opentype";

impl Fixer for FontMediaType {
    fn name(&self) -> &'static str {
        "font-media-type"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["CSS-007"]
    }
    fn description(&self) -> &'static str {
        r#"fix the "application/application/x-font-ttf" typo"#
    }
    fn apply(&self, book: &mut Book) -> Outcome {
        if edit_opf(book, |t| t.replace(BAD_FONT_TYPE, GOOD_FONT_TYPE)) {
            Outcome::change("corrected font media-type")
        } else {
            Outcome::none()
        }
    }
}

/// OPF-032: `Guide references "…" which is not a valid "OPS Content Document"`.
///
/// The `guide` is the EPUB 2 predecessor of the landmarks nav, and every
/// `<reference>` in it is required to point at a content document. Conversion
/// tools sometimes point one at the cover *image* instead of the cover page —
/// one real book has three `.jpg` references. A reading system cannot navigate
/// to a JPEG, so the entry does nothing but fail validation.
///
/// Only references that resolve to something which is not markup are removed.
/// A reference to a missing file is a different defect with a different repair
/// and is left to [`crate::fixers::resources::DanglingResources`]; a reference
/// whose fragment does not exist is left to `BrokenFragments`. Both of those
/// can recover the link, and deleting it first would take the chance away.
pub struct GuideReferences;

impl Fixer for GuideReferences {
    fn name(&self) -> &'static str {
        "guide-references"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-032"]
    }
    fn description(&self) -> &'static str {
        "drop guide entries pointing at something that is not a content document"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let present: Vec<String> = book.names().to_vec();
        let mut edits = Edits::new();
        let mut dropped = 0u32;

        for node in nodes.iter().filter(|n| n.name == "reference") {
            let Some(href) = node.attr("href") else {
                continue;
            };
            let Some((target, _)) = resolve_href(&opf_name, &href.value) else {
                continue;
            };
            // A target that is not in the book at all is somebody else's
            // problem, and one that is markup is doing its job.
            if !present.contains(&target) || ends_with_any(&target, crate::util::MARKUP) {
                continue;
            }
            edits.delete(node.element_span(&nodes));
            dropped += 1;
        }

        if dropped == 0 {
            return Outcome::none();
        }
        book.set_text(&opf_name, edits.apply(&text));
        Outcome::change(format!(
            "removed {dropped} guide reference(s) pointing at a non-content file"
        ))
    }
}
