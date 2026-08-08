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
/// container: external URLs, empty hrefs, and paths that climb out of the root.
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
