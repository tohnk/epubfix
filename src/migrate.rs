//! Opt-in EPUB 2 → EPUB 3 migration.
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

/// Rewrite an href written relative to `from_doc` so it works from `to_doc`.
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
    match fragment {
        Some(f) => format!("{rel}#{f}"),
        None => rel,
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

/// Build a nav document from the NCX. Returns its archive name.
fn build_nav(book: &mut Book, opf_name: &str, outcome: &mut Outcome) -> Option<String> {
    // A book that already declares a nav needs nothing here.
    if book
        .opf_text()
        .is_some_and(|t| t.contains("properties=\"nav\"") || t.contains("properties='nav'"))
    {
        return None;
    }

    let ncx_name = book.ncx_name().map(str::to_owned)?;
    let ncx_src = book.ncx_text().map(str::to_owned)?;
    let nodes = match scan(&ncx_src) {
        Ok(n) => n,
        Err(e) => {
            outcome.push_finding(format!("could not parse the NCX, no nav generated ({e})"));
            return None;
        }
    };

    let dir = dirname(opf_name);
    let mut nav_doc = format!("{dir}nav.xhtml");
    let mut n = 2;
    while book.name_taken(&nav_doc) {
        nav_doc = format!("{dir}nav{n}.xhtml");
        n += 1;
    }

    let find_root = |name: &str| {
        nodes
            .iter()
            .position(|x| x.name == name && x.kind == NodeKind::Start)
    };

    let toc = find_root("navmap")
        .map(|r| collect_nav(&ncx_src, &nodes, r, "navpoint", &ncx_name, &nav_doc))
        .unwrap_or_default();
    if toc.is_empty() {
        outcome.push_finding("the NCX has no navMap, so no nav document was generated".to_string());
        return None;
    }
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

    let mut body = format!(
        "  <nav epub:type=\"toc\" id=\"toc\">\n    <h1>Contents</h1>\n{}  </nav>\n",
        render_list(&toc, 0)
    );
    if !pages.is_empty() {
        let _ = write!(
            body,
            "  <nav epub:type=\"page-list\" id=\"page-list\" hidden=\"hidden\">\n\
             \x20   <h1>Pages</h1>\n{}  </nav>\n",
            render_list(&pages, 0)
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

    let doc = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!DOCTYPE html>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" \
         xmlns:epub=\"http://www.idpf.org/2007/ops\">\n\
         <head>\n  <title>{title}</title>\n  <meta charset=\"utf-8\"/>\n</head>\n\
         <body>\n{body}</body>\n</html>\n"
    );

    book.add_text_entry(&nav_doc, doc);
    outcome.push_change(format!(
        "generated {} from the NCX ({} entries{})",
        basename(&nav_doc),
        toc.len(),
        if pages.is_empty() {
            String::new()
        } else {
            format!(", {} page targets", pages.len())
        }
    ));
    Some(nav_doc)
}

// ---------------------------------------------------------------------------
// Package document
// ---------------------------------------------------------------------------

/// `opf:` attributes on a `dc:` element, and the EPUB 3 property replacing each.
fn refines_property(attr: &str) -> Option<&'static str> {
    match attr {
        "opf:role" => Some("role"),
        "opf:file-as" => Some("file-as"),
        "opf:scheme" => Some("identifier-type"),
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

/// Turn `opf:role` / `opf:file-as` / `opf:scheme` into `<meta refines>` lines,
/// minting an id on the `dc:` element when it has none.
fn modernise_dc_metadata(
    text: &str,
    nodes: &[Node],
    taken: &mut HashSet<String>,
    edits: &mut Edits,
) -> Vec<String> {
    let mut refines = Vec::new();
    for node in nodes.iter().filter(|n| n.kind != NodeKind::End) {
        if !text[node.span.clone()].contains("<dc:") {
            continue;
        }
        let legacy: Vec<_> = node
            .attrs
            .iter()
            .filter(|a| refines_property(&a.name).is_some())
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
            let property = refines_property(&attr.name).expect("filtered above");
            edits.delete(attr.span_with_space.clone());
            refines.push(format!(
                "    <meta refines=\"#{id}\" property=\"{property}\">{}</meta>",
                attr.value
            ));
        }
    }
    refines
}

/// Declare `svg` / `scripted` / `mathml` / `remote-resources` on manifest items
/// whose documents need them.
fn declare_properties(book: &Book, opf_name: &str, nodes: &[Node], edits: &mut Edits) -> u32 {
    let mut count = 0;
    for node in nodes.iter().filter(|n| n.name == "item") {
        let Some(href) = node.attr("href") else {
            continue;
        };
        let Some((target, _)) = resolve_href(opf_name, &href.value) else {
            continue;
        };
        let Some(doc) = book.text(&target) else {
            continue;
        };
        let props = document_properties(doc);
        if props.is_empty() {
            continue;
        }
        let existing = node.attr("properties");
        let mut all: Vec<String> = existing
            .map(|a| a.value.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        for p in props {
            if !all.iter().any(|x| x == p) {
                all.push(p.to_string());
            }
        }
        let value = format!("properties=\"{}\"", all.join(" "));
        match existing {
            // Already declared everything it needs.
            Some(a) if a.value.split_whitespace().count() == all.len() => continue,
            Some(a) => edits.replace(a.span.clone(), value),
            None => edits.insert(node.name_end, format!(" {value}")),
        }
        count += 1;
    }
    count
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

    let refines = modernise_dc_metadata(&text, &nodes, &mut taken, &mut edits);
    if !refines.is_empty() {
        outcome.push_change(format!(
            "converted {} legacy opf: attribute(s) to <meta refines>",
            refines.len()
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

    let propped = declare_properties(book, opf_name, &nodes, &mut edits);
    if propped > 0 {
        outcome.push_change(format!("declared manifest properties on {propped} item(s)"));
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
    if let Some(spine) = nodes
        .iter()
        .find(|n| n.name == "spine" && n.kind != NodeKind::End)
        && spine.attr("toc").is_none()
        && let Some(ncx_id) = nodes
            .iter()
            .filter(|n| n.name == "item")
            .find(|n| {
                n.attr("media-type")
                    .is_some_and(|m| m.value == "application/x-dtbncx+xml")
            })
            .and_then(|n| n.attr("id"))
    {
        edits.insert(spine.name_end, format!(" toc=\"{}\"", ncx_id.value));
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
