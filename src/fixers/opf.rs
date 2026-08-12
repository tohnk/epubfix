//! Fixes that apply to the package document (`.opf`).

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::language::{self, Policy};
use crate::markup::{Edits, NodeKind, scan};
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

/// RSC-005: `element "metadata" incomplete; missing required element
/// "dc:language"`.
///
/// Required by both EPUB versions, and position inside `<metadata>` does not
/// matter — measured, inserted first and last, both clean. What epubcheck does
/// *not* do is check the value: `<dc:language>zz</dc:language>` validates
/// without a murmur. So this is a field where being wrong is silent, and it is
/// worth being careful about, since reading systems hyphenate and choose
/// speech voices from it.
///
/// See [`crate::language`] for how the value is arrived at. The short version:
/// what the documents already declare, else what the text says, else nothing.
/// Never the system locale, which is what Calibre does and is how an English
/// machine turns a Hungarian novel into `<dc:language>en</dc:language>`.
pub struct DcLanguage {
    pub policy: Policy,
}

impl Default for DcLanguage {
    fn default() -> Self {
        DcLanguage {
            policy: Policy::EnglishOnly,
        }
    }
}

impl Fixer for DcLanguage {
    fn name(&self) -> &'static str {
        "dc-language"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "add the required <dc:language>, taken from the documents or from the text"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(opf) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&opf) else {
            return Outcome::none();
        };

        // Only an absent element is an error. One that is present but odd is
        // the author's business, and epubcheck does not mind either way.
        if nodes
            .iter()
            .any(|n| n.name == "language" && n.kind != NodeKind::End)
        {
            return Outcome::none();
        }
        let Some(metadata) = nodes
            .iter()
            .position(|n| n.name == "metadata" && n.kind == NodeKind::Start)
        else {
            return Outcome::none();
        };
        let Some(close) = nodes[metadata].close else {
            return Outcome::none();
        };

        let Some(found) = language::detect(book, self.policy) else {
            let mut outcome = Outcome::finding(match language::observed(book) {
                Some(seen) => format!(
                    "no <dc:language>, and the text reads as \"{}\" rather than English, so \
                     nothing was written — set it by hand, or re-run with \
                     --language-detect=any",
                    seen.language
                ),
                None => "no <dc:language>, and neither the documents nor the text say what \
                         language the book is in; it needs setting by hand"
                    .to_string(),
            });
            outcome.push_finding(
                "a wrong <dc:language> is worse than a missing one: reading systems hyphenate \
                 and choose speech voices from it"
                    .to_string(),
            );
            return outcome;
        };

        // Match the prefix the file already uses for Dublin Core rather than
        // assuming `dc:` — it is a namespace binding, not a fixed spelling.
        let prefix = dc_prefix(&opf).unwrap_or_else(|| "dc".to_string());

        let indent = line_indent(&opf, nodes[close].span.start);
        let mut edits = Edits::new();
        edits.insert(
            nodes[close].span.start,
            format!(
                "{indent}  <{prefix}:language>{}</{prefix}:language>\n{indent}",
                found.language
            ),
        );
        book.set_text(&opf_name, edits.apply(&opf));

        let how = match found.source {
            language::Source::Declared => "which is what the content documents declare".to_string(),
            language::Source::Detected { samples, share } => {
                format!("detected from the text ({share} of {samples} samples)")
            }
        };
        Outcome::change(format!(
            "added <dc:language>{}</dc:language> — {how}",
            found.language
        ))
    }
}

/// The prefix this package document binds Dublin Core to.
fn dc_prefix(opf: &str) -> Option<String> {
    let at = opf.find("=\"http://purl.org/dc/elements/1.1/\"")?;
    let decl = &opf[..at];
    let name = decl.rsplit(|c: char| c.is_whitespace()).next()?;
    name.strip_prefix("xmlns:").map(str::to_string)
}

/// The whitespace opening the line that byte `at` sits on, so an inserted
/// element lines up with its neighbours.
fn line_indent(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[start..at]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect()
}
