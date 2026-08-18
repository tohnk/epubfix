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
//! Replacing a presentational attribute is not automatically invisible. These
//! attributes contribute at the presentational-hints origin, below author
//! stylesheets and above the UA default. This module converts the cases it can
//! express faithfully, and leaves the rest in place for a report rather than
//! silently dropping behavior it cannot reproduce.
//!
//! "Faithfully" means at the same place in the cascade, which is what
//! [`HINT_LAYER`] is for: a generated class rule outranks the author's own
//! element selectors and would change the page in the opposite direction, while
//! the same rule inside a cascade layer loses to every unlayered author
//! declaration exactly as the attribute did.

use std::collections::HashMap;

use crate::book::Book;
use crate::css::Stylesheet;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, Node, NodeKind, scan};
use crate::util::{basename, ends_with_any};

/// What to do with a presentational attribute the ruleset rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Presentation {
    /// Keep whatever the attribute was actually doing: strip it when a
    /// stylesheet was already overriding it, and otherwise reproduce it as a
    /// generated rule in the [`HINT_LAYER`], where the attribute itself sat.
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
    /// The override test is an approximation ([`crate::css`]), and it decides
    /// only between "delete" and "reproduce", never between two different
    /// renderings. That matters more than it looks: this branch used to write
    /// an *inline* style, which put the declaration above every author rule
    /// when the attribute had been below them, so being wrong about the
    /// override changed the page. The approximation has real blind spots — it
    /// reads `.css` entries only, so a `<style>` block in the document is
    /// invisible to it, as is any `#id` rule. A layered rule loses to both
    /// without having to know they exist, which takes the guess off the
    /// rendering path entirely.
    #[default]
    Faithful,
    /// Always convert, whether or not a stylesheet was already overriding it,
    /// and convert to an *inline* style rather than a layered rule. This is the
    /// mode that deliberately promotes: it is for a book whose stylesheet
    /// should not be winning.
    ///
    /// An inline style sits *above* author rules in the cascade, where the
    /// attribute sat below them — so on a Calibre book whose stylesheet sets
    /// `vertical-align: middle` on the rows, converting 693 `valign="top"`
    /// attributes changes the rendering that [`Presentation::Faithful`] leaves
    /// alone. It is the right choice only for a book with no stylesheet worth
    /// the name.
    Preserve,
    /// Always remove attributes that have a supported CSS conversion, and report
    /// the ones no stylesheet was overriding. Attributes with no conversion are
    /// still left in place, because none of the modes should silently lose
    /// behavior the fixer cannot reproduce.
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
    (&["img", "object"], &["hspace", "vspace", "align", "border"]),
    (&["hr"], &["size", "noshade", "width", "align"]),
    (&["br"], &["clear"]),
    // `align` is an error under both rulesets — measured, a Calibre *Stranger*
    // carries `<p class="Cite" align="center">` and epubcheck rejects it as
    // EPUB 2 and EPUB 3 alike — and maps to `text-align`, so it joins `clear`.
    (
        &["p", "div", "span", "h1", "h2", "h3", "h4", "h5", "h6"],
        &["clear", "align"],
    ),
    (&["li"], &["value"]),
];

/// The subset that is *also* invalid under XHTML 1.1, so worth removing from an
/// EPUB 2 book. Everything omitted here is legal in EPUB 2 and is left alone:
/// removing it would change rendering for no validation benefit.
///
/// One row per element — the lookup takes the first match, so an element that
/// appears twice would silently lose the attributes of the second row.
const XHTML11: &[(&[&str], &[&str])] = &[
    // Not a table, but the same defect and the same repair: the presentational
    // attributes an early-2000s converter puts on <body>. Measured, these are
    // errors under *both* rulesets, unlike everything else here — one real book
    // carries `<body text="#000000" link="#0000ff">` in all 95 of its
    // documents, which is 190 errors from one habit.
    (
        &["body"],
        &[
            "text",
            "link",
            "alink",
            "vlink",
            "bgcolor",
            "background",
            "tag",
            "xml:space",
        ],
    ),
    (&["table"], &["align", "bgcolor", "bordercolor", "valign"]),
    (
        &["thead", "tbody", "tfoot", "tr", "col", "colgroup"],
        &["bgcolor"],
    ),
    (&["td", "th"], &["bgcolor", "width", "height", "nowrap"]),
    (
        &["img", "object"],
        &["hspace", "vspace", "border", "tag", "xml:space"],
    ),
    (&["hr"], &["size", "noshade"]),
    (&["br"], &["clear"]),
    // `align` was dropped from these in XHTML 1.1; Gutenberg's `tag` and
    // `xml:space` are validation noise under either ruleset.
    (
        &["p", "div", "span", "h1", "h2", "h3", "h4", "h5", "h6"],
        &["clear", "align", "tag", "xml:space"],
    ),
    (&["html"], &["style", "tag", "xml:space"]),
    (&["a"], &["tag", "xml:space"]),
    (&["link"], &["tag", "xml:space"]),
];

fn removals(element: &str, version: u32) -> &'static [&'static str] {
    let table = if version >= 3 { HTML5 } else { XHTML11 };
    table
        .iter()
        .find(|(els, _)| els.contains(&element))
        .map_or(&[][..], |(_, attrs)| *attrs)
}

/// List markers that show no number, so an `<li value>` under one cannot
/// change what the reader sees.
///
/// A whitelist rather than a blacklist of the numbered ones: `decimal`,
/// `lower-roman` and friends are a long list that `@counter-style` lets a book
/// extend, and guessing wrong in that direction deletes something visible.
const UNNUMBERED_MARKERS: &[&str] = &[
    "disc", "circle", "square", "none", "inside", "outside", "inherit", "initial", "unset",
];

/// True when removing `attr` from `node` provably cannot change the page.
///
/// Distinct from "epubfix has no CSS conversion for this". These attributes are
/// not presentation that needs rehousing, they are debris that means nothing —
/// so there is nothing to preserve and deleting is lossless rather than lossy.
/// Each answer here was measured, not assumed:
///
/// * **`tag`** is Project Gutenberg's converter leaking its own internals:
///   `<a tag="{http://www.w3.org/1999/xhtml}a">`. The value is the qualified
///   name of the element already carrying it, so it says nothing the tag does
///   not. Only deleted when it really does name its own element, which is what
///   makes it debris rather than something a book meant.
/// * **`xml:space`** has no effect on rendering. Measured in an XHTML document
///   served as `application/xhtml+xml`: a `<div xml:space="preserve">` holding
///   three lines and runs of spaces computes `white-space: normal` and renders
///   at exactly the height of the same `<div>` without it. Whitespace in XHTML
///   is CSS's business, and no engine honours the XML attribute.
/// * **`value` on an `<li>` in a `<ul>`** shows only where the marker is
///   numbered — which a stylesheet can arrange, so this is not free. Measured:
///   in a plain `<ul>` the markers are bullets with or without it, but a `<ul>`
///   restyled `list-style-type: decimal` renders `5.` `6.` with it and `1.`
///   `2.` without. So it is inert only when every rule that could reach the
///   list sets an unnumbered marker, or none sets one at all and the UA default
///   `disc` stands.
fn is_inert(node: &Node, attr: &crate::markup::Attr, src: &str) -> bool {
    match attr.name.as_str() {
        "tag" => {
            let local = attr.value.rsplit('}').next().unwrap_or(&attr.value);
            local.eq_ignore_ascii_case(node.raw_name(src).rsplit(':').next().unwrap_or(""))
        }
        "xml:space" => true,
        _ => false,
    }
}

/// Whether an `<li value>` inside `list` can show, given the book's stylesheet.
fn marker_is_numbered(list: &Node, sheet: &Stylesheet) -> bool {
    let classes = list.attr("class").map_or("", |a| a.value.as_str());
    let mut declared: Vec<&str> = sheet.declared_values(&list.name, classes, "list-style-type");
    declared.extend(sheet.declared_values(&list.name, classes, "list-style"));
    // Nothing declared: the UA default for <ul> is `disc`, which shows no number.
    if declared.is_empty() {
        return false;
    }
    // Numbered unless every declaration is made only of unnumbered keywords.
    !declared.iter().all(|v| {
        v.split_whitespace()
            .all(|t| UNNUMBERED_MARKERS.contains(&t.trim_end_matches(',')))
    })
}

/// The CSS property a presentational attribute stands in for, or `None` where
/// this inline-conversion helper cannot express it. `None` no longer authorizes
/// deletion: the stylesheet pass may handle it, otherwise it is reported and
/// left in place.
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
        "height" | "size" => "height",
        "nowrap" => "white-space",
        "hspace" => "margin-left",
        "vspace" => "margin-top",
        "clear" => "clear",
        "border" => "border",
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
        // <hr size> is a height, not a CSS property named "size".
        "size" => format!("height: {}", length(v)?),
        "hspace" => format!("margin-left: {0}; margin-right: {0}", length(v)?),
        "vspace" => format!("margin-top: {0}; margin-bottom: {0}", length(v)?),
        "clear" => format!("clear: {}", v.to_ascii_lowercase()),
        "border" => {
            if v == "0" {
                "border: 0".to_string()
            } else {
                format!("border: {} solid", length(v)?)
            }
        }
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

/// True when a source attribute value is safe to place in generated CSS text.
///
/// This is intentionally a small allow-list by exclusion. Values such as a
/// colour name or a CSS declaration list can contain punctuation, but must not
/// be allowed to close the generated style element or introduce a new rule.
fn safe_css_fragment(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    !value.trim().is_empty()
        && !value.contains('&')
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | '{' | '}'))
        && !lower.contains("</style")
}

/// The cascade layer every generated hint-priority rule goes into.
///
/// A presentational attribute contributes *below* every author declaration,
/// whatever that declaration's specificity. No selector reproduces that: the
/// obvious `.generated-class { border: 0 }` is specificity (0,1,0) and beats an
/// author `img { border: 2px solid }` at (0,0,1), which is the opposite of what
/// the attribute did. Placing it first in `<head>` does not help either, because
/// specificity is compared before order of appearance.
///
/// A cascade layer does reproduce it exactly: an unlayered author declaration
/// wins over a layered one regardless of specificity. Measured, on real XHTML:
///
/// | | wins |
/// | --- | --- |
/// | `<img border="0">` vs `img { border: 2px }` | the stylesheet |
/// | `.gen { border: 0 }` vs `img { border: 2px }` | ❌ the generated rule |
/// | `@layer { .gen { border: 0 } }` vs `img { border: 2px }` | ✅ the stylesheet |
///
/// The cost is that a reader with no `@layer` support skips the whole block, so
/// the attribute's effect is lost there. That is the same outcome as
/// `--strip-presentation`, which this tool already offers on purpose.
const HINT_LAYER: &str = "epubfix-hints";

/// Wrap generated rules in a `<style>` element, layered or not.
///
/// Only hint-priority rules are layered. A rule standing in for an inline
/// `style` attribute has to keep *beating* author rules, so it stays unlayered
/// and is placed last in `<head>` instead.
fn style_block(label: &str, rules: &[&str], layered: bool) -> String {
    let body = rules.join("\n");
    let body = if layered {
        format!("@layer {HINT_LAYER} {{\n{body}\n}}")
    } else {
        body
    };
    format!("\n<style type=\"text/css\">\n/* epubfix generated {label} */\n{body}\n</style>\n")
}

/// The class name for a rule template, minting one the first time it is seen.
///
/// Keyed on the template rather than the element, so identical declarations
/// share a class and a rule. Without that, one class was minted per attribute
/// *occurrence*: a book with the same `border="0"` on three hundred images
/// would carry three hundred identically-worded rules and three hundred class
/// names. Returns the new rule to emit, or `None` when an existing class
/// already covers it.
fn class_for(
    tokens: &mut HashMap<String, String>,
    serial: &mut u32,
    prefix: &str,
    template: &str,
) -> (String, Option<String>) {
    if let Some(existing) = tokens.get(template) {
        return (existing.clone(), None);
    }
    *serial += 1;
    let token = format!(".{prefix}-{serial}");
    tokens.insert(template.to_string(), token.clone());
    let rule = template.replace("{token}", &token);
    (token, Some(rule))
}

/// Append a generated class without touching any other attributes.
fn add_generated_class(edits: &mut Edits, node: &Node, token: &str) {
    match node.attr("class") {
        Some(class) if class.value.trim().is_empty() => {
            edits.replace(class.span.clone(), format!("class=\"{token}\""));
        }
        Some(class) => edits.replace(
            class.span.clone(),
            format!("class=\"{} {token}\"", class.value),
        ),
        None => edits.insert(node.name_end, format!(" class=\"{token}\"")),
    }
}

fn nearest_table(nodes: &[Node], node: usize) -> Option<usize> {
    let mut parent = nodes[node].parent;
    while let Some(i) = parent {
        if nodes[i].name == "table" {
            return Some(i);
        }
        parent = nodes[i].parent;
    }
    None
}

/// Where a generated rule has to sit to reproduce what it replaced.
///
/// The two kinds of source sit on opposite sides of the author stylesheet, so
/// one insertion point cannot serve both. Measured in a browser rather than
/// reasoned about, because the difference is invisible until it is wrong:
///
/// | replacing | original beat author rules? | generated block goes |
/// | --- | --- | --- |
/// | a presentational hint (`cellpadding`, `link`, `noshade`) | no | first in `<head>` |
/// | an inline `style` attribute | yes | last in `<head>` |
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// Replacing a presentational hint. Those contribute *below* author rules,
    /// so the generated rule has to lose to them exactly as the attribute did.
    /// First in `<head>`, ahead of the book's own stylesheets.
    Hint,
    /// Replacing an inline `style` attribute, which outranks every normal
    /// author declaration whatever its specificity — "element-attached styles"
    /// is a cascade criterion above specificity, not a specificity value. Last
    /// in `<head>`, so at least equal-specificity author rules still lose to it.
    Inline,
}

/// Convert legacy presentation that needs selectors or a stylesheet rather
/// than one inline declaration.
///
/// Up to two `<style>` blocks are generated per document, one at each end of
/// `<head>`, because what a rule replaces decides where it has to sit — see
/// [`Placement`]. Unsupported constructs are deliberately not touched;
/// [`LegacyTableAttrs`] reports them afterwards.
///
/// One gap survives and cannot be closed with a selector: an author rule that
/// reaches `<html>` through a class or id (`html.dark { }`) still beats the
/// generated rule, where the inline attribute would have won. Specificity is
/// compared component-wise, so no quantity of classes or pseudo-classes ever
/// outranks a single id, and `!important` would overshoot in the other
/// direction — a normal inline style *loses* to an author `!important`, so
/// marking the repair important would win where the original lost. EPUB 2 is
/// the only version this fires for, and class-themed `<html>` is not a thing
/// the books it fires on do.
pub struct StylesheetPresentation;

impl Fixer for StylesheetPresentation {
    fn name(&self) -> &'static str {
        "stylesheet-presentation"
    }

    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }

    fn description(&self) -> &'static str {
        "move legacy presentation that needs selectors into a generated stylesheet"
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the conversion table, class assignment and stylesheet emission share one \
                  document walk"
    )]
    fn apply(&self, book: &mut Book) -> Outcome {
        let version = book.epub_version();
        let mut outcome = Outcome::none();
        let mut converted = 0u32;
        let mut documents = 0u32;
        let mut serial = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let Some(head) = nodes
                .iter()
                .position(|n| n.name == "head" && n.kind == NodeKind::Start)
            else {
                continue;
            };
            // Just inside `<head>`, and just before `</head>`. A head with no
            // end tag has no second insertion point, and an inline style that
            // cannot be placed faithfully is one this pass declines to move at
            // all — it stays put and `LegacyTableAttrs` reports it.
            let head_start = nodes[head].span.end;
            let head_end = nodes[head].close.map(|c| nodes[c].span.start);

            let mut edits = Edits::new();
            let mut rules: Vec<(Placement, String)> = Vec::new();
            let mut tokens: HashMap<String, String> = HashMap::new();
            let mut classes: Vec<Vec<String>> = vec![Vec::new(); nodes.len()];

            for (i, node) in nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.kind != NodeKind::End)
            {
                let drop = removals(&node.name, version);
                for attr in &node.attrs {
                    if !drop.contains(&attr.name.as_str()) {
                        continue;
                    }

                    let mut rule: Option<String> = None;
                    let mut targets: Vec<usize> = Vec::new();
                    let mut placement = Placement::Hint;

                    match (node.name.as_str(), attr.name.as_str()) {
                        ("html", "style")
                            if safe_css_fragment(&attr.value) && head_end.is_some() =>
                        {
                            placement = Placement::Inline;
                            rule = Some(format!("html {{ {} }}", attr.value.trim()));
                        }
                        ("body", "link" | "alink" | "vlink") if safe_css_fragment(&attr.value) => {
                            let pseudo = match attr.name.as_str() {
                                "link" => ":link",
                                "alink" => ":active",
                                _ => ":visited",
                            };
                            rule = Some(format!("a{pseudo} {{ color: {}; }}", attr.value.trim()));
                        }
                        ("table", "cellpadding") => {
                            if let Some(value) = length(&attr.value) {
                                for (j, child) in nodes.iter().enumerate() {
                                    if child.kind != NodeKind::End
                                        && matches!(child.name.as_str(), "td" | "th")
                                        && nearest_table(&nodes, j) == Some(i)
                                    {
                                        targets.push(j);
                                    }
                                }
                                rule = Some(format!("{{token}} {{ padding: {value}; }}"));
                            }
                        }
                        ("table", "cellspacing") => {
                            if let Some(value) = length(&attr.value) {
                                targets.push(i);
                                rule = Some(format!(
                                    "{{token}} {{ border-spacing: {value}; border-collapse: separate; }}"
                                ));
                            }
                        }
                        ("table", "bordercolor") if safe_css_fragment(&attr.value) => {
                            targets.push(i);
                            rule = Some(format!(
                                "{{token}} {{ border-color: {}; }}",
                                attr.value.trim()
                            ));
                        }
                        ("hr", "noshade") => {
                            targets.push(i);
                            rule = Some("{token} { border-style: solid; }".to_string());
                        }
                        _ => {}
                    }

                    let Some(rule) = rule else {
                        continue;
                    };
                    // Two tables with the same `cellpadding` want the same rule,
                    // not two identical ones under different names.
                    let (token, new_rule) =
                        class_for(&mut tokens, &mut serial, "epubfix-presentation", &rule);
                    if let Some(new_rule) = new_rule {
                        rules.push((placement, new_rule));
                    }
                    for target in targets {
                        let class = token.trim_start_matches('.');
                        if !classes[target].iter().any(|c| c == class) {
                            classes[target].push(class.to_string());
                        }
                    }
                    edits.delete(attr.span_with_space.clone());
                    converted += 1;
                }
            }

            for (i, names) in classes
                .iter()
                .enumerate()
                .filter(|(_, names)| !names.is_empty())
            {
                add_generated_class(&mut edits, &nodes[i], &names.join(" "));
            }

            // A class may be shared by several target nodes, so the class edits
            // go in first. Then one block per placement: hints ahead of the
            // book's own stylesheets so their cascade stays authoritative, and
            // former inline styles behind them so theirs does.
            let mut block = |at: usize, kind: Placement, label: &str| {
                let css: Vec<&str> = rules
                    .iter()
                    .filter(|(p, _)| *p == kind)
                    .map(|(_, r)| r.as_str())
                    .collect();
                if css.is_empty() {
                    return false;
                }
                edits.insert(at, style_block(label, &css, kind == Placement::Hint));
                true
            };
            let mut wrote = block(head_start, Placement::Hint, "presentation");
            if let Some(at) = head_end {
                wrote |= block(at, Placement::Inline, "presentation, was an inline style");
            }
            if wrote {
                documents += 1;
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if converted > 0 {
            outcome.push_change(format!(
                "converted {converted} legacy presentation attribute(s) into generated CSS in \
                 {documents} document(s)"
            ));
        }
        outcome
    }
}

/// What becomes of one presentational attribute.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rehouse {
    /// Into the element's own `style`, above every author rule. Only
    /// `--preserve-presentation` asks for this.
    Inline,
    /// Into a generated class rule in the [`HINT_LAYER`], which is where the
    /// attribute itself sat: below every author declaration.
    Layer,
    /// Deleted. Either the book's stylesheet was already overriding it, so this
    /// cannot show, or `--strip-presentation` asked for it regardless.
    Drop,
    /// Left exactly where it is, because there is nowhere to put a rule. Losing
    /// the declaration is worse than leaving a validation error behind.
    Keep,
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
        "convert or report presentational attributes the EPUB version's ruleset rejects"
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one walk answering three questions (strip, convert, report) is clearer \
                  split across lines than across functions"
    )]
    fn apply(&self, book: &mut Book) -> Outcome {
        let version = book.epub_version();
        let mut outcome = Outcome::none();
        let sheet = author_styles(book);
        // Attributes no author rule was overriding, so removing them may show;
        // and, under --preserve-presentation, ones with no CSS spelling at all.
        let mut unoverridden: Vec<String> = Vec::new();
        let mut no_equivalent: Vec<String> = Vec::new();
        // Convertible, but the document has no <head> to put the rule in.
        let mut homeless: Vec<String> = Vec::new();
        // Deleted because they provably did nothing, counted apart from the
        // conversions so the report does not claim presentation was moved.
        let mut inert_tally: Vec<(String, usize)> = Vec::new();
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

            // Where a hint-priority block can go, if this document needs one.
            // Every document should have a head by now — `documents::HeadContent`
            // runs well before this — but a document without one has nowhere to
            // put a rule, and losing the declaration is worse than leaving the
            // attribute for the report.
            let head_start = nodes
                .iter()
                .find(|n| n.name == "head" && n.kind == NodeKind::Start)
                .map(|n| n.span.end);

            let mut edits = Edits::new();
            let mut rules: Vec<String> = Vec::new();
            let mut tokens: HashMap<String, String> = HashMap::new();
            let mut classes: Vec<Vec<String>> = vec![Vec::new(); nodes.len()];
            let mut serial = 0u32;

            for (i, node) in nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.kind != NodeKind::End)
            {
                let drop = removals(&node.name, version);
                let classes_attr = node.attr("class").map_or("", |a| a.value.as_str());
                let mut carried: Vec<String> = Vec::new();

                // HTML5 keeps `value` on <li> inside an <ol>, where it numbers
                // the list; inside a <ul> it says nothing and is an error.
                let mut enclosing_ul = None;
                if node.name == "li" {
                    let mut cur = node.parent;
                    while let Some(p) = cur {
                        match nodes[p].name.as_str() {
                            "ul" => {
                                enclosing_ul = Some(p);
                                break;
                            }
                            "ol" => break,
                            _ => cur = nodes[p].parent,
                        }
                    }
                }
                let in_ul = enclosing_ul.is_some();

                for attr in &node.attrs {
                    if !drop.contains(&attr.name.as_str()) {
                        continue;
                    }
                    if attr.name == "value" && !in_ul {
                        continue;
                    }
                    // Debris rather than presentation: nothing to rehouse,
                    // because there is nothing it was doing. Deleting is
                    // lossless in every mode, so this is decided before the
                    // policy is consulted at all.
                    let inert = is_inert(node, attr, &text)
                        || (attr.name == "value"
                            && enclosing_ul
                                .is_some_and(|ul| !marker_is_numbered(&nodes[ul], &sheet)));
                    if inert {
                        edits.delete(attr.span_with_space.clone());
                        match inert_tally.iter_mut().find(|(n, _)| *n == attr.name) {
                            Some((_, c)) => *c += 1,
                            None => inert_tally.push((attr.name.clone(), 1)),
                        }
                        continue;
                    }

                    let declaration = inline_style(&node.name, &attr.name, &attr.value);
                    if attr.name == "value" || declaration.is_none() {
                        no_equivalent.push(attr.name.clone());
                        continue;
                    }

                    // Was this attribute doing anything? If a stylesheet
                    // already sets the property it has been overridden for as
                    // long as the book has existed, and removing it cannot
                    // change the page.
                    let overridden = css_property(&node.name, &attr.name)
                        .is_some_and(|prop| sheet.declares(&node.name, classes_attr, prop));
                    let declaration = declaration.expect("checked above");
                    let action = match self.mode {
                        // The mode that deliberately promotes: an inline style
                        // outranks author rules where the attribute sat below
                        // them, which is documented as the one setting that can
                        // change a page the book was getting right.
                        Presentation::Preserve => Rehouse::Inline,
                        Presentation::Strip => Rehouse::Drop,
                        // Already overridden, so the attribute has not been
                        // doing anything for as long as the book has existed
                        // and deleting it cannot show.
                        Presentation::Faithful if overridden => Rehouse::Drop,
                        // Otherwise reproduce it where it stood. This used to
                        // write an inline style, which put the declaration
                        // *above* every author rule when the attribute had been
                        // below them — and leaned on `sheet.declares` being
                        // right about the overriding, which it cannot always be:
                        // it reads `.css` entries only, so a <style> block in
                        // the document is invisible to it, as is any `#id` rule.
                        // A layered rule needs no such guess.
                        Presentation::Faithful => match head_start {
                            Some(_) => Rehouse::Layer,
                            None => Rehouse::Keep,
                        },
                    };

                    if action == Rehouse::Keep {
                        homeless.push(attr.name.clone());
                        continue;
                    }
                    edits.delete(attr.span_with_space.clone());
                    bump(&attr.name);
                    match action {
                        Rehouse::Inline => {
                            carried.push(declaration);
                            converted += 1;
                        }
                        Rehouse::Layer => {
                            let template = format!("{{token}} {{ {declaration}; }}");
                            let (token, new_rule) =
                                class_for(&mut tokens, &mut serial, "epubfix-hint", &template);
                            if let Some(new_rule) = new_rule {
                                rules.push(new_rule);
                            }
                            let class = token.trim_start_matches('.');
                            if !classes[i].iter().any(|c| c == class) {
                                classes[i].push(class.to_string());
                            }
                            converted += 1;
                        }
                        Rehouse::Drop => {
                            if !overridden {
                                unoverridden.push(attr.name.clone());
                            }
                        }
                        Rehouse::Keep => unreachable!("handled above"),
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

            for (i, names) in classes
                .iter()
                .enumerate()
                .filter(|(_, names)| !names.is_empty())
            {
                add_generated_class(&mut edits, &nodes[i], &names.join(" "));
            }
            if let Some(at) = head_start
                && !rules.is_empty()
            {
                let css: Vec<&str> = rules.iter().map(String::as_str).collect();
                edits.insert(at, style_block("presentation", &css, true));
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
        if !inert_tally.is_empty() {
            let total: usize = inert_tally.iter().map(|(_, c)| c).sum();
            inert_tally.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let detail: Vec<String> = inert_tally
                .iter()
                .map(|(n, c)| format!("{n}x{c}"))
                .collect();
            outcome.push_change(format!(
                "removed {total} attribute(s) that could not have been doing anything [{}]",
                detail.join(", ")
            ));
        }
        if !homeless.is_empty() {
            outcome.push_finding(format!(
                "{} attribute(s) could be converted but sit in a document with no <head> to hold \
                 the rule, so they were left in place [{}]",
                homeless.len(),
                summarise(&homeless)
            ));
        }
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
                "moved {converted} legacy attribute(s) into generated CSS and stripped {} that a \
                 stylesheet already overrode, for EPUB {version} [{detail}]",
                total - converted
            ),
            Presentation::Faithful if converted > 0 => format!(
                "moved {total} legacy attribute(s) into generated CSS for EPUB {version} \
                 [{detail}]"
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
            "{} attribute(s) have no conversion implemented and were left in place rather than \
             deleted [{}]",
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

/// RSC-005: `value of attribute "width" is invalid; must be an integer`.
///
/// HTML5 keeps `width` and `height` on `<img>` — unlike the table attributes
/// above, they are not obsolete — but narrows the value to a bare integer count
/// of CSS pixels. XHTML 1.1 accepted a *length*, percentages included, and a
/// Harper Collins *Hobbit* uses `<img width="100%"/>` throughout. Legal as EPUB
/// 2, an error the moment the book is retagged.
///
/// Stripping is not an option here and that is the whole reason this is separate
/// from [`LegacyTableAttrs`]. `width="100%"` is doing real work — the image
/// fills the column — and removing it drops the picture back to its natural
/// size, which is a visible change on a page nobody asked to change. So the
/// value always moves to inline CSS, whatever `--strip-presentation` says: there
/// the choice was between two valid renderings, and here it is between keeping
/// the layout and losing it.
///
/// A bare integer is already correct and is left exactly as it is.
pub struct ImageDimensions;

impl Fixer for ImageDimensions {
    fn name(&self) -> &'static str {
        "image-dimensions"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "move a non-integer <img> width or height into CSS, where it is still valid"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() < 3 {
            return outcome;
        }
        let (mut moved, mut trimmed) = (0u32, 0u32);

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in nodes.iter().filter(|n| {
                matches!(n.name.as_str(), "img" | "object")
                    && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
            }) {
                let mut declarations: Vec<String> = Vec::new();
                for attr in node
                    .attrs
                    .iter()
                    .filter(|a| matches!(a.name.as_str(), "width" | "height"))
                {
                    let value = attr.value.trim();
                    // Already what HTML5 asks for.
                    if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    // `196px` says exactly what `196` says — the attribute's
                    // unit *is* the CSS pixel — so the unit simply comes off and
                    // the value stays an attribute. Moving it to CSS would work
                    // too and would be the larger edit for no gain. A Kodansha
                    // *Wild Sheep Chase* writes every one of its rules and
                    // ornaments this way, 220 of them.
                    if let Some(px) = strip_px(value) {
                        edits.replace(attr.span.clone(), format!("{}=\"{px}\"", attr.name));
                        trimmed += 1;
                        continue;
                    }
                    let Some(css) = length(value) else {
                        outcome.push_finding(format!(
                            "{}: <{}> has {}=\"{value}\", which HTML5 needs to be a whole number \
                             of pixels and which is not a length anything can convert",
                            basename(&doc),
                            node.name,
                            attr.name
                        ));
                        continue;
                    };
                    declarations.push(format!("{}: {css}", attr.name));
                    edits.delete(attr.span_with_space.clone());
                    moved += 1;
                }
                merge_style(&mut edits, node, &declarations);
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if trimmed > 0 {
            outcome.push_change(format!(
                "dropped the redundant \"px\" from {trimmed} <img> width/height attribute(s), \
                 which HTML5 needs to be a whole number of pixels"
            ));
        }
        if moved > 0 {
            outcome.push_change(format!(
                "moved {moved} <img> width/height value(s) into CSS, which HTML5 needs to be a \
                 whole number of pixels on the attribute"
            ));
        }
        outcome
    }
}

/// `196px` -> `196`, when that is all the value is.
fn strip_px(value: &str) -> Option<&str> {
    let digits = value.get(..value.len().checked_sub(2)?)?;
    value[value.len() - 2..]
        .eq_ignore_ascii_case("px")
        .then_some(digits)
        .filter(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
}
