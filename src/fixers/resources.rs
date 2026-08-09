//! References that do not land: missing files, and undefined fragments.
//!
//! Both of these recover deterministically or not at all. Nothing here does
//! fuzzy matching — a repair that is "probably right" is worse than a reported
//! defect, because nobody checks a link that looks fixed.

use std::collections::HashMap;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Attr, Edits, Node, id_attrs, scan};
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

        // Basename -> archive entries with that name, for the relocation case.
        let mut by_basename: HashMap<String, Vec<String>> = HashMap::new();
        for name in book.names() {
            by_basename
                .entry(basename(name).to_ascii_lowercase())
                .or_default()
                .push(name.clone());
        }
        let present: Vec<String> = book.names().to_vec();

        let (mut repointed, mut dropped) = (0u32, 0u32);
        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in &nodes {
                for attr in node.attrs.iter().filter(|a| is_reference(a)) {
                    let Some((target, fragment)) = resolve_href(&doc, &attr.value) else {
                        continue;
                    };
                    if present.contains(&target) {
                        continue;
                    }

                    // Same file, different path?
                    let matches = by_basename
                        .get(&basename(&target).to_ascii_lowercase())
                        .map_or(&[][..], Vec::as_slice);
                    if let [only] = matches {
                        let rel = relative_to(&doc, only);
                        let value = match &fragment {
                            Some(f) => format!("{rel}#{f}"),
                            None => rel,
                        };
                        edits.replace(attr.span.clone(), format!("{}=\"{value}\"", attr.name));
                        repointed += 1;
                        continue;
                    }

                    // Genuinely absent.
                    if is_pure_include(node) {
                        edits.delete(node.element_span(&nodes));
                        dropped += 1;
                    } else {
                        outcome.push_finding(format!(
                            "{}: <{}> points at \"{}\", which is not in the book and has no \
                             match anywhere; it carries content, so it was left alone",
                            basename(&doc),
                            node.name,
                            attr.value
                        ));
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

/// Express `target` as a path relative to the directory holding `from`.
fn relative_to(from: &str, target: &str) -> String {
    let from_dir: Vec<&str> = from.split('/').collect();
    let to: Vec<&str> = target.split('/').collect();
    let shared = from_dir
        .iter()
        .take(from_dir.len() - 1)
        .zip(to.iter().take(to.len() - 1))
        .take_while(|(a, b)| a == b)
        .count();
    let ups = from_dir.len() - 1 - shared;
    let mut out = "../".repeat(ups);
    out.push_str(&to[shared..].join("/"));
    out
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

#[cfg(test)]
mod tests {
    use super::relative_to;

    #[test]
    fn relative_paths_climb_only_as_far_as_needed() {
        assert_eq!(
            relative_to("OEBPS/Text/a.xhtml", "OEBPS/Text/b.xhtml"),
            "b.xhtml"
        );
        assert_eq!(
            relative_to("OEBPS/Text/a.xhtml", "OEBPS/Styles/s.css"),
            "../Styles/s.css"
        );
        assert_eq!(
            relative_to("OEBPS/a.xhtml", "OEBPS/Styles/s.css"),
            "Styles/s.css"
        );
        assert_eq!(relative_to("a.xhtml", "b.xhtml"), "b.xhtml");
    }
}
