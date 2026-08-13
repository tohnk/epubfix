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
use crate::css::Stylesheet;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, Node, NodeKind, scan};
use crate::util::ends_with_any;

/// What to do with a presentational attribute the ruleset rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Presentation {
    /// Keep whatever the attribute was actually doing: convert it to an inline
    /// declaration when nothing was overriding it, strip it when something was.
    ///
    /// Neither of the other two is right on its own, because they answer a
    /// question the book has already answered. A presentational attribute
    /// contributes *below* author stylesheets, so if a rule sets the property
    /// the attribute has been inert for as long as the book has existed and
    /// removing it cannot change the page. If no rule sets it, the attribute is
    /// the only thing holding the layout up and removing it drops to the user
    /// agent default.
    ///
    /// *The Hero of Ages* is the case that forced this. Its Ars Arcanum table
    /// carries `width` on 75 cells, its stylesheet declares no width at all, and
    /// stripping all 75 reflows a reference table people actually consult.
    /// Meanwhile the same run's `valign` attributes are overridden 9 times in
    /// 10, where converting would be the harmful move. One policy cannot be
    /// right for both; asking is right for both.
    ///
    /// The override test is an approximation ([`crate::css`]), and this is the
    /// one place its answer changes bytes rather than a report. That is
    /// tolerable because its bias runs the safe way: it matches element names
    /// and classes anywhere in a selector, so it *over*-reports "declared",
    /// which lands on stripping — exactly what this tool did before.
    #[default]
    Faithful,
    /// Always convert, whether or not a stylesheet was already overriding it.
    ///
    /// An inline style sits *above* author rules in the cascade, where the
    /// attribute sat below them — so on a Calibre book whose stylesheet sets
    /// `vertical-align: middle` on the rows, converting 693 `valign="top"`
    /// attributes changes the rendering that [`Presentation::Faithful`] leaves
    /// alone. It is the right choice only for a book with no stylesheet worth
    /// the name.
    Preserve,
    /// Always remove it, and report the ones no stylesheet was overriding.
    ///
    /// The tidiest markup and the least faithful rendering.
    Strip,
}

/// Attributes HTML5 removed, by element.
const HTML5: &[(&[&str], &[&str])] = &[
    // Not a table, but the same defect and the same repair: the presentational
    // attributes an early-2000s converter puts on <body>. Measured, these are
    // errors under *both* rulesets, unlike everything else here — one real book
    // carries `<body text="#000000" link="#0000ff">` in all 95 of its
    // documents, which is 190 errors from one habit.
    (
        &["body"],
        &["text", "link", "alink", "vlink", "bgcolor", "background"],
    ),
    (
        &["table"],
        &[
            "align",
            "bgcolor",
            "bordercolor",
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
    // Not a table, but the same defect and the same repair: the presentational
    // attributes an early-2000s converter puts on <body>. Measured, these are
    // errors under *both* rulesets, unlike everything else here — one real book
    // carries `<body text="#000000" link="#0000ff">` in all 95 of its
    // documents, which is 190 errors from one habit.
    (
        &["body"],
        &["text", "link", "alink", "vlink", "bgcolor", "background"],
    ),
    (&["table"], &["align", "bgcolor", "bordercolor", "valign"]),
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

/// The CSS property a presentational attribute stands in for, or `None` where
/// there is no single-property equivalent.
///
/// `cellpadding` and `cellspacing` are the honest `None`s: they describe the
/// cells and the table's border model, so there is nothing to write on the
/// element carrying them. `frame`, `rules`, `summary`, `char` and `charoff`
/// have no CSS spelling at all.
fn css_property(element: &str, attr: &str) -> Option<&'static str> {
    Some(match attr {
        "valign" => "vertical-align",
        "bgcolor" => "background-color",
        // <body text="#000"> is the page's text colour and nothing else.
        // `link`/`vlink`/`alink` are deliberately absent: those need `a:link`
        // and `a:visited` selectors, which an inline style cannot express.
        "text" => "color",
        "width" => "width",
        "height" => "height",
        "nowrap" => "white-space",
        "hspace" => "margin-left",
        "vspace" => "margin-top",
        // Alignment means different things on different elements: text
        // alignment in a cell, floating for an image or a whole table.
        "align" => match element {
            "img" | "object" | "table" => "float",
            _ => "text-align",
        },
        _ => return None,
    })
}

/// The inline declarations equivalent to one presentational attribute.
///
/// Returns nothing when the attribute has no faithful CSS spelling, which is
/// the caller's cue to strip it and say so rather than invent something.
fn inline_style(element: &str, attr: &str, value: &str) -> Option<String> {
    let v = value.trim();
    Some(match attr {
        "valign" => format!("vertical-align: {v}"),
        "bgcolor" => format!("background-color: {v}"),
        "text" => format!("color: {v}"),
        "nowrap" => "white-space: nowrap".to_string(),
        "width" | "height" => format!("{attr}: {}", length(v)?),
        "hspace" => format!("margin-left: {0}; margin-right: {0}", length(v)?),
        "vspace" => format!("margin-top: {0}; margin-bottom: {0}", length(v)?),
        "align" => match (element, v.to_ascii_lowercase().as_str()) {
            // HTML's own rendering rules: on an image or a table, horizontal
            // alignment floats the box; on a table, centring is auto margins.
            ("img" | "object" | "table", "left" | "right") => {
                format!("float: {}", v.to_ascii_lowercase())
            }
            ("table", "center") => "margin-left: auto; margin-right: auto".to_string(),
            ("img" | "object", a @ ("top" | "middle" | "bottom")) => {
                format!("vertical-align: {a}")
            }
            ("img" | "object" | "table", _) => return None,
            _ => format!("text-align: {}", v.to_ascii_lowercase()),
        },
        _ => return None,
    })
}

/// A legacy length: bare numbers are pixels, percentages pass through.
fn length(v: &str) -> Option<String> {
    if let Some(pct) = v.strip_suffix('%')
        && pct.trim().parse::<f64>().is_ok()
    {
        return Some(v.to_string());
    }
    v.parse::<f64>().ok().map(|_| format!("{v}px"))
}

/// Append `declarations` to whatever `style` the element already has.
fn merge_style(edits: &mut Edits, node: &Node, declarations: &[String]) {
    if declarations.is_empty() {
        return;
    }
    let added = declarations.join("; ");
    match node.attr("style") {
        Some(existing) => {
            let base = existing.value.trim().trim_end_matches(';');
            let value = if base.is_empty() {
                added
            } else {
                format!("{base}; {added}")
            };
            edits.replace(existing.span.clone(), format!("style=\"{value}\""));
        }
        None => edits.insert(node.name_end, format!(" style=\"{added}\"")),
    }
}

/// Every stylesheet in the book, folded into one index.
pub(crate) fn author_styles(book: &Book) -> Stylesheet {
    let mut sheet = Stylesheet::default();
    for name in book.names() {
        if ends_with_any(name, &[".css"])
            && let Some(text) = book.text(name)
        {
            sheet.parse(text);
        }
    }
    sheet
}

pub struct LegacyTableAttrs {
    pub mode: Presentation,
}

impl Default for LegacyTableAttrs {
    fn default() -> Self {
        LegacyTableAttrs {
            mode: Presentation::Strip,
        }
    }
}

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
        let sheet = author_styles(book);
        // Attributes no author rule was overriding, so removing them may show;
        // and, under --preserve-presentation, ones with no CSS spelling at all.
        let mut unoverridden: Vec<String> = Vec::new();
        let mut no_equivalent: Vec<String> = Vec::new();
        let mut converted = 0usize;
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
                let classes = node.attr("class").map_or("", |a| a.value.as_str());
                let mut carried: Vec<String> = Vec::new();

                for attr in &node.attrs {
                    if !drop.contains(&attr.name.as_str()) {
                        continue;
                    }
                    edits.delete(attr.span_with_space.clone());
                    bump(&attr.name);

                    // Was this attribute doing anything? If a stylesheet
                    // already sets the property it has been overridden for as
                    // long as the book has existed, and removing it cannot
                    // change the page.
                    let overridden = css_property(&node.name, &attr.name)
                        .is_some_and(|prop| sheet.declares(&node.name, classes, prop));
                    let inline = match self.mode {
                        Presentation::Preserve => true,
                        Presentation::Strip => false,
                        Presentation::Faithful => !overridden,
                    };

                    if inline {
                        match inline_style(&node.name, &attr.name, &attr.value) {
                            Some(decl) => {
                                carried.push(decl);
                                converted += 1;
                            }
                            None => no_equivalent.push(attr.name.clone()),
                        }
                    } else if !overridden {
                        unoverridden.push(attr.name.clone());
                    }
                }
                merge_style(&mut edits, node, &carried);

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

        report(
            &mut outcome,
            self.mode,
            version,
            &mut tally,
            converted,
            &unoverridden,
            &no_equivalent,
        );
        outcome
    }
}

/// Turn the tallies into the run's report lines.
fn report(
    outcome: &mut Outcome,
    mode: Presentation,
    version: u32,
    tally: &mut [(String, usize)],
    converted: usize,
    unoverridden: &[String],
    no_equivalent: &[String],
) {
    let total: usize = tally.iter().map(|(_, c)| c).sum();
    if total > 0 {
        tally.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let detail: Vec<String> = tally.iter().map(|(n, c)| format!("{n}x{c}")).collect();
        let detail = detail.join(", ");
        let line = match mode {
            Presentation::Preserve => format!(
                "converted {total} legacy attribute(s) to inline style for EPUB {version} \
                 [{detail}]"
            ),
            Presentation::Strip => {
                format!("stripped {total} legacy attribute(s) for EPUB {version} [{detail}]")
            }
            // Say which was which rather than pick a word that is half true.
            Presentation::Faithful if converted > 0 && converted < total => format!(
                "moved {converted} legacy attribute(s) into inline CSS and stripped {} that a \
                 stylesheet already overrode, for EPUB {version} [{detail}]",
                total - converted
            ),
            Presentation::Faithful if converted > 0 => format!(
                "moved {total} legacy attribute(s) into inline CSS for EPUB {version} [{detail}]"
            ),
            Presentation::Faithful => format!(
                "stripped {total} legacy attribute(s) a stylesheet already overrode, for EPUB \
                 {version} [{detail}]"
            ),
        };
        outcome.push_change(line);
    }
    if !unoverridden.is_empty() {
        outcome.push_finding(format!(
            "{} of those {total} attribute(s) were not already overridden by a stylesheet rule, \
             so removing them may change how the page looks [{}]",
            unoverridden.len(),
            summarise(unoverridden)
        ));
    }
    if !no_equivalent.is_empty() {
        outcome.push_finding(format!(
            "{} attribute(s) have no single-property CSS equivalent and were removed rather than \
             converted [{}]",
            no_equivalent.len(),
            summarise(no_equivalent)
        ));
    }
}

/// `["valign", "valign", "bgcolor"]` becomes `"valignx2, bgcolorx1"`.
fn summarise(names: &[String]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for n in names {
        match counts.iter_mut().find(|(k, _)| *k == n.as_str()) {
            Some((_, c)) => *c += 1,
            None => counts.push((n, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    counts
        .iter()
        .map(|(n, c)| format!("{n}x{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}
