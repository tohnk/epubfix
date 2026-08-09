//! Structural invariants that must hold across any repair.
//!
//! These are the checks worth running over a real library after a sweep, and
//! they catch the dangerous class of bug: a fix that silences epubcheck while
//! quietly breaking navigation. `tests/verify.rs` proves the checks themselves
//! fire; every other test file calls [`verify`] on its own fixtures.

use std::collections::{BTreeSet, HashMap, HashSet};

use epubfix::markup::{NodeKind, id_attrs, scan};
use epubfix::refs::resolve_href;
use epubfix::util::{MARKUP, ends_with_any};

/// What changed between two versions of a book, and what looks wrong about it.
#[derive(Debug, Default)]
pub struct Report {
    /// Entries whose bytes differ.
    pub changed: Vec<String>,
    /// Entries that disappeared entirely.
    pub dropped: Vec<String>,
    /// Documents whose visible text is not the same, ignoring whitespace.
    pub text_changed: Vec<String>,
    /// Invariant violations: broken links, duplicate ids, vanished anchors.
    pub problems: Vec<String>,
}

impl Report {
    pub fn assert_sound(&self) {
        assert!(
            self.problems.is_empty(),
            "structural problems after fixing: {:#?}",
            self.problems
        );
    }
}

type Archive = [(String, Vec<u8>)];

fn get<'a>(files: &'a Archive, name: &str) -> Option<&'a [u8]> {
    files
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, d)| d.as_slice())
}

fn text_of(files: &Archive, name: &str) -> Option<String> {
    get(files, name).and_then(|d| String::from_utf8(d.to_vec()).ok())
}

fn is_markup(name: &str) -> bool {
    ends_with_any(name, MARKUP)
}

/// Documents that can carry internal links: content documents plus the NCX and
/// the package document.
fn is_linky(name: &str) -> bool {
    ends_with_any(name, &[".xhtml", ".html", ".htm", ".ncx", ".opf"])
}

/// Every id declared in one document, in order.
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

/// Visible text: everything outside tags, whitespace collapsed, with the
/// contents of `script` and `style` dropped.
pub fn visible_text(src: &str) -> String {
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
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every defect an archive has on its own terms, as comparable strings.
///
/// Not an assertion of correctness: real books arrive with dangling stylesheet
/// links and fragments that never resolved. The point is to subtract one set
/// from another, so the harness asks whether the run made anything *worse*
/// rather than whether the result is perfect.
fn defects(files: &Archive) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let names: HashSet<&str> = files.iter().map(|(n, _)| n.as_str()).collect();

    for (name, _) in files.iter().filter(|(n, _)| is_markup(n)) {
        let Some(src) = text_of(files, name) else {
            continue;
        };
        let mut seen: HashMap<String, usize> = HashMap::new();
        for id in ids(&src) {
            *seen.entry(id).or_default() += 1;
        }
        for (id, count) in seen.iter().filter(|(_, c)| **c > 1) {
            found.insert(format!("{name}: duplicate id \"{id}\" ({count} times)"));
        }
    }

    let mut id_cache: HashMap<String, HashSet<String>> = HashMap::new();
    for (name, _) in files.iter().filter(|(n, _)| is_linky(n)) {
        let Some(src) = text_of(files, name) else {
            continue;
        };
        for href in hrefs(&src) {
            let Some((target, fragment)) = resolve_href(name, &href) else {
                continue;
            };
            if !names.contains(target.as_str()) {
                found.insert(format!("{name}: link to missing file \"{target}\""));
                continue;
            }
            let Some(fragment) = fragment else { continue };
            if !is_markup(&target) {
                continue;
            }
            let set = id_cache.entry(target.clone()).or_insert_with(|| {
                text_of(files, &target).map_or_else(HashSet::new, |t| ids(&t).into_iter().collect())
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

/// Compare a book before and after a run.
///
/// The comparison is differential throughout: a defect the book arrived with is
/// not this run's fault and is not its responsibility to fix.
#[must_use]
pub fn verify(before: &Archive, after: &Archive) -> Report {
    let mut r = Report::default();

    for (name, data) in before {
        match get(after, name) {
            None => r.dropped.push(name.clone()),
            Some(now) if now != data.as_slice() => r.changed.push(name.clone()),
            Some(_) => {}
        }
    }
    // A rename shows up as a drop plus a new entry; that is legitimate, so only
    // flag a drop when nothing new appeared to replace it.
    if after.len() < before.len() {
        for d in &r.dropped {
            r.problems.push(format!("entry disappeared: {d}"));
        }
    }

    // Deliberately no "every id survives" check here. This harness runs across
    // the whole pipeline, and `xml-ids` renames ids for a living — a rename is
    // not a loss. What matters is that no reference stopped resolving, and the
    // differential check below covers exactly that.
    for (name, _) in before.iter().filter(|(n, _)| is_markup(n)) {
        let (Some(was), Some(now)) = (text_of(before, name), text_of(after, name)) else {
            continue;
        };
        if visible_text(&was) != visible_text(&now) {
            r.text_changed.push(name.clone());
        }
    }

    r.problems
        .extend(defects(after).difference(&defects(before)).cloned());

    r
}

fn hrefs(src: &str) -> Vec<String> {
    let Ok(nodes) = scan(src) else {
        return Vec::new();
    };
    nodes
        .iter()
        .flat_map(|n| n.attrs.iter())
        .filter(|a| a.name.ends_with("href") || a.name == "src")
        .map(|a| a.value.clone())
        .collect()
}
