//! Just enough CSS to answer one question: does the book's own stylesheet
//! already set this property on this element?
//!
//! That question decides whether stripping a presentational attribute is
//! invisible or not. Presentational attributes contribute at the
//! presentational-hints origin, below author stylesheets: if an author rule
//! sets the property the attribute was doing nothing and removing it changes
//! nothing on screen, and if no rule does, removing it falls back to the user
//! agent default and the page may shift.
//!
//! This is an approximation and is meant to be. A real answer needs the whole
//! cascade — specificity, inheritance, media queries, the lot — and would be a
//! browser. What is here is a selector scan for element names and class names,
//! which is what Calibre-generated books use and essentially all they use:
//! `.calibre7 { vertical-align: middle }` against `class="calibre7"`.
//!
//! Two callers do let it change bytes, and both are arranged so that being
//! wrong costs nothing visible. The presentational-attribute pass uses it to
//! choose between deleting an attribute and reproducing it at the same place in
//! the cascade — never between two different renderings. The `<li value>` pass
//! asks it whether a list marker is numbered, and treats "cannot tell" as
//! "leave it alone". A wrong answer costs a line of noise or a missed repair,
//! not a changed page.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::util::re;

/// One `selector { declarations }` rule. Nothing here understands nesting or
/// at-rules; `@media` blocks are handled by the brace-skipping in [`Stylesheet::parse`].
static RULE_RE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)([^{}]+)\{([^{}]*)\}"));
/// A class in a selector, e.g. the `calibre7` of `p.calibre7 > td`.
static CLASS_RE: LazyLock<Regex> = LazyLock::new(|| re(r"\.(-?[_a-zA-Z][-_a-zA-Z0-9]*)"));
/// A bare element name at the start of a compound selector.
static ELEMENT_RE: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[\s>+~,])([a-zA-Z][a-zA-Z0-9]*)"));
/// A `property: value` declaration.
static DECL_RE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)([-a-zA-Z]+)\s*:([^;]*)"));

/// Which (class, property) and (element, property) pairs the book declares, and
/// with what values.
///
/// The values are kept because one question needs them: whether an `<li value>`
/// can show. It only can where the list marker is numbered, so `list-style-type:
/// disc` and `list-style-type: decimal` have to be told apart — "is the property
/// declared" answers both the same way and would leave every styled list alone.
#[derive(Debug, Default)]
pub struct Stylesheet {
    by_class: HashMap<(String, String), Vec<String>>,
    by_element: HashMap<(String, String), Vec<String>>,
}

impl Stylesheet {
    /// Fold one stylesheet's text into the index.
    pub fn parse(&mut self, css: &str) {
        for rule in RULE_RE.captures_iter(css) {
            let selector = &rule[1];
            // The tail of an at-rule prelude, e.g. the "@media print" left over
            // once its inner rules have matched. It selects nothing itself.
            if selector.trim_start().starts_with('@') {
                continue;
            }
            let props: Vec<(String, String)> = DECL_RE
                .captures_iter(&rule[2])
                .map(|d| (d[1].to_ascii_lowercase(), d[2].trim().to_ascii_lowercase()))
                .collect();
            if props.is_empty() {
                continue;
            }
            for c in CLASS_RE.captures_iter(selector) {
                for (p, v) in &props {
                    self.by_class
                        .entry((c[1].to_string(), p.clone()))
                        .or_default()
                        .push(v.clone());
                }
            }
            for e in ELEMENT_RE.captures_iter(selector) {
                let name = e[1].to_ascii_lowercase();
                for (p, v) in &props {
                    self.by_element
                        .entry((name.clone(), p.clone()))
                        .or_default()
                        .push(v.clone());
                }
            }
        }
    }

    /// Every value any rule that could match this element gives `property`.
    ///
    /// No cascade is applied — these are all the candidates, in no order. A
    /// caller that needs a yes/no answer should be asking something true of
    /// *all* of them, not of the first.
    pub fn declared_values(&self, element: &str, classes: &str, property: &str) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        if let Some(vs) = self
            .by_element
            .get(&(element.to_string(), property.to_string()))
        {
            out.extend(vs.iter().map(String::as_str));
        }
        for c in classes.split_whitespace() {
            if let Some(vs) = self.by_class.get(&(c.to_string(), property.to_string())) {
                out.extend(vs.iter().map(String::as_str));
            }
        }
        out
    }

    /// True if some rule that could match this element sets `property`.
    ///
    /// Shorthands count: `margin` covers `margin-top`, and `background` covers
    /// `background-color`. Missing that would report an override as absent
    /// whenever a stylesheet used the shorthand, which is most of the time.
    pub fn declares(&self, element: &str, classes: &str, property: &str) -> bool {
        let shorthand = property.split_once('-').map(|(head, _)| head);
        let mut wanted = vec![property.to_string()];
        if let Some(s) = shorthand {
            wanted.push(s.to_string());
        }
        if property == "background-color" {
            wanted.push("background".to_string());
        }

        wanted.iter().any(|p| {
            self.by_element
                .contains_key(&(element.to_string(), p.clone()))
                || classes
                    .split_whitespace()
                    .any(|c| self.by_class.contains_key(&(c.to_string(), p.clone())))
        })
    }

    /// True if any rule names `element` in its selector, whatever the property.
    ///
    /// The other question this module answers is about *removing* a
    /// declaration. This one is about *retagging*: a fixer that renames a
    /// `<span>` to a `<div>` keeps the id, the class and the content, but every
    /// selector that reached the element by its element name — `span`,
    /// `.foo span`, `span.bar` — stops matching it. Class-only selectors are
    /// unaffected, which is why the question is about the element name alone.
    pub fn names_element(&self, element: &str) -> bool {
        self.by_element.keys().any(|(e, _)| e == element)
    }

    pub fn is_empty(&self) -> bool {
        self.by_class.is_empty() && self.by_element.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::Stylesheet;

    fn sheet(css: &str) -> Stylesheet {
        let mut s = Stylesheet::default();
        s.parse(css);
        s
    }

    #[test]
    fn class_and_element_rules_are_both_found() {
        let s = sheet(".calibre7 { vertical-align: middle } td { padding: 0 }");
        assert!(s.declares("tr", "calibre7", "vertical-align"));
        assert!(s.declares("td", "", "padding"));
        assert!(!s.declares("tr", "other", "vertical-align"));
        assert!(!s.declares("td", "", "vertical-align"));
    }

    #[test]
    fn a_shorthand_counts_as_setting_its_longhands() {
        let s = sheet(".x { margin: 0 } .y { background: red }");
        assert!(s.declares("p", "x", "margin-left"));
        assert!(s.declares("p", "y", "background-color"));
    }

    #[test]
    fn compound_and_descendant_selectors_are_picked_apart() {
        let s = sheet("table.calibre1 > tbody td.cell, p.lead { text-align: center }");
        assert!(s.declares("td", "cell", "text-align"));
        assert!(s.declares("p", "lead", "text-align"));
        assert!(s.declares("table", "", "text-align"));
    }

    #[test]
    fn rules_inside_a_media_block_still_count() {
        let s = sheet("@media print { .x { vertical-align: top } }");
        assert!(s.declares("td", "x", "vertical-align"));
    }

    /// Retagging asks a different question from stripping: not "is this
    /// property already set", but "does anything reach this element by name at
    /// all". A class-only rule follows the element through a rename; an
    /// element-named one does not.
    #[test]
    fn an_element_named_in_any_selector_is_reported_as_targeted() {
        let s = sheet(".smallcaps { font-variant: small-caps } div.wrap p { margin: 0 }");
        assert!(!s.names_element("span"), "only a class rule mentions it");
        assert!(s.names_element("div"));
        assert!(s.names_element("p"));

        for css in [
            "span { font-variant: small-caps }",
            ".foo span { color: red }",
            "span.bar { color: red }",
            "p, span { color: red }",
        ] {
            assert!(sheet(css).names_element("span"), "missed: {css}");
        }
    }

    #[test]
    fn an_empty_sheet_declares_nothing() {
        assert!(sheet("").is_empty());
        assert!(!sheet("").declares("td", "x", "width"));
    }
}
