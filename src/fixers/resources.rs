//! References that do not land: missing files, and undefined fragments.
//!
//! Both of these recover deterministically or not at all. Nothing here does
//! fuzzy matching — a repair that is "probably right" is worse than a reported
//! defect, because nobody checks a link that looks fixed.

use std::collections::HashMap;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Attr, Edits, Node, NodeKind, id_attrs, scan};
use crate::paths::{Resolution, Resolver, relative_to};
use crate::refs::resolve_href;
use crate::util::{basename, ends_with_any};

/// Attributes that name another resource.
///
/// `xlink:href` matters: full-page illustrations are often wrapped in SVG as
/// `<svg:image xlink:href="images/plate.jpg"/>`, and missing it would leave
/// those unrepaired. Matching on the suffix catches any prefix, deliberately.
fn is_reference(attr: &Attr) -> bool {
    attr.name == "src" || attr.name == "href" || attr.name.ends_with(":href")
}

/// The elements whose entire purpose is to pull in a resource, and which can
/// therefore be deleted when that resource does not exist.
fn is_pure_include(node: &Node) -> bool {
    match node.name.as_str() {
        "link" => node
            .attr("rel")
            .is_some_and(|r| r.value.to_ascii_lowercase().contains("stylesheet")),
        "script" => node.attr("src").is_some(),
        _ => false,
    }
}

/// RSC-007: `Referenced resource could not be found`.
///
/// Two causes, distinguishable without guessing:
///
/// * **Wrong path to a file that exists.** Calibre writes `../styles/x.css`
///   while the manifest says `Styles/x.css`. If exactly one entry in the archive
///   has that basename, the reference is repointed at it. If none or several do,
///   nothing happens.
/// * **The file is genuinely gone.** Calibre drops the Adobe page-template but
///   leaves 100 `<link>`s behind; Kobo leaves a `<script src="kobo.js">` with no
///   script. Those elements contribute nothing but the reference, so they go.
///   An `<img>` or an `<a>` never does: those carry content or navigation, and
///   their absence is a defect to report, not to paper over.
pub struct DanglingResources;

impl Fixer for DanglingResources {
    fn name(&self) -> &'static str {
        "dangling-resources"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-007"]
    }
    fn description(&self) -> &'static str {
        "repoint references whose file moved, and drop dead stylesheet/script includes"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let resolver = Resolver::new(book);

        let (mut repointed, mut dropped) = (0u32, 0u32);
        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in &nodes {
                for attr in node.attrs.iter().filter(|a| is_reference(a)) {
                    let fragment = resolve_href(&doc, &attr.value).and_then(|(_, f)| f);
                    match resolver.resolve(&doc, &attr.value) {
                        Resolution::Fine | Resolution::NotOurs => {}
                        Resolution::Moved { target } => {
                            let rel = relative_to(&doc, &target);
                            let value = match &fragment {
                                Some(f) => format!("{rel}#{f}"),
                                None => rel,
                            };
                            edits.replace(attr.span.clone(), format!("{}=\"{value}\"", attr.name));
                            repointed += 1;
                        }
                        Resolution::Ambiguous { count } => outcome.push_finding(format!(
                            "{}: <{}> points at \"{}\", and {count} files share that name, so \
                             there is no way to tell which was meant",
                            basename(&doc),
                            node.name,
                            attr.value
                        )),
                        // Genuinely absent.
                        Resolution::Missing => {
                            if is_pure_include(node) {
                                edits.delete(node.element_span(&nodes));
                                dropped += 1;
                            } else {
                                outcome.push_finding(format!(
                                    "{}: <{}> points at \"{}\", which is not in the book and has \
                                     no match anywhere; it carries content, so it was left alone",
                                    basename(&doc),
                                    node.name,
                                    attr.value
                                ));
                            }
                        }
                    }
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if repointed > 0 {
            outcome.push_change(format!(
                "repointed {repointed} reference(s) at the moved file"
            ));
        }
        if dropped > 0 {
            outcome.push_change(format!(
                "removed {dropped} stylesheet/script include(s) with no file behind them"
            ));
        }
        outcome
    }
}

/// RSC-012: `Fragment identifier is not defined`.
///
/// Three deterministic recoveries, in order. None of them is string similarity,
/// which is the point — in one real book the broken link `#1b` had to reach
/// `id="Oneb"`, which no edit-distance heuristic would ever pair up, but which
/// the backlink identifies unambiguously.
pub struct BrokenFragments;

impl Fixer for BrokenFragments {
    fn name(&self) -> &'static str {
        "broken-fragments"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-012"]
    }
    fn description(&self) -> &'static str {
        "recover undefined fragment targets via backlinks, unique relocation, or by dropping them"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let defines = id_definitions(book);
        let backlink = backlinks(book);

        let names: Vec<String> = book.names().to_vec();
        let (mut recovered, mut relocated, mut dropped) = (0u32, 0u32, 0u32);

        for name in &names {
            let Some(text) = book.text(name).map(str::to_owned) else {
                continue;
            };
            if !is_linky(name) {
                continue;
            }
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in &nodes {
                for attr in node.attrs.iter().filter(|a| is_reference(a)) {
                    let Some((target, Some(fragment))) = resolve_href(name, &attr.value) else {
                        continue;
                    };
                    if !ends_with_any(&target, crate::util::MARKUP) {
                        continue;
                    }
                    let defined_here = defines
                        .get(&fragment)
                        .is_some_and(|docs| docs.contains(&target));
                    if defined_here {
                        continue;
                    }
                    let path = attr.value.split_once('#').map_or("", |(p, _)| p);

                    // 1. Backlink reciprocity: whatever links back to this
                    //    element's own id is the anchor it meant.
                    if let Some(own) = id_attrs(node).next()
                        && let Some(intended) = backlink.get(&own.value)
                        && defines
                            .get(intended)
                            .is_some_and(|docs| docs.contains(&target))
                    {
                        edits.replace(
                            attr.span.clone(),
                            format!("{}=\"{path}#{intended}\"", attr.name),
                        );
                        recovered += 1;
                        continue;
                    }

                    // 2. Defined in exactly one other document: the anchor moved.
                    match defines.get(&fragment).map(Vec::as_slice) {
                        Some([only]) => {
                            let rel = relative_to(name, only);
                            edits.replace(
                                attr.span.clone(),
                                format!("{}=\"{rel}#{fragment}\"", attr.name),
                            );
                            relocated += 1;
                            continue;
                        }
                        Some(many) if many.len() > 1 => {
                            outcome.push_finding(format!(
                                "{}: \"#{fragment}\" is defined in {} documents, so there is no \
                                 way to tell which was meant",
                                basename(name),
                                many.len()
                            ));
                            continue;
                        }
                        _ => {}
                    }

                    // 3. Defined nowhere at all. If the document exists, linking
                    //    to it beats linking nowhere — Calibre splits books *at*
                    //    anchors, discarding the id as it goes.
                    if names.contains(&target) && !path.is_empty() {
                        edits.replace(attr.span.clone(), format!("{}=\"{path}\"", attr.name));
                        dropped += 1;
                    } else {
                        outcome.push_finding(format!(
                            "{}: \"{}\" resolves to nothing at all",
                            basename(name),
                            attr.value
                        ));
                    }
                }
            }

            if !edits.is_empty() {
                book.set_text(name, edits.apply(&text));
            }
        }

        if recovered > 0 {
            outcome.push_change(format!(
                "recovered {recovered} fragment target(s) from their backlinks"
            ));
        }
        if relocated > 0 {
            outcome.push_change(format!(
                "repointed {relocated} fragment(s) at the document that defines them"
            ));
        }
        if dropped > 0 {
            outcome.push_change(format!(
                "dropped {dropped} fragment(s) defined nowhere, leaving the link on the document"
            ));
        }
        outcome
    }
}

/// Every id in the book, and which documents define it.
fn id_definitions(book: &Book) -> HashMap<String, Vec<String>> {
    let mut defines: HashMap<String, Vec<String>> = HashMap::new();
    for doc in book.markup_names() {
        let Some(text) = book.text(&doc) else {
            continue;
        };
        let Ok(nodes) = scan(text) else { continue };
        for attr in nodes.iter().flat_map(id_attrs) {
            defines
                .entry(attr.value.clone())
                .or_default()
                .push(doc.clone());
        }
    }
    defines
}

/// `fragment -> id of the element linking to it`.
///
/// Footnote markup is symmetric: the reference and the note each carry an id
/// and link to the other's. So when a forward link is mistyped, whatever links
/// back to *its* id names the anchor it meant.
fn backlinks(book: &Book) -> HashMap<String, String> {
    let mut backlink = HashMap::new();
    for name in book.names() {
        let Some(text) = book.text(name) else {
            continue;
        };
        let Ok(nodes) = scan(text) else { continue };
        for node in &nodes {
            let (Some(href), Some(id)) = (node.attr("href"), id_attrs(node).next()) else {
                continue;
            };
            if let Some((_, frag)) = href.value.split_once('#') {
                backlink.insert(frag.to_string(), id.value.clone());
            }
        }
    }
    backlink
}

fn is_linky(name: &str) -> bool {
    ends_with_any(name, &[".xhtml", ".html", ".htm", ".ncx", ".opf"])
}

/// True if `node` sits inside a `<nav>`.
fn in_nav(nodes: &[Node], node: &Node) -> bool {
    let mut cur = node.parent;
    while let Some(p) = cur {
        if nodes[p].name == "nav" {
            return true;
        }
        cur = nodes[p].parent;
    }
    false
}

/// The visible text of an element, for naming it in a report.
fn label(text: &str, nodes: &[Node], node: &Node) -> String {
    let Some(close) = node.close else {
        return String::new();
    };
    let inner = &text[node.span.end..nodes[close].span.start];
    let stripped: String = inner
        .split('<')
        .map(|chunk| chunk.split_once('>').map_or(chunk, |(_, after)| after))
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// HTM-025: `Non-registered URI scheme type found in href`.
///
/// Conversion tools leave their own private links behind. A Kindle-derived
/// book carries `<a href="kindle:pos:fid:0001:off:0000000000">`, which no
/// reading system on earth can follow once the file is an EPUB — the position
/// it names belongs to a different format. The same goes for the handful of
/// other reader-private schemes below.
///
/// Measured against EPUB Check 5.2.1, which keeps a narrower list than the IANA
/// registry: `http`, `https`, `mailto`, `ftp`, `tel`, `urn`, `news` and
/// `javascript` pass, while `kindle`, `calibre`, `ibooks`, `kobo`, `epub` — and
/// also `sms`, `about` and `webcal`, which are somebody's real intention — draw
/// the warning.
///
/// That difference decides the scope. Only the reader-private schemes are
/// touched, because only those are *provably* dead: the link cannot resolve
/// anywhere, for anyone, ever. A `sms:` or `x-custom:` link is a warning about
/// a link that might well work, and guessing at it is not this fixer's job.
///
/// The repair is to drop the `href` and nothing else. An `<a>` with no `href`
/// is valid in both rulesets — measured, both ways — so the text stays where it
/// is, and any `id` on the anchor stays with it, which matters because inbound
/// fragments may well be pointing at it.
pub struct DeadSchemes;

/// Schemes belonging to one reading system's internal addressing, which cannot
/// resolve in a distributed EPUB.
const READER_PRIVATE: &[&str] = &["kindle", "calibre", "ibooks", "kobo", "epub"];

/// The scheme of `value`, lowercased, if it has one.
///
/// A bare `foo.xhtml#bar` has no scheme; neither does `#bar`. The grammar is
/// RFC 3986's: a letter, then letters, digits, `+`, `-` or `.`.
fn scheme_of(value: &str) -> Option<String> {
    let (head, _) = value.split_once(':')?;
    if head.is_empty() || !head.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    head.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        .then(|| head.to_ascii_lowercase())
}

impl Fixer for DeadSchemes {
    fn name(&self) -> &'static str {
        "dead-schemes"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["HTM-025"]
    }
    fn description(&self) -> &'static str {
        "drop hrefs using a reading system's private scheme, which cannot resolve anywhere"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let mut dropped = 0u32;
        let mut seen: Vec<String> = Vec::new();

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in nodes.iter().filter(|n| n.kind != NodeKind::End) {
                for attr in node.attrs.iter().filter(|a| is_reference(a)) {
                    let Some(scheme) = scheme_of(attr.value.trim()) else {
                        continue;
                    };
                    if !READER_PRIVATE.contains(&scheme.as_str()) {
                        continue;
                    }
                    // Only an anchor can lose its reference and still be
                    // itself. An <img> without a src is a different error.
                    if node.name != "a" || attr.name != "href" {
                        outcome.push_finding(format!(
                            "{}: <{}> uses the {scheme}: scheme, which no reading system can \
                             follow, but removing the reference would leave the element \
                             invalid, so it was left alone",
                            basename(&doc),
                            node.name
                        ));
                        continue;
                    }
                    edits.delete(attr.span_with_space.clone());
                    dropped += 1;
                    if !seen.contains(&scheme) {
                        seen.push(scheme.clone());
                    }
                    // Inside a <nav> the anchor was a way of getting somewhere,
                    // and now it is not. The document stays valid — measured,
                    // in a real nav document as well as an ordinary one — but
                    // the reader loses that entry, which is worth saying out
                    // loud rather than burying in a count.
                    if in_nav(&nodes, node) {
                        outcome.push_finding(format!(
                            "{}: the \"{}\" navigation entry pointed at a {scheme}: address and \
                             now points nowhere; it is still valid, but if you want it working \
                             it needs a target choosing by hand",
                            basename(&doc),
                            label(&text, &nodes, node)
                        ));
                    }
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if dropped > 0 {
            seen.sort();
            outcome.push_change(format!(
                "dropped {dropped} dead {} link(s), keeping the text and any id",
                seen.iter()
                    .map(|s| format!("{s}:"))
                    .collect::<Vec<_>>()
                    .join("/")
            ));
        }
        outcome
    }
}

#[cfg(test)]
mod scheme_tests {
    use super::scheme_of;

    #[test]
    fn a_scheme_is_recognised_only_where_there_is_one() {
        assert_eq!(scheme_of("kindle:pos:fid:1"), Some("kindle".into()));
        assert_eq!(scheme_of("KINDLE:pos"), Some("kindle".into()));
        assert_eq!(scheme_of("x-cus+tom.1:a"), Some("x-cus+tom.1".into()));
        // Relative paths, including the ones with a colon in them.
        assert_eq!(scheme_of("chapter.xhtml#frag"), None);
        assert_eq!(scheme_of("#frag"), None);
        assert_eq!(scheme_of("../Images/a.jpg"), None);
        assert_eq!(scheme_of("2:1 Corinthians.xhtml"), None);
    }
}
