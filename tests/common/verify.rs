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
            let mut out = Vec::new();
            for node in &nodes {
                // An element's own `name` and `id` are one identity when they
                // agree — the legacy `<a name="x" id="x">` — not two. Counting
                // them separately reported 22 duplicates in a book epubcheck
                // called clean, which is the worst kind of wrong: a report that
                // sends someone looking for a defect that is not there.
                let mut own: Vec<String> = Vec::new();
                for attr in id_attrs(node) {
                    if !own.contains(&attr.value) {
                        own.push(attr.value.clone());
                    }
                }
                out.extend(own);
            }
            out
        },
    )
}

/// Visible text: everything outside tags, whitespace collapsed, with the
/// contents of `script` and `style` dropped, and character references decoded.
///
/// The decoding is shared with `epubfix::verify` rather than reimplemented:
/// migration rewrites `&mdash;` as `&#8212;` without changing a word, and a
/// harness that compared the two spellings byte-for-byte would report every
/// migrated book as having lost text.
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
    epubfix::verify::decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

    // Stylesheets point at files too. `filenames` and `css-paths` both rewrite
    // `url()` and `@import`, and until now a fixer could leave a stylesheet
    // pointing at a file it had just renamed and every test in the suite would
    // still call the result sound. Kept as its own small matcher rather than
    // borrowed from the library, for the same reason the rest of this file is.
    for (name, _) in files
        .iter()
        .filter(|(n, _)| n.to_ascii_lowercase().ends_with(".css"))
    {
        let Some(src) = text_of(files, name) else {
            continue;
        };
        for raw in css_urls(&src) {
            let Some((target, _)) = resolve_href(name, &raw) else {
                continue;
            };
            if !names.contains(target.as_str()) {
                found.insert(format!("{name}: url() to missing file \"{target}\""));
            }
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

/// Every content document the navigation can lead a reader to.
///
/// The doc comment at the top of this file has always claimed these checks
/// catch "a fix that silences epubcheck while quietly breaking navigation".
/// They did not: `ncx-dead-entries` deleted a chapter's table-of-contents entry
/// over a misspelled path, and every fixture test in the suite called
/// `assert_sound()` on the result and passed. A deleted entry keeps the page,
/// its text, its ids and every link, and takes an epubcheck error away with it.
///
/// Three things make this an invariant rather than a tripwire, all learned
/// from that bug and from the fixture that caught the first attempt.
///
/// The unit is the **document**, so an entry that loses its `#fragment` to
/// `xml-ids` doing its job is not reported as a lost chapter.
///
/// A path is resolved **leniently** — exactly, then by unique filename —
/// because `ch1.xhtml` naming a file at `Text/ch1.xhtml` still leads somewhere,
/// and asking only for an exact archive name is the very mistake being guarded
/// against. Non-markup targets are excluded: a `<guide>` entry pointing at a
/// JPEG is not a place a reader can be sent, which is what OPF-032 says.
///
/// And **an entry with an anchor leads to wherever that anchor is**, not to the
/// document it names. `a_fragment_defined_in_exactly_one_other_document_is_
/// repointed` is the fixture: a navPoint labelled "Page xii" points at
/// `v1_preface.html#page_xii` and the id lives in `v1_ack.html` after a content
/// split, `broken-fragments` follows it there, and the preface stops being
/// named. That is the repair working. Counting the written document called it a
/// lost chapter and failed the test, correctly.
fn nav_reachable(files: &Archive) -> BTreeSet<String> {
    let leaf = |n: &str| n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();

    // id -> the one document defining it, absent when two or more do.
    let mut owner: HashMap<String, Option<String>> = HashMap::new();
    for (name, _) in files.iter().filter(|(n, _)| is_markup(n)) {
        let Some(src) = text_of(files, name) else {
            continue;
        };
        for id in ids(&src) {
            owner
                .entry(id)
                .and_modify(|slot| {
                    if slot.as_deref() != Some(name.as_str()) {
                        *slot = None;
                    }
                })
                .or_insert_with(|| Some(name.clone()));
        }
    }

    let mut out = BTreeSet::new();
    for (name, _) in files.iter().filter(|(n, _)| is_linky(n)) {
        let Some(src) = text_of(files, name) else {
            continue;
        };
        let Ok(nodes) = scan(&src) else { continue };

        for (i, node) in nodes.iter().enumerate() {
            let attr = match node.name.as_str() {
                "content" => node.attr("src"),
                "reference" => node.attr("href"),
                "a" if in_nav(&nodes, i) => node.attr("href"),
                _ => None,
            };
            let Some(attr) = attr else { continue };
            let Some((target, _)) = resolve_href(name, &attr.value) else {
                continue;
            };
            if !ends_with_any(&target, MARKUP) {
                continue;
            }
            if let Some(home) = attr
                .value
                .split_once('#')
                .and_then(|(_, f)| owner.get(f))
                .and_then(Clone::clone)
            {
                out.insert(home);
                continue;
            }
            if files.iter().any(|(n, _)| *n == target) {
                out.insert(target);
                continue;
            }
            // Not where it says, but possibly where it means: one entry with
            // the same filename is the file that moved.
            let want = leaf(&target);
            let mut hits = files.iter().filter(|(n, _)| leaf(n) == want);
            if let (Some((only, _)), None) = (hits.next(), hits.next()) {
                out.insert(only.clone());
            }
        }
    }
    out
}

/// Is this element inside an EPUB 3 `<nav>`?
fn in_nav(nodes: &[epubfix::markup::Node], i: usize) -> bool {
    let mut cur = nodes[i].parent;
    while let Some(p) = cur {
        if nodes[p].name == "nav" {
            return true;
        }
        cur = nodes[p].parent;
    }
    false
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

    let reachable = nav_reachable(after);
    for gone in nav_reachable(before).difference(&reachable) {
        r.problems
            .push(format!("navigation no longer reaches \"{gone}\""));
    }

    r.problems
        .extend(defects(after).difference(&defects(before)).cloned());

    r
}

/// Every path a stylesheet names, from `url(...)` and `@import`.
fn css_urls(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find("url(") {
        rest = &rest[i + 4..];
        if let Some(j) = rest.find(')') {
            out.push(rest[..j].trim().trim_matches(['"', '\'']).to_string());
            rest = &rest[j + 1..];
        } else {
            break;
        }
    }
    let mut rest = src;
    while let Some(i) = rest.find("@import") {
        rest = &rest[i + 7..];
        let line = rest.split(';').next().unwrap_or("").trim();
        let arg = line
            .strip_prefix("url(")
            .map_or(line, |a| a.split(')').next().unwrap_or(a));
        let arg = arg.trim().trim_matches(['"', '\'']);
        if !arg.is_empty() {
            out.push(arg.to_string());
        }
    }
    out.retain(|u| !u.starts_with("data:") && !u.contains("://"));
    out
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
