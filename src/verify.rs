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
use crate::markup::{NodeKind, id_attrs, scan, well_formed};
use crate::paths::{Resolution, Resolver};
use crate::refs::resolve_href;
use crate::util::{MARKUP, ends_with_any};

/// Every id declared in one document.
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

/// Decode character references so `&mdash;` and `&#8212;` compare equal. The
/// EPUB 3 migration rewrites one form into the other without changing a word.
///
/// Public so `tests/common/verify.rs` can share it. That harness is a
/// deliberately independent implementation — checking the library's gate with
/// the library's gate would let a bug in the gate hide itself, which is how a
/// navigation blind spot survived once already — but *this* is a lookup table
/// of character references, where there is nothing to be independent about and
/// a copy that drifts just reports text as changed when it was not.
pub fn decode_entities(src: &str) -> String {
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

/// Files an EPUB requires to be well-formed XML. CSS is textual but not XML.
fn is_xml(name: &str) -> bool {
    ends_with_any(
        name,
        &[".xhtml", ".html", ".htm", ".ncx", ".opf", ".svg", ".xml"],
    )
}

/// Every defect a book has on its own terms, as comparable strings.
///
/// Deliberately *not* an assertion of correctness — plenty of real books are
/// already missing a stylesheet or pointing at an anchor that was never there.
/// The point is to be able to subtract one set from another.
/// Problems this tool can see in a book, as it stands.
///
/// Deliberately narrow, and narrower than epubcheck by a long way: duplicate
/// ids, links to files that are not there, fragments that resolve to nothing.
/// It exists to answer "did this operation make anything worse", and it is
/// reused at the end of a run to answer "is there anything left I can see" —
/// which is a different and much weaker claim than "this book is now valid".
pub fn remaining(book: &Book) -> Vec<String> {
    defects(book).into_iter().collect()
}

/// Stylesheet references that land nowhere.
///
/// A reference is a reference whether it is spelled `src="x"` or `url(x)`, and
/// `filenames` and `css-paths` both rewrite the second kind — so a stylesheet
/// left pointing at a renamed font was invisible to the gate that exists to
/// notice exactly that.
fn dead_css_urls(src: &str, here: &str, present: &HashSet<String>) -> BTreeSet<String> {
    use crate::fixers::css_paths::{IMPORT_RE, URL_RE, unquote};

    let mut found = BTreeSet::new();
    let mut check = |raw: &str| {
        if let Some((target, _)) = resolve_href(here, raw)
            && !present.contains(&target)
        {
            found.insert(format!("{here}: url() to missing file \"{target}\""));
        }
    };
    for c in IMPORT_RE.captures_iter(src) {
        if let Some(m) = c.get(1).or_else(|| c.get(2)) {
            check(unquote(m.as_str()));
        }
    }
    let imported: Vec<_> = IMPORT_RE.find_iter(src).map(|m| m.range()).collect();
    for c in URL_RE.captures_iter(src) {
        let arg = c.get(1).expect("group 1 always matches");
        if !imported.iter().any(|r| r.contains(&arg.start())) {
            check(unquote(arg.as_str()));
        }
    }
    found
}

fn defects(book: &Book) -> BTreeSet<String> {
    let mut found = BTreeSet::new();

    // Renames are pending until the archive is repacked, so `names()` still
    // reports what the book was called on the way in while its markup already
    // says what it will be called on the way out. Checking one against the
    // other reports every renamed file as missing — which is how this was
    // found, on a fixture whose cover image the filenames fixer had just
    // correctly renamed. So the scan asks what the book is about to become.
    let renamed = book.renames();
    let finally = |name: &str| renamed.get(name).map_or(name, String::as_str).to_string();
    let present: HashSet<String> = book.names().iter().map(|n| finally(n)).collect();
    let original: HashMap<String, String> = book
        .names()
        .iter()
        .map(|n| (finally(n), n.clone()))
        .collect();

    // A document that will not parse is the most serious thing there is — EPUB
    // Check calls it fatal and stops reading the file — and it is the one thing
    // every fixer is guaranteed to miss, because they all skip what they cannot
    // scan. The parser already knows exactly what is wrong; the only mistake
    // was throwing that away.
    for name in book.names() {
        if !is_xml(name) {
            continue;
        }
        let Some(text) = book.text(name) else {
            continue;
        };
        if let Err(e) = well_formed(text) {
            let line = text[..e.position.min(text.len())].lines().count();
            found.insert(format!(
                "{name}: not well-formed XML at line {line} ({}) — nothing can read this \
                 document, and no fixer can touch it",
                e.message
            ));
        }
    }

    // Content documents plus the package and the NCX: `duplicate-ids` repairs
    // all three, so the scan that reports what is left has to look at all
    // three. A second `<item id="x">` in a manifest is as much an error as a
    // second one in a chapter, and only the chapter was being asked about.
    let mut with_ids = book.markup_names();
    with_ids.extend(book.opf_name().map(str::to_owned));
    with_ids.extend(book.ncx_name().map(str::to_owned));
    with_ids.sort();
    with_ids.dedup();
    for name in with_ids {
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

    for name in book.names() {
        if ends_with_any(name, &[".css"])
            && let Some(src) = book.text(name)
        {
            found.extend(dead_css_urls(src, &finally(name), &present));
        }
    }

    let mut id_cache: HashMap<String, HashSet<String>> = HashMap::new();
    for name in book.names() {
        if !is_linky(name) {
            continue;
        }
        let Some(src) = book.text(name) else { continue };
        let Ok(nodes) = scan(src) else { continue };
        // The document's own path may have moved too, so its relative links
        // have to be read from where it is going to live.
        let here = finally(name);
        for attr in nodes
            .iter()
            .flat_map(|n| n.attrs.iter())
            .filter(|a| a.name.ends_with("href") || a.name == "src")
        {
            let Some((target, fragment)) = resolve_href(&here, &attr.value) else {
                continue;
            };
            if !present.contains(&target) {
                found.insert(format!("{here}: link to missing file \"{target}\""));
                continue;
            }
            let (Some(fragment), true) = (fragment, ends_with_any(&target, MARKUP)) else {
                continue;
            };
            let set = id_cache.entry(target.clone()).or_insert_with(|| {
                original
                    .get(&target)
                    .and_then(|o| book.text(o))
                    .map_or_else(HashSet::new, |t| ids(t).into_iter().collect())
            });
            if !set.contains(&fragment) {
                found.insert(format!(
                    "{here}: link to \"{target}#{fragment}\" but no such id exists"
                ));
            }
        }
    }

    found
}

/// Every content document the book's navigation can lead a reader to.
///
/// Three spellings of one thing, and all three count because a book may carry
/// any combination: the NCX's `<content src>`, an EPUB 3 nav document's
/// `<a href>`, and the OPF `<guide>`.
///
/// # Why this exists
///
/// [`check`] used to model entries, ids, visible text and references, and had
/// no idea what a table of contents was. A real bug walked straight through it:
/// `ncx-dead-entries` read a misspelled path as a missing chapter and removed
/// the entry, its label and everything nested under it. The text, the ids and
/// every reference were left exactly as they were — the whole of what this file
/// could see — and the book measured *cleaner* afterwards, because the error
/// went with the navigation. A gate that cannot see the table of contents
/// cannot notice one being thrown away.
///
/// # The three things that make it an invariant rather than a tripwire
///
/// **Documents, not destinations.** The unit is the file, so an entry that
/// loses its `#fragment` to id sanitisation is not reported as a lost chapter.
/// A way *into* the book is what matters here.
///
/// **The [`Resolver`], not string equality** — the whole point. A navPoint
/// saying `ch1.xhtml` for a file at `Text/ch1.xhtml` leads somewhere real, and
/// asking only whether an archive entry is spelled exactly that is how the bug
/// happened in the first place. Anything the resolver cannot place led nowhere
/// to begin with and cannot be lost.
///
/// **An entry with an anchor leads to wherever that anchor is**, which is not
/// always the document it names. A Complete Works of Aristotle navPoint labelled
/// "Page xii" points at `v1_preface.html#page_xii` and the id is in
/// `v1_ack.html`, because a content split moved it; `broken-fragments` follows
/// it there, and the preface stops being named. Counting the written document
/// would call that repair a lost chapter, when the entry was only ever about
/// page xii. So a fragment resolves to whichever document defines it — and back
/// to the written one when nothing does, or when two documents do and there is
/// no answer.
///
/// Non-markup targets are excluded, for the reason the resolver rung is there:
/// a `<guide>` entry pointing at a JPEG is not a place a reader can be sent,
/// which is precisely what OPF-032 says about it.
fn nav_reachable(book: &Book) -> BTreeSet<String> {
    let resolver = Resolver::new(book);

    // id -> the one document defining it, absent when two or more do.
    let mut owner: HashMap<String, Option<String>> = HashMap::new();
    for name in book.markup_names() {
        let Some(text) = book.text(&name) else {
            continue;
        };
        for id in ids(text) {
            owner
                .entry(id)
                .and_modify(|slot| {
                    if slot.as_ref() != Some(&name) {
                        *slot = None;
                    }
                })
                .or_insert_with(|| Some(name.clone()));
        }
    }

    let mut out = BTreeSet::new();
    for name in book.names() {
        if !is_linky(name) {
            continue;
        }
        let Some(src) = book.text(name) else { continue };
        let Ok(nodes) = scan(src) else { continue };

        for (i, node) in nodes.iter().enumerate() {
            // `<content>` and `<reference>` have no XHTML meaning, so neither
            // needs qualifying by which file it was found in.
            let attr = match node.name.as_str() {
                "content" => node.attr("src"),
                "reference" => node.attr("href"),
                "a" if in_nav(&nodes, i) => node.attr("href"),
                _ => None,
            };
            let Some(attr) = attr else { continue };
            let written = match resolver.resolve(name, &attr.value) {
                Resolution::Fine => resolve_href(name, &attr.value).map(|(t, _)| t),
                Resolution::Moved { target } => Some(target),
                _ => None,
            };
            let Some(written) = written.filter(|t| ends_with_any(t, MARKUP)) else {
                continue;
            };
            let anchored = attr
                .value
                .split_once('#')
                .and_then(|(_, f)| owner.get(f))
                .and_then(Clone::clone);
            out.insert(anchored.unwrap_or(written));
        }
    }
    out
}

/// Is this element inside an EPUB 3 `<nav>`?
fn in_nav(nodes: &[crate::markup::Node], i: usize) -> bool {
    let mut cur = nodes[i].parent;
    while let Some(p) = cur {
        if nodes[p].name == "nav" {
            return true;
        }
        cur = nodes[p].parent;
    }
    false
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
/// or newly duplicated, no visible text changed, no reference that used to
/// resolve stopped resolving, and no place the navigation could reach became
/// unreachable.
pub fn check(before: &Book, after: &Book) -> Vec<String> {
    let mut problems = Vec::new();
    let present: HashSet<&str> = after.names().iter().map(String::as_str).collect();

    for name in before.names() {
        if !present.contains(name.as_str()) {
            problems.push(format!("entry disappeared: {name}"));
        }
    }

    // Losing a destination is losing a place in the book the reader could get
    // to, and it is invisible to every other test here: the page keeps its
    // text, its ids and its links, and only the way in is gone.
    let reachable = nav_reachable(after);
    for gone in nav_reachable(before).difference(&reachable) {
        problems.push(format!("navigation no longer reaches \"{gone}\""));
    }

    for name in before.markup_names() {
        let (Some(was), Some(now)) = (before.text(&name), after.text(&name)) else {
            continue;
        };
        let after_ids: HashSet<String> = ids(now).into_iter().collect();
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
