//! RSC-005: presentational table attributes that the governing ruleset rejects.
//!
//! Which attributes are actually errors depends on the EPUB version, and the
//! difference is not cosmetic:
//!
//! * **EPUB 3** content documents are validated as HTML5, which removed the
//!   whole presentational set.
//! * **EPUB 2** content documents are validated as XHTML 1.1, which is HTML 4.01
//!   Strict in XML clothing. That keeps `align` and `valign` on rows and cells
//!   but *not* on `<table>`, and drops `bgcolor`, `nowrap`, cell `width`/`height`
//!   and `hspace`/`vspace` everywhere.
//!
//! The EPUB 2 set here was measured against EPUB Check 5.2.1 rather than read
//! off a specification, and `tests/tables.rs` pins it.
//!
//! Stripping a presentational attribute is not automatically invisible. These
//! attributes contribute at the presentational-hints origin, below author
//! stylesheets and above the UA default: if any author rule sets the property
//! the attribute was already doing nothing, and if none does, removing it falls
//! back to the UA default. In Calibre-generated books — the common case — a
//! generated stylesheet sets them, so stripping changes nothing on screen.

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, NodeKind, scan};

/// Attributes HTML5 removed, by element.
const HTML5: &[(&[&str], &[&str])] = &[
    (
        &["table"],
        &[
            "align",
            "bgcolor",
            "cellpadding",
            "cellspacing",
            "frame",
            "rules",
            "width",
            "valign",
            "summary",
        ],
    ),
    (&["caption"], &["align"]),
    (
        &["col", "colgroup"],
        &["align", "bgcolor", "valign", "char", "charoff", "width"],
    ),
    (
        &["thead", "tbody", "tfoot", "tr"],
        &["align", "bgcolor", "valign", "char", "charoff"],
    ),
    (
        &["td", "th"],
        &[
            "align", "bgcolor", "valign", "width", "height", "nowrap", "char", "charoff",
        ],
    ),
    (&["img", "object"], &["hspace", "vspace", "align"]),
];

/// The subset that is *also* invalid under XHTML 1.1, so worth removing from an
/// EPUB 2 book. Everything omitted here is legal in EPUB 2 and is left alone:
/// removing it would change rendering for no validation benefit.
const XHTML11: &[(&[&str], &[&str])] = &[
    (&["table"], &["align", "bgcolor", "valign"]),
    (
        &["thead", "tbody", "tfoot", "tr", "col", "colgroup"],
        &["bgcolor"],
    ),
    (&["td", "th"], &["bgcolor", "width", "height", "nowrap"]),
    (&["img", "object"], &["hspace", "vspace"]),
];

fn removals(element: &str, version: u32) -> &'static [&'static str] {
    let table = if version >= 3 { HTML5 } else { XHTML11 };
    table
        .iter()
        .find(|(els, _)| els.contains(&element))
        .map_or(&[][..], |(_, attrs)| *attrs)
}

pub struct LegacyTableAttrs;

impl Fixer for LegacyTableAttrs {
    fn name(&self) -> &'static str {
        "legacy-table-attrs"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "strip presentational table attributes the EPUB version's ruleset rejects"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let version = book.epub_version();
        let mut outcome = Outcome::none();
        // Tally by attribute name so a dry run says what it would take out.
        let mut tally: Vec<(String, usize)> = Vec::new();
        let mut bump = |name: &str| match tally.iter_mut().find(|(n, _)| n == name) {
            Some((_, c)) => *c += 1,
            None => tally.push((name.to_string(), 1)),
        };

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let nodes = match scan(&text) {
                Ok(n) => n,
                Err(e) => {
                    outcome.push_finding(format!(
                        "{doc}: could not parse, attributes left alone ({e})"
                    ));
                    continue;
                }
            };

            let mut edits = Edits::new();
            for node in nodes.iter().filter(|n| n.kind != NodeKind::End) {
                let drop = removals(&node.name, version);
                for attr in &node.attrs {
                    if drop.contains(&attr.name.as_str()) {
                        edits.delete(attr.span_with_space.clone());
                        bump(&attr.name);
                    }
                }

                // HTML5 keeps `border` on <table>, but only as "" or "1".
                if version >= 3
                    && node.name == "table"
                    && let Some(border) = node.attr("border")
                    && !matches!(border.value.as_str(), "" | "1")
                {
                    if border.value.trim() == "0" {
                        edits.delete(border.span_with_space.clone());
                    } else {
                        edits.replace(border.span.clone(), "border=\"1\"");
                    }
                    bump("border");
                }
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        let total: usize = tally.iter().map(|(_, c)| c).sum();
        if total > 0 {
            tally.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let detail: Vec<String> = tally.iter().map(|(n, c)| format!("{n}x{c}")).collect();
            outcome.push_change(format!(
                "stripped {total} legacy attribute(s) for EPUB {version} [{}]",
                detail.join(", ")
            ));
        }
        outcome
    }
}
