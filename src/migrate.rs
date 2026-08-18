//! EPUB 2 → EPUB 3 migration.
//!
//! Some books declare EPUB 2 but carry markup that only validates as HTML5 —
//! most often verse set in `<blockquote>`, which XHTML 1.1 requires to hold
//! block-level children while HTML5 accepts flow content. Repairing the markup
//! would mean wrapping thousands of inline runs in `<div>`s across hundreds of
//! files; flipping the declaration fixes all of it with one edit.
//!
//! Whether the declaration or the markup was originally at fault is not
//! decidable, and does not need to be: [`migrate`] runs behind the invariant
//! checks in [`crate::verify`], so it only lands if the result preserves every
//! link, id and word of text. If it does, the result is right either way.
//!
//! Everything EPUB 3 additionally requires is derivable from the file itself:
//!
//! | Requirement | Where it comes from |
//! | --- | --- |
//! | `<package version="3.0">` | attribute edit |
//! | `<!DOCTYPE html>` in content documents | replaces the XHTML 1.1 DOCTYPE |
//! | named entities rewritten as numeric | XHTML 1.1's DTD declared them; HTML5's does not |
//! | one `dcterms:modified` | generated, and only when absent |
//! | one manifest item with `properties="nav"` | a nav document built from the NCX |
//! | `opf:role` / `file-as` / `scheme` | rewritten as `<meta refines>` |
//! | `properties="svg\|scripted\|mathml\|remote-resources"` | scanned from each document |
//!
//! The last two rows of that list came from the spec this was written against.
//! The DOCTYPE and entity rows did not — they were found by running EPUB Check
//! against a migrated book, and the entity one is fatal: `<!DOCTYPE html>`
//! declares no entities, so an unconverted `&mdash;` stops the parse dead.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::book::Book;
use crate::entities;
use crate::fixers::Outcome;
use crate::markup::{Edits, Node, NodeKind, scan};
use crate::refs::resolve_href;
use crate::util::{basename, dirname, re};

static DOCTYPE_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?is)<!DOCTYPE\s+html\b[^>\[]*(?:\[[^\]]*\])?[^>]*>"));
static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| re(r"&([A-Za-z][A-Za-z0-9]*);"));
static PKG_VERSION_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(<package\b[^>]*?\bversion=")([^"]*)(")"#));

/// Bring `book` up to what EPUB 3 requires.
///
/// With `bump_version` the package declaration is changed too — that is
/// migration proper, and a policy choice. Without it, everything else still
/// applies: a book that already *declares* EPUB 3 while carrying EPUB 2 markup
/// is simply broken against its own declaration, and repairing that is ordinary
/// work. The two share every step because the requirements are the same.
///
/// Returns the outcome; on abort the book is untouched by the caller, which
/// works on a clone.
pub fn apply(book: &mut Book, bump_version: bool) -> Outcome {
    let mut outcome = Outcome::none();

    let Some(opf_name) = book.opf_name().map(str::to_owned) else {
        outcome.push_finding("no package document, cannot migrate".to_string());
        return outcome;
    };

    // 1. Content documents: DOCTYPE and entities, before anything reads them.
    rewrite_doctypes(book, &mut outcome);
    if !convert_entities(book, &mut outcome) {
        return outcome;
    }

    // 2. The nav document, built from the NCX.
    let nav = build_nav(book, &opf_name, &mut outcome);

    // 3. The package document.
    rewrite_package(book, &opf_name, nav.as_deref(), bump_version, &mut outcome);

    outcome
}

// ---------------------------------------------------------------------------
// Content documents
// ---------------------------------------------------------------------------

fn rewrite_doctypes(book: &mut Book, outcome: &mut Outcome) {
    let mut count = 0u32;
    for doc in book.markup_names() {
        let Some(text) = book.text(&doc).map(str::to_owned) else {
            continue;
        };
        let Some(m) = DOCTYPE_RE.find(&text) else {
            continue;
        };
        if m.as_str().eq_ignore_ascii_case("<!DOCTYPE html>") {
            continue;
        }
        let mut edits = Edits::new();
        edits.replace(m.range(), "<!DOCTYPE html>");
        book.set_text(&doc, edits.apply(&text));
        count += 1;
    }
    if count > 0 {
        outcome.push_change(format!(
            "replaced the XHTML 1.1 DOCTYPE in {count} document(s)"
        ));
    }
}

/// Rewrite `&mdash;` and friends as numeric references.
///
/// Returns false if a document uses an entity XHTML 1.1 never declared, which
/// means the source was already broken and migrating would make it fatal.
fn convert_entities(book: &mut Book, outcome: &mut Outcome) -> bool {
    let mut count = 0u32;
    let mut ok = true;
    for doc in book.markup_names() {
        let Some(text) = book.text(&doc).map(str::to_owned) else {
            continue;
        };
        let mut edits = Edits::new();
        let mut unknown: Vec<String> = Vec::new();
        for c in ENTITY_RE.captures_iter(&text) {
            let name = &c[1];
            if matches!(name, "amp" | "lt" | "gt" | "quot" | "apos") {
                continue;
            }
            match entities::lookup(name) {
                Some(cp) => {
                    edits.replace(c.get(0).expect("group 0").range(), format!("&#{cp};"));
                    count += 1;
                }
                None => {
                    if !unknown.contains(&name.to_string()) {
                        unknown.push(name.to_string());
                    }
                }
            }
        }
        if !unknown.is_empty() {
            outcome.push_finding(format!(
                "{}: uses entities XHTML 1.1 never declared ({}), so the document is \
                 already malformed and migrating it would make that fatal",
                basename(&doc),
                unknown.join(", ")
            ));
            ok = false;
            continue;
        }
        if !edits.is_empty() {
            book.set_text(&doc, edits.apply(&text));
        }
    }
    if count > 0 {
        outcome.push_change(format!(
            "rewrote {count} named character entit(ies) as numeric references"
        ));
    }
    ok
}

// ---------------------------------------------------------------------------
// Nav document
// ---------------------------------------------------------------------------

/// One entry in a navigation list.
struct NavItem {
    label: String,
    href: String,
    children: Vec<NavItem>,
}

/// The raw source between an element's start and end tags.
fn inner_source<'a>(src: &'a str, nodes: &[Node], i: usize) -> &'a str {
    match nodes[i].close {
        Some(c) => &src[nodes[i].span.end..nodes[c].span.start],
        None => "",
    }
}

/// The text of a `<navLabel><text>…</text></navLabel>` child.
fn nav_label(src: &str, nodes: &[Node], parent: usize) -> String {
    let label = nodes.iter().position(|n| {
        n.parent == Some(parent) && n.name == "navlabel" && n.kind == NodeKind::Start
    });
    let Some(label) = label else {
        return String::new();
    };
    let text = nodes
        .iter()
        .position(|n| n.parent == Some(label) && n.name == "text" && n.kind == NodeKind::Start);
    text.map_or_else(String::new, |t| {
        inner_source(src, nodes, t).trim().to_string()
    })
}

/// Characters that must be percent-encoded when a path is written into a
/// generated `href`.
///
/// Everything outside this set is either illegal in a URI or would have to be
/// XML-escaped in the attribute instead, and percent-encoding settles both at
/// once — `&` becomes `%26` rather than needing `&amp;`. `/` is exempt because
/// it separates the segments being written, and `-._~` are the unreserved
/// punctuation RFC 3986 never requires encoding for.
const HREF_UNSAFE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Rewrite an href written relative to `from_doc` so it works from `to_doc`.
///
/// # Why this re-encodes
///
/// [`resolve_href`] percent-*decodes* on the way in, because its job is to name
/// an archive entry and the archive holds real bytes, not escapes. That makes
/// the path it returns a filename rather than a URL, and writing a filename
/// straight into an `href` is wrong the moment it contains a character a URL
/// reserves. An NCX pointing at `Text/my%20ch.xhtml` used to produce
/// `<a href="Text/my ch.xhtml">` in the generated nav — a raw space, which is
/// not a legal URI, from a book that had spelled it correctly.
///
/// The gate in [`crate::verify::check`] cannot catch this, which is why it is
/// worth stating: it compares references *after* resolution, and resolution
/// decodes both sides, so the broken spelling and the correct one look
/// identical to it.
fn rebase(from_doc: &str, to_doc: &str, href: &str) -> String {
    let Some((target, fragment)) = resolve_href(from_doc, href) else {
        return href.to_string();
    };
    let base = dirname(to_doc);
    let rel = target.strip_prefix(base).map_or_else(
        || {
            // Different directory: climb out and back down.
            let ups = base.matches('/').count();
            format!("{}{}", "../".repeat(ups), target)
        },
        str::to_string,
    );
    let encode = |s: &str| percent_encoding::utf8_percent_encode(s, HREF_UNSAFE).to_string();
    match fragment {
        Some(f) => format!("{}#{}", encode(&rel), encode(&f)),
        None => encode(&rel),
    }
}

/// Collect `navPoint` / `pageTarget` children of `parent`, recursively.
fn collect_nav(
    src: &str,
    nodes: &[Node],
    parent: usize,
    tag: &str,
    ncx: &str,
    nav_doc: &str,
) -> Vec<NavItem> {
    nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent == Some(parent) && n.name == tag && n.kind == NodeKind::Start)
        .map(|(i, _)| {
            let href = nodes
                .iter()
                .find(|n| n.parent == Some(i) && n.name == "content")
                .and_then(|n| n.attr("src"))
                .map_or_else(String::new, |a| rebase(ncx, nav_doc, &a.value));
            NavItem {
                label: nav_label(src, nodes, i),
                href,
                children: collect_nav(src, nodes, i, tag, ncx, nav_doc),
            }
        })
        .collect()
}

fn render_list(items: &[NavItem], depth: usize) -> String {
    let pad = "  ".repeat(depth + 2);
    let mut out = format!("{pad}<ol>\n");
    for item in items {
        let label = if item.label.is_empty() {
            "Untitled"
        } else {
            &item.label
        };
        let _ = write!(out, "{pad}  <li><a href=\"{}\">{label}</a>", item.href);
        if item.children.is_empty() {
            out.push_str("</li>\n");
        } else {
            out.push('\n');
            out.push_str(&render_list(&item.children, depth + 2));
            let _ = writeln!(out, "{pad}  </li>");
        }
    }
    let _ = writeln!(out, "{pad}</ol>");
    out
}

/// The EPUB 3 structural-semantics term for an EPUB 2 `guide` reference type.
///
/// Both spellings validate — measured, with all seventeen guide types in one
/// landmarks nav and then again unmapped, and epubcheck was silent either way.
/// The mapping is still worth doing, because a reading system looking for the
/// start of the book matches `bodymatter`, not `text`, and a landmarks nav
/// nothing can read is only decorative.
fn landmark_type(guide_type: &str) -> &str {
    match guide_type {
        "title-page" => "titlepage",
        "acknowledgements" => "acknowledgments",
        "notes" => "endnotes",
        "text" => "bodymatter",
        other => other,
    }
}

/// Landmarks entries built from the OPF `guide`.
///
/// The `guide` is the EPUB 2 spelling of landmarks and stays legal in EPUB 3,
/// so this adds the modern form rather than replacing it. Entries pointing at
/// something that is not a content document are skipped: they are already an
/// OPF-032 error where they stand, and copying one into the nav would move the
/// error rather than fix it. The same goes for a document outside the spine —
/// measured, a nav landmark aimed at a non-spine document is RSC-011.
fn collect_landmarks(book: &Book, opf_name: &str, nav_doc: &str) -> Vec<(String, NavItem)> {
    let Some(opf_src) = book.opf_text() else {
        return Vec::new();
    };
    let Ok(nodes) = scan(opf_src) else {
        return Vec::new();
    };
    let present: Vec<String> = book.names().to_vec();
    let hrefs: std::collections::HashMap<String, String> = nodes
        .iter()
        .filter(|n| n.name == "item" && n.kind != NodeKind::End)
        .filter_map(|n| n.attr("id").zip(n.attr("href")))
        .map(|(i, h)| (i.value.clone(), h.value.clone()))
        .collect();
    let spine: Vec<String> = nodes
        .iter()
        .filter(|n| n.name == "itemref" && n.kind != NodeKind::End)
        .filter_map(|n| n.attr("idref").and_then(|i| hrefs.get(&i.value)))
        .filter_map(|h| resolve_href(opf_name, h).map(|(t, _)| t))
        .collect();

    nodes
        .iter()
        .filter(|n| n.name == "reference" && n.kind != NodeKind::End)
        .filter_map(|n| {
            let href = n.attr("href")?;
            let kind = n.attr("type").map_or("bodymatter", |a| a.value.as_str());
            let (target, _) = resolve_href(opf_name, &href.value)?;
            if !present.contains(&target)
                || !crate::util::ends_with_any(&target, crate::util::MARKUP)
                || !spine.contains(&target)
            {
                return None;
            }
            let label = n
                .attr("title")
                .map_or_else(|| kind.to_string(), |a| a.value.clone());
            Some((
                landmark_type(kind).to_string(),
                NavItem {
                    label,
                    href: rebase(opf_name, nav_doc, &href.value),
                    children: Vec::new(),
                },
            ))
        })
        .collect()
}

/// Render the landmarks list. Each anchor needs an `epub:type`, which epubcheck
/// enforces, and an empty `<ol>` is itself an error — so the caller must not
/// call this with nothing.
fn render_landmarks(items: &[(String, NavItem)]) -> String {
    let mut out = String::from("    <ol>\n");
    for (kind, item) in items {
        let label = if item.label.is_empty() {
            kind.as_str()
        } else {
            &item.label
        };
        let _ = writeln!(
            out,
            "      <li><a epub:type=\"{kind}\" href=\"{}\">{label}</a></li>",
            item.href
        );
    }
    out.push_str("    </ol>\n");
    out
}

/// Assemble the nav document itself.
fn nav_document(
    title: &str,
    toc: &[NavItem],
    pages: &[NavItem],
    landmarks: &[(String, NavItem)],
) -> String {
    let mut body = format!(
        "  <nav epub:type=\"toc\" id=\"toc\">\n    <h1>Contents</h1>\n{}  </nav>\n",
        render_list(toc, 0)
    );
    if !pages.is_empty() {
        let _ = write!(
            body,
            "  <nav epub:type=\"page-list\" id=\"page-list\" hidden=\"hidden\">\n\
             \x20   <h1>Pages</h1>\n{}  </nav>\n",
            render_list(pages, 0)
        );
    }
    // An empty <ol> is itself an error, so a book with no guide gets no
    // landmarks section at all rather than a blank one.
    if !landmarks.is_empty() {
        let _ = write!(
            body,
            "  <nav epub:type=\"landmarks\" id=\"landmarks\" hidden=\"hidden\">\n\
             \x20   <h1>Landmarks</h1>\n{}  </nav>\n",
            render_landmarks(landmarks)
        );
    }

    // The NCX declares the HTML 4 entity set; the nav document will not.
    let body = ENTITY_RE.replace_all(&body, |c: &regex::Captures| {
        let name = &c[1];
        if matches!(name, "amp" | "lt" | "gt" | "quot" | "apos") {
            c[0].to_string()
        } else {
            entities::lookup(name).map_or_else(|| c[0].to_string(), |cp| format!("&#{cp};"))
        }
    });

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!DOCTYPE html>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" \
         xmlns:epub=\"http://www.idpf.org/2007/ops\">\n\
         <head>\n  <title>{title}</title>\n  <meta charset=\"utf-8\"/>\n</head>\n\
         <body>\n{body}</body>\n</html>\n"
    )
}

/// Build a nav document from the NCX. Returns its archive name.
fn build_nav(book: &mut Book, opf_name: &str, outcome: &mut Outcome) -> Option<String> {
    // A book that already declares a nav needs nothing here.
    if let Some(opf_src) = book.opf_text()
        && let Ok(nodes) = scan(opf_src)
        && nodes.iter().filter(|n| n.name == "item").any(|n| {
            n.attr("properties")
                .is_some_and(|p| p.value.split_whitespace().any(|v| v == "nav"))
        })
    {
        return None;
    }

    let dir = dirname(opf_name);
    let mut nav_doc = format!("{dir}nav.xhtml");
    let mut n = 2;
    while book.name_taken(&nav_doc) {
        nav_doc = format!("{dir}nav{n}.xhtml");
        n += 1;
    }

    // The nav inherits its order from the NCX, so the NCX is sorted into
    // spine order first — the reading order a nav must carry (NAV-011). A
    // book that shipped with its own nav never reaches this point, and its
    // NCX is left exactly as the publisher wrote it.
    outcome.merge(crate::fixers::ncx::sort_to_spine(book));

    // The NCX is the first choice of source; a book whose NCX carries no
    // navigation at all — an empty <navMap>, most often — falls back to the
    // spine, where the headings hold the table of contents the reading order
    // implies.
    let (mut toc, pages, title) = match book.ncx_name().zip(book.ncx_text()) {
        Some((ncx_name, ncx_src)) => {
            let ncx_name = ncx_name.to_string();
            let ncx_src = ncx_src.to_string();
            match scan(&ncx_src) {
                Ok(nodes) => {
                    let find_root = |name: &str| {
                        nodes
                            .iter()
                            .position(|x| x.name == name && x.kind == NodeKind::Start)
                    };
                    let toc = find_root("navmap")
                        .map(|r| collect_nav(&ncx_src, &nodes, r, "navpoint", &ncx_name, &nav_doc))
                        .unwrap_or_default();
                    let pages = find_root("pagelist")
                        .map(|r| collect_nav(&ncx_src, &nodes, r, "pagetarget", &ncx_name, &nav_doc))
                        .unwrap_or_default();
                    let title = find_root("doctitle")
                        .and_then(|d| {
                            nodes
                                .iter()
                                .position(|n| n.parent == Some(d) && n.name == "text")
                        })
                        .map_or_else(
                            || "Contents".to_string(),
                            |t| inner_source(&ncx_src, &nodes, t).trim().to_string(),
                        );
                    (toc, pages, title)
                }
                Err(e) => {
                    outcome.push_finding(format!("could not parse the NCX ({e})"));
                    (Vec::new(), Vec::new(), "Contents".to_string())
                }
            }
        }
        None => (Vec::new(), Vec::new(), "Contents".to_string()),
    };

    let mut from_spine = false;
    if toc.is_empty() {
        toc = spine_toc(book, &nav_doc);
        if toc.is_empty() {
            outcome.push_finding(
                "the NCX holds no navigation and the spine holds no headings, so no nav \
                 document was generated"
                    .to_string(),
            );
            return None;
        }
        from_spine = true;
        populate_empty_navmap(book, outcome);
    }

    let landmarks = collect_landmarks(book, opf_name, &nav_doc);
    book.add_text_entry(&nav_doc, nav_document(&title, &toc, &pages, &landmarks));
    let source = if from_spine { "the spine" } else { "the NCX" };
    outcome.push_change(format!(
        "generated {} from {source} ({} entries{})",
        basename(&nav_doc),
        toc.len(),
        {
            let mut extra = String::new();
            for (n, what) in [(pages.len(), "page targets"), (landmarks.len(), "landmarks")] {
                if n > 0 {
                    let _ = write!(extra, ", {n} {what}");
                }
            }
            extra
        }
    ));
    Some(nav_doc)
}

/// Build a table of contents from the spine, for a book whose NCX carries no
/// navigation at all.
///
/// Each spine document contributes the headings it holds — `h1`, `h2`, `h3`,
/// nested by level. A document with no headings contributes one entry named
/// by its `<title>`, or by its filename when it has none or when an earlier
/// document already used the same title — a converter habit that would fill
/// the whole TOC with one repeated string. The order is the spine order,
/// which is the reading order, so a nav generated this way cannot draw the
/// NAV-011 warning an NCX order can.
///
/// `base` is the document the hrefs are written relative to — the nav
/// document, or the NCX when its navMap is being repopulated.
#[allow(
    clippy::too_many_lines,
    reason = "the spine walk and the heading stack are one pass; splitting them would \
              separate the state the nesting depends on"
)]
fn spine_toc(book: &Book, base: &str) -> Vec<NavItem> {
    let docs = crate::refs::spine_documents(book);
    if docs.is_empty() {
        return Vec::new();
    }

    let mut roots: Vec<NavItem> = Vec::new();
    let mut open: Vec<NavItem> = Vec::new();
    let mut used_labels: Vec<String> = Vec::new();
    for doc in &docs {
        let Some(src) = book.text(doc) else {
            continue;
        };
        let Ok(nodes) = scan(src) else { continue };

        let headings: Vec<(usize, NavItem)> = nodes
            .iter()
            .filter_map(|n| {
                if !matches!(n.name.as_str(), "h1" | "h2" | "h3") || n.kind != NodeKind::Start {
                    return None;
                }
                let close = n.close?;
                let label = src[n.span.end..nodes[close].span.start].trim().to_string();
                if label_text(&label).is_empty() {
                    return None;
                }
                let fragment = n
                    .attr("id")
                    .filter(|a| !crate::util::is_bad_id(&a.value))
                    .map(|a| a.value.as_str());
                Some((
                    match n.name.as_str() {
                        "h1" => 0,
                        "h2" => 1,
                        _ => 2,
                    },
                    NavItem {
                        label,
                        href: href_from(base, doc, fragment),
                        children: Vec::new(),
                    },
                ))
            })
            .collect();

        // Every document starts at the top level; a heading stack left open
        // belongs to the document it opened in.
        while let Some(done) = open.pop() {
            if let Some(parent) = open.last_mut() {
                parent.children.push(done);
            } else {
                roots.push(done);
            }
        }

        if headings.is_empty() {
            let label = doc_title(src, &nodes)
                .filter(|t| !used_labels.contains(t))
                .unwrap_or_else(|| {
                    basename(doc).rsplit_once('.').map_or_else(
                        || basename(doc).to_string(),
                        |(stem, _)| stem.to_string(),
                    )
                });
            used_labels.push(label.clone());
            roots.push(NavItem {
                label,
                href: href_from(base, doc, None),
                children: Vec::new(),
            });
            continue;
        }

        for (level, heading) in headings {
            while open.len() > level {
                let done = open.pop().expect("len checked");
                if let Some(parent) = open.last_mut() {
                    parent.children.push(done);
                } else {
                    roots.push(done);
                }
            }
            open.push(heading);
        }
    }
    while let Some(done) = open.pop() {
        if let Some(parent) = open.last_mut() {
            parent.children.push(done);
        } else {
            roots.push(done);
        }
    }
    roots
}

/// A document's `<title>`, when it has one worth reading.
fn doc_title(src: &str, nodes: &[Node]) -> Option<String> {
    let title = nodes
        .iter()
        .find(|n| n.name == "title" && n.kind == NodeKind::Start)?;
    let close = title.close?;
    Some(src[title.span.end..nodes[close].span.start].trim().to_string()).filter(|t| !t.is_empty())
}

/// A reference to `doc`, written relative to `base` and percent-encoded the
/// way [`rebase`] writes hrefs.
fn href_from(base: &str, doc: &str, fragment: Option<&str>) -> String {
    let rel = crate::paths::relative_to(base, doc);
    let encode = |s: &str| percent_encoding::utf8_percent_encode(s, HREF_UNSAFE).to_string();
    match fragment {
        Some(f) => format!("{}#{}", encode(&rel), encode(f)),
        None => encode(&rel),
    }
}

/// The plain text of a label: tags stripped, entity references kept — the
/// nav document and the NCX both accept those as-is.
fn label_text(markup: &str) -> String {
    static TAG_RE: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]*>"));
    TAG_RE
        .replace_all(markup, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// An NCX whose `<navMap>` holds no navPoints is itself an error — epubcheck
/// reports the navMap incomplete — and the nav generated beside it makes the
/// empty one pointless. Write the same entries into it, so the NCX stays
/// valid and keeps serving EPUB 2 reading systems. Labels must be plain text
/// here, so markup is stripped.
fn populate_empty_navmap(book: &mut Book, outcome: &mut Outcome) {
    let Some(ncx_name) = book.ncx_name().map(str::to_owned) else {
        return;
    };
    let Some(text) = book.text(&ncx_name).map(str::to_owned) else {
        return;
    };
    let Ok(nodes) = scan(&text) else {
        return;
    };
    let Some(navmap) = nodes
        .iter()
        .find(|n| n.name == "navmap" && matches!(n.kind, NodeKind::Start | NodeKind::Empty))
    else {
        return;
    };

    let taken: std::collections::HashSet<&str> = nodes
        .iter()
        .filter_map(|n| n.attr("id"))
        .map(|a| a.value.as_str())
        .collect();
    let mut serial = 0u32;
    let ncx_toc = spine_toc(book, &ncx_name);
    let body = render_navpoints(&ncx_toc, 1, &taken, &mut serial);
    if body.is_empty() {
        return;
    }

    let mut edits = Edits::new();
    match navmap.kind {
        NodeKind::Empty => edits.replace(navmap.span.clone(), format!("<navMap>\n{body}  </navMap>")),
        _ => {
            if let Some(close) = navmap.close {
                edits.replace(navmap.span.end..nodes[close].span.start, format!("\n{body}  "));
            }
        }
    }
    book.set_text(&ncx_name, edits.apply(&text));
    outcome.push_change(format!(
        "wrote the {} spine entr(ies) into the NCX navMap",
        ncx_toc.len()
    ));
}

/// `<navPoint>` elements for `items`, ids unique against `taken`, `playOrder`
/// sequential from `serial`.
fn render_navpoints(
    items: &[NavItem],
    depth: usize,
    taken: &HashSet<&str>,
    serial: &mut u32,
) -> String {
    let pad = "  ".repeat(depth + 1);
    let mut out = String::new();
    for item in items {
        *serial += 1;
        let mut id = format!("np{serial}");
        while taken.contains(id.as_str()) {
            *serial += 1;
            id = format!("np{serial}");
        }
        let label = label_text(&item.label);
        let _ = writeln!(out, "{pad}<navPoint id=\"{id}\" playOrder=\"{serial}\">");
        let _ = writeln!(out, "{pad}  <navLabel><text>{label}</text></navLabel>");
        let _ = writeln!(out, "{pad}  <content src=\"{}\"/>", item.href);
        if !item.children.is_empty() {
            out.push_str(&render_navpoints(&item.children, depth + 1, taken, serial));
        }
        let _ = writeln!(out, "{pad}</navPoint>");
    }
    out
}

// ---------------------------------------------------------------------------
// Package document
// ---------------------------------------------------------------------------

/// The OPF namespace, whatever a package document chooses to call it.
const OPF_NS: &str = "http://www.idpf.org/2007/opf";

/// Every prefix bound to [`OPF_NS`] anywhere in this package document.
///
/// Matching the literal string `opf:` was wrong, and quietly so. A prefix is
/// just a local name for a namespace, and Calibre binds one *per element*:
///
/// ```xml
/// <dc:creator xmlns:ns0="http://www.idpf.org/2007/opf" ns0:role="aut">
/// <dc:contributor xmlns:ns1="http://www.idpf.org/2007/opf" ns1:role="bkp">
/// <dc:identifier xmlns:ns2="http://www.idpf.org/2007/opf" ns2:scheme="calibre">
/// ```
///
/// Three different prefixes for one namespace in one file, none of them `opf`.
/// The converter saw no legacy attributes, reported none, and left all three
/// behind for epubcheck to reject.
///
/// `opf` itself is always included: books write it without declaring it, and the
/// package element usually binds it anyway.
fn opf_prefixes(nodes: &[Node]) -> Vec<String> {
    let mut out = vec!["opf".to_string()];
    for attr in nodes.iter().flat_map(|n| &n.attrs) {
        if attr.value.trim() == OPF_NS
            && let Some(prefix) = attr.name.strip_prefix("xmlns:")
            && !out.iter().any(|p| p == prefix)
        {
            out.push(prefix.to_string());
        }
    }
    out
}

/// An OPF-namespace attribute on a `dc:` element, and the EPUB 3 property
/// replacing it.
fn refines_property(attr: &str, prefixes: &[String]) -> Option<&'static str> {
    let (prefix, local) = attr.split_once(':')?;
    if !prefixes.iter().any(|p| p == prefix) {
        return None;
    }
    match local {
        "role" => Some("role"),
        "file-as" => Some("file-as"),
        "scheme" => Some("identifier-type"),
        _ => None,
    }
}

/// Element/attribute pairs that pull a resource *into* the document.
///
/// This is the whole of the `remote-resources` question, and the distinction it
/// draws is the one an earlier version missed. That version asked only whether
/// an `href` or `src` held an absolute URL, which made every ordinary
/// `<a href="https://...">` in the book look like a remote resource. On one
/// clean book — a radio series with a link to its programme page in each
/// chapter — that was three spurious property declarations on a package that
/// needed none.
///
/// Measured against EPUB Check 5.2.1, one construct per book, each pointing at
/// an off-container URL from a document declaring nothing:
///
/// | needs the property | silent |
/// |---|---|
/// | `img@src`, `audio@src`, `video@src`, `source@src`, `track@src` | `a@href` |
/// | `iframe@src`, `embed@src`, `object@data`, `script@src` | `area@href` |
/// | `image@href` / `image@xlink:href` (SVG), `input@src` | `link@href` |
///
/// A hyperlink is a place the reader may choose to go; it is not something the
/// document loads. `<link>` to a remote stylesheet is its own error — remote
/// stylesheets are not permitted at all — and declaring a property does not
/// make it one, so it stays out too.
fn embeds_remotely(element: &str, attr: &str) -> bool {
    let attr = attr.rsplit(':').next().unwrap_or(attr);
    match element {
        "img" | "audio" | "video" | "source" | "track" | "iframe" | "embed" | "input"
        | "script" => attr == "src",
        "object" => attr == "data",
        // SVG, where the same job is done by href or the XLink spelling of it.
        "image" | "use" => attr == "href",
        _ => false,
    }
}

fn is_remote(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://") || value.starts_with("//")
}

/// True for a `<script>` the reading system will execute.
///
/// A `<script type="application/ld+json">` is a data block, not code, and
/// epubcheck agrees: it asks for `scripted` on an executable script and says
/// nothing about a JSON one.
fn is_executable_script(node: &Node) -> bool {
    match node.attr("type") {
        None => true,
        Some(t) => {
            let t = t.value.trim().to_ascii_lowercase();
            let t = t.split(';').next().unwrap_or(&t).trim().to_string();
            t.is_empty()
                || t == "module"
                || t.ends_with("/javascript")
                || t.ends_with("/ecmascript")
        }
    }
}

/// What extra manifest properties a content document needs.
///
/// Everything here is measured rather than reasoned about. `svg` and `mathml`
/// want the element *inline* — an `<img src="pic.svg">` needs neither, since
/// the SVG is a separate resource with its own manifest entry. `scripted`
/// covers HTML forms as well as scripts, which is easy to miss from the name.
fn document_properties(text: &str) -> Vec<&'static str> {
    fn want(p: &'static str, props: &mut Vec<&'static str>) {
        if !props.contains(&p) {
            props.push(p);
        }
    }

    let Ok(nodes) = scan(text) else {
        return Vec::new();
    };
    let mut props: Vec<&'static str> = Vec::new();

    for n in &nodes {
        if n.kind == NodeKind::End {
            continue;
        }
        match n.name.as_str() {
            "svg" => want("svg", &mut props),
            "math" => want("mathml", &mut props),
            // Measured: a bare <input> outside a form draws nothing, but a
            // <form> does, script or no script.
            "form" => want("scripted", &mut props),
            "script" if is_executable_script(n) => want("scripted", &mut props),
            _ => {}
        }
        if n.attrs
            .iter()
            .any(|a| embeds_remotely(&n.name, &a.name) && is_remote(&a.value))
        {
            want("remote-resources", &mut props);
        }
    }

    props.sort_unstable();
    props
}

fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = era * 400 + yoe + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Mint an id that is not already used in the package document.
fn unique_id(stem: &str, taken: &mut HashSet<String>) -> String {
    let mut candidate = stem.to_string();
    let mut n = 2;
    while taken.contains(&candidate) {
        candidate = format!("{stem}-{n}");
        n += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

/// The local name of an attribute in the OPF namespace, if it is in it.
fn opf_local<'a>(attr: &'a str, prefixes: &[String]) -> Option<&'a str> {
    let (prefix, local) = attr.split_once(':')?;
    prefixes.iter().any(|p| p == prefix).then_some(local)
}

/// The `opf:event` on a `<dc:date>`, whatever prefix the book binds it under.
fn date_event<'a>(node: &'a Node, prefixes: &[String]) -> Option<&'a crate::markup::Attr> {
    node.attrs
        .iter()
        .find(|a| opf_local(&a.name, prefixes) == Some("event"))
}

/// EPUB 2's `<dc:date opf:event="...">` down to EPUB 3's single `<dc:date>`.
///
/// Two rules, and a book can break both at once. EPUB 3 has no `opf:event` —
/// the events it distinguished are `<meta property="dcterms:*">` now — and it
/// permits **at most one** `dc:date`, the publication date. A Penguin *Complete
/// Poems of Keats* has two, one of them `opf:event="converted"`, which is 2
/// errors from one habit; *Essays and Aphorisms* and a *Slaughterhouse-Five*
/// have the attribute without the duplicate.
///
/// Measured on Keats: dropping the attribute alone leaves 1 error and dropping
/// the extra date alone leaves 0 — but only because the second date happened to
/// be the one carrying the attribute. Both rules are applied, so it does not
/// matter which way round a book writes them.
///
/// The survivor is chosen rather than assumed: an `opf:event="publication"` says
/// outright which date is the publication date, and only failing that does the
/// first one win. `dcterms:modified` is added separately by the migration and
/// carries what a `modification` event used to say.
fn modernise_dates(text: &str, nodes: &[Node], prefixes: &[String], edits: &mut Edits) -> u32 {
    let dates: Vec<&Node> = nodes
        .iter()
        .filter(|n| {
            n.name == "date"
                && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
                && text[n.span.clone()].contains("<dc:")
        })
        .collect();
    if dates.is_empty() {
        return 0;
    }

    let keep = dates
        .iter()
        .position(|n| {
            date_event(n, prefixes).is_some_and(|a| a.value.eq_ignore_ascii_case("publication"))
        })
        .or_else(|| dates.iter().position(|n| date_event(n, prefixes).is_none()))
        .unwrap_or(0);

    let mut fixed = 0;
    for (i, node) in dates.iter().enumerate() {
        if i != keep {
            edits.delete(crate::markup::line_span(text, node.element_span(nodes)));
            fixed += 1;
            continue;
        }
        if let Some(attr) = date_event(node, prefixes) {
            edits.delete(attr.span_with_space.clone());
            fixed += 1;
        }
    }
    fixed
}

/// Turn `opf:role` / `opf:file-as` / `opf:scheme` into `<meta refines>` lines,
/// minting an id on the `dc:` element when it has none.
fn modernise_dc_metadata(
    text: &str,
    nodes: &[Node],
    taken: &mut HashSet<String>,
    edits: &mut Edits,
    dates_fixed: &mut u32,
) -> Vec<String> {
    let mut refines = Vec::new();
    let prefixes = opf_prefixes(nodes);
    *dates_fixed += modernise_dates(text, nodes, &prefixes, edits);
    for node in nodes.iter().filter(|n| n.kind != NodeKind::End) {
        if !text[node.span.clone()].contains("<dc:") {
            continue;
        }
        let legacy: Vec<_> = node
            .attrs
            .iter()
            .filter(|a| refines_property(&a.name, &prefixes).is_some())
            .collect();
        if legacy.is_empty() {
            continue;
        }

        let id = if let Some(a) = node.attr("id") {
            a.value.clone()
        } else {
            let id = unique_id(&node.name.replace(':', "-"), taken);
            edits.insert(node.name_end, format!(" id=\"{id}\""));
            id
        };
        for attr in legacy {
            let property = refines_property(&attr.name, &prefixes).expect("filtered above");
            edits.delete(attr.span_with_space.clone());
            refines.push(format!(
                "    <meta refines=\"#{id}\" property=\"{property}\">{}</meta>",
                attr.value
            ));
        }
    }
    refines
}

/// The manifest properties this module derives from document content.
///
/// Anything outside this list — `nav`, `cover-image` — says something about the
/// item's *role* that no content scan could work out, so it is carried through
/// untouched.
const DERIVED: &[&str] = &["mathml", "remote-resources", "scripted", "svg"];

/// Which media types each derived property is defined for.
///
/// Not a refinement — a correctness rule, measured. Every manifest item used to
/// be scanned and given whatever its bytes earned, and an `.svg` file is
/// readable text, so an SVG image was handed `properties="svg"`:
///
/// ```text
/// properties on an image/svg+xml item     epubcheck
/// (none)                                  OPF-014: remote-resources should be declared
/// remote-resources                        clean
/// svg                                     OPF-012 + OPF-015
/// ```
///
/// So the split is per *property*, not per item: `svg`, `mathml` and `scripted`
/// describe an XHTML content document and are undefined anywhere else, while
/// `remote-resources` describes any item that can reference something remote —
/// an SVG that pulls in a remote image genuinely needs it.
fn property_applies(property: &str, media_type: &str) -> bool {
    match property {
        "remote-resources" => matches!(
            media_type,
            "application/xhtml+xml" | "image/svg+xml" | "text/css"
        ),
        _ => media_type == "application/xhtml+xml",
    }
}

/// Make every manifest item's derived properties match what its document now
/// contains.
///
/// This is a sync rather than an accumulate, and it runs last, and both of
/// those are the same lesson learned the same way. An earlier version computed
/// properties during migration — before the fixers ran — and only ever added.
/// On a book with `<script src="js/kobo.js">` pointing at a file that is not in
/// the archive, it declared `scripted`, and then `dangling-resources` deleted
/// the script that had been the only reason for it. The book went from two
/// errors to one, and that one was OPF-015, `the property "scripted" should not
/// be declared` — introduced by the tool, in the same run that removed its
/// cause.
///
/// So: **derived declarations are computed from final state, never from initial
/// state**, and a property that is no longer earned is removed as readily as a
/// missing one is added.
pub fn finalise_properties(book: &mut Book) -> Outcome {
    // Properties are an EPUB 3 concept; an EPUB 2 manifest has no such attribute.
    if book.epub_version() < 3 {
        return Outcome::none();
    }
    let Some(opf_name) = book.opf_name().map(str::to_owned) else {
        return Outcome::none();
    };
    let Some(opf_src) = book.text(&opf_name).map(str::to_owned) else {
        return Outcome::none();
    };
    let Ok(nodes) = scan(&opf_src) else {
        return Outcome::none();
    };

    let mut edits = Edits::new();
    let (mut added, mut removed) = (0u32, 0u32);

    for node in nodes.iter().filter(|n| n.name == "item") {
        let Some(href) = node.attr("href") else {
            continue;
        };
        let Some((target, _)) = resolve_href(&opf_name, &href.value) else {
            continue;
        };
        let Some(doc) = book.text(&target) else {
            continue;
        };

        let media_type = node.attr("media-type").map_or("", |a| a.value.as_str());
        let existing = node.attr("properties");
        let was: Vec<&str> = existing.map_or(Vec::new(), |a| a.value.split_whitespace().collect());
        let earned: Vec<&'static str> = document_properties(doc)
            .into_iter()
            .filter(|p| property_applies(p, media_type))
            .collect();

        // Keep everything this module does not own, then add what the finished
        // document actually earns.
        let mut now: Vec<String> = was
            .iter()
            .filter(|p| !DERIVED.contains(p))
            .map(|p| (*p).to_string())
            .collect();
        now.extend(earned.iter().map(|p| (*p).to_string()));

        let gained = earned.iter().filter(|p| !was.contains(p)).count();
        let lost = was
            .iter()
            .filter(|p| DERIVED.contains(p) && !earned.contains(p))
            .count();
        if gained == 0 && lost == 0 {
            continue;
        }
        added += u32::try_from(gained).unwrap_or(u32::MAX);
        removed += u32::try_from(lost).unwrap_or(u32::MAX);

        match (existing, now.is_empty()) {
            (Some(a), true) => edits.delete(a.span_with_space.clone()),
            (Some(a), false) => {
                edits.replace(a.span.clone(), format!("properties=\"{}\"", now.join(" ")));
            }
            (None, false) => {
                edits.insert(node.name_end, format!(" properties=\"{}\"", now.join(" ")));
            }
            (None, true) => {}
        }
    }

    if edits.is_empty() {
        return Outcome::none();
    }
    book.set_text(&opf_name, edits.apply(&opf_src));

    let mut outcome = Outcome::none();
    if added > 0 {
        outcome.push_change(format!("declared {added} manifest propert(ies)"));
    }
    if removed > 0 {
        outcome.push_change(format!(
            "withdrew {removed} manifest propert(ies) nothing in the book still needs"
        ));
    }
    outcome
}

fn rewrite_package(
    book: &mut Book,
    opf_name: &str,
    nav_doc: Option<&str>,
    bump_version: bool,
    outcome: &mut Outcome,
) {
    let Some(text) = book.text(opf_name).map(str::to_owned) else {
        return;
    };
    let nodes = match scan(&text) {
        Ok(n) => n,
        Err(e) => {
            outcome.push_finding(format!("could not parse the package document ({e})"));
            return;
        }
    };

    let mut edits = Edits::new();
    let mut taken: HashSet<String> = nodes
        .iter()
        .filter_map(|n| n.attr("id"))
        .map(|a| a.value.clone())
        .collect();

    if bump_version
        && let Some(c) = PKG_VERSION_RE.captures(&text)
        && &c[2] != "3.0"
    {
        edits.replace(c.get(2).expect("group 2").range(), "3.0");
        outcome.push_change(format!("package version {} -> 3.0", &c[2]));
    }

    let mut dates_fixed = 0u32;
    let refines = modernise_dc_metadata(&text, &nodes, &mut taken, &mut edits, &mut dates_fixed);
    if !refines.is_empty() {
        outcome.push_change(format!(
            "converted {} legacy opf: attribute(s) to <meta refines>",
            refines.len()
        ));
    }
    if dates_fixed > 0 {
        outcome.push_change(format!(
            "brought {dates_fixed} <dc:date> element(s) up to EPUB 3, which drops opf:event and \
             permits one date"
        ));
    }

    // New metadata goes just inside </metadata>.
    if let Some(close) = nodes
        .iter()
        .position(|n| n.name == "metadata" && n.kind == NodeKind::Start)
        .and_then(|i| nodes[i].close)
    {
        let mut additions = refines;
        // Only mint a timestamp when there is none. Regenerating it every run
        // would break the guarantee that a clean book is never rewritten.
        if !text.contains("dcterms:modified") {
            additions.push(format!(
                "    <meta property=\"dcterms:modified\">{}</meta>",
                utc_now()
            ));
            outcome.push_change("added the required dcterms:modified timestamp".to_string());
        }
        if !additions.is_empty() {
            edits.insert(
                nodes[close].span.start,
                format!("{}\n  ", additions.join("\n")),
            );
        }
    }

    if let Some(nav) = nav_doc
        && let Some(manifest) = nodes
            .iter()
            .position(|n| n.name == "manifest" && n.kind == NodeKind::Start)
    {
        let href = nav.strip_prefix(dirname(opf_name)).unwrap_or(nav);
        let id = unique_id("nav", &mut taken);
        edits.insert(
            nodes[manifest].span.end,
            format!(
                "\n    <item id=\"{id}\" href=\"{href}\" \
                 media-type=\"application/xhtml+xml\" properties=\"nav\"/>"
            ),
        );
    }

    if !edits.is_empty() {
        book.set_text(opf_name, edits.apply(&text));
    }
}

// ---------------------------------------------------------------------------
// Downgrade
// ---------------------------------------------------------------------------

/// Retag a book as EPUB 2, for content that never needed EPUB 3.
///
/// The content documents are already correct for EPUB 2 — that is what
/// [`crate::version::assess`] established before this runs — so only the package
/// document changes: the version, the EPUB 3-only constructs that would now be
/// errors, and the `toc` attribute EPUB 2 needs to find the NCX.
pub fn downgrade(book: &mut Book) -> Outcome {
    let mut outcome = Outcome::none();
    let Some(opf_name) = book.opf_name().map(str::to_owned) else {
        return outcome;
    };
    let Some(text) = book.text(&opf_name).map(str::to_owned) else {
        return outcome;
    };
    let nodes = match scan(&text) {
        Ok(n) => n,
        Err(e) => {
            outcome.push_finding(format!("could not parse the package document ({e})"));
            return outcome;
        }
    };

    let mut edits = Edits::new();

    if let Some(c) = PKG_VERSION_RE.captures(&text)
        && &c[2] != "2.0"
    {
        edits.replace(c.get(2).expect("group 2").range(), "2.0");
        outcome.push_change(format!("package version {} -> 2.0", &c[2]));
    }

    // <meta property="..."> is EPUB 3 syntax; EPUB 2 wants name/content.
    let mut dropped_meta = 0u32;
    for (i, node) in nodes.iter().enumerate() {
        if node.name != "meta" || node.kind == NodeKind::End || node.attr("property").is_none() {
            continue;
        }
        edits.delete(nodes[i].element_span(&nodes));
        dropped_meta += 1;
    }
    if dropped_meta > 0 {
        outcome.push_change(format!(
            "removed {dropped_meta} EPUB 3-only <meta property> element(s)"
        ));
    }

    // manifest/@properties does not exist in EPUB 2.
    let mut dropped_props = 0u32;
    for node in nodes.iter().filter(|n| n.name == "item") {
        if let Some(a) = node.attr("properties") {
            edits.delete(a.span_with_space.clone());
            dropped_props += 1;
        }
    }
    if dropped_props > 0 {
        outcome.push_change(format!(
            "removed {dropped_props} manifest properties attribute(s)"
        ));
    }

    // EPUB 2 finds the NCX through spine/@toc.
    // Absent or empty: `toc=""` names no manifest item, so it is the same
    // defect and takes the same repair.
    if let Some(spine) = nodes
        .iter()
        .find(|n| n.name == "spine" && n.kind != NodeKind::End)
        && spine
            .attr("toc")
            .is_none_or(|a| a.value.trim().is_empty())
        && let Some(ncx_id) = nodes
            .iter()
            .filter(|n| n.name == "item")
            .find(|n| {
                n.attr("media-type")
                    .is_some_and(|m| m.value == "application/x-dtbncx+xml")
            })
            .and_then(|n| n.attr("id"))
    {
        match spine.attr("toc") {
            Some(a) => edits.replace(a.span.clone(), format!("toc=\"{}\"", ncx_id.value)),
            None => edits.insert(spine.name_end, format!(" toc=\"{}\"", ncx_id.value)),
        }
        outcome.push_change("pointed spine/@toc at the NCX".to_string());
    }

    if !edits.is_empty() {
        book.set_text(&opf_name, edits.apply(&text));
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One row per construct measured against EPUB Check 5.2.1, each in its own
    /// book with a manifest item declaring nothing.
    #[test]
    fn manifest_properties_match_what_epubcheck_asks_for() {
        let doc = |body: &str| {
            format!(
                "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\
                 <body>{body}</body></html>"
            )
        };
        let props = |body: &str| document_properties(&doc(body));
        let r = "https://example.com/r";

        // remote-resources is about embedding, not linking. The first row is
        // the regression: a book with a link in every chapter had three
        // properties proposed for it and needed none.
        assert_eq!(
            props(&format!(r#"<p><a href="{r}.html">l</a></p>"#)),
            [""; 0]
        );
        assert_eq!(
            props(&format!(
                r#"<p><map name="m"><area shape="rect" coords="0,0,1,1" href="{r}.html"/></map></p>"#
            )),
            [""; 0]
        );
        for embed in [
            format!(r#"<img src="{r}.jpg" alt="x"/>"#),
            format!(r#"<audio src="{r}.mp3"/>"#),
            format!(r#"<video><source src="{r}.mp4"/></video>"#),
            format!(r#"<iframe src="{r}.html"/>"#),
            format!(r#"<embed src="{r}.swf"/>"#),
            format!(r#"<object data="{r}.swf"/>"#),
        ] {
            assert_eq!(
                props(&embed),
                ["remote-resources"],
                "{embed} loads a remote resource"
            );
        }
        // Relative references are not remote, however they are spelled.
        assert_eq!(
            props(r#"<p><img src="../Images/x.jpg" alt="x"/></p>"#),
            [""; 0]
        );

        // scripted covers forms as well as scripts, but not a JSON data block.
        assert_eq!(props("<script>var x=1;</script>"), ["scripted"]);
        assert_eq!(
            props(r#"<script type="text/javascript">x()</script>"#),
            ["scripted"]
        );
        assert_eq!(
            props(r#"<form action="x"><input name="q"/></form>"#),
            ["scripted"]
        );
        assert_eq!(
            props(r#"<script type="application/ld+json">{}</script>"#),
            [""; 0]
        );
        assert_eq!(props(r#"<p><input type="text" name="q"/></p>"#), [""; 0]);

        // svg and mathml want the element inline; a referenced .svg file has
        // its own manifest entry and needs nothing here.
        assert_eq!(
            props(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#),
            ["svg"]
        );
        assert_eq!(props(r#"<p><img src="pic.svg" alt="x"/></p>"#), [""; 0]);
        assert_eq!(
            props(r#"<math xmlns="http://www.w3.org/1998/Math/MathML"><mi>x</mi></math>"#),
            ["mathml"]
        );

        // Combinations come back sorted and deduplicated.
        assert_eq!(
            props(&format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><image xlink:href="{r}.png"/></svg>
                   <svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#
            )),
            ["remote-resources", "svg"]
        );
    }

    #[test]
    fn utc_now_looks_like_a_timestamp() {
        let s = utc_now();
        assert_eq!(s.len(), 20, "{s}");
        assert!(s.ends_with('Z'), "{s}");
        // Sanity: this code will not be running before 2020 or after 2200.
        let year: i32 = s[..4].parse().unwrap();
        assert!((2020..2200).contains(&year), "{s}");
    }

    #[test]
    fn rebase_keeps_same_directory_hrefs_intact() {
        assert_eq!(
            rebase("OEBPS/toc.ncx", "OEBPS/nav.xhtml", "ch1.xhtml#frost"),
            "ch1.xhtml#frost"
        );
        assert_eq!(
            rebase("OEBPS/nav/toc.ncx", "OEBPS/nav.xhtml", "../Text/ch1.xhtml"),
            "Text/ch1.xhtml"
        );
    }

    /// `resolve_href` decodes on the way in, so anything written back into an
    /// `href` has to be encoded again. A book that spelled its own reference
    /// correctly used to come out of the migration with a raw space in it.
    #[test]
    fn a_percent_encoded_href_survives_the_round_trip() {
        assert_eq!(
            rebase(
                "OEBPS/toc.ncx",
                "OEBPS/nav.xhtml",
                "Text/my%20ch.xhtml#a%20b"
            ),
            "Text/my%20ch.xhtml#a%20b"
        );
        // An ampersand cannot be left bare: the href is written into an XML
        // attribute, so encoding it serves the URI and the markup at once.
        assert_eq!(
            rebase(
                "OEBPS/toc.ncx",
                "OEBPS/nav.xhtml",
                "Text/Tom%20%26%20Jerry.xhtml"
            ),
            "Text/Tom%20%26%20Jerry.xhtml"
        );
        // Climbing out of a directory must not encode the separators.
        assert_eq!(
            rebase(
                "OEBPS/nav/toc.ncx",
                "OEBPS/nav.xhtml",
                "../Text/a%20b.xhtml"
            ),
            "Text/a%20b.xhtml"
        );
    }

    #[test]
    fn properties_are_detected_from_document_content() {
        assert_eq!(document_properties("<p>plain</p>"), Vec::<&str>::new());
        assert_eq!(
            document_properties("<p><svg xmlns=\"x\"><rect/></svg></p>"),
            vec!["svg"]
        );
        assert_eq!(
            document_properties("<script src=\"a.js\"></script>"),
            vec!["scripted"]
        );
        assert_eq!(
            document_properties("<img src=\"https://x.test/a.png\"/>"),
            vec!["remote-resources"]
        );
    }
}
