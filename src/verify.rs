//! Structural invariants, used to gate risky transformations.
//!
//! An ordinary fixer changes a handful of bytes and can be reasoned about by
//! reading it. The EPUB 3 migration rewrites the package document, adds a file
//! and touches every content document, so reasoning is not enough: it runs
//! against a clone of the book and is only kept if [`check`] finds nothing
//! wrong. That turns "was the EPUB 2 declaration the mistake, or the markup?" —
//! which is undecidable — into "did the result preserve everything?", which is
//! not.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::markup::{NodeKind, id_attrs, scan};
use crate::refs::resolve_href;
use crate::util::{MARKUP, ends_with_any};

/// Every id declared in one document.
fn ids(src: &str) -> Vec<String> {
    scan(src).map_or_else(
        |_| Vec::new(),
        |nodes| {
            nodes
                .iter()
                .flat_map(id_attrs)
                .map(|a| a.value.clone())
                .collect()
        },
    )
}

/// Decode character references so `&mdash;` and `&#8212;` compare equal. The
/// EPUB 3 migration rewrites one form into the other without changing a word.
fn decode_entities(src: &str) -> String {
    static ENTITY: LazyLock<Regex> =
        LazyLock::new(|| crate::util::re(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);"));
    ENTITY
        .replace_all(src, |c: &regex::Captures| {
            let body = &c[1];
            let cp = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok()
            } else if let Some(dec) = body.strip_prefix('#') {
                dec.parse().ok()
            } else {
                crate::entities::lookup(body).or(match body {
                    "amp" => Some(38),
                    "lt" => Some(60),
                    "gt" => Some(62),
                    "quot" => Some(34),
                    "apos" => Some(39),
                    _ => None,
                })
            };
            cp.and_then(char::from_u32)
                .map_or_else(|| c[0].to_string(), |ch| ch.to_string())
        })
        .into_owned()
}

/// Visible text: everything outside tags, whitespace collapsed, `head`,
/// `script` and `style` contents dropped.
fn visible_text(src: &str) -> String {
    let Ok(nodes) = scan(src) else {
        return src.split_whitespace().collect::<Vec<_>>().join(" ");
    };
    let mut out = String::new();
    let mut cursor = 0usize;
    let mut suppress = 0u32;
    for n in &nodes {
        if cursor < n.span.start && suppress == 0 {
            out.push_str(&src[cursor..n.span.start]);
            out.push(' ');
        }
        match (n.kind, n.name.as_str()) {
            (NodeKind::Start, "script" | "style" | "head") => suppress += 1,
            (NodeKind::End, "script" | "style" | "head") => suppress = suppress.saturating_sub(1),
            _ => {}
        }
        cursor = n.span.end;
    }
    if suppress == 0 && cursor < src.len() {
        out.push_str(&src[cursor..]);
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Documents that can carry internal links.
fn is_linky(name: &str) -> bool {
    ends_with_any(name, &[".xhtml", ".html", ".htm", ".ncx", ".opf"])
}

/// Every defect a book has on its own terms, as comparable strings.
///
/// Deliberately *not* an assertion of correctness — plenty of real books are
/// already missing a stylesheet or pointing at an anchor that was never there.
/// The point is to be able to subtract one set from another.
fn defects(book: &Book) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let present: HashSet<&str> = book.names().iter().map(String::as_str).collect();

    for name in book.markup_names() {
        let Some(text) = book.text(&name) else {
            continue;
        };
        let mut seen: HashMap<String, usize> = HashMap::new();
        for id in ids(text) {
            *seen.entry(id).or_default() += 1;
        }
        for (id, n) in seen.iter().filter(|(_, n)| **n > 1) {
            found.insert(format!("{name}: duplicate id \"{id}\" ({n} times)"));
        }
    }

    let mut id_cache: HashMap<String, HashSet<String>> = HashMap::new();
    for name in book.names() {
        if !is_linky(name) {
            continue;
        }
        let Some(src) = book.text(name) else { continue };
        let Ok(nodes) = scan(src) else { continue };
        for attr in nodes
            .iter()
            .flat_map(|n| n.attrs.iter())
            .filter(|a| a.name.ends_with("href") || a.name == "src")
        {
            let Some((target, fragment)) = resolve_href(name, &attr.value) else {
                continue;
            };
            if !present.contains(target.as_str()) {
                found.insert(format!("{name}: link to missing file \"{target}\""));
                continue;
            }
            let (Some(fragment), true) = (fragment, ends_with_any(&target, MARKUP)) else {
                continue;
            };
            let set = id_cache.entry(target.clone()).or_insert_with(|| {
                book.text(&target)
                    .map_or_else(HashSet::new, |t| ids(t).into_iter().collect())
            });
            if !set.contains(&fragment) {
                found.insert(format!(
                    "{name}: link to \"{target}#{fragment}\" but no such id exists"
                ));
            }
        }
    }

    found
}

/// Compare a book against a transformed copy of itself.
///
/// **The gate is differential, not absolute.** An earlier version asserted that
/// every internal reference resolves, which sounds right and is wrong: it
/// refused nine books out of a real 37-book library, every one of them for a
/// defect that was already in the input — a `<link>` to a `page-template.xpgt`
/// Calibre had dropped, a Kobo `<script>` with no file behind it, fragments
/// pointing at ids that never existed. None of it had anything to do with the
/// operation being gated, and refusing on that basis turns away exactly the
/// books that most need help.
///
/// So the question is not "is the result perfect" but "did this make anything
/// worse". An empty result means it did not: no entry vanished, no id was lost
/// or newly duplicated, no visible text changed, and no reference that used to
/// resolve stopped resolving.
pub fn check(before: &Book, after: &Book) -> Vec<String> {
    let mut problems = Vec::new();
    let present: HashSet<&str> = after.names().iter().map(String::as_str).collect();

    for name in before.names() {
        if !present.contains(name.as_str()) {
            problems.push(format!("entry disappeared: {name}"));
        }
    }

    for name in before.markup_names() {
        let (Some(was), Some(now)) = (before.text(&name), after.text(&name)) else {
            continue;
        };
        let after_ids = ids(now);
        for id in ids(was) {
            if !after_ids.contains(&id) {
                problems.push(format!("{name}: id \"{id}\" was lost"));
            }
        }
        if visible_text(was) != visible_text(now) {
            problems.push(format!("{name}: visible text changed"));
        }
    }

    // Anything newly broken. Defects the book arrived with are not this
    // operation's fault and are not its responsibility to fix.
    problems.extend(defects(after).difference(&defects(before)).cloned());

    problems
}
