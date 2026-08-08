//! Fixes that apply to the package document (`.opf`).

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::util::re;

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
