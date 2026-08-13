//! RSC-005: elements nested somewhere their content model forbids.
//!
//! Both repairs here remove markup and keep text, which is the opposite of the
//! usual instinct but the only safe direction: the words on the page are the
//! book, and a `<blockquote>` or an `<a>` is a claim about them that the file
//! is already failing to make legally.
//!
//! Measured against EPUB Check 5.2.1, in both rulesets, since neither repair
//! would be worth making if only one version cared:
//!
//! | | EPUB 2 | EPUB 3 |
//! | --- | --- | --- |
//! | `<blockquote>` inside `<p>`, `<span>` or `<a>` | RSC-005 | RSC-005 |
//! | `<blockquote>` as a child of `<body>` | clean | clean |
//! | `<a>` inside `<a>`, directly | RSC-005 | RSC-005 |
//! | `<a>` inside `<a>` via a `<span>` | RSC-005 | RSC-005 |
//!
//! The last row is the one worth noticing: the rule is about *ancestry*, not
//! parentage, so a scan that only looked at the immediate parent would miss it.

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, Node, NodeKind, id_attrs, scan};
use crate::util::basename;

/// Elements that may only hold phrasing content, so a `<blockquote>` inside one
/// is an error under both rulesets.
///
/// Deliberately a short list of the ones that actually turn up, rather than an
/// attempt at the whole HTML content model. A wrong entry here would unwrap
/// legal markup, and the cost of a missing entry is only a missed repair.
const PHRASING_ONLY: &[&str] = &[
    "p", "span", "a", "em", "strong", "i", "b", "u", "s", "small", "big", "cite", "code", "dfn",
    "kbd", "samp", "var", "abbr", "acronym", "sub", "sup", "tt", "strike", "font", "label", "h1",
    "h2", "h3", "h4", "h5", "h6", "dt", "pre", "caption", "legend", "title",
];

/// The nearest ancestor of `i` whose name is in `names`, if any.
fn ancestor<'a>(nodes: &'a [Node], i: usize, names: &[&str]) -> Option<(usize, &'a Node)> {
    let mut cur = nodes[i].parent;
    while let Some(p) = cur {
        if names.contains(&nodes[p].name.as_str()) {
            return Some((p, &nodes[p]));
        }
        cur = nodes[p].parent;
    }
    None
}

/// Replace an element's start and end tags with `open`/`close`, keeping
/// everything between them exactly as it is.
///
/// This is the whole of "unwrap" and "retag" — the inner bytes are never
/// touched, so nested markup, entities and whitespace all survive by
/// construction.
fn rewrap(edits: &mut Edits, nodes: &[Node], i: usize, open: &str, close: &str) {
    let node = &nodes[i];
    match node.close {
        Some(c) => {
            edits.replace(node.span.clone(), open.to_string());
            edits.replace(nodes[c].span.clone(), close.to_string());
        }
        // Self-closing or unclosed: one tag to replace, but the replacement
        // still has to be a whole element. Writing only `open` here emitted an
        // unclosed `<span>` for the real `<a id="page_viii"/>` inside another
        // anchor, and turned two content-model errors into two fatal ones.
        None => edits.replace(node.span.clone(), format!("{open}{close}")),
    }
}

// ---------------------------------------------------------------------------
// Nested anchors
// ---------------------------------------------------------------------------

/// RSC-005: `The "a" element cannot contain any nested "a" elements`.
///
/// The inner anchor is the one that goes, because the outer one is what the
/// reader is actually looking at: a nested anchor is unreachable, since the
/// enclosing link swallows the whole area. Its text stays put.
///
/// An id on the inner anchor may well be a link target, so it is not simply
/// discarded — the anchor becomes a `<span>` carrying the same id, which is
/// legal inside an `<a>` and keeps every fragment resolving. Without an id
/// there is nothing left worth keeping and the tags go entirely.
pub struct NestedAnchors;

impl Fixer for NestedAnchors {
    fn name(&self) -> &'static str {
        "nested-anchors"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "unwrap an <a> nested inside another <a>, keeping its text and any id"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let (mut unwrapped, mut kept) = (0u32, 0u32);

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for i in 0..nodes.len() {
                if nodes[i].kind == NodeKind::End || nodes[i].name != "a" {
                    continue;
                }
                if ancestor(&nodes, i, &["a"]).is_none() {
                    continue;
                }

                // An id here may be a live link target, so it needs somewhere
                // to live. A <span> is legal inside an <a> and holds it.
                let ids: Vec<&str> = id_attrs(&nodes[i]).map(|a| a.value.as_str()).collect();
                if ids.is_empty() {
                    rewrap(&mut edits, &nodes, i, "", "");
                    unwrapped += 1;
                } else {
                    // One id is enough: a second would be a duplicate.
                    rewrap(
                        &mut edits,
                        &nodes,
                        i,
                        &format!("<span id=\"{}\">", ids[0]),
                        "</span>",
                    );
                    kept += 1;
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if unwrapped > 0 {
            outcome.push_change(format!("unwrapped {unwrapped} nested anchor(s)"));
        }
        if kept > 0 {
            outcome.push_change(format!(
                "turned {kept} nested anchor(s) into <span>s to keep their ids reachable"
            ));
        }
        outcome
    }
}

// ---------------------------------------------------------------------------
// Misplaced blockquotes
// ---------------------------------------------------------------------------

/// RSC-005: `element "blockquote" not allowed here`.
///
/// Two shapes, two repairs, because one repair does not cover both.
///
/// **A `<blockquote>` directly inside a `<p>`** is a paragraph that was never
/// closed. Splitting it — `</p>` before the quotation, a fresh `<p>` after —
/// is not a guess about what the author meant: it is what an HTML parser
/// already does, because `<blockquote>` is on the list of elements that
/// implicitly close an open `<p>`. Every browser has been rendering the file
/// that way all along, so the split changes the XML to agree with the page
/// rather than the other way round. The reopened paragraph is written with the
/// original's attributes, and a paragraph half that would come out empty is
/// dropped instead of being emitted blank.
///
/// **A `<blockquote>` inside anything else that only takes phrasing content**
/// — a `<span>`, a `<cite>` — gets no such help, since none of those
/// auto-close. There the quotation keeps its boundaries and loses only its
/// *block* status: it becomes `<span class="blockquote">`, which is phrasing
/// content and so legal exactly where the original was not.
///
/// That second repair has a limit. A `<blockquote>` holding block children
/// cannot become a `<span>`, because the `<p>` inside it would then be the
/// thing in the wrong place. Those are reported.
pub struct MisplacedBlockquotes;

/// Elements that cannot appear inside phrasing content, so a `<blockquote>`
/// holding one of them cannot be demoted to a `<span>`.
const BLOCK_CHILD: &[&str] = &[
    "p",
    "div",
    "blockquote",
    "table",
    "ul",
    "ol",
    "dl",
    "pre",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "address",
    "form",
    "fieldset",
];

/// The attribute text for the replacement `<span>`: everything the
/// `<blockquote>` carried, plus a `blockquote` class so a stylesheet can still
/// reach it.
///
/// Merging into an existing `class` rather than adding a second one matters —
/// two `class` attributes on one element is not a smaller error than the one
/// being fixed. Each surviving attribute is copied as its own source bytes, so
/// quoting and entities come through untouched.
fn span_attrs(text: &str, node: &Node) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut merged = false;
    for a in &node.attrs {
        if a.name == "class" {
            parts.push(format!("class=\"blockquote {}\"", a.value));
            merged = true;
        } else {
            parts.push(text[a.span.clone()].to_string());
        }
    }
    if !merged {
        parts.insert(0, "class=\"blockquote\"".to_string());
    }
    parts.join(" ")
}

/// Cut `paragraph` open around each blockquote in `quotes`.
///
/// Each quotation ends up between two paragraphs rather than inside one. A
/// resulting half with nothing but whitespace in it is not written at all —
/// an empty `<p></p>` is valid in both rulesets but renders as a blank line,
/// which would be a visible change made for no reason.
fn split_paragraph(
    edits: &mut Edits,
    text: &str,
    nodes: &[Node],
    paragraph: usize,
    quotes: &[usize],
) {
    let Some(p_close) = nodes[paragraph].close else {
        return;
    };
    let open = &text[nodes[paragraph].span.clone()];

    // The pieces of the original paragraph, one between each pair of
    // quotations. Thinking in fragments rather than in cuts is what makes the
    // empty ones easy: a fragment with no content simply gets no tags, and the
    // only bytes ever deleted are the paragraph's own two tags.
    let spans: Vec<_> = quotes
        .iter()
        .map(|&q| nodes[q].element_span(nodes))
        .collect();
    let mut bounds = vec![nodes[paragraph].span.end];
    bounds.extend(spans.iter().map(|s| s.end));
    let mut ends: Vec<usize> = spans.iter().map(|s| s.start).collect();
    ends.push(nodes[p_close].span.start);

    let last = bounds.len() - 1;
    for (i, (&from, &to)) in bounds.iter().zip(&ends).enumerate() {
        let empty = text[from..to].trim().is_empty();
        match (i, empty) {
            // The first fragment already has the original opening tag, and the
            // last already has the original closing one.
            (0, true) => edits.delete(nodes[paragraph].span.clone()),
            (0, false) => edits.insert(to, "</p>"),
            (i, true) if i == last => edits.delete(nodes[p_close].span.clone()),
            (_, true) => {}
            (i, false) if i == last => edits.insert(from, open.to_string()),
            (_, false) => {
                edits.insert(from, open.to_string());
                edits.insert(to, "</p>");
            }
        }
    }
}

impl Fixer for MisplacedBlockquotes {
    fn name(&self) -> &'static str {
        "misplaced-blockquotes"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "demote a <blockquote> stranded in phrasing content to a <span>, keeping every word"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let (mut demoted, mut divided) = (0u32, 0u32);

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            // Paragraphs to split, and where inside each. Collected first so
            // several quotations in one paragraph can be cut in one pass and
            // each knows about its neighbours.
            let mut splits: Vec<(usize, Vec<usize>)> = Vec::new();

            for i in 0..nodes.len() {
                if nodes[i].kind == NodeKind::End || nodes[i].name != "blockquote" {
                    continue;
                }
                let Some((host_i, host)) = ancestor(&nodes, i, PHRASING_ONLY) else {
                    continue;
                };

                // A paragraph is closed by a following block element anyway, so
                // making that explicit is transcription, not interpretation.
                if host.name == "p" && nodes[i].parent == Some(host_i) {
                    match splits.iter_mut().find(|(p, _)| *p == host_i) {
                        Some((_, quotes)) => quotes.push(i),
                        None => splits.push((host_i, vec![i])),
                    }
                    continue;
                }

                let blocky: Vec<&str> = nodes
                    .iter()
                    .filter(|n| n.parent == Some(i) && n.kind != NodeKind::End)
                    .map(|n| n.name.as_str())
                    .filter(|n| BLOCK_CHILD.contains(n))
                    .collect();
                if !blocky.is_empty() {
                    outcome.push_finding(format!(
                        "{}: <blockquote> inside <{}> holds block content (<{}>), and only a <p> \
                         can be split around it, so it needs a look",
                        basename(&doc),
                        host.name,
                        blocky[0]
                    ));
                    continue;
                }

                rewrap(
                    &mut edits,
                    &nodes,
                    i,
                    &format!("<span {}>", span_attrs(&text, &nodes[i])),
                    "</span>",
                );
                demoted += 1;
            }

            for (host_i, quotes) in &splits {
                split_paragraph(&mut edits, &text, &nodes, *host_i, quotes);
                divided += u32::try_from(quotes.len()).unwrap_or(u32::MAX);
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if divided > 0 {
            outcome.push_change(format!(
                "split {divided} paragraph(s) around a <blockquote> they had swallowed"
            ));
        }
        if demoted > 0 {
            outcome.push_change(format!(
                "demoted {demoted} misplaced <blockquote>(s) to <span class=\"blockquote\">"
            ));
        }
        outcome
    }
}

/// RSC-005: inline content directly inside a `<blockquote>`, when there is not
/// much of it.
///
/// XHTML 1.1 gives `<blockquote>` a block-only content model and HTML5 gives it
/// flow, so verse written as bare text and `<br/>` is an error under EPUB 2 and
/// clean under EPUB 3. Thousands of them across hundreds of files is a book
/// whose *declaration* is wrong, and the retagger moves the declaration rather
/// than rewrite a third of the book. A dozen of them is not evidence of
/// anything: it is a dozen paragraphs, and wrapping each run in a `<div>` is a
/// far smaller change than converting the book.
///
/// So this covers exactly the case the diagnostic used to hand back with "wrap
/// them in a `<div>` by hand". Above the threshold it does nothing, because by
/// then the book has already been retagged and is EPUB 3.
///
/// The `<div>` is the neutral container — no margins of its own — and it goes
/// around each *run* of inline content between the block children, so a
/// blockquote holding a `<p>` and some loose verse keeps both in order.
///
/// Measured: `<blockquote>bare verse<br/>more</blockquote>` is 4 errors, and
/// the same content inside a `<div>` is 0. Only the containers where that holds
/// are touched — see [`crate::version::WRAPPABLE`].
pub struct InlineInBlock;

impl Fixer for InlineInBlock {
    fn name(&self) -> &'static str {
        "inline-in-block"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "wrap a short run of inline content in a <div> where XHTML 1.1 wants a block"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() >= 3 {
            return Outcome::none();
        }
        // Only the handful case. A book past the threshold has been retagged
        // already, or was told not to be, and rewriting it wholesale is the
        // change this tool exists to avoid.
        if !crate::version::few_enough_to_wrap(crate::version::assess(book).inline_in_block) {
            return Outcome::none();
        }
        let mut wrapped = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for (i, node) in nodes.iter().enumerate() {
                if node.kind != NodeKind::Start
                    || !crate::version::WRAPPABLE.contains(&node.name.as_str())
                {
                    continue;
                }
                let Some(close) = node.close else { continue };

                // Everything this element holds that is *not* an inline child:
                // those are already legal here, and the gaps between them are
                // the runs that are not.
                let mut blocks: Vec<std::ops::Range<usize>> = nodes
                    .iter()
                    .filter(|c| {
                        c.parent == Some(i)
                            && c.kind != NodeKind::End
                            && !crate::version::INLINE.contains(&c.name.as_str())
                    })
                    .map(|c| c.element_span(&nodes))
                    .collect();
                blocks.sort_by_key(|r| r.start);

                let mut cursor = node.span.end;
                let end = nodes[close].span.start;
                let mut gaps: Vec<std::ops::Range<usize>> = Vec::new();
                for b in &blocks {
                    if b.start > cursor {
                        gaps.push(cursor..b.start);
                    }
                    cursor = cursor.max(b.end);
                }
                if end > cursor {
                    gaps.push(cursor..end);
                }

                for gap in gaps {
                    let raw = &text[gap.clone()];
                    if raw.trim().is_empty() {
                        continue;
                    }
                    // Wrap the content, not the whitespace around it, so the
                    // file's own layout is left as it was.
                    let lead = raw.len() - raw.trim_start().len();
                    let trail = raw.len() - raw.trim_end().len();
                    edits.insert(gap.start + lead, "<div>".to_string());
                    edits.insert(gap.end - trail, "</div>".to_string());
                    wrapped += 1;
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if wrapped == 0 {
            return Outcome::none();
        }
        Outcome::change(format!(
            "wrapped {wrapped} run(s) of inline content in a <div>, which is what XHTML 1.1 \
             wants there"
        ))
    }
}

/// RSC-005: `element "col" not allowed here; expected … "colgroup" … or "tr"`.
///
/// XHTML 1.1 lets `<col>` sit directly inside `<table>`; HTML5 does not, and
/// requires it inside a `<colgroup>`. So this is the mirror of the fixers in
/// [`crate::fixers::legacy_html`]: EPUB 3 is the stricter ruleset here, and a
/// book only meets the error on being retagged.
///
/// A Gollancz *Well of Ascension* writes
/// `<table><col/><col/><col/><tr>…`, three column definitions and no group.
/// Wrapping the run in a `<colgroup>` is the whole repair: a `<colgroup>` with
/// explicit `<col>` children has exactly the effect its children had on their
/// own, so no column width moves.
///
/// Only a run that starts immediately inside the `<table>`. A `<col>` already
/// inside a `<colgroup>` is where it belongs, and one somewhere else entirely is
/// damage this should not try to reason about.
pub struct ColumnGroups;

impl Fixer for ColumnGroups {
    fn name(&self) -> &'static str {
        "column-groups"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "put a bare <col> inside the <colgroup> HTML5 requires"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() < 3 {
            return Outcome::none();
        }
        let mut wrapped = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for (i, table) in nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.name == "table" && n.kind == NodeKind::Start)
            {
                // The run of <col> children, in document order. A gap ends it:
                // only the ones opening the table are this fixer's business.
                let cols: Vec<&Node> = nodes
                    .iter()
                    .filter(|n| n.parent == Some(i) && n.kind != NodeKind::End)
                    .take_while(|n| n.name == "col")
                    .collect();
                let (Some(first), Some(last)) = (cols.first(), cols.last()) else {
                    continue;
                };
                edits.insert(first.span.start, "<colgroup>".to_string());
                edits.insert(last.element_span(&nodes).end, "</colgroup>".to_string());
                wrapped += u32::try_from(cols.len()).unwrap_or(u32::MAX);
                let _ = table;
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if wrapped == 0 {
            return Outcome::none();
        }
        Outcome::change(format!(
            "put {wrapped} bare <col> element(s) inside the <colgroup> HTML5 requires"
        ))
    }
}
