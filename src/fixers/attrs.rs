//! Attributes and namespaces that make a whole document fail to validate.

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, NodeKind, scan};
use crate::util::basename;

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

// ---------------------------------------------------------------------------
// Custom data attributes
// ---------------------------------------------------------------------------

/// `HTM_061`: `"data-X" is not a valid custom data attribute`.
///
/// Kindle conversion leaves `data-AmznRemoved` and `data-AmznRemoved-M8`
/// behind. Nothing reads them, and the name is invalid because HTML5 requires
/// a custom data attribute to be XML-compatible and free of ASCII uppercase.
///
/// The exact rule was measured rather than taken from the spec, and the two
/// differ in a way that matters. epubcheck rejects `data-`, `data-2foo`,
/// `data-AmznRemoved` and `data-AmznRemoved-M8`; it accepts `data-foo`,
/// `data-foo2`, `data-a`, `data-foo-`, `data-foo_bar`, `data-foo.bar` — and
/// also `data-xml-lang` and `data-xmlfoo`, which the HTML specification says
/// should be invalid. Since the point is to satisfy the validator, the
/// measured rule wins and the `xml` prefix is not treated as a defect.
///
/// **This is EPUB 3 only, and deliberately so.** Under XHTML 1.1 there is no
/// such thing as a custom data attribute, so epubcheck rejects *every*
/// `data-*` attribute in an EPUB 2 book, valid name or not. Stripping them all
/// would be silent data loss to satisfy a declaration that is very likely the
/// thing at fault — so in EPUB 2 the well-formed ones are reported instead.
pub struct DataAttributes;

/// True if `name` is a `data-` attribute HTML5 would reject.
///
/// Callers have already established the `data-` prefix.
fn is_invalid_data_name(name: &str) -> bool {
    let suffix = &name["data-".len()..];
    if suffix.is_empty() {
        return true;
    }
    // "XML-compatible" means the part after the hyphen is a valid XML Name, so
    // it cannot open with a digit.
    let first = suffix.chars().next().unwrap_or('0');
    if !(first.is_alphabetic() || first == '_') {
        return true;
    }
    name.chars().any(|c| c.is_ascii_uppercase())
}

impl Fixer for DataAttributes {
    fn name(&self) -> &'static str {
        "data-attributes"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["HTM_061", "RSC-005"]
    }
    fn description(&self) -> &'static str {
        "remove custom data attributes whose names HTML5 rejects"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let epub2 = book.epub_version() < 3;
        let mut removed = 0u32;
        let mut legal_in_epub2 = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in nodes.iter().filter(|n| n.kind != NodeKind::End) {
                for attr in &node.attrs {
                    if !attr.name.starts_with("data-") {
                        continue;
                    }
                    // The scanner lowercases names, and uppercase is half of
                    // what makes one of these invalid, so ask the source.
                    if is_invalid_data_name(attr.raw_name(&text)) {
                        edits.delete(attr.span_with_space.clone());
                        removed += 1;
                    } else if epub2 {
                        legal_in_epub2 += 1;
                    }
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if removed > 0 {
            outcome.push_change(format!(
                "removed {removed} custom data attribute(s) with names HTML5 rejects"
            ));
        }
        if legal_in_epub2 > 0 {
            outcome.push_finding(format!(
                "{legal_in_epub2} well-formed data-* attribute(s) are still errors because the \
                 book declares EPUB 2, which has no custom data attributes at all; removing \
                 them would be data loss, so the declaration is the thing to look at"
            ));
        }
        outcome
    }
}

// ---------------------------------------------------------------------------
// Missing XHTML namespace
// ---------------------------------------------------------------------------

/// RSC-005: `elements from namespace "" are not allowed`.
///
/// A content document whose root `<html>` carries no `xmlns` puts every element
/// in it into no namespace, and epubcheck rejects the document wholesale — one
/// message standing in for the entire file. It turns up on cover pages written
/// by hand or by a tool that only ever emitted HTML.
///
/// Adding the declaration is about as safe as a repair gets: an XHTML document
/// missing the XHTML namespace has exactly one thing it could have meant, the
/// namespace is what every other document in the book already uses, and the
/// change is a single attribute on a single element.
///
/// An explicit `xmlns=""` deeper in the document draws the same message but is
/// left alone — that is a deliberate statement, wrong though it is, and
/// undoing it means deciding what the author meant.
pub struct XhtmlNamespace;

impl Fixer for XhtmlNamespace {
    fn name(&self) -> &'static str {
        "xhtml-namespace"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "declare the XHTML namespace on a root <html> that is missing it"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let mut declared = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };

            let Some(html) = nodes
                .iter()
                .find(|n| n.name == "html" && n.kind != NodeKind::End)
            else {
                continue;
            };
            if html.attr("xmlns").is_some() {
                continue;
            }

            let mut edits = Edits::new();
            edits.insert(html.name_end, format!(" xmlns=\"{XHTML_NS}\""));
            book.set_text(&doc, edits.apply(&text));
            declared += 1;

            let empty = nodes
                .iter()
                .filter(|n| n.kind != NodeKind::End && n.name != "html")
                .filter(|n| n.attr("xmlns").is_some_and(|a| a.value.is_empty()))
                .count();
            if empty > 0 {
                outcome.push_finding(format!(
                    "{}: {empty} element(s) also declare xmlns=\"\" explicitly, which keeps \
                     them out of the XHTML namespace; those were left alone",
                    basename(&doc)
                ));
            }
        }

        if declared > 0 {
            outcome.push_change(format!(
                "declared the XHTML namespace on {declared} document(s) that lacked it"
            ));
        }
        outcome
    }
}

/// RSC-005: `attribute "name" not allowed here` on an `<a>`.
///
/// `<a name="x">` is how anchors were written before `id` existed, and XHTML 1.1
/// removed it — measured, an EPUB 2 book carrying one is an error whether or not
/// it also has an `id`, while an EPUB 3 book is clean either way. So this is
/// EPUB 2 only.
///
/// Almost always the name simply duplicates the id, because that is what every
/// converter emits for compatibility: one real book has 303 of
/// `<a name="_Toc252778366" id="_Toc252778366">`. There the `name` says nothing
/// the `id` does not, and dropping it loses nothing at all — every `#_Toc…`
/// link goes on resolving through the id.
///
/// Three shapes, three answers:
///
/// * **`name` equals the `id`** — drop the `name`. Nothing is lost.
/// * **no `id` at all** — rename the attribute to `id`. The anchor keeps its
///   name, its position and every link into it, and becomes legal.
/// * **`name` differs from the `id`** — the element answers to two names and
///   only one can survive. If anything links to the `name`, that is reported
///   rather than guessed at; if nothing does, the dead `name` goes.
pub struct AnchorNames;

impl Fixer for AnchorNames {
    fn name(&self) -> &'static str {
        "anchor-names"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "replace the removed name attribute on an <a> with the id it stands for"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        // HTML5 accepts `name` on an anchor, so under EPUB 3 there is nothing
        // to repair and the edit would be noise on a valid book.
        if book.epub_version() >= 3 {
            return Outcome::none();
        }
        let mut outcome = Outcome::none();
        let index = book.reference_index();
        let (mut dropped, mut promoted) = (0u32, 0u32);

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in nodes.iter().filter(|n| n.name == "a" && n.kind != NodeKind::End) {
                let Some(name) = node.attr("name") else {
                    continue;
                };
                match node.attr("id") {
                    Some(id) if id.value == name.value => {
                        edits.delete(name.span_with_space.clone());
                        dropped += 1;
                    }
                    Some(_) => {
                        if index.is_referenced(&doc, &name.value) {
                            outcome.push_finding(format!(
                                "{}: <a name=\"{}\"> is not allowed under EPUB 2 and the element \
                                 already has a different id, but something links to the name, so \
                                 one of the two targets has to be chosen by hand",
                                basename(&doc),
                                name.value
                            ));
                        } else {
                            edits.delete(name.span_with_space.clone());
                            dropped += 1;
                        }
                    }
                    None => {
                        edits.replace(name.span.clone(), format!("id=\"{}\"", name.value));
                        promoted += 1;
                    }
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if dropped > 0 {
            outcome.push_change(format!(
                "removed {dropped} <a name> attribute(s) the id already stood for"
            ));
        }
        if promoted > 0 {
            outcome.push_change(format!("turned {promoted} <a name> into the id it meant"));
        }
        outcome
    }
}

/// RSC-005: `value of attribute "X" is invalid; must be equal to …`.
///
/// XHTML 1.1 declares a handful of attributes as enumerations, and the DTD
/// spells every keyword in lower case, so `dir="LTR"` and `valign="TOP"` are
/// errors — not because the value is wrong but because its *case* is. One real
/// book has `dir="LTR"` on all 95 of its documents.
///
/// HTML5 matches enumerated values ASCII-case-insensitively, so this is EPUB 2
/// only; under EPUB 3 the same markup is clean and the edit would be noise.
///
/// The repair fires only when the value already *is* one of the keywords apart
/// from case. A `dir="sideways"` is a different defect — a value that means
/// nothing — and lower-casing it would dress up the error rather than fix it,
/// so it is left for the report.
pub struct KeywordCase;

/// `attribute -> the keywords it accepts`, as the XHTML 1.1 DTD spells them.
const KEYWORDS: &[(&str, &[&str])] = &[
    ("dir", &["ltr", "rtl"]),
    ("valign", &["top", "middle", "bottom", "baseline"]),
    ("align", &["left", "center", "right", "justify", "char"]),
    ("clear", &["left", "all", "right", "none"]),
    ("shape", &["rect", "circle", "poly", "default"]),
    ("scope", &["row", "col", "rowgroup", "colgroup"]),
    ("rules", &["none", "groups", "rows", "cols", "all"]),
    (
        "frame",
        &[
            "void", "above", "below", "hsides", "lhs", "rhs", "vsides", "box", "border",
        ],
    ),
    ("method", &["get", "post"]),
];

impl Fixer for KeywordCase {
    fn name(&self) -> &'static str {
        "keyword-case"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "lower-case an enumerated attribute value that XHTML 1.1 spells in lower case"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() >= 3 {
            return Outcome::none();
        }
        let mut counts: Vec<(&str, u32)> = Vec::new();

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in nodes.iter().filter(|n| n.kind != NodeKind::End) {
                for attr in &node.attrs {
                    let Some((name, keywords)) =
                        KEYWORDS.iter().find(|(n, _)| *n == attr.name)
                    else {
                        continue;
                    };
                    let lower = attr.value.to_ascii_lowercase();
                    // Already right, or not a keyword at all: not ours.
                    if lower == attr.value || !keywords.contains(&lower.as_str()) {
                        continue;
                    }
                    edits.replace(attr.span.clone(), format!("{}=\"{lower}\"", attr.name));
                    match counts.iter_mut().find(|(n, _)| n == name) {
                        Some(slot) => slot.1 += 1,
                        None => counts.push((name, 1)),
                    }
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if counts.is_empty() {
            return Outcome::none();
        }
        let total: u32 = counts.iter().map(|(_, c)| c).sum();
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        Outcome::change(format!(
            "lower-cased {total} enumerated attribute value(s) [{}]",
            counts
                .iter()
                .map(|(n, c)| format!("{n}x{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// Attributes HTML5 removed outright, on elements that still exist.
///
/// `shape` and `coords` describe a region of a client-side image map. HTML5
/// keeps them on `<area>`, where they mean something, and dropped them from
/// `<a>`, where they only ever applied inside a `<map>`. XHTML 1.1 allowed them
/// on any `<a>`, and Calibre writes `shape="rect"` on every link it generates —
/// a *Skylark* contents page has 33 of them, and every one is an error the
/// moment the book is retagged.
///
/// They carry no presentation and no behaviour on an ordinary link, so this is
/// a removal with nothing to weigh: no reading system has ever done anything
/// with `shape` on an `<a>` outside a map, and the link is unchanged without it.
/// That is why they are not left to `legacy-table-attrs`, which would strip them
/// and then report "no single-property CSS equivalent" — true, and beside the
/// point, since there is nothing to express.
///
/// EPUB 2 only checks against XHTML 1.1, where they are legal, so nothing
/// happens there.
pub struct ObsoleteAttributes;

/// `element -> attributes HTML5 does not allow on it`.
const OBSOLETE: &[(&str, &[&str])] = &[("a", &["shape", "coords"])];

impl Fixer for ObsoleteAttributes {
    fn name(&self) -> &'static str {
        "obsolete-attributes"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "remove image-map attributes HTML5 dropped from <a>"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() < 3 {
            return Outcome::none();
        }
        let mut removed = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            // Start and Empty only: an end tag has no attributes to remove.
            for node in nodes
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Start | NodeKind::Empty))
            {
                let Some((_, names)) = OBSOLETE.iter().find(|(el, _)| *el == node.name) else {
                    continue;
                };
                for attr in node.attrs.iter().filter(|a| names.contains(&a.name.as_str())) {
                    edits.delete(attr.span_with_space.clone());
                    removed += 1;
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if removed == 0 {
            return Outcome::none();
        }
        Outcome::change(format!(
            "removed {removed} image-map attribute(s) HTML5 does not allow on <a>"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::is_invalid_data_name;

    /// Measured against EPUB Check 5.2.1: every name below was put on one
    /// element in one EPUB 3 book and the `HTM_061` messages read off.
    #[test]
    fn data_names_match_what_epubcheck_rejects() {
        for bad in [
            "data-",
            "data-2foo",
            "data-AmznRemoved",
            "data-AmznRemoved-M8",
        ] {
            assert!(is_invalid_data_name(bad), "{bad} draws HTM_061");
        }
        for ok in [
            "data-foo",
            "data-foo2",
            "data-a",
            "data-foo-",
            "data-foo_bar",
            "data-foo.bar",
            // The HTML specification reserves an "xml" prefix; epubcheck does
            // not enforce that, and the validator is what we are fixing for.
            "data-xml-lang",
            "data-xmlfoo",
        ] {
            assert!(!is_invalid_data_name(ok), "{ok} is accepted");
        }
    }
}
