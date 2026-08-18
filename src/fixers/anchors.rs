//! RSC-005: `element "a" not allowed here` — anchors stranded in table structure.
//!
//! Kindle-derived markup routinely leaves anchors between rows:
//!
//! ```text
//! … </td></tr><a id="filepos1429403"></a><tr valign="top"><td …
//! ```
//!
//! Only `tr`, `script` and `template` (plus the section elements, in `table`)
//! may sit there, in either XHTML 1.1 or HTML5, so this is version-independent
//! as a *diagnosis*. The repair is not.
//!
//! Most of these anchors are live link targets — an index elsewhere in the book
//! points at them — so deleting them silently breaks navigation. The obvious
//! repair, moving the anchor to just after `</table>`, is valid in EPUB 3 but
//! **produces a fresh error in EPUB 2**: `<a>` is inline content and XHTML 1.1's
//! `<body>` accepts only block-level children. Verified against EPUB Check
//! 5.2.1, both ways.
//!
//! So the primary repair here is to migrate the *id* onto the nearest legal
//! element — the following row, else the preceding one, else the table itself.
//! That keeps the link landing in the same place, is valid under both rulesets,
//! and handles the stranded-after-the-last-row case that relocation was meant
//! for. Anything that does not fit becomes a finding rather than a guess.

use std::collections::{HashMap, HashSet};

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, Node, NodeKind, id_attrs, scan};
use crate::refs::ReferenceIndex;
use crate::util::basename;

/// What may legally appear directly inside each table-structure element.
fn permitted_children(parent: &str) -> Option<&'static [&'static str]> {
    match parent {
        "table" => Some(&[
            "caption", "colgroup", "col", "thead", "tbody", "tfoot", "tr", "script", "template",
        ]),
        "thead" | "tbody" | "tfoot" => Some(&["tr", "script", "template"]),
        "tr" => Some(&["td", "th", "script", "template"]),
        _ => None,
    }
}

/// Does this element enclose anything at all?
fn has_content(nodes: &[Node], i: usize) -> bool {
    nodes[i].has_text
        || nodes
            .iter()
            .any(|n| n.parent == Some(i) && n.kind != NodeKind::End)
}

/// The nearest ancestor `table`, if there is one.
fn enclosing_table(nodes: &[Node], mut i: usize) -> Option<usize> {
    while let Some(p) = nodes[i].parent {
        if nodes[p].name == "table" {
            return Some(p);
        }
        i = p;
    }
    None
}

/// A legal sibling that could carry the orphaned id: the next one in document
/// order, else the previous one, else the containing element itself. Only
/// candidates that do not already have an id qualify.
fn migration_target(nodes: &[Node], i: usize, parent: usize) -> Option<usize> {
    let allowed = permitted_children(&nodes[parent].name)?;
    let is_candidate = |j: usize| {
        let n = &nodes[j];
        n.kind != NodeKind::End
            && n.parent == Some(parent)
            && allowed.contains(&n.name.as_str())
            && n.attr("id").is_none()
    };

    (i + 1..nodes.len())
        .find(|&j| is_candidate(j))
        .or_else(|| (0..i).rev().find(|&j| is_candidate(j)))
        .or_else(|| nodes[parent].attr("id").is_none().then_some(parent))
}

/// What to do about one misplaced element.
enum Action {
    /// Empty and unreferenced: it can just go.
    Delete,
    /// Move its id onto `target`, then delete it.
    Migrate { target: usize, id: String },
    /// Move the whole element to just past byte offset `after`.
    Relocate { after: usize },
    /// Leave it alone and tell the user.
    Report(String),
}

/// Decide the fate of the misplaced element at `i`, without touching anything.
fn plan(
    nodes: &[Node],
    i: usize,
    parent: usize,
    doc: &str,
    version: u32,
    index: &ReferenceIndex,
    occurrences: &HashMap<&str, usize>,
) -> Action {
    let node = &nodes[i];
    let parent_name = &nodes[parent].name;

    // Diagnosing a stray element is in scope; repairing an arbitrary one is not.
    if node.name != "a" {
        return Action::Report(format!(
            "<{}> is not valid inside <{parent_name}> and was left alone",
            node.name
        ));
    }

    if has_content(nodes, i) {
        // Never delete something with visible content. Moving it out of the
        // table is only valid under HTML5.
        return match (
            version >= 3,
            enclosing_table(nodes, i).and_then(|t| nodes[t].close),
        ) {
            (true, Some(close)) => Action::Relocate {
                after: nodes[close].span.end,
            },
            _ => Action::Report(format!(
                "<a> with content is misplaced inside <{parent_name}>; moving it would \
                 not be valid in EPUB {version}, so it needs a look"
            )),
        };
    }

    // Empty. Which of its ids does anything actually link to?
    let mut live: Vec<&str> = id_attrs(node)
        .map(|a| a.value.as_str())
        .filter(|v| index.is_referenced(doc, v))
        .collect();
    live.sort_unstable();
    live.dedup();

    match live.len() {
        0 => Action::Delete,
        1 => {
            let id = live[0];
            if occurrences.get(id).copied().unwrap_or(0) > 1 {
                return Action::Report(format!(
                    "id \"{id}\" appears more than once, so the stranded <a> was left alone"
                ));
            }
            match migration_target(nodes, i, parent) {
                Some(target) => Action::Migrate {
                    target,
                    id: id.to_string(),
                },
                None => {
                    Action::Report(format!("nowhere to move id \"{id}\" from the stranded <a>"))
                }
            }
        }
        _ => Action::Report(format!(
            "<a> carries two live ids ({}) and cannot be merged onto one element",
            live.join(", ")
        )),
    }
}

/// Repair one document, returning its new text if anything changed.
fn fix_document(
    doc: &str,
    text: &str,
    version: u32,
    index: &ReferenceIndex,
    outcome: &mut Outcome,
) -> Option<String> {
    let short = basename(doc);
    let nodes = match scan(text) {
        Ok(n) => n,
        Err(e) => {
            outcome.push_finding(format!(
                "{short}: could not parse, anchors left alone ({e})"
            ));
            return None;
        }
    };

    // How many times each id value occurs, so migrating one never creates a
    // duplicate.
    let mut occurrences: HashMap<&str, usize> = HashMap::new();
    for attr in nodes.iter().flat_map(id_attrs) {
        *occurrences.entry(attr.value.as_str()).or_default() += 1;
    }

    let mut edits = Edits::new();
    let (mut deleted, mut migrated, mut relocated) = (0u32, 0u32, 0u32);
    // Rows that have already been given an id by an earlier anchor in this
    // pass. `migration_target` asks the unedited tree whether a candidate has
    // one, so two anchors stranded between the same pair of rows both chose
    // the same row and both wrote to it: `<tr id="pos1" id="pos2">`, a
    // duplicate attribute, which is not well-formed XML at all. The pass guard
    // caught the malformed document and rolled the whole pass back, so a book
    // with two stranded anchors got no repair rather than a wrong one — but it
    // got no repair.
    let mut claimed: HashSet<usize> = HashSet::new();

    for i in 0..nodes.len() {
        let node = &nodes[i];
        if node.kind == NodeKind::End {
            continue;
        }
        let Some(parent) = node.parent else { continue };
        let Some(allowed) = permitted_children(&nodes[parent].name) else {
            continue;
        };
        if allowed.contains(&node.name.as_str()) {
            continue;
        }

        let span = node.element_span(&nodes);
        match plan(&nodes, i, parent, doc, version, index, &occurrences) {
            Action::Delete => {
                edits.delete(span);
                deleted += 1;
            }
            Action::Migrate { target, id } if claimed.insert(target) => {
                edits.insert(nodes[target].name_end, format!(" id=\"{id}\""));
                edits.delete(span);
                migrated += 1;
            }
            // Somewhere to put it, but somebody is already there. An element
            // carries one id, so the second anchor stays where it is.
            Action::Migrate { id, .. } => outcome.push_finding(format!(
                "{short}: id \"{id}\" is on a stranded <a>, and the one element it could move \
                 to has just taken another id, so it was left alone"
            )),
            Action::Relocate { after } => {
                edits.delete(span.clone());
                edits.insert(after, text[span].to_string());
                relocated += 1;
            }
            Action::Report(why) => outcome.push_finding(format!("{short}: {why}")),
        }
    }

    if edits.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    if deleted > 0 {
        parts.push(format!("{deleted} empty anchor(s) removed"));
    }
    if migrated > 0 {
        parts.push(format!("{migrated} anchor id(s) moved to a legal element"));
    }
    if relocated > 0 {
        parts.push(format!("{relocated} anchor(s) moved out of the table"));
    }
    outcome.push_change(format!("{short}: {}", parts.join(", ")));

    Some(edits.apply(text))
}

pub struct MisplacedAnchors;

impl Fixer for MisplacedAnchors {
    fn name(&self) -> &'static str {
        "misplaced-anchors"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "remove or rehome anchors stranded between table rows, preserving link targets"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let index = book.reference_index();
        let version = book.epub_version();

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            if let Some(fixed) = fix_document(&doc, &text, version, &index, &mut outcome) {
                book.set_text(&doc, fixed);
            }
        }

        outcome
    }
}
