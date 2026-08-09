//! Shared helpers and pre-compiled regexes.

use std::sync::LazyLock;

use regex::Regex;

/// Extensions we are willing to decode as UTF-8 and rewrite.
pub const TEXTUAL: &[&str] = &[
    ".html", ".xhtml", ".htm", ".opf", ".ncx", ".css", ".xml", ".xpgt", ".svg",
];

/// Extensions treated as content documents (where `id`/`name` live).
pub const MARKUP: &[&str] = &[".html", ".xhtml", ".htm"];

/// Characters that are illegal in an XML Name.
///
/// This is deliberately stricter than the XML specification, which allows a
/// wide swathe of Unicode: an id only has to be unique and stable, so ASCII
/// costs nothing and avoids every normalisation question at once.
///
/// It used to do double duty as the "unsafe filename" class too, which was a
/// mistake worth naming. A URL path segment permits far more than an XML Name
/// does — `!$&'()*+,;=` are all legal unencoded, and non-ASCII is fine — so
/// borrowing this class for filenames condemned hundreds of files per book for
/// nothing. Filename safety now lives in `fixers::filenames`, measured against
/// its own rules.
static NOT_NAME_CHAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9_.\-]").unwrap());

/// Compile a regex that is known-good at author time.
///
/// Every pattern in this crate is a literal, so a failure here is a bug in the
/// source rather than something a malformed EPUB can trigger.
pub fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("built-in regex should compile")
}

pub fn ends_with_any(name: &str, exts: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    exts.iter().any(|e| lower.ends_with(e))
}

/// An XML Name must start with a letter or underscore (we ignore the wider
/// Unicode allowances, since epubcheck-clean ASCII ids are what we want anyway).
pub fn valid_start(v: &str) -> bool {
    v.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

/// True if `v` would be rejected as an XML Name.
pub fn is_bad_id(v: &str) -> bool {
    !v.is_empty() && (!valid_start(v) || NOT_NAME_CHAR.is_match(v))
}

pub fn sanitise_id(v: &str) -> String {
    let new = NOT_NAME_CHAR.replace_all(v, "_").into_owned();
    if valid_start(&new) {
        new
    } else {
        format!("id_{new}")
    }
}

/// Last path component of a zip entry name.
pub fn basename(name: &str) -> &str {
    match name.rfind('/') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// Everything before the last path component, including the trailing slash.
pub fn dirname(name: &str) -> &str {
    match name.rfind('/') {
        Some(i) => &name[..=i],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_sanitised_to_valid_xml_names() {
        assert_eq!(sanitise_id("1chapter"), "id_1chapter");
        assert_eq!(sanitise_id("foo:bar"), "foo_bar");
        assert_eq!(sanitise_id("a b"), "a_b");
        assert_eq!(sanitise_id("ok-id.2"), "ok-id.2");
        // A leading colon sanitises to "_", which is already a legal XML Name
        // start, so no "id_" prefix is needed.
        assert_eq!(sanitise_id(":lead"), "_lead");
    }

    #[test]
    fn bad_id_detection() {
        assert!(is_bad_id("1a"));
        assert!(is_bad_id("a:b"));
        assert!(!is_bad_id("a"));
        assert!(!is_bad_id("_a-b.c"));
        assert!(
            !is_bad_id(""),
            "empty ids are left alone, as in the Python original"
        );
    }

    #[test]
    fn path_splitting() {
        assert_eq!(basename("OEBPS/Text/ch1.xhtml"), "ch1.xhtml");
        assert_eq!(dirname("OEBPS/Text/ch1.xhtml"), "OEBPS/Text/");
        assert_eq!(basename("mimetype"), "mimetype");
        assert_eq!(dirname("mimetype"), "");
    }
}
