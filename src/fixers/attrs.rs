//! Attributes and namespaces that make a whole document fail to validate.

use std::fmt::Write as _;

use crate::book::Book;
use crate::fixers::resources::label;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Attr, Edits, Node, NodeKind, scan};
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

            for node in nodes
                .iter()
                .filter(|n| n.name == "a" && n.kind != NodeKind::End)
            {
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
                    let Some((name, keywords)) = KEYWORDS.iter().find(|(n, _)| *n == attr.name)
                    else {
                        continue;
                    };
                    // Trimmed as well as folded. `valign=" TOP "` is the same
                    // RSC-005 as `valign="TOP"` — measured, `value of attribute
                    // "valign" is invalid` either way — and matching the
                    // untrimmed value against the keyword table missed it, so
                    // the book was told "nothing to do" with the error intact.
                    let lower = attr.value.trim().to_ascii_lowercase();
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
        counts.sort_by_key(|b| std::cmp::Reverse(b.1));
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
///
/// # Inside a `<map>` they are the only thing there is
///
/// The paragraph above rests on the word *outside*, and the first version of
/// this did not check it. An `<a shape="poly" coords="…">` inside a `<map>` is
/// not decoration — those numbers **are** the clickable region, and HTML5 did
/// not so much drop them as move them to `<area>`, which is what a map holds
/// now. Stripping the pair there flattens a working image map into a set of
/// hotspots addressing nothing, and validates clean.
///
/// No book in a 179-book library has a `<map>` at all — its 33 occurrences are
/// all in *Skylark*, all ordinary Calibre-written links like
/// `<a href="cover.xhtml" shape="rect">`. So this branch is written against
/// synthetic fixtures instead, which is sound here in a way it usually is not:
/// `<area>` is fully specified, and every rule below was measured against
/// EPUB Check 5.2.1 rather than read off the standard.
///
/// ```text
/// <map><a href shape coords>North</a></map>          2x RSC-005 (the defect)
/// <map><area shape coords href alt/><a href>…</a>    clean   <- the repair
/// <area alt> with no href                            missing required "href"
/// <area href> with no alt                            missing required "alt"
/// <area href alt="">                                 clean
/// <area> with neither href nor alt                   clean
/// <area shape="default" coords>                      "coords" not allowed here
/// <area coords> with no shape                        clean
/// shape="circ" / "rectangle" / "polygon"             value ... is invalid
/// shape="RECT"                                       clean
/// <area> nested deeper inside the <map>              clean
/// <map id> with no name                              missing required "name"
/// ```
///
/// # The repair adds rather than replaces
///
/// The obvious conversion — turn the `<a>` into an `<area>` — throws away the
/// anchor's text, and a `<map>`'s content is *rendered*: that text is a visible
/// link, and often the accessible list of destinations that makes the image map
/// usable without a pointer. So the `<area>` is **inserted** carrying the
/// geometry, and the `<a>` stays exactly where it was minus the two attributes
/// it may not have. Both point at the same place, which is what the original
/// markup meant in the readers that honoured it. Measured clean, and nothing is
/// lost.
///
/// The anchor's own words become the `alt`, since `href` and `alt` are
/// co-required on an `<area>` and the words are what the destination is called.
/// An anchor with no `href` gets neither, which is the one other clean
/// combination.
pub struct ObsoleteAttributes;

/// `element -> attributes HTML5 does not allow on it`.
const OBSOLETE: &[(&str, &[&str])] = &[("a", &["shape", "coords"])];

/// Is this element inside a client-side image map?
fn in_image_map(nodes: &[Node], i: usize) -> bool {
    let mut cur = nodes[i].parent;
    while let Some(p) = cur {
        if nodes[p].name == "map" {
            return true;
        }
        cur = nodes[p].parent;
    }
    false
}

/// The `shape` keyword HTML5 accepts for this value, if any.
///
/// The HTML 3.2 spellings are mapped rather than copied, because carrying one
/// into a new element only trades the old error for `value of attribute
/// "shape" is invalid`. A value that is none of these is not guessed at.
fn area_shape(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "rect" | "rectangle" => Some("rect"),
        "circle" | "circ" => Some("circle"),
        "poly" | "polygon" => Some("poly"),
        "default" => Some("default"),
        _ => None,
    }
}

/// A double quote would close the attribute it is being written into. Nothing
/// else needs touching: the value comes from the document's own bytes, so any
/// `&` or `<` that mattered is already a character reference — the file would
/// not be well-formed XML otherwise.
fn quote_safe(value: &str) -> String {
    value.replace('"', "&quot;")
}

/// The `<area>` an image-map `<a>` means, or `None` when its `shape` is one no
/// ruleset accepts and there is nothing certain to write.
fn area_for(anchor: &Node, text: &str, nodes: &[Node]) -> Option<String> {
    let shape = match anchor.attr("shape") {
        Some(a) => area_shape(&a.value)?,
        // What HTML falls back to when only coordinates are given.
        None => "rect",
    };
    let mut out = format!("<area shape=\"{shape}\"");
    // `default` is the whole image, and coordinates on it are an error.
    if shape != "default"
        && let Some(coords) = anchor.attr("coords")
    {
        let _ = write!(out, " coords=\"{}\"", quote_safe(&coords.value));
    }
    if let Some(href) = anchor.attr("href") {
        let _ = write!(
            out,
            " href=\"{}\" alt=\"{}\"",
            quote_safe(&href.value),
            quote_safe(&label(text, nodes, anchor))
        );
    }
    out.push_str("/>");
    Some(out)
}

impl Fixer for ObsoleteAttributes {
    fn name(&self) -> &'static str {
        "obsolete-attributes"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "move an image map's geometry onto the <area> HTML5 wants, and drop it where it is inert"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() < 3 {
            return Outcome::none();
        }
        let mut outcome = Outcome::none();
        let (mut removed, mut converted, mut named) = (0u32, 0u32, 0u32);

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            // Start and Empty only: an end tag has no attributes to remove.
            for (i, node) in nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| matches!(n.kind, NodeKind::Start | NodeKind::Empty))
            {
                // A map the hotspots hang off has to be addressable, and `name`
                // is what HTML5 requires. `usemap` matches it or the `id`, so
                // copying one to the other cannot break the link.
                if node.name == "map" && node.attr("name").is_none() {
                    match node.attr("id") {
                        Some(id) => {
                            let at = id.span.end;
                            edits.replace(at..at, format!(" name=\"{}\"", quote_safe(&id.value)));
                            named += 1;
                        }
                        None => outcome.push_finding(format!(
                            "{}: <map> has neither a name nor an id, so nothing can address it \
                             and there is no value to give it — it needs a person",
                            basename(&doc)
                        )),
                    }
                    continue;
                }

                let Some((_, obsolete)) = OBSOLETE.iter().find(|(el, _)| *el == node.name) else {
                    continue;
                };
                let present: Vec<&Attr> = node
                    .attrs
                    .iter()
                    .filter(|a| obsolete.contains(&a.name.as_str()))
                    .collect();
                if present.is_empty() {
                    continue;
                }

                // Inside a map these are the hotspot, not decoration, so they
                // move to the element that is allowed to carry them.
                if in_image_map(&nodes, i) {
                    let Some(area) = area_for(node, &text, &nodes) else {
                        outcome.push_finding(format!(
                            "{}: <a shape=\"{}\"> inside a <map> is a shape no ruleset knows, so \
                             there is no <area> to write for it and the hotspot is left as it is",
                            basename(&doc),
                            node.attr("shape").map_or("", |a| a.value.as_str())
                        ));
                        continue;
                    };
                    edits.replace(node.span.start..node.span.start, area);
                    converted += 1;
                }
                for attr in present {
                    edits.delete(attr.span_with_space.clone());
                    removed += 1;
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if converted > 0 {
            outcome.push_change(format!(
                "gave {converted} image-map hotspot(s) the <area> HTML5 keeps their shape on, \
                 leaving the link and its text in place"
            ));
        }
        if named > 0 {
            outcome.push_change(format!(
                "named {named} <map> from its own id, which is what addresses it"
            ));
        }
        if removed > 0 {
            outcome.push_change(format!(
                "removed {removed} image-map attribute(s) HTML5 does not allow on <a>"
            ));
        }
        outcome
    }
}

/// RSC-005: `attribute "value" not allowed here` on `<meta>`.
///
/// XHTML 1.1 spells the meta data as `content`, and an EPUB 2 book carrying
/// `value` on all of its `<meta name="...">` elements is an error on every
/// page — *Perfume* has the same broken tag on all 26 of its front-matter
/// files. HTML5 brought `value` back, so this is EPUB 2 only.
///
/// The rename is exact: same position, same value, only the attribute name
/// moves to the one XHTML 1.1 declares.
///
/// A `<meta>` that already carries `content` is left alone, and only the first
/// `value` on an element is ever renamed. Renaming into a name the tag already
/// uses would produce two `content` attributes, which is not an epubcheck
/// complaint but a well-formedness violation — the document would stop opening
/// at all. Trading a validation error for an unreadable file is the one outcome
/// worse than leaving the error in place.
pub struct MetaValue;

impl Fixer for MetaValue {
    fn name(&self) -> &'static str {
        "meta-value"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "rename the invalid value attribute on <meta> to the content XHTML 1.1 declares"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() >= 3 {
            return Outcome::none();
        }
        let mut renamed = 0u32;
        let mut occupied = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in nodes.iter().filter(|n| n.name == "meta") {
                let Some(attr) = node.attrs.iter().find(|a| a.name == "value") else {
                    continue;
                };
                // The name has to be free before anything can move into it.
                if node.attr("content").is_some() {
                    occupied += 1;
                    continue;
                }
                edits.replace(attr.span.clone(), format!("content=\"{}\"", attr.value));
                renamed += 1;
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        let mut outcome = Outcome::none();
        if renamed > 0 {
            outcome.push_change(format!(
                "renamed {renamed} <meta value=> attribute(s) to the content XHTML 1.1 declares"
            ));
        }
        if occupied > 0 {
            outcome.push_finding(format!(
                "{occupied} <meta> element(s) carry both value= and content=, so the invalid \
                 attribute was left in place rather than renamed onto the one already there"
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
