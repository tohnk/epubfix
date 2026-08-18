//! Fixes that apply to the NCX table of contents (`.ncx`).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Captures, NoExpand, Regex};

use crate::book::Book;
use crate::fixers::filenames::as_url_path;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, NodeKind, scan};
use crate::paths::{Resolution, Resolver, relative_to};
use crate::refs::resolve_href;
use crate::util::basename;
use crate::util::re;

static NAV_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"<(?:navPoint|navTarget|pageTarget)\b[^>]*>"));
static CONTENT_SRC_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"<content\s+src="([^"]+)""#));
static PLAY_ORDER_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"playOrder="\d*""#));
static META_RE: LazyLock<Regex> = LazyLock::new(|| re(r"<meta\b[^>]*/?>"));
static CONTENT_ATTR_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"content="[^"]*""#));

/// RSC-005: `playOrder` values that are 0-based, duplicated, gapped, or that
/// disagree between navPoints pointing at the same target.
///
/// Renumbers from 1 in document order, giving every distinct `<content src>` a
/// single number — which is what the spec means by "consistent".
pub struct PlayOrder;

impl Fixer for PlayOrder {
    fn name(&self) -> &'static str {
        "ncx-play-order"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "renumber toc.ncx playOrder consecutively from 1"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.ncx_text().map(str::to_owned) else {
            return Outcome::none();
        };

        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut counter: usize = 0;

        let contents: Vec<(usize, String)> = CONTENT_SRC_RE
            .captures_iter(&text)
            .map(|c| (c.get(0).unwrap().start(), c[1].to_string()))
            .collect();

        let new = NAV_TAG_RE
            .replace_all(&text, |c: &Captures| {
                let tag = &c[0];
                if !tag.contains("playOrder") {
                    return tag.to_string();
                }
                let m = c.get(0).expect("group 0 always matches");
                // The target is the first <content src> after the opening tag;
                // a nav element without its own inherits the next one, matching
                // the numbering readers actually apply.
                let idx = contents.partition_point(|(start, _)| *start < m.end());
                let key = if idx < contents.len() {
                    contents[idx].1.clone()
                } else {
                    format!("@{}", m.start())
                };

                let n = *seen.entry(key).or_insert_with(|| {
                    counter += 1;
                    counter
                });
                PLAY_ORDER_RE
                    .replace(tag, NoExpand(&format!("playOrder=\"{n}\"")))
                    .into_owned()
            })
            .into_owned();

        if new == text {
            return Outcome::none();
        }
        book.set_text(&name, new);
        Outcome::change(format!("renumbered playOrder ({counter} target(s))"))
    }
}

/// NCX-001: `dtb:uid` must equal the OPF unique-identifier byte for byte.
pub struct DtbUid;

impl Fixer for DtbUid {
    fn name(&self) -> &'static str {
        "ncx-uid"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["NCX-001"]
    }
    fn description(&self) -> &'static str {
        "sync toc.ncx dtb:uid to the OPF unique-identifier"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.ncx_text().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(opf) = book.opf_text().map(str::to_owned) else {
            return Outcome::none();
        };

        // Asked of the parsed package rather than of its bytes. The regex this
        // replaced was built and compiled at run time from the identifier's own
        // id — so it paid a regex compile per book, needed `regex::escape` to
        // be safe against the id, and matched only `id="x"` spelled with double
        // quotes and no space around the `=`. All three go away at once, and
        // the value now comes from the element's text content rather than from
        // a lookahead to the next `<`.
        let Ok(nodes) = scan(&opf) else {
            return Outcome::none();
        };
        let Some(unique) = nodes
            .iter()
            .find(|n| n.name == "package" && n.kind != NodeKind::End)
            .and_then(|n| n.attr("unique-identifier"))
            .map(|a| a.value.clone())
        else {
            return Outcome::none();
        };
        let Some(ident) = nodes
            .iter()
            .enumerate()
            .find(|(_, n)| {
                n.name == "identifier"
                    && n.kind == NodeKind::Start
                    && n.attr("id").is_some_and(|a| a.value == unique)
            })
            .and_then(|(_, n)| {
                let close = n.close?;
                Some(opf[n.span.end..nodes[close].span.start].trim().to_string())
            })
        else {
            return Outcome::none();
        };

        let new = META_RE
            .replace_all(&text, |c: &Captures| {
                let tag = &c[0];
                if !tag.contains("dtb:uid") {
                    return tag.to_string();
                }
                CONTENT_ATTR_RE
                    .replace(tag, NoExpand(&format!("content=\"{ident}\"")))
                    .into_owned()
            })
            .into_owned();

        if new == text {
            return Outcome::none();
        }
        book.set_text(&name, new);
        Outcome::change("synced NCX dtb:uid to OPF identifier")
    }
}

/// RSC-007, reported against the NCX: a table-of-contents entry pointing at a
/// document that is not in the book.
///
/// # A path that is wrong is not a document that is gone
///
/// This used to ask one question — is an archive entry called exactly what the
/// `<content src>` says — and remove the entry when the answer was no. That
/// reads a *misspelled path* as a missing chapter, and it is the same mistake
/// [`crate::fixers::opf::PackageReferences`] was written to stop making in the
/// package document. An NCX saying `ch1.xhtml` for a file at `Text/ch1.xhtml`
/// lost the entry, its label, and every entry nested under it, and epubcheck
/// reported a clean book afterwards. Nothing else in the pipeline repoints an
/// NCX destination: `dangling-resources` walks the book's markup and the NCX is
/// not markup, and `broken-fragments` only reaches an entry carrying a `#`.
///
/// So the [`Resolver`] decides, exactly as it does for the OPF. The file is
/// usually *there* and the path is wrong, and then the entry is repointed and
/// keeps its fragment. Only [`Resolution::Missing`] — no file of that name in
/// the archive under any path, spelling or case — removes anything, and an
/// ambiguous match is reported rather than guessed at.
///
/// Two things have to stay right about the removal: it is nesting-aware, since
/// deleting a parent takes its children with it, and it runs before
/// `ncx-play-order`, which closes the gaps left behind.
pub struct DeadNavEntries;

impl Fixer for DeadNavEntries {
    fn name(&self) -> &'static str {
        "ncx-dead-entries"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-007"]
    }
    fn description(&self) -> &'static str {
        "repoint navigation entries whose document moved, and remove the ones with no document"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let resolver = Resolver::new(book);

        // Content documents carry nav lists too, in EPUB 3.
        let mut targets: Vec<String> = book.markup_names();
        if let Some(ncx) = book.ncx_name() {
            targets.push(ncx.to_string());
        }

        for name in targets {
            let Some(text) = book.text(&name).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();
            let (mut repointed, mut removed) = (0u32, 0u32);
            // Byte ranges already scheduled, so a nested entry inside a removed
            // parent is neither deleted twice nor repaired for nothing.
            let mut covered: Vec<std::ops::Range<usize>> = Vec::new();

            for (i, node) in nodes.iter().enumerate() {
                // navPoint/navTarget/pageTarget in an NCX; <li> in an EPUB 3
                // nav document. All of them wrap a single destination.
                let entry = match node.name.as_str() {
                    "navpoint" | "navtarget" | "pagetarget" | "li" => node,
                    _ => continue,
                };
                if entry.kind == NodeKind::End {
                    continue;
                }

                // The document this entry names, from <content src> or <a href>.
                let dest = nodes.iter().filter(|c| c.parent == Some(i)).find_map(|c| {
                    match c.name.as_str() {
                        "content" => c.attr("src"),
                        "a" => c.attr("href"),
                        _ => None,
                    }
                });
                let Some(dest) = dest else { continue };

                // A parent is always seen before its children, so anything
                // inside one already going is settled.
                let span = entry.element_span(&nodes);
                if covered
                    .iter()
                    .any(|c| c.start <= span.start && span.end <= c.end)
                {
                    continue;
                }

                match resolver.resolve(&name, &dest.value) {
                    Resolution::Fine | Resolution::NotOurs => {}
                    Resolution::Moved { target } => {
                        let rel = as_url_path(&relative_to(&name, &target));
                        let value = match dest.value.split_once('#') {
                            Some((_, f)) if !f.is_empty() => format!("{rel}#{f}"),
                            _ => rel,
                        };
                        edits.replace(dest.span.clone(), format!("{}=\"{value}\"", dest.name));
                        repointed += 1;
                    }
                    Resolution::Ambiguous { count } => outcome.push_finding(format!(
                        "{}: a navigation entry points at \"{}\", and {count} files share that \
                         name, so there is no way to tell which was meant — the entry is left \
                         intact for someone who knows the book",
                        basename(&name),
                        dest.value
                    )),
                    Resolution::Missing => {
                        covered.push(span.clone());
                        edits.delete(span);
                        removed += 1;
                    }
                }
            }

            if repointed == 0 && removed == 0 {
                continue;
            }
            book.set_text(&name, edits.apply(&text));
            if repointed > 0 {
                outcome.push_change(format!(
                    "{}: repointed {repointed} navigation entr(ies) whose document moved",
                    basename(&name)
                ));
            }
            if removed > 0 {
                outcome.push_change(format!(
                    "{}: removed {removed} navigation entr(ies) for missing documents",
                    basename(&name)
                ));
            }
        }

        outcome
    }
}

/// RSC-005: `The "id" attribute does not have a unique value`, in the NCX.
///
/// Safe to renumber because `navPoint` ids are not link targets — nothing
/// addresses them by fragment. That is verified per book rather than assumed:
/// RSC-005: `<pageList>` carrying one of `id`/`class` but not the other.
///
/// The NCX schema makes these two co-required, which is an unusual enough rule
/// to be worth stating precisely, because getting it half-right is what this
/// fixer did. Measured against EPUB Check 5.2.1, all four combinations:
///
/// | `id` | `class` | |
/// |---|---|---|
/// | absent | absent | **clean** |
/// | present | absent | `element "pageList" missing required attribute "class"` |
/// | absent | present | `element "pageList" missing required attribute "id"` |
/// | present | present | **clean** |
///
/// So the defect is *exactly one* of them, and the repair is to supply its
/// partner. An earlier version fired whenever *either* was missing, which meant
/// the `neither` row — the commonest shape in the wild, and a clean one — got
/// both attributes bolted on. It proposed that on three books that validated
/// with no errors at all.
///
/// The same mistake as the SVG trigger, in a different costume: acting on
/// "this element is missing something a schema mentions" rather than on an
/// error that exists. There is no reading of the NCX schema that makes the
/// `neither` row invalid, and one run of epubcheck says so.
pub struct PageListAttrs;

impl Fixer for PageListAttrs {
    fn name(&self) -> &'static str {
        "ncx-pagelist-attrs"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "complete the co-required id/class pair on a <pageList> that has only one"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        // Every id already in the file, so a supplied one cannot collide.
        let mut taken: Vec<String> = nodes
            .iter()
            .filter_map(|n| n.attr("id"))
            .map(|a| a.value.clone())
            .collect();

        let mut edits = Edits::new();
        let mut fixed = 0u32;
        for node in nodes
            .iter()
            .filter(|n| n.name == "pagelist" && n.kind != NodeKind::End)
        {
            let add = match (node.attr("id").is_some(), node.attr("class").is_some()) {
                // Both, or neither: the schema is satisfied either way.
                (true, true) | (false, false) => continue,
                (true, false) => " class=\"pagelist\"".to_string(),
                (false, true) => {
                    let id = unique("pagelist", &taken);
                    taken.push(id.clone());
                    format!(" id=\"{id}\"")
                }
            };
            edits.insert(node.name_end, add);
            fixed += 1;
        }

        if fixed == 0 {
            return Outcome::none();
        }
        book.set_text(&name, edits.apply(&text));
        Outcome::change(format!(
            "completed the id/class pair on {fixed} half-attributed <pageList> element(s)"
        ))
    }
}

/// RSC-005: `element "navPoint" missing required attribute "id"`.
///
/// The NCX schema requires an `id` on every `navPoint`, `navTarget` and
/// `pageTarget`, and a Kobo build of *BAKEMONOGATARI* writes none at all. It is
/// a required attribute with no meaning attached: nothing in the book links to a
/// navPoint by id, and a reading system navigates by `<content src>`. So any
/// unique value satisfies the schema and changes nothing, which makes this one
/// of the few defects where inventing a value is exactly right rather than a
/// guess.
///
/// Numbered in document order and checked against every id already in the file,
/// so a book that has some and not others keeps the ones it has.
pub struct NavPointIds;

impl Fixer for NavPointIds {
    fn name(&self) -> &'static str {
        "ncx-entry-ids"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "give every NCX navigation entry the id its schema requires"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let mut taken: Vec<String> = nodes
            .iter()
            .filter_map(|n| n.attr("id"))
            .map(|a| a.value.clone())
            .collect();
        let mut edits = Edits::new();
        let mut added = 0u32;

        // Start and Empty only: an end tag carries no attributes, and the
        // branch below writes one.
        for node in nodes.iter().filter(|n| {
            matches!(n.name.as_str(), "navpoint" | "navtarget" | "pagetarget")
                && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
        }) {
            if node.attr("id").is_some() {
                continue;
            }
            let id = unique(&format!("{}-1", node.name), &taken);
            taken.push(id.clone());
            edits.insert(node.name_end, format!(" id=\"{id}\""));
            added += 1;
        }

        if added == 0 {
            return Outcome::none();
        }
        book.set_text(&name, edits.apply(&text));
        Outcome::change(format!(
            "gave {added} NCX navigation entr(ies) the id the schema requires"
        ))
    }
}

/// `base`, or `base_2`, `base_3`, ... — whichever is not in `taken`.
fn unique(base: &str, taken: &[String]) -> String {
    let mut candidate = base.to_string();
    // `taken` is finite, so at worst this stops one past its length.
    for n in 2..=taken.len() + 2 {
        if !taken.contains(&candidate) {
            break;
        }
        candidate = format!("{base}_{n}");
    }
    candidate
}

/// RSC-005 / NAV-011: sort the NCX navMap into spine order.
///
/// The NCX is the source the EPUB 3 nav document is generated from, so
/// getting the NCX right is what makes the generated nav follow the reading
/// order — with no second step working backwards. Measured on five books
/// whose generated navs drew NAV-011: every mismatch was the NCX listing
/// front-matter documents first while the spine held them last.
///
/// Sorting moves whole subtrees at every level — a child navPoint belongs to
/// its parent and travels with it, and its own children are sorted under it.
/// A navPoint whose target is not in the spine cannot be positioned, and
/// keeps the place of the entry it followed. Everything else — labels, ids,
/// playOrder, byte content — is untouched; only the order of navPoint
/// subtrees changes, so a book already in order is not rewritten.
///
/// This runs in two places: inside the migration, before the nav is
/// generated from the NCX, and as [`SpineOrder`] for EPUB 3 books that keep
/// the NCX alongside their nav — an EPUB 2-era reading system falls back to
/// the NCX, so the two tables of contents must not disagree about the
/// order. An EPUB 2 book is never touched: its NCX order is legal, and there
/// is no nav to disagree with it.
pub(crate) fn sort_to_spine(book: &mut Book) -> Outcome {
    let mut outcome = Outcome::none();
    let Some(name) = book.ncx_name().map(str::to_owned) else {
        return outcome;
    };
    let Some(text) = book.text(&name).map(str::to_owned) else {
        return outcome;
    };
    let Ok(nodes) = scan(&text) else {
        return outcome;
    };

    let spine: HashMap<String, usize> = crate::refs::spine_documents(book)
        .into_iter()
        .enumerate()
        .map(|(i, doc)| (doc, i))
        .collect();
    if spine.is_empty() {
        return outcome;
    }
    let Some(navmap) = nodes
        .iter()
        .position(|n| n.name == "navmap" && n.kind == NodeKind::Start)
    else {
        return outcome;
    };
    let Some(close) = nodes[navmap].close else {
        return outcome;
    };

    // The key is the target's place in the reading order: its document's
    // spine position, then the byte position of the fragment inside that
    // document. Measured, NAV-011 compares fragments too — a link without one
    // lands at the start of the document, and an anchor found later in the
    // text comes after it.
    let key_of = |i: usize| -> Option<(usize, usize)> {
        let content = nodes
            .iter()
            .find(|c| c.parent == Some(i) && c.name == "content" && c.kind != NodeKind::End)?;
        let (target, fragment) = resolve_href(&name, &content.attr("src")?.value)?;
        let doc = *spine.get(&target)?;
        let frag = match fragment {
            None => 0,
            Some(f) => book
                .text(&target)
                .map_or(usize::MAX, |text| fragment_offset(text, &f)),
        };
        Some((doc, frag))
    };

    let (inner, moved) = ordered_inner(&text, &nodes, navmap, close, &key_of);
    if moved == 0 {
        return outcome;
    }
    let mut edits = Edits::new();
    edits.replace(navmap_end(&nodes, navmap)..nodes[close].span.start, inner);
    book.set_text(&name, edits.apply(&text));
    outcome.push_change(format!(
        "sorted {moved} NCX navMap entr(ies) into spine order"
    ));
    outcome
}

/// The NCX of an EPUB 3 book, kept in spine order so it agrees with the nav.
///
/// EPUB 3 permits the NCX as a legacy fallback, and old reading systems use
/// it; if it disagrees with the nav about the order of chapters, the two
/// devices show two different tables of contents. epubcheck does not flag
/// the NCX order in an EPUB 3 book — which is exactly why this exists.
/// EPUB 2 books are left alone: their NCX is the only TOC they have, and its
/// order is legal as written.
pub struct SpineOrder;

impl Fixer for SpineOrder {
    fn name(&self) -> &'static str {
        "ncx-order"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005", "NAV-011"]
    }
    fn description(&self) -> &'static str {
        "sort an EPUB 3 book's NCX navMap into spine order, so it agrees with its nav"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() < 3 {
            return Outcome::none();
        }
        sort_to_spine(book)
    }
}

/// NAV-011: an EPUB 3 book shipped with a nav whose table of contents is not
/// in reading order.
///
/// A nav epubfix generates cannot have this defect — the NCX it is built from
/// is sorted first — but a nav that arrived with the book can, and that file
/// is what the reader sees. A *Spice and Wolf* lists the front matter first
/// while the spine holds it last. The fix sorts the `<li>` entries of the toc
/// nav into spine order — whole subtrees, and fragments within a document,
/// exactly as [`sort_to_spine`] sorts navPoints — and touches nothing else:
/// labels, classes and ids stay byte-for-byte, and the page-list and
/// landmarks navs are left alone.
pub struct NavOrder;

impl Fixer for NavOrder {
    fn name(&self) -> &'static str {
        "nav-order"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["NAV-011"]
    }
    fn description(&self) -> &'static str {
        "sort an existing EPUB 3 nav's table of contents into spine order"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() < 3 {
            return outcome;
        }
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return outcome;
        };
        let Some(opf_src) = book.opf_text() else {
            return outcome;
        };
        let Ok(opf_nodes) = scan(opf_src) else {
            return outcome;
        };
        // The book's own nav document, by the manifest item declaring it.
        let Some(nav_item) = opf_nodes.iter().find(|n| {
            n.name == "item"
                && n.kind != NodeKind::End
                && n.attr("properties").is_some_and(|p| {
                    p.value.split_whitespace().any(|v| v == "nav")
                })
        }) else {
            return outcome;
        };
        let Some((nav_doc, _)) = nav_item
            .attr("href")
            .and_then(|h| resolve_href(&opf_name, &h.value))
        else {
            return outcome;
        };
        let Some(text) = book.text(&nav_doc).map(str::to_owned) else {
            return outcome;
        };
        let Ok(nodes) = scan(&text) else {
            return outcome;
        };

        let spine: HashMap<String, usize> = crate::refs::spine_documents(book)
            .into_iter()
            .enumerate()
            .map(|(i, doc)| (doc, i))
            .collect();
        if spine.is_empty() {
            return outcome;
        }
        let Some(toc_nav) = nodes.iter().position(|n| {
            n.name == "nav"
                && n.kind == NodeKind::Start
                && n.attr("epub:type").is_some_and(|p| {
                    p.value.split_whitespace().any(|v| v == "toc")
                })
        }) else {
            return outcome;
        };
        let Some(close) = nodes[toc_nav].close else {
            return outcome;
        };

        // The key is the first link a <li> carries — its label anchor — in
        // the same reading-order terms as the NCX sorter: spine position,
        // then the fragment's byte position in the target document.
        let key_of = |i: usize| -> Option<(usize, usize)> {
            let anchor = nodes[i + 1..close]
                .iter()
                .find(|n| n.name == "a" && n.kind == NodeKind::Start)?;
            let (target, fragment) = resolve_href(&nav_doc, &anchor.attr("href")?.value)?;
            let doc = *spine.get(&target)?;
            let frag = match fragment {
                None => 0,
                Some(f) => book
                    .text(&target)
                    .map_or(usize::MAX, |text| fragment_offset(text, &f)),
            };
            Some((doc, frag))
        };

        let (inner, moved) = ordered_container(&text, &nodes, toc_nav, close, &key_of);
        if moved == 0 {
            return outcome;
        }
        let mut edits = Edits::new();
        edits.replace(nodes[toc_nav].span.end..nodes[close].span.start, inner);
        book.set_text(&nav_doc, edits.apply(&text));
        outcome.push_change(format!(
            "sorted {moved} toc entr(ies) into spine order in the nav document"
        ));
        outcome
    }
}

/// Byte offset just past the navMap open tag.
fn navmap_end(nodes: &[crate::markup::Node], navmap: usize) -> usize {
    nodes[navmap].span.end
}

/// The first byte position of `id="frag"` or `name="frag"` in a document, in
/// either quote style. A fragment defined nowhere sorts last — there is
/// nothing to put it before.
fn fragment_offset(text: &str, frag: &str) -> usize {
    for spelling in [
        format!("id=\"{frag}\""),
        format!("id='{frag}'"),
        format!("name=\"{frag}\""),
        format!("name='{frag}'"),
    ] {
        if let Some(pos) = text.find(&spelling) {
            return pos;
        }
    }
    usize::MAX
}

/// The bytes between a parent's open tag and its close tag, with the direct
/// navPoint children reordered to follow `key_of` — recursively, so every
/// level of nesting is sorted. Returns the rendered text and how many
/// navPoints moved.
///
/// Only the *order* of child subtrees changes: the bytes before the first
/// child (the navLabel and content of a navPoint parent), the separator
/// between children, and the bytes after the last child are kept as written,
/// and every subtree's own text is kept byte-for-byte.
fn ordered_inner(
    src: &str,
    nodes: &[crate::markup::Node],
    parent: usize,
    close: usize,
    key_of: &dyn Fn(usize) -> Option<(usize, usize)>,
) -> (String, usize) {
    let start = nodes[parent].span.end;
    let end = nodes[close].span.start;
    let children: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            n.parent == Some(parent) && n.name == "navpoint" && n.kind == NodeKind::Start
        })
        .map(|(i, _)| i)
        .collect();
    if children.is_empty() {
        return (src[start..end].to_string(), 0);
    }
    // A subtree is only sortable if every child is a complete element: the
    // rendering below stitches each one from its open tag to its close tag,
    // and a child with no close tag has no span to stitch. Leaving it out
    // would drop its bytes, so the whole container is left exactly as written
    // instead. Measured, an unclosed `<navPoint>` used to panic here —
    // `navPoint with a close tag` — which the unwind guard turned into a
    // failed book. A book nothing can sort is not a book nothing can read.
    if children.iter().any(|&c| nodes[c].close.is_none()) {
        return (src[start..end].to_string(), 0);
    }

    // An entry with no resolvable target keeps the position of the one it
    // followed — it cannot be placed by the spine, and this keeps it where
    // the book put it.
    let mut last = (0usize, 0usize);
    let mut keys: Vec<(usize, usize)> = Vec::with_capacity(children.len());
    for &child in &children {
        if let Some(k) = key_of(child) {
            last = k;
        }
        keys.push(last);
    }
    let mut order: Vec<usize> = (0..children.len()).collect();
    order.sort_by_key(|&i| keys[i]);

    let mut moved = 0usize;
    let mut rendered: Vec<String> = Vec::with_capacity(children.len());
    for (slot, &i) in order.iter().enumerate() {
        if i != slot {
            moved += 1;
        }
        let child = children[i];
        let child_close = nodes[child].close.expect("navPoint with a close tag");
        let (inner, sub_moved) = ordered_inner(src, nodes, child, child_close, key_of);
        moved += sub_moved;
        rendered.push(format!(
            "{}{}{}",
            &src[nodes[child].span.start..nodes[child].span.end],
            inner,
            &src[nodes[child_close].span.start..nodes[child_close].span.end]
        ));
    }

    let last_child = children[children.len() - 1];
    let last_close = nodes[last_child].close.expect("navPoint with a close tag");
    let prefix = &src[start..nodes[children[0]].span.start];
    let suffix = &src[nodes[last_close].span.end..end];
    let sep = if children.len() > 1 {
        let first_close = nodes[children[0]].close.expect("navPoint with a close tag");
        src[nodes[first_close].span.end..nodes[children[1]].span.start].to_string()
    } else {
        String::new()
    };
    (format!("{prefix}{}{suffix}", rendered.join(&sep)), moved)
}

/// The rendered *inner* of a nav container — a `<nav epub:type="toc">`, an
/// `<li>`, or anything holding `<ol>` lists — with every direct `<ol>` child
/// replaced by its re-rendered form. Returns the bytes between the open and
/// close tags, and how many `<li>` entries moved.
///
/// The nav's shape is `<nav><h1>…</h1><ol><li><a>…</a><ol>…</ol></li></ol></nav>`,
/// so the recursion alternates: a container rebuilds its `<ol>`s, an `<ol>`
/// sorts its `<li>`s, and each `<li>` is a container again.
fn ordered_container(
    src: &str,
    nodes: &[crate::markup::Node],
    container: usize,
    close: usize,
    key_of: &dyn Fn(usize) -> Option<(usize, usize)>,
) -> (String, usize) {
    let ols: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            *i > container
                && *i < close
                && n.parent == Some(container)
                && n.name == "ol"
                && n.kind == NodeKind::Start
        })
        .map(|(i, _)| i)
        .collect();
    if ols.is_empty() || ols.iter().any(|&o| nodes[o].close.is_none()) {
        return (
            src[nodes[container].span.end..nodes[close].span.start].to_string(),
            0,
        );
    }

    let mut moved = 0usize;
    let mut rendered: Vec<(usize, String)> = Vec::new();
    for &ol in &ols {
        let ol_close = nodes[ol].close.expect("ol with a close tag");
        let (text, m) = ordered_ol(src, nodes, ol, ol_close, key_of);
        moved += m;
        rendered.push((ol, text));
    }

    let mut out = String::new();
    let mut cursor = nodes[container].span.end;
    for (ol, text) in &rendered {
        let ol_close = nodes[*ol].close.expect("ol close");
        out.push_str(&src[cursor..nodes[*ol].span.start]);
        out.push_str(text);
        cursor = nodes[ol_close].span.end;
    }
    out.push_str(&src[cursor..nodes[close].span.start]);
    (out, moved)
}
/// The rendered `<ol>` subtree — open tag, sorted `<li>` children, close tag.
/// Only the order of `<li>` subtrees changes; everything else is kept
/// byte-for-byte, exactly as [`ordered_inner`] keeps navPoints.
fn ordered_ol(
    src: &str,
    nodes: &[crate::markup::Node],
    ol: usize,
    close: usize,
    key_of: &dyn Fn(usize) -> Option<(usize, usize)>,
) -> (String, usize) {
    let start = nodes[ol].span.end;
    let end = nodes[close].span.start;
    let items: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent == Some(ol) && n.name == "li" && n.kind == NodeKind::Start)
        .map(|(i, _)| i)
        .collect();
    if items.is_empty() || items.iter().any(|&i| nodes[i].close.is_none()) {
        return (
            src[nodes[ol].span.start..nodes[close].span.end].to_string(),
            0,
        );
    }

    let mut last = (0usize, 0usize);
    let mut keys: Vec<(usize, usize)> = Vec::with_capacity(items.len());
    for &item in &items {
        if let Some(k) = key_of(item) {
            last = k;
        }
        keys.push(last);
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| keys[i]);

    let mut moved = 0usize;
    let mut rendered: Vec<String> = Vec::with_capacity(items.len());
    for (slot, &i) in order.iter().enumerate() {
        if i != slot {
            moved += 1;
        }
        let item = items[i];
        let item_close = nodes[item].close.expect("li with a close tag");
        let (inner, sub_moved) = ordered_container(src, nodes, item, item_close, key_of);
        moved += sub_moved;
        rendered.push(format!(
            "{}{}{}",
            &src[nodes[item].span.start..nodes[item].span.end],
            inner,
            &src[nodes[item_close].span.start..nodes[item_close].span.end]
        ));
    }

    let last_item = items[items.len() - 1];
    let last_close = nodes[last_item].close.expect("li with a close tag");
    let prefix = &src[start..nodes[items[0]].span.start];
    let suffix = &src[nodes[last_close].span.end..end];
    let sep = if items.len() > 1 {
        let first_close = nodes[items[0]].close.expect("li with a close tag");
        src[nodes[first_close].span.end..nodes[items[1]].span.start].to_string()
    } else {
        String::new()
    };
    (
        format!(
            "{}{prefix}{}{suffix}{}",
            &src[nodes[ol].span.start..nodes[ol].span.end],
            rendered.join(&sep),
            &src[nodes[close].span.start..nodes[close].span.end]
        ),
        moved,
    )
}
