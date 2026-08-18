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

/// The same list for EPUB 3, where HTML5 widened three content models.
///
/// `<a>` became transparent: it may hold flow content such as a `<div>`, and
/// *Valdor* has exactly that valid shape, `<a><div>advertisement</div></a>`.
/// `<caption>` and `<dt>` take flow content outright — a `<p>` in either is
/// correct HTML5, and demoting it to a `<span>` would rewrite valid markup and
/// turn a block into an inline for no validation benefit.
///
/// `<legend>` stays, because it takes phrasing content and nothing else — with
/// the single exception of headings, which the caller allows for separately.
const EPUB3_PHRASING_ONLY: &[&str] = &[
    "p", "span", "em", "strong", "i", "b", "u", "s", "small", "big", "cite", "code", "dfn", "kbd",
    "samp", "var", "abbr", "acronym", "sub", "sup", "tt", "strike", "font", "label", "h1", "h2",
    "h3", "h4", "h5", "h6", "pre", "legend", "title",
];

fn phrasing_only(version: u32) -> &'static [&'static str] {
    if version >= 3 {
        EPUB3_PHRASING_ONLY
    } else {
        PHRASING_ONLY
    }
}

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
/// reader is mostly looking at: the enclosing link covers the whole area. Its
/// text stays put.
///
/// An id on the inner anchor may well be a link target, so it is not simply
/// discarded — the anchor becomes a `<span>` carrying the same id, which is
/// legal inside an `<a>` and keeps every fragment resolving. Without an id
/// there is nothing left worth keeping and the tags go entirely.
///
/// # An inner `href` is the one thing this cannot keep
///
/// Two links cannot share one run of text, so an inner anchor with a
/// destination of its own loses it, and "it was unreachable anyway" is a
/// weaker claim than it sounds: an EPUB content document is parsed as XML, so
/// unlike the HTML tag-soup rules — which would have split the two into
/// siblings — the nesting is real, and which of the two a reading system
/// activates on a click is its own business. So the destination is named
/// rather than dropped in silence.
///
/// Measured across a 179-book library, this fires on nothing. There are three
/// nested anchors in the whole of it, all in one Kenny *Ancient Philosophy*,
/// and all three are `<a id="page_viii"/>` — self-closing page markers with no
/// `href`, which is exactly the case the `<span>` handles without loss. The
/// report exists for the book that is not in this library.
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
        // Destinations that could not survive the unwrap, named at the end
        // rather than lost quietly.
        let mut discarded: Vec<String> = Vec::new();

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
                if let Some(href) = nodes[i].attr("href") {
                    discarded.push(format!("{}: {}", basename(&doc), href.value));
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
        if !discarded.is_empty() {
            const SHOWN: usize = 3;
            let more = discarded.len().saturating_sub(SHOWN);
            discarded.truncate(SHOWN);
            outcome.push_finding(format!(
                "{} nested <a> had a destination of its own, which cannot be kept — two links \
                 cannot share one run of text, and the enclosing one is the link the page is \
                 built around. If any of these mattered they need re-placing outside the outer \
                 anchor [{}{}]",
                more + SHOWN.min(discarded.len()),
                discarded.join("; "),
                if more > 0 {
                    format!("; and {more} more")
                } else {
                    String::new()
                }
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
/// **A block element directly inside a `<p>`** is a paragraph that was never
/// closed. Splitting it — `</p>` before the block, a fresh `<p>` after —
/// is not a guess about what the author meant: it is what an HTML parser
/// already does, because block elements implicitly close an open `<p>`. Every
/// browser has been rendering the file that way all along, so the split changes
/// the XML to agree with the page rather than the other way round. The reopened
/// paragraph is written with the original's attributes, and a paragraph half
/// that would come out empty is dropped instead of being emitted blank.
///
/// **A block element inside anything else that only takes phrasing content**
/// — a `<span>`, a `<cite>` — gets no such help, since none of those
/// auto-close. There the element keeps its boundaries and loses only its
/// *block* status: it becomes `<span class="original-name">`, which is phrasing
/// content and so legal exactly where the original was not.
///
/// That second repair has a limit. A block element holding block children
/// cannot become a `<span>`, because the `<p>` inside it would then be the
/// thing in the wrong place. Those are reported.
pub struct MisplacedBlocks;

/// Elements that cannot appear inside phrasing content, so a block element
/// holding one of them cannot be demoted to a `<span>`. Also the list of
/// elements that trigger this fixer when found inside phrasing content.
///
/// `center` looks redundant, because [`super::legacy_html::CenterElements`]
/// runs first in the default pipeline and there are none left by the time this
/// pass starts. It is not: `--only misplaced-blockquotes` runs this fixer on
/// its own, and then a `<center>` stranded in a `<p>` is this pass's to repair
/// like any other block. The same goes for a run where the `<center>` pass was
/// rolled back. Removing the entry would silently drop both.
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
    "center",
];

/// The attribute text for the replacement `<span>`: everything the
/// element carried, plus its original name as a class so a stylesheet can still
/// reach it.
///
/// Merging into an existing `class` rather than adding a second one matters —
/// two `class` attributes on one element is not a smaller error than the one
/// being fixed. Each surviving attribute is copied as its own source bytes, so
/// quoting and entities come through untouched.
fn span_attrs(text: &str, node: &Node) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut merged = false;
    let name = &node.name;
    for a in &node.attrs {
        if a.name == "class" {
            parts.push(format!("class=\"{} {}\"", name, a.value));
            merged = true;
        } else {
            parts.push(text[a.span.clone()].to_string());
        }
    }
    if !merged {
        parts.insert(0, format!("class=\"{name}\""));
    }
    parts.join(" ")
}

/// Cut `paragraph` open around each block element in `blocks`.
///
/// Each block element ends up between two paragraphs rather than inside one. A
/// resulting half with nothing but whitespace in it is not written at all —
/// an empty `<p></p>` is valid in both rulesets but renders as a blank line,
/// which would be a visible change made for no reason.
fn split_paragraph(
    edits: &mut Edits,
    text: &str,
    nodes: &[Node],
    paragraph: usize,
    blocks: &[usize],
) {
    let Some(p_close) = nodes[paragraph].close else {
        return;
    };
    let open = &text[nodes[paragraph].span.clone()];
    // The continuation fragments reopen with the paragraph's own tag, and an
    // `id` on it may only be worn by one of them. Copying it onto every piece
    // turns `<p id="intro">before<blockquote/>after</p>` into two elements
    // both called `intro` — measured, one RSC-005 traded for two `Duplicate ID
    // "intro"`, and `duplicate-ids` runs earlier in the pipeline so nothing
    // comes along afterwards to clean it up. The first fragment keeps the
    // original tag, ids and all, because it is the one every existing link
    // already resolves to.
    let reopen = &{
        let mut strip = Edits::new();
        for attr in id_attrs(&nodes[paragraph]) {
            strip.delete(
                attr.span_with_space.start - nodes[paragraph].span.start
                    ..attr.span_with_space.end - nodes[paragraph].span.start,
            );
        }
        strip.apply(open)
    };

    // The pieces of the original paragraph, one between each pair of
    // quotations. Thinking in fragments rather than in cuts is what makes the
    // empty ones easy: a fragment with no content simply gets no tags, and the
    // only bytes ever deleted are the paragraph's own two tags.
    let spans: Vec<_> = blocks
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
            (i, false) if i == last => edits.insert(from, reopen.clone()),
            (_, false) => {
                edits.insert(from, reopen.clone());
                edits.insert(to, "</p>");
            }
        }
    }
}

impl Fixer for MisplacedBlocks {
    fn name(&self) -> &'static str {
        // Kept under its original name: the repair has grown from blockquotes
        // to block elements in general, but `--only misplaced-blockquotes`
        // in somebody's script must keep working.
        "misplaced-blockquotes"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "split paragraphs around block elements, or demote them to <span>s when stranded in phrasing content"
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one walk over the node list deciding split, demote and promote; the three \
                  share the same ancestor analysis"
    )]
    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let (mut demoted, mut divided, mut promoted) = (0u32, 0u32, 0u32);
        let version = book.epub_version();
        let phrasing = phrasing_only(version);
        let styles_span = crate::fixers::tables::author_styles(book).names_element("span");

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            // Paragraphs to split, and where inside each. Collected first so
            // several blocks in one paragraph can be cut in one pass and
            // each knows about its neighbours.
            let mut splits: Vec<(usize, Vec<usize>)> = Vec::new();

            // A <span> holding block children is a <div> wearing the wrong
            // name — *Alice in Wonderland* wraps its title blocks in
            // <span id="id1">. Where the context is block-level, the name is
            // the only thing wrong: the id, class and content all stay, and
            // the nesting becomes legal. Inside phrasing content a <div>
            // would be the next error, so those are left to the report below.
            //
            // Collected before the block scan, because a block whose parent
            // is about to be promoted is repaired by the promotion and must
            // not also be split, demoted or reported.
            let candidates: Vec<usize> = nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.kind != NodeKind::End && n.name == "span")
                .filter(|(_, n)| {
                    n.parent
                        .is_some_and(|p| !phrasing.contains(&nodes[p].name.as_str()))
                })
                .filter(|(i, _)| {
                    nodes.iter().any(|n| {
                        n.parent == Some(*i)
                            && n.kind != NodeKind::End
                            && BLOCK_CHILD.contains(&n.name.as_str())
                    })
                })
                .map(|(i, _)| i)
                .collect();

            // The rename is only free when nothing was reaching the element by
            // its name. A book whose stylesheet says `span` anywhere — `span`,
            // `.foo span`, `span.bar` — has rules that stop matching the moment
            // this becomes a <div>, and that is a visible change to the page
            // rather than the invisible one the repair is supposed to be. Class
            // and id selectors survive the rename untouched, so the question is
            // about the element name alone. This mirrors what the <u> and
            // <center> repairs already do: ask the book's own stylesheet before
            // assuming a retag is invisible.
            let (promotable, blocked): (std::collections::HashSet<usize>, Vec<usize>) =
                if styles_span {
                    (std::collections::HashSet::new(), candidates)
                } else {
                    (candidates.into_iter().collect(), Vec::new())
                };
            if !blocked.is_empty() {
                outcome.push_finding(format!(
                    "{}: {} <span>(s) hold block content and belong in a <div>, but the book's \
                     stylesheet has rules for <span> that renaming would stop matching, so they \
                     need a look",
                    basename(&doc),
                    blocked.len()
                ));
            }
            let blocked: std::collections::HashSet<usize> = blocked.into_iter().collect();

            for i in 0..nodes.len() {
                if nodes[i].kind == NodeKind::End || !BLOCK_CHILD.contains(&nodes[i].name.as_str())
                {
                    continue;
                }
                // Only a block whose *direct* parent is phrasing-only is
                // misplaced. A <p> inside a <blockquote> is a legal child of a
                // block element, not a defect, and treating it as one demoted
                // the whole inside of every blockquote in the book to <span>s.
                let Some(parent) = nodes[i].parent else {
                    continue;
                };
                if !phrasing.contains(&nodes[parent].name.as_str()) {
                    continue;
                }
                // HTML5 lets a <legend> hold heading content alongside its
                // phrasing, so this one pairing is legal in EPUB 3 and an
                // error in EPUB 2.
                if version >= 3
                    && nodes[parent].name == "legend"
                    && matches!(
                        nodes[i].name.as_str(),
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    )
                {
                    continue;
                }
                // The parent span is about to become a <div>, which turns
                // this from an error into legal nesting.
                if promotable.contains(&parent) {
                    continue;
                }
                // The parent span *would* have become a <div>, and was left
                // alone to protect the stylesheet. Demoting this block to a
                // <span> instead would be the very rendering change that
                // decision was avoiding, only made one level down; the finding
                // above already says the pair needs a person.
                if blocked.contains(&parent) {
                    continue;
                }
                let Some((host_i, host)) = ancestor(&nodes, i, phrasing) else {
                    continue;
                };

                // A paragraph is closed by a following block element anyway, so
                // making that explicit is transcription, not interpretation.
                if host.name == "p" && nodes[i].parent == Some(host_i) {
                    match splits.iter_mut().find(|(p, _)| *p == host_i) {
                        Some((_, blocks)) => blocks.push(i),
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
                        "{}: <{}> inside <{}> holds block content (<{}>), and only a <p> \
                         can be split around it, so it needs a look",
                        basename(&doc),
                        nodes[i].name,
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

            for (host_i, blocks) in &splits {
                split_paragraph(&mut edits, &text, &nodes, *host_i, blocks);
                divided += u32::try_from(blocks.len()).unwrap_or(u32::MAX);
            }

            for i in promotable {
                edits.replace(nodes[i].span.start..nodes[i].name_end, "<div".to_string());
                if let Some(close) = nodes[i].close {
                    edits.replace(nodes[close].span.clone(), "</div>".to_string());
                }
                promoted += 1;
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if divided > 0 {
            outcome.push_change(format!(
                "split {divided} paragraph(s) around a block element they had swallowed"
            ));
        }
        if demoted > 0 {
            outcome.push_change(format!(
                "demoted {demoted} misplaced block element(s) to <span>"
            ));
        }
        if promoted > 0 {
            outcome.push_change(format!(
                "promoted {promoted} <span> element(s) holding block content to <div>"
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
                // only the ones opening the table are this fixer's business,
                // and the wrap spans first..last, so collecting past a gap
                // would pull whatever sits in it — a <thead>, a <tr> — inside
                // the <colgroup> along with them.
                //
                // A <caption> is the one thing that legally precedes them, and
                // the run has to be found *after* it: `<caption>…</caption>`
                // followed by bare <col>s used to stop the search on its first
                // step and leave the error unrepaired.
                let cols: Vec<&Node> = nodes
                    .iter()
                    .filter(|n| n.parent == Some(i) && n.kind != NodeKind::End)
                    .skip_while(|n| n.name == "caption")
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
