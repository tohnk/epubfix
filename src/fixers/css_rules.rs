//! CSS-001: properties an EPUB style sheet may not contain.
//!
//! EPUB 3 does not accept every valid CSS property. Two are prohibited outright,
//! because the reading system needs to control them: `direction` and
//! `unicode-bidi` decide how bidirectional text is laid out, and EPUB requires
//! that to be expressed in the *markup* — `dir="rtl"` on the element — where a
//! reading system can see it without parsing a stylesheet.
//!
//! EPUB 2 does not check, which is what makes this the largest single cause of
//! `--migrate-epub3` leaving a book worse than it found it: a Calibre stylesheet
//! writes `direction: ltr` beside every text rule, and the moment the book is
//! retagged, every one of them is an error. A Penguin *1984* has 51 and reaches
//! 0 errors on removing them alone.

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::Edits;
use crate::util::{basename, ends_with_any, re};

/// A prohibited property, and the value that makes removing it a no-op.
///
/// Both entries are the CSS *initial* value: the one every element already has
/// unless something says otherwise. A declaration setting a property to the
/// value it would have anyway changes nothing on the page, which is what makes
/// deleting it invisible rather than merely convenient.
///
/// Measured, across the library: every single occurrence is the initial value.
/// 51 in *1984* and 7 in *The Well of Ascension*, all `direction: ltr`, and not
/// one `rtl` anywhere. That is what a converter emitting boilerplate looks like.
const PROHIBITED: &[(&str, &str)] = &[("direction", "ltr"), ("unicode-bidi", "normal")];

/// One `property: value` declaration, with the separator that ends it.
static DECLARATION: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(^|[;{\s])(direction|unicode-bidi)\s*:\s*([^;}]*)\s*(;?)"));

pub struct ProhibitedProperties;

impl Fixer for ProhibitedProperties {
    fn name(&self) -> &'static str {
        "css-prohibited"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["CSS-001"]
    }
    fn description(&self) -> &'static str {
        "remove the bidi properties an EPUB style sheet may not contain"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        // EPUB 2 does not check, and a declaration that is not an error is not
        // this tool's to delete.
        if book.epub_version() < 3 {
            return outcome;
        }
        let mut removed = 0u32;

        for name in book.names().to_vec() {
            if !ends_with_any(&name, &[".css"]) {
                continue;
            }
            let Some(text) = book.text(&name).map(str::to_owned) else {
                continue;
            };
            let mut edits = Edits::new();

            for c in DECLARATION.captures_iter(&text) {
                let property = c[2].to_ascii_lowercase();
                let value = c[3].trim().to_ascii_lowercase();
                let initial = PROHIBITED
                    .iter()
                    .find(|(p, _)| *p == property)
                    .map(|(_, v)| *v);
                if initial != Some(value.as_str()) {
                    // A meaningful value: the layout depends on it, and the
                    // repair is to move it onto the elements as a `dir`
                    // attribute — which needs knowing what the selector matches.
                    outcome.push_finding(format!(
                        "{}: \"{property}: {value}\" is not allowed in an EPUB style sheet, and \
                         it is not the default either, so the text depends on it; it needs \
                         moving onto the elements as dir=\"{value}\" by hand",
                        basename(&name)
                    ));
                    continue;
                }
                // Keep whatever opened the declaration; drop the rest.
                let whole = c.get(0).unwrap().range();
                let keep = c.get(1).map_or(0, |m| m.len());
                edits.delete(whole.start + keep..whole.end);
                removed += 1;
            }

            if !edits.is_empty() {
                book.set_text(&name, edits.apply(&text));
            }
        }

        if removed > 0 {
            outcome.push_change(format!(
                "removed {removed} bidi declaration(s) an EPUB style sheet may not hold, \
                 every one of them setting the value the element already had"
            ));
        }
        outcome
    }
}
