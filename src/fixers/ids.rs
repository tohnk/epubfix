//! RSC-005: `id` and `name` attributes that are not valid XML Names.
//!
//! Two things make this less simple than a search and replace.
//!
//! **`name` is not a synonym for `id`.** It is an ID-like token on exactly two
//! elements, `<a>` and `<map>`. Everywhere else — `<meta name>`, `<input name>`,
//! `<param name>` — it is an arbitrary string with no XML Name constraint, and
//! rewriting it corrupts valid markup. `<meta name="calibre:cover">` is the case
//! that bites in practice. So this fixer needs to know which element an
//! attribute sits on, which is why it goes through the scanner.
//!
//! **Ids are scoped to a document.** The rename map is therefore keyed on
//! `(document, old value)`, and a sanitised value is checked against the ids
//! already present in that document — otherwise `a:b` could be renamed onto an
//! existing `a_b` and trade one epubcheck error for a duplicate-id error.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, id_attrs, scan};
use crate::refs::resolve_href;
use crate::util::{basename, is_bad_id, re, sanitise_id};

/// An `href`/`src` attribute value, captured so its span can be edited.
static HREF_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?:href|src)\s*=\s*(?:"([^"]*)"|'([^']*)')"#));

/// Sanitise `value`, then make sure the result is not already in use.
fn unique_id(value: &str, taken: &HashSet<String>) -> String {
    let base = sanitise_id(value);
    if !taken.contains(&base) {
        return base;
    }
    let mut n = 2u32;
    let mut candidate = format!("{base}_{n}");
    while taken.contains(&candidate) {
        n += 1;
        candidate = format!("{base}_{n}");
    }
    candidate
}

pub struct XmlIds;

impl Fixer for XmlIds {
    fn name(&self) -> &'static str {
        "xml-ids"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "rewrite id/name values that are not valid XML Names, and the links to them"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let docs = book.markup_names();

        // Pass 1: per document, decide what each bad id becomes.
        let mut renames: HashMap<(String, String), String> = HashMap::new();
        for doc in &docs {
            let Some(text) = book.text(doc) else { continue };
            let nodes = match scan(text) {
                Ok(n) => n,
                Err(e) => {
                    outcome.push_finding(format!("{doc}: could not parse, ids left alone ({e})"));
                    continue;
                }
            };

            let mut taken: HashSet<String> = nodes
                .iter()
                .flat_map(id_attrs)
                .map(|a| a.value.clone())
                .collect();

            for attr in nodes.iter().flat_map(id_attrs) {
                if !is_bad_id(&attr.value) {
                    continue;
                }
                let key = (doc.clone(), attr.value.clone());
                if renames.contains_key(&key) {
                    continue;
                }
                let new = unique_id(&attr.value, &taken);
                taken.insert(new.clone());
                renames.insert(key, new);
            }
        }

        if renames.is_empty() {
            return outcome;
        }

        // Pass 2: rewrite the attributes themselves.
        for doc in &docs {
            let Some(text) = book.text(doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();
            for node in &nodes {
                for attr in id_attrs(node) {
                    if let Some(new) = renames.get(&(doc.clone(), attr.value.clone())) {
                        edits.replace(attr.span.clone(), format!("{}=\"{new}\"", attr.name));
                    }
                }
            }
            if !edits.is_empty() {
                book.set_text(doc, edits.apply(&text));
            }
        }

        // Pass 3: repoint every link whose target id moved. This covers the NCX
        // and the OPF guide as well as the content documents, and resolves each
        // href so a fragment is only rewritten for the document it addresses.
        let names: Vec<String> = book.names().to_vec();
        for name in &names {
            let Some(text) = book.text(name).map(str::to_owned) else {
                continue;
            };
            let mut edits = Edits::new();
            for c in HREF_RE.captures_iter(&text) {
                let Some(m) = c.get(1).or_else(|| c.get(2)) else {
                    continue;
                };
                let value = m.as_str();
                let Some((target, Some(fragment))) = resolve_href(name, value) else {
                    continue;
                };
                let Some(new) = renames.get(&(target, fragment)) else {
                    continue;
                };
                // Keep the path exactly as written; only the fragment moves.
                let prefix = value.split_once('#').map_or(value, |(p, _)| p);
                edits.replace(m.range(), format!("{prefix}#{new}"));
            }
            if !edits.is_empty() {
                book.set_text(name, edits.apply(&text));
            }
        }

        outcome.push_change(format!("sanitised {} id(s)", renames.len()));
        outcome
    }
}

/// RSC-005: `Duplicate ID "x"` — the same id twice in one content document.
///
/// Kobo injects reading-location spans and does not check them against what is
/// already there; one chapter of a real book had twelve copies of
/// `id="kobo.40.N"`. [`crate::fixers::ncx::NcxDuplicateIds`] does the same job
/// for the NCX, and this is the same logic pointed at the content documents,
/// with one extra worry: in a content document an id is very often a live link
/// target, and renaming the one somebody links to breaks the link.
///
/// So the *first* occurrence always keeps the name, and after that any
/// duplicate that something references is reported rather than renamed. That
/// leaves an error behind on purpose. A book where two referenced elements
/// share an id has already lost the information about which link meant which,
/// and no amount of renaming recovers it.
pub struct ContentDuplicateIds;

impl Fixer for ContentDuplicateIds {
    fn name(&self) -> &'static str {
        "content-duplicate-ids"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "make duplicated ids in a content document unique, leaving referenced ones alone"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let index = book.reference_index();
        let mut renamed = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };

            let mut taken: HashSet<String> = nodes
                .iter()
                .flat_map(id_attrs)
                .map(|a| a.value.clone())
                .collect();
            let mut seen: HashSet<String> = HashSet::new();
            let mut edits = Edits::new();

            for attr in nodes.iter().flat_map(id_attrs) {
                // The first one keeps the name, so every link that resolves
                // today still resolves to the same element afterwards.
                if seen.insert(attr.value.clone()) {
                    continue;
                }
                if index.is_referenced(&doc, &attr.value) {
                    outcome.push_finding(format!(
                        "{}: id \"{}\" appears more than once and something links to it, so \
                         the duplicate was left alone",
                        basename(&doc),
                        attr.value
                    ));
                    continue;
                }
                let new = unique_id(&attr.value, &taken);
                taken.insert(new.clone());
                edits.replace(attr.span.clone(), format!("{}=\"{new}\"", attr.name));
                renamed += 1;
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if renamed > 0 {
            outcome.push_change(format!("made {renamed} duplicated id(s) unique"));
        }
        outcome
    }
}
