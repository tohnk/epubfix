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
use crate::util::{is_bad_id, re, sanitise_id};

/// An `href`/`src` attribute value, captured so its span can be edited.
static HREF_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?:href|src)\s*=\s*(?:"([^"]*)"|'([^']*)')"#));

/// OPF attributes whose whole value is the id of another element in the package.
///
/// `refines` is deliberately absent: it holds a URL with a fragment, not a bare
/// id, and is handled alongside the hrefs.
const OPF_ID_ATTRS: &[&str] = &[
    "idref",
    "toc",
    "fallback",
    "media-overlay",
    "unique-identifier",
];

/// Sanitise `value`, then make sure the result is not already in use.
pub(crate) fn unique_id(value: &str, taken: &HashSet<String>) -> String {
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

    #[allow(
        clippy::too_many_lines,
        reason = "four passes over the same rename map; splitting would scatter the invariant \
                  that every pass reads the same decisions"
    )]
    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        // The NCX has ids and they have to be XML Names just as much: one real
        // book names every navPoint with a UUID, and 61 of the 96 start with a
        // digit, which an XML Name may not. Content documents alone left those
        // untouched through a whole run.
        let mut docs = book.markup_names();
        if let Some(ncx) = book.ncx_name().map(str::to_owned)
            && !docs.contains(&ncx)
        {
            docs.push(ncx);
        }
        // The OPF names its manifest items by id and refers to them through
        // `idref`/`toc`, so an invalid manifest id is an error epubcheck
        // reports against the package itself — *Lost Worlds of 2001* has four
        // ids opening with "(", which every markup-only pass sailed past.
        if let Some(opf) = book.opf_name().map(str::to_owned)
            && !docs.contains(&opf)
        {
            docs.push(opf);
        }

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

        // Pass 4: the OPF names its own ids through attributes that are not
        // hrefs, so pass 3 could not see them. Renaming an id without these is
        // worse than leaving it alone — an invalid id is an RSC-005 against the
        // package, while a `unique-identifier` pointing at nothing is OPF-030
        // and a `refines` pointing at nothing is a metadata refinement that has
        // silently stopped refining anything.
        if let Some(opf) = book.opf_name().map(str::to_owned)
            && let Some(text) = book.text(&opf).map(str::to_owned)
            && let Ok(nodes) = scan(&text)
        {
            let mut edits = Edits::new();
            for node in &nodes {
                for attr in &node.attrs {
                    // A bare id: the whole value is the name that moved.
                    if OPF_ID_ATTRS.contains(&attr.name.as_str()) {
                        if let Some(new) = renames.get(&(opf.clone(), attr.value.clone())) {
                            edits.replace(attr.span.clone(), format!("{}=\"{new}\"", attr.name));
                        }
                        continue;
                    }
                    // `refines` is a URL, almost always the bare "#id" form but
                    // legal with a path, so it is resolved like any other link
                    // and only the fragment moves.
                    if attr.name == "refines"
                        && let Some((target, Some(fragment))) = resolve_href(&opf, &attr.value)
                        && let Some(new) = renames.get(&(target, fragment))
                    {
                        let prefix = attr
                            .value
                            .split_once('#')
                            .map_or(attr.value.as_str(), |(p, _)| p);
                        edits.replace(attr.span.clone(), format!("refines=\"{prefix}#{new}\""));
                    }
                }
            }
            if !edits.is_empty() {
                book.set_text(&opf, edits.apply(&text));
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
/// `id="kobo.40.N"`. The NCX has the same problem from a different tool — one
/// *Hobbit* uses `id="chap-1"` on four separate `navPoint`s.
///
/// This used to be two fixers with one algorithm between them, and the copies
/// had drifted. The NCX one checked each candidate name against the ids it had
/// *seen so far* rather than every id in the document, so on `np`, `np`, `np_2`
/// it renamed the duplicate onto the innocent `np_2` and then renamed that to
/// `np_2_2` to resolve the collision it had just made. Two elements changed
/// where one was wrong. Pointing the correct implementation at both files is
/// the whole of the fix.
///
/// The rule: the *first* occurrence always keeps its name, so every link that
/// resolves today still resolves to the same element afterwards, and every
/// candidate replacement is checked against the document's whole id set rather
/// than a running prefix of it.
///
/// This used to stop short of a duplicate something linked to, on the grounds
/// that renaming it might break the link. It cannot. A fragment resolves to the
/// **first** element with that id in tree order, so every link already lands on
/// the first occurrence and renaming any later one cannot change where a single
/// link goes. Nothing can be deliberately pointing at the second, either — it is
/// unreachable by definition. Measured: two `id="ch"` with an `<a href="#ch">`
/// is 2 errors, and renaming only the second is 0.
pub struct DuplicateIds;

impl Fixer for DuplicateIds {
    fn name(&self) -> &'static str {
        "duplicate-ids"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "make duplicated ids unique, in content documents and the NCX alike"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let mut renamed = 0u32;

        // Every file where an id has to be unique. The NCX is XML with ids in
        // it exactly like the rest, and there was never a reason for it to have
        // its own copy of this.
        let mut targets = book.markup_names();
        if let Some(ncx) = book.ncx_name().map(str::to_owned)
            && !targets.contains(&ncx)
        {
            targets.push(ncx);
        }

        for doc in targets {
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

            // Per element, not per attribute. `<a name="x" id="x">` is the
            // legacy anchor pattern -- one identity written twice on purpose,
            // and required to match -- so counting the attributes separately
            // saw a duplicate that was not there and renamed the id to `x_2`,
            // desynchronising the pair. One real book had 303 of them.
            for attr in nodes.iter().flat_map(|n| {
                let mut own: Vec<&crate::markup::Attr> = Vec::new();
                for a in id_attrs(n) {
                    if !own.iter().any(|b| b.value == a.value) {
                        own.push(a);
                    }
                }
                own
            }) {
                // The first one keeps the name, so every link that resolves
                // today still resolves to the same element afterwards.
                if seen.insert(attr.value.clone()) {
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
