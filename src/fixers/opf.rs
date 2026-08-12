//! Fixes that apply to the package document (`.opf`).

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::language::{self, Policy};
use crate::markup::{Edits, NodeKind, line_span, scan};
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

/// CSS-007, OPF-035, OPF-037: a manifest `media-type` that is wrong or
/// superseded.
///
/// Three epubcheck codes, one repair — replace the declared type with the one
/// that means the same thing today — so they are one fixer rather than three.
/// The mapping is a fixed table, and every entry is a rename with no judgement
/// in it: the resource is unchanged and only the label the package puts on it
/// moves to the current spelling.
///
/// * **CSS-007** is a doubled prefix, `application/application/x-font-ttf`,
///   which is simply a typo some tool wrote.
/// * **OPF-035** is `text/html` in an EPUB 2 package, where content documents
///   are XHTML and must say so.
/// * **OPF-037** is the OEBPS 1.2 vocabulary — `text/x-oeb1-document` and
///   `text/x-oeb1-css` — left behind by a converter.
///
/// Measured: each of the three validates clean once the type is corrected.
pub struct MediaTypes;

/// `wrong media type -> what it means now`.
const MEDIA_TYPES: &[(&str, &str)] = &[
    // CSS-007: a doubled prefix, and the modern spelling of the font type.
    ("application/application/x-font-ttf", "application/vnd.ms-opentype"),
    // OPF-037: the OEBPS 1.2 vocabulary.
    ("text/x-oeb1-document", "application/xhtml+xml"),
    ("text/x-oeb1-css", "text/css"),
    // OPF-035: HTML where an EPUB wants XHTML.
    ("text/html", "application/xhtml+xml"),
];

impl Fixer for MediaTypes {
    fn name(&self) -> &'static str {
        "media-types"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["CSS-007", "OPF-035", "OPF-037"]
    }
    fn description(&self) -> &'static str {
        "replace a manifest media-type that is mistyped or superseded"
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

        let mut edits = Edits::new();
        let mut fixed: Vec<String> = Vec::new();

        for node in nodes.iter().filter(|n| n.name == "item") {
            let Some(attr) = node.attr("media-type") else {
                continue;
            };
            // Exact match on the whole attribute value. A substring rewrite
            // would turn "text/html-something" into nonsense.
            let Some((_, right)) = MEDIA_TYPES
                .iter()
                .find(|(wrong, _)| attr.value.trim() == *wrong)
            else {
                continue;
            };
            edits.replace(attr.span.clone(), format!("media-type=\"{right}\""));
            fixed.push(attr.value.trim().to_string());
        }

        if fixed.is_empty() {
            return Outcome::none();
        }
        book.set_text(&opf_name, edits.apply(&text));
        let count = fixed.len();
        fixed.sort();
        fixed.dedup();
        Outcome::change(format!(
            "corrected {count} manifest media-type(s) [{}]",
            fixed.join(", ")
        ))
    }
}

/// OPF-091 and OPF-099: manifest items that cannot mean what they say.
///
/// Both are the package document describing itself wrongly, and both have one
/// right answer, so they share a fixer.
///
/// **OPF-091** — `<item href="chapter.xhtml#part2">`. A manifest entry names a
/// *resource*, and a fragment names something inside one, so the two cannot be
/// combined: there is no file called `chapter.xhtml#part2`. Dropping the
/// fragment leaves the item pointing at the file it was always about, and it
/// takes an RSC-001 and an RSC-008 with it, since the phantom filename was also
/// being reported as missing and undeclared.
///
/// **OPF-099** — the manifest listing the package document itself. A manifest
/// is the list of everything *else*; the OPF is what does the listing, and an
/// entry for it describes nothing. Removing that one entry is the whole repair.
pub struct ManifestItems;

impl Fixer for ManifestItems {
    fn name(&self) -> &'static str {
        "manifest-items"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-091", "OPF-099"]
    }
    fn description(&self) -> &'static str {
        "drop a fragment from a manifest href, and the entry a manifest makes for itself"
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

        let mut edits = Edits::new();
        let (mut trimmed, mut dropped) = (0u32, 0u32);

        for node in nodes.iter().filter(|n| n.name == "item") {
            let Some(href) = node.attr("href") else {
                continue;
            };
            let Some((target, fragment)) = resolve_href(&opf_name, &href.value) else {
                continue;
            };
            if target == opf_name {
                edits.delete(line_span(&text, node.element_span(&nodes)));
                dropped += 1;
            } else if fragment.is_some() {
                let path = href.value.split_once('#').map_or("", |(p, _)| p);
                edits.replace(href.span.clone(), format!("href=\"{path}\""));
                trimmed += 1;
            }
        }

        if trimmed == 0 && dropped == 0 {
            return Outcome::none();
        }
        book.set_text(&opf_name, edits.apply(&text));

        let mut outcome = Outcome::none();
        if trimmed > 0 {
            outcome.push_change(format!(
                "dropped the fragment from {trimmed} manifest href(s), which name files"
            ));
        }
        if dropped > 0 {
            outcome.push_change(format!(
                "removed {dropped} manifest entr(ies) for the package document itself"
            ));
        }
        outcome
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

        // Line up with the other children of <metadata> rather than with the
        // closing tag, and insert at the start of its line so the indentation
        // already sitting there is not counted twice.
        let close_at = nodes[close].span.start;
        let line_start = opf[..close_at].rfind('\n').map_or(0, |i| i + 1);
        let indent = nodes
            .iter()
            .rfind(|n| n.parent == Some(metadata) && n.kind != NodeKind::End)
            .map_or_else(
                || format!("{}  ", line_indent(&opf, close_at)),
                |last| line_indent(&opf, last.span.start),
            );

        let mut edits = Edits::new();
        edits.insert(
            line_start,
            format!(
                "{indent}<{prefix}:language>{}</{prefix}:language>\n",
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

/// The Dublin Core elements a package document must have, in both versions.
///
/// The spec that asked for this fixer named only `identifier` and `language`.
/// Measured, `title` belongs here too:
///
/// ```text
/// <dc:title/>  absent  EPUB 2  ERROR   element "metadata" incomplete
/// <dc:title/>  empty   EPUB 2  WARNING title tag is empty
/// <dc:title/>  empty   EPUB 3  ERROR   character content of element invalid
/// ```
///
/// So deleting an empty one trades a warning for a hard error under EPUB 2 and
/// one error for another under EPUB 3 — the section 5e mistake exactly.
const REQUIRED_DC: &[&str] = &["identifier", "title", "language"];

/// OPF-054: `Date value "" is not valid … zero-length string`.
///
/// A Harper Collins build of *The Hobbit* pads its metadata with empty elements:
///
/// ```xml
/// <dc:date/>
/// <dc:subject/>
/// <dc:description/>
/// <dc:rights/>
/// ```
///
/// Only `dc:date` is reported, because it is the only one of the four with a
/// value grammar to violate — the empty string is not a W3C date. The other
/// three are legal and equally meaningless. An element asserting nothing is not
/// information, so removing it cannot lose any.
///
/// The exception is [`REQUIRED_DC`], where the element must exist even when its
/// content is useless. Those are reported instead: an empty `<dc:title/>` is a
/// real defect, but it needs a title, not a deletion, and only a person knows
/// what the title is.
pub struct EmptyMetadata;

impl Fixer for EmptyMetadata {
    fn name(&self) -> &'static str {
        "empty-metadata"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-054"]
    }
    fn description(&self) -> &'static str {
        "remove <dc:*> metadata elements with no content, keeping the required ones"
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
        // No prefix bound to Dublin Core means no `dc:*` elements to weigh up.
        let Some(prefix) = dc_prefix(&text) else {
            return Outcome::none();
        };
        let Some(metadata) = nodes
            .iter()
            .position(|n| n.name == "metadata" && n.kind == NodeKind::Start)
        else {
            return Outcome::none();
        };

        let mut outcome = Outcome::none();
        let mut edits = Edits::new();
        let mut removed: Vec<String> = Vec::new();

        // Start and Empty only: an end tag is the other half of an element
        // already considered, and a comment or processing instruction is not an
        // element at all.
        for node in nodes
            .iter()
            .filter(|n| n.parent == Some(metadata) && matches!(n.kind, NodeKind::Start | NodeKind::Empty))
        {
            let raw = node.raw_name(&text);
            let Some(local) = raw.strip_prefix(&format!("{prefix}:")) else {
                continue;
            };
            // A start tag with no matching end is malformed, not empty; saying
            // otherwise would delete the element and everything after it.
            let empty = match (node.kind, node.close) {
                (NodeKind::Empty, _) => true,
                (NodeKind::Start, Some(close)) => text[node.span.end..nodes[close].span.start]
                    .trim()
                    .is_empty(),
                _ => false,
            };
            if !empty {
                continue;
            }
            if REQUIRED_DC.contains(&local.to_ascii_lowercase().as_str()) {
                outcome.push_finding(format!(
                    "<{raw}> is empty, and both EPUB versions require it — deleting it would \
                     trade one error for another, so it needs a real value"
                ));
                continue;
            }
            edits.delete(line_span(&text, node.element_span(&nodes)));
            removed.push(raw.to_string());
        }

        if !removed.is_empty() {
            book.set_text(&opf_name, edits.apply(&text));
            outcome.push_change(format!(
                "removed {} empty metadata element(s) [{}]",
                removed.len(),
                removed.join(", ")
            ));
        }
        outcome
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

/// PKG-005, PKG-006, PKG-007: the OCF `mimetype` entry.
///
/// OCF requires the archive to open with an entry named `mimetype`, stored
/// uncompressed, holding exactly `application/epub+zip` and nothing else — no
/// trailing newline, no BOM, no ZIP extra field.
///
/// [`Book::save`] has always written exactly that, so any book this tool
/// rewrites comes out correct. The gap was that nothing *noticed*: a book whose
/// only defect was its mimetype entry drew "nothing to do", was never
/// rewritten, and kept both errors. The repair existed and could not reach
/// disk. So this fixer changes no bytes itself; it exists to report the
/// condition, which is what causes the archive to be repacked.
///
/// Found by working down epubcheck's message catalogue rather than from a book,
/// which is the argument for doing that: no book in the library has this defect,
/// and one with it would have been quietly returned unrepaired.
pub struct MimetypeEntry;

impl Fixer for MimetypeEntry {
    fn name(&self) -> &'static str {
        "mimetype"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["PKG-005", "PKG-006", "PKG-007"]
    }
    fn description(&self) -> &'static str {
        "put the OCF mimetype entry first, uncompressed, with exactly the required bytes"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let (first, stored, exact) = book.mimetype_state();
        let mut wrong: Vec<&str> = Vec::new();
        if !first {
            wrong.push("was not the first entry");
        }
        if !stored {
            wrong.push("was compressed");
        }
        if !exact {
            wrong.push("did not hold exactly \"application/epub+zip\"");
        }
        if wrong.is_empty() {
            return Outcome::none();
        }
        Outcome::change(format!("rewrote the mimetype entry, which {}", wrong.join(" and ")))
    }
}

/// OPF-016 and OPF-017: `<rootfile>` with no usable `full-path`.
///
/// `META-INF/container.xml` is how a reading system finds the package document,
/// and its `full-path` is the only thing in the archive that says where that
/// is. Missing (OPF-016) or empty (OPF-017), nothing can open the book — both
/// come with RSC-003, "no rootfile tag with media type
/// application/oebps-package+xml was found".
///
/// It is repairable because the answer is in the archive: there is a package
/// document, and this tool has already found it — by extension, which is how it
/// can read a book whose container is broken in the first place. Writing that
/// path into the attribute is not a guess, it is copying down where the file
/// actually is.
///
/// Only when exactly one package document exists. A multiple-rendition EPUB has
/// several and which one is primary is a real decision, so that is reported.
pub struct ContainerRootfile;

const CONTAINER: &str = "META-INF/container.xml";

impl Fixer for ContainerRootfile {
    fn name(&self) -> &'static str {
        "container-rootfile"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-016", "OPF-017"]
    }
    fn description(&self) -> &'static str {
        "point container.xml at the package document when its full-path is missing or empty"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(CONTAINER).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let packages: Vec<&String> = book
            .names()
            .iter()
            .filter(|n| n.to_ascii_lowercase().ends_with(".opf"))
            .collect();
        if packages.len() > 1 {
            return Outcome::finding(format!(
                "container.xml has no usable full-path and the book holds {} package documents, \
                 so which one is the rendition to open is a real choice and not ours",
                packages.len()
            ));
        }

        let mut edits = Edits::new();
        let mut fixed = 0u32;

        for node in nodes.iter().filter(|n| n.name == "rootfile") {
            match node.attr("full-path") {
                // Present and pointing somewhere: not ours.
                Some(a) if !a.value.trim().is_empty() => continue,
                Some(a) => edits.replace(a.span.clone(), format!("full-path=\"{opf_name}\"")),
                // Absent: add it, just after the element name.
                None => edits.insert(node.name_end, format!(" full-path=\"{opf_name}\"")),
            }
            fixed += 1;
        }

        if fixed == 0 {
            return Outcome::none();
        }
        book.set_text(CONTAINER, edits.apply(&text));
        Outcome::change(format!(
            "pointed {fixed} container rootfile(s) at \"{opf_name}\", where the package document is"
        ))
    }
}

/// CSS-003, CSS-004, RSC-027, RSC-028, HTM-058: a text entry that is not UTF-8.
///
/// EPUB requires UTF-8 for every XML and CSS resource. A tool that wrote UTF-16
/// leaves a file no reading system has to accept, and if the file also *declares*
/// itself UTF-8 — which happens, because the declaration is boilerplate the tool
/// did not update — the mismatch is fatal rather than merely wrong. Measured on
/// that shape: `FATAL(RSC-016)`, and epubcheck stops reading the file.
///
/// [`Book`] decodes UTF-16 on the way in and [`Book::save`] writes every text
/// entry as UTF-8, so the bytes are already right by the time this runs. What is
/// left is the declaration that still says otherwise, and saying so — without a
/// reported change the book is never rewritten and the repair never lands, the
/// same trap as the mimetype entry.
pub struct Encoding;

impl Fixer for Encoding {
    fn name(&self) -> &'static str {
        "encoding"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["CSS-003", "CSS-004", "RSC-027", "RSC-028", "HTM-058"]
    }
    fn description(&self) -> &'static str {
        "re-encode a UTF-16 text entry as UTF-8, and correct what it declares"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let names: Vec<String> = book.transcoded().to_vec();
        if names.is_empty() {
            return Outcome::none();
        }

        for name in &names {
            let Some(text) = book.text(name).map(str::to_owned) else {
                continue;
            };
            let fixed = redeclare_utf8(&text);
            if fixed != text {
                book.set_text(name, fixed);
            }
        }

        Outcome::change(format!(
            "re-encoded {} entr(ies) from UTF-16 to UTF-8 [{}]",
            names.len(),
            names
                .iter()
                .map(|n| crate::util::basename(n).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// Rewrite whatever the file says about its own encoding.
///
/// Two places say it, and both have to move or the file contradicts its bytes:
/// the XML declaration's `encoding=`, and a CSS `@charset`. A declaration that
/// is absent is correct already — UTF-8 is the default in both formats.
fn redeclare_utf8(text: &str) -> String {
    static XML_ENC: LazyLock<Regex> =
        LazyLock::new(|| re(r#"(<\?xml\b[^>]*?encoding=")([^"]*)(")"#));
    static CHARSET: LazyLock<Regex> = LazyLock::new(|| re(r#"(@charset\s+")([^"]*)(")"#));

    let once = XML_ENC.replacen(text, 1, "${1}utf-8${3}");
    CHARSET.replacen(&once, 1, "${1}utf-8${3}").into_owned()
}
