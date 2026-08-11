//! RSC-007 inside stylesheets.
//!
//! A scanner that only opens markup never sees these at all, and they are not
//! rare: *Butcher's Crossing* ships two `@font-face` rules whose `src` doubles
//! the package directory, which is two of that book's three errors.
//!
//! What to delete when nothing resolves depends on the construct, because the
//! unit that becomes meaningless differs:
//!
//! | Construct | Action |
//! | --- | --- |
//! | `@font-face` | delete the whole at-rule — a face with no source is nothing |
//! | `@import` | delete the statement |
//! | any other `url()` | delete only that declaration, not the rule around it |
//!
//! Deleting a dead `@font-face` is behaviour-preserving, and provably so rather
//! than probably: the font could never load, so whatever the rules using that
//! family fall back to is what has been rendering all along. In *Butcher's
//! Crossing* every such rule says `"Adobe Garamond Pro", serif`, and `serif` is
//! what the reader has always seen.

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::Edits;
use crate::paths::{Resolution, Resolver, relative_to};
use crate::util::{basename, ends_with_any, re};

/// A `url(...)` argument, with its quotes if it has any.
static URL_RE: LazyLock<Regex> = LazyLock::new(|| re(r"url\(\s*([^)]*?)\s*\)"));
/// An `@import` statement, in either the `url(...)` or the bare-string form.
static IMPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?s)@import\s+(?:url\(\s*([^)]*?)\s*\)|("[^"]*"|'[^']*'))[^;]*;"#));

/// Strip one level of matching quotes.
fn unquote(v: &str) -> &str {
    let v = v.trim();
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// The span of the at-rule whose *body* contains `at`, given the byte offset of
/// its `@`.
///
/// Counts braces forward from the prelude, so a nested block inside the rule
/// cannot end it early.
fn at_rule_span(css: &str, start: usize) -> Option<std::ops::Range<usize>> {
    let bytes = css.as_bytes();
    let open = css[start..].find('{')? + start;
    let mut depth = 0i32;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    // Take the trailing newline with it, so removing a rule does
                    // not leave a blank line behind.
                    let end = css[i + 1..]
                        .find('\n')
                        .filter(|n| css[i + 1..i + 1 + n].trim().is_empty())
                        .map_or(i + 1, |n| i + 1 + n + 1);
                    return Some(start..end);
                }
            }
            _ => {}
        }
    }
    None
}

/// The `@font-face` rule containing byte offset `at`, if there is one.
fn enclosing_font_face(css: &str, at: usize) -> Option<std::ops::Range<usize>> {
    let start = css[..at].rfind("@font-face")?;
    let span = at_rule_span(css, start)?;
    span.contains(&at).then_some(span)
}

/// The single declaration (`prop: value;`) containing byte offset `at`.
///
/// Bounded by the surrounding braces so it can never run past its own rule.
fn declaration_span(css: &str, at: usize) -> std::ops::Range<usize> {
    let start = css[..at].rfind([';', '{']).map_or(0, |i| i + 1);
    let end = css[at..].find(';').map_or_else(
        || css[at..].find('}').map_or(css.len(), |i| at + i),
        |i| at + i + 1,
    );
    start..end
}

pub struct CssPaths;

impl Fixer for CssPaths {
    fn name(&self) -> &'static str {
        "css-paths"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-007"]
    }
    fn description(&self) -> &'static str {
        "repoint stylesheet url()/@import references, and drop the ones with no file behind them"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let resolver = Resolver::new(book);
        let mut outcome = Outcome::none();
        let (mut repointed, mut faces, mut imports, mut decls) = (0u32, 0u32, 0u32, 0u32);

        for name in book.names().to_vec() {
            if !ends_with_any(&name, &[".css"]) {
                continue;
            }
            let Some(css) = book.text(&name).map(str::to_owned) else {
                continue;
            };
            let mut edits = Edits::new();
            // At-rules already condemned, so a second url() inside the same
            // @font-face does not try to delete it twice.
            let mut dropped: Vec<std::ops::Range<usize>> = Vec::new();

            // @import first: its url() would otherwise be handled as a plain
            // declaration, which is the wrong unit to remove.
            for m in IMPORT_RE.captures_iter(&css) {
                let Some(arg) = m.get(1).or_else(|| m.get(2)) else {
                    continue;
                };
                let raw = unquote(arg.as_str());
                match resolver.resolve(&name, raw) {
                    Resolution::Moved { target } => {
                        edits.replace(arg.range(), format!("\"{}\"", relative_to(&name, &target)));
                        repointed += 1;
                    }
                    Resolution::Missing => {
                        edits.delete(m.get(0).expect("group 0").range());
                        imports += 1;
                    }
                    Resolution::Ambiguous { count } => outcome.push_finding(format!(
                        "{}: @import \"{raw}\" matches {count} files, so there is no way to \
                         tell which was meant",
                        basename(&name)
                    )),
                    Resolution::Fine | Resolution::NotOurs => {}
                }
            }
            let imported: Vec<_> = IMPORT_RE.find_iter(&css).map(|m| m.range()).collect();

            for m in URL_RE.captures_iter(&css) {
                let arg = m.get(1).expect("group 1 always matches");
                if imported.iter().any(|r| r.contains(&arg.start())) {
                    continue;
                }
                let raw = unquote(arg.as_str());
                match resolver.resolve(&name, raw) {
                    Resolution::Moved { target } => {
                        edits.replace(arg.range(), format!("\"{}\"", relative_to(&name, &target)));
                        repointed += 1;
                    }
                    Resolution::Missing => {
                        if let Some(face) = enclosing_font_face(&css, arg.start()) {
                            if !dropped.iter().any(|d| d.start == face.start) {
                                edits.delete(face.clone());
                                dropped.push(face);
                                faces += 1;
                            }
                        } else {
                            edits.delete(declaration_span(&css, arg.start()));
                            decls += 1;
                        }
                    }
                    Resolution::Ambiguous { count } => outcome.push_finding(format!(
                        "{}: url({raw}) matches {count} files, so there is no way to tell \
                         which was meant",
                        basename(&name)
                    )),
                    Resolution::Fine | Resolution::NotOurs => {}
                }
            }

            if !edits.is_empty() {
                book.set_text(&name, edits.apply(&css));
            }
        }

        if repointed > 0 {
            outcome.push_change(format!(
                "repointed {repointed} stylesheet reference(s) at the file they meant"
            ));
        }
        if faces > 0 {
            outcome.push_change(format!(
                "removed {faces} @font-face rule(s) whose font is not in the book, leaving \
                 the fallback that was already rendering"
            ));
        }
        if imports > 0 {
            outcome.push_change(format!("removed {imports} @import of a missing stylesheet"));
        }
        if decls > 0 {
            outcome.push_change(format!(
                "removed {decls} declaration(s) pointing at a file that is not in the book"
            ));
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::{at_rule_span, declaration_span, enclosing_font_face, unquote};

    #[test]
    fn quotes_come_off_either_kind() {
        assert_eq!(unquote(r#" "a.otf" "#), "a.otf");
        assert_eq!(unquote("'a.otf'"), "a.otf");
        assert_eq!(unquote("a.otf"), "a.otf");
    }

    #[test]
    fn an_at_rule_spans_to_its_matching_brace() {
        let css = "@font-face {\n  src: url(x.otf);\n}\np { color: red }\n";
        let span = at_rule_span(css, 0).unwrap();
        assert_eq!(&css[span.clone()], "@font-face {\n  src: url(x.otf);\n}\n");
        assert!(css[span.end..].starts_with("p {"));
    }

    #[test]
    fn a_url_finds_the_font_face_around_it_and_not_one_before_it() {
        let css = "@font-face { src: url(a.otf); }\np { background: url(b.png) }\n";
        let a = css.find("a.otf").unwrap();
        let b = css.find("b.png").unwrap();
        assert!(enclosing_font_face(css, a).is_some());
        assert!(
            enclosing_font_face(css, b).is_none(),
            "the earlier @font-face has already closed"
        );
    }

    #[test]
    fn a_declaration_stops_at_its_own_semicolon_and_braces() {
        let css = "p { color: red; background: url(x.png); margin: 0 }";
        let at = css.find("x.png").unwrap();
        assert_eq!(&css[declaration_span(css, at)], " background: url(x.png);");

        let last = "p { background: url(x.png) }";
        let at = last.find("x.png").unwrap();
        assert_eq!(
            &last[declaration_span(last, at)],
            " background: url(x.png) "
        );
    }
}
