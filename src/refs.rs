//! Resolving internal links, and the index of what points where.
//!
//! Several fixers need to know whether anything in the book links to a given
//! anchor before they touch it. Getting that wrong in the permissive direction
//! leaves a redundant anchor behind; getting it wrong in the strict direction
//! silently breaks an index nobody notices for months. So everything here errs
//! toward "assume it is referenced".

use std::collections::HashSet;
use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use regex::Regex;

use crate::util::{dirname, re};

/// Any `href=` / `src=` / `xlink:href=` attribute, in either quote style.
static REF_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?:href|src)\s*=\s*(?:"([^"]*)"|'([^']*)')"#));

/// True if `href` points somewhere outside the container.
fn is_external(href: &str) -> bool {
    // A scheme (http:, mailto:, data:) or a protocol-relative URL.
    href.starts_with("//")
        || href.find(':').is_some_and(|i| {
            let scheme = &href[..i];
            !scheme.is_empty()
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && !href[..i].contains('/')
        })
}

/// Collapse `.` and `..` segments in a container-relative path.
fn normalise(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// Resolve `href`, as written inside `from`, to `(archive name, fragment)`.
///
/// Returns `None` for anything that does not address a resource in this
/// container: external URLs and empty hrefs.
///
/// # A climb past the root is clamped, not rejected
///
/// `../../outside.xhtml` from `OEBPS/a.xhtml` comes back as `outside.xhtml`
/// rather than `None`, because [`normalise`] runs out of segments to pop and
/// stops. This doc comment used to claim otherwise, and an audit read the
/// mismatch as an underflow bug to fix — but the clamping is the more useful
/// behaviour of the two, and it is worth saying why.
///
/// `None` means [`crate::paths::Resolution::NotOurs`], and every fixer treats
/// that as *skip this, it is not a container reference*. A reference that
/// climbs out of the container is not somebody else's business, though: it is
/// broken, and clamping it to a root-relative path means the resolver either
/// finds the file the author meant or reports [`crate::paths::Resolution::Missing`]
/// — after which the link is repaired or unlinked and named. Returning `None`
/// would trade both outcomes for silence.
///
/// Nothing outside the archive can be reached either way: the result is only
/// ever looked up among the container's own entry names.
pub fn resolve_href(from: &str, href: &str) -> Option<(String, Option<String>)> {
    let href = href.trim();
    if href.is_empty() || is_external(href) {
        return None;
    }

    let (path, fragment) = match href.split_once('#') {
        Some((p, f)) => (p, (!f.is_empty()).then(|| f.to_string())),
        None => (href, None),
    };

    let path = percent_decode_str(path).decode_utf8().ok()?.into_owned();
    let fragment = match fragment {
        Some(f) => Some(percent_decode_str(&f).decode_utf8().ok()?.into_owned()),
        None => None,
    };

    // A bare "#frag" addresses the referring document itself.
    let target = if path.is_empty() {
        from.to_string()
    } else if path.starts_with('/') {
        normalise(&path)
    } else {
        normalise(&format!("{}{}", dirname(from), path))
    };

    if target.is_empty() {
        return None;
    }
    Some((target, fragment))
}

/// Every archive entry the book points at, from markup and stylesheets alike.
///
/// The manifest is deliberately not consulted: the question this answers is
/// "does anything in the book *use* this file", and the manifest is the thing
/// being checked against. Nor is it a defect index — a reference that lands
/// nowhere simply contributes nothing here.
pub fn referenced_targets(book: &crate::book::Book) -> HashSet<String> {
    use crate::fixers::css_paths::{IMPORT_RE, URL_RE, unquote};

    let mut out = HashSet::new();
    for name in book.names() {
        let Some(text) = book.text(name) else {
            continue;
        };
        let mut add = |raw: &str| {
            if let Some((target, _)) = resolve_href(name, raw) {
                out.insert(target);
            }
        };
        if crate::util::ends_with_any(name, &[".css"]) {
            for c in URL_RE.captures_iter(text) {
                add(unquote(&c[1]));
            }
            for c in IMPORT_RE.captures_iter(text) {
                if let Some(m) = c.get(1).or_else(|| c.get(2)) {
                    add(unquote(m.as_str()));
                }
            }
        } else {
            for c in REF_RE.captures_iter(text) {
                if let Some(m) = c.get(1).or_else(|| c.get(2)) {
                    add(m.as_str());
                }
            }
        }
    }
    out
}

/// The spine's content documents, in reading order, as archive names.
///
/// Everything that wants to put things in spine order — the NCX sorter, the
/// spine-based nav fallback — needs exactly this list, and the two used to
/// build it themselves. Duplicates are dropped: an itemref that names the
/// same document twice is one place in the reading order.
pub fn spine_documents(book: &crate::book::Book) -> Vec<String> {
    let Some(opf_name) = book.opf_name() else {
        return Vec::new();
    };
    let Some(opf_src) = book.opf_text() else {
        return Vec::new();
    };
    let Ok(nodes) = crate::markup::scan(opf_src) else {
        return Vec::new();
    };

    let hrefs: std::collections::HashMap<String, String> = nodes
        .iter()
        .filter(|n| n.name == "item" && n.kind != crate::markup::NodeKind::End)
        .filter_map(|n| n.attr("id").zip(n.attr("href")))
        .map(|(i, h)| (i.value.clone(), h.value.clone()))
        .collect();
    let Some(spine) = nodes
        .iter()
        .position(|n| n.name == "spine" && n.kind == crate::markup::NodeKind::Start)
    else {
        return Vec::new();
    };

    let mut out: Vec<String> = Vec::new();
    for node in nodes.iter().filter(|n| {
        n.parent == Some(spine)
            && n.name == "itemref"
            && n.kind != crate::markup::NodeKind::End
    }) {
        let Some(idref) = node.attr("idref") else {
            continue;
        };
        let Some(href) = hrefs.get(&idref.value) else {
            continue;
        };
        if let Some((target, _)) = resolve_href(opf_name, href)
            && !out.contains(&target)
        {
            out.push(target);
        }
    }
    out
}

/// Every `(document, fragment)` pair the book links to.
#[derive(Debug, Default)]
pub struct ReferenceIndex {
    resolved: HashSet<(String, String)>,
    /// Fragments whose target document could not be resolved. Any id matching
    /// one of these is treated as referenced, because we cannot prove it is not.
    unresolved: HashSet<String>,
}

impl ReferenceIndex {
    /// Scan one document's text for references and record what it points at.
    pub fn add_document(&mut self, name: &str, text: &str) {
        for c in REF_RE.captures_iter(text) {
            let href = c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str());
            match resolve_href(name, href) {
                Some((target, Some(fragment))) => {
                    self.resolved.insert((target, fragment));
                }
                Some((_, None)) => {}
                None => {
                    // Unresolvable, but it may still carry a fragment we should
                    // not orphan.
                    if let Some((_, f)) = href.split_once('#')
                        && !f.is_empty()
                    {
                        self.unresolved.insert(f.to_string());
                    }
                }
            }
        }
    }

    /// Does anything link to `#id` in `doc`?
    pub fn is_referenced(&self, doc: &str, id: &str) -> bool {
        self.resolved.contains(&(doc.to_string(), id.to_string())) || self.unresolved.contains(id)
    }

    pub fn len(&self) -> usize {
        self.resolved.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resolved.is_empty() && self.unresolved.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_paths_against_the_referring_document() {
        let f = |h: &str| resolve_href("OEBPS/Text/ch1.xhtml", h);
        assert_eq!(f("ch2.xhtml"), Some(("OEBPS/Text/ch2.xhtml".into(), None)));
        assert_eq!(
            f("../Images/x.png"),
            Some(("OEBPS/Images/x.png".into(), None))
        );
        assert_eq!(
            f("./ch2.xhtml#top"),
            Some(("OEBPS/Text/ch2.xhtml".into(), Some("top".into())))
        );
        assert_eq!(
            f("#self"),
            Some(("OEBPS/Text/ch1.xhtml".into(), Some("self".into()))),
            "a bare fragment addresses the referring document"
        );
    }

    #[test]
    fn percent_escapes_are_decoded_to_archive_names() {
        assert_eq!(
            resolve_href("OEBPS/content.opf", "cover%20image.jpg"),
            Some(("OEBPS/cover image.jpg".into(), None))
        );
    }

    #[test]
    fn external_and_empty_references_resolve_to_nothing() {
        let f = |h: &str| resolve_href("OEBPS/ch1.xhtml", h);
        assert_eq!(f("https://example.com/a#b"), None);
        assert_eq!(f("mailto:a@b.c"), None);
        assert_eq!(f("//cdn.example.com/x.png"), None);
        assert_eq!(f(""), None);
        assert_eq!(f("  "), None);
    }

    #[test]
    fn a_colon_in_a_filename_is_not_a_scheme() {
        assert_eq!(
            resolve_href("OEBPS/ch1.xhtml", "sub/a:b.xhtml"),
            Some(("OEBPS/sub/a:b.xhtml".into(), None)),
            "the colon is past a slash, so this is a path not a URL scheme"
        );
    }

    #[test]
    fn index_records_targets_and_is_conservative_about_unknowns() {
        let mut idx = ReferenceIndex::default();
        idx.add_document(
            "OEBPS/toc.ncx",
            r#"<content src="Text/ch1.xhtml#intro"/><a href='Text/ch1.xhtml#two'>x</a>"#,
        );
        assert!(idx.is_referenced("OEBPS/Text/ch1.xhtml", "intro"));
        assert!(idx.is_referenced("OEBPS/Text/ch1.xhtml", "two"));
        assert!(!idx.is_referenced("OEBPS/Text/ch1.xhtml", "three"));
        assert!(!idx.is_referenced("OEBPS/Text/ch2.xhtml", "intro"));

        // An external link carrying a fragment keeps that fragment alive
        // everywhere, since we cannot tell what it was meant to address.
        idx.add_document("OEBPS/ch9.xhtml", r#"<a href="https://x.test/p#keepme">"#);
        assert!(idx.is_referenced("anywhere.xhtml", "keepme"));
    }
}
