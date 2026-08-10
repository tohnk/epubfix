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
