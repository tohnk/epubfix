//! Working out what a broken reference was *meant* to point at.
//!
//! There are several ways a path can be wrong and only one way it can be right:
//! the file it names has to be in the archive. So rather than special-casing
//! each kind of mistake, this generates the paths the reference could plausibly
//! have meant and lets **existence in the archive** decide. The first candidate
//! that exists wins; if none does, the caller is told so and decides what to do
//! with the referencing construct.
//!
//! The ladder, in order:
//!
//! | | Candidate | The mistake it catches |
//! | --- | --- | --- |
//! | 1 | relative to the referring file | none — this is the correct reading |
//! | 2 | relative to the archive root | an author treating a relative URL as root-relative |
//! | 3 | relative to the package document | the same, anchored on the OPF instead |
//! | 4 | the unique entry with that basename | the file moved |
//! | 5 | as 4, ignoring case | `styles/` written for `Styles/` |
//!
//! Candidate 2 is *Butcher's Crossing*: `OEBPS/Styles/nyrb.css` contains
//! `url(OEBPS/Fonts/AGaramondPro-Regular.otf)`, which read correctly resolves
//! to `OEBPS/Styles/OEBPS/Fonts/…` — the doubled directory epubcheck reports.
//! CSS resolves against the stylesheet, not the document that links it, which
//! is exactly the rule the author got wrong.
//!
//! Note that the root is taken from the archive rather than assumed: this
//! library has `OPS/`, `ops/`, `OEBPS/html/` and `CompletePoems/` as package
//! directories. `OEBPS` is a convention, not a rule.
//!
//! Candidates 4 and 5 require the match to be **unique**. Two files with the
//! same basename and no way to choose between them is a report, not a guess.

use crate::book::Book;
use crate::refs::resolve_href;
use crate::util::{basename, dirname};

/// What became of one reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// It already points at a file that exists. Nothing to do.
    Fine,
    /// Not a container reference at all — an external URL, a `data:` URI, an
    /// empty href. Out of scope rather than broken.
    NotOurs,
    /// It resolves nowhere, but exactly one candidate exists.
    Moved { target: String },
    /// Nothing plausible exists.
    Missing,
    /// Several files match and there is no way to tell which was meant.
    Ambiguous { count: usize },
}

/// Express `target` as a path relative to the directory holding `from`.
pub fn relative_to(from: &str, target: &str) -> String {
    let from_dir: Vec<&str> = from.split('/').collect();
    let to: Vec<&str> = target.split('/').collect();
    let shared = from_dir
        .iter()
        .take(from_dir.len() - 1)
        .zip(to.iter().take(to.len() - 1))
        .take_while(|(a, b)| a == b)
        .count();
    let ups = from_dir.len() - 1 - shared;
    let mut out = "../".repeat(ups);
    out.push_str(&to[shared..].join("/"));
    out
}

/// Everything needed to resolve a reference, gathered once per book.
pub struct Resolver {
    names: Vec<String>,
    opf_dir: String,
}

impl Resolver {
    pub fn new(book: &Book) -> Self {
        Resolver {
            names: book.names().to_vec(),
            opf_dir: book.opf_name().map(dirname).unwrap_or_default().to_string(),
        }
    }

    fn exists(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// Entries whose basename matches, optionally ignoring case.
    fn by_basename(&self, want: &str, fold: bool) -> Vec<&String> {
        self.names
            .iter()
            .filter(|n| {
                let b = basename(n);
                if fold {
                    b.eq_ignore_ascii_case(want)
                } else {
                    b == want
                }
            })
            .collect()
    }

    /// Where the reference `raw`, written inside `from`, actually points.
    pub fn resolve(&self, from: &str, raw: &str) -> Resolution {
        let Some((literal, _)) = resolve_href(from, raw) else {
            return Resolution::NotOurs;
        };
        if self.exists(&literal) {
            return Resolution::Fine;
        }

        // The path as written, read from the archive root and from the package
        // directory. Only meaningful for a relative path — one already anchored
        // at the root has no second reading.
        let written = raw.split(['#', '?']).next().unwrap_or(raw).trim();
        if !written.starts_with('/') && !written.starts_with("..") {
            for base in ["", self.opf_dir.as_str()] {
                let Some((candidate, _)) = resolve_href(&format!("{base}x"), written) else {
                    continue;
                };
                if candidate != literal && self.exists(&candidate) {
                    return Resolution::Moved { target: candidate };
                }
            }
        }

        // The file moved. Match on the name alone, exactly first, then ignoring
        // case — a book whose manifest says `Styles/` and whose markup says
        // `styles/` is common, and the fold must not mask a genuine pair of
        // files that differ only in case.
        for fold in [false, true] {
            match self.by_basename(basename(&literal), fold).as_slice() {
                [only] => {
                    return Resolution::Moved {
                        target: (*only).clone(),
                    };
                }
                many if many.len() > 1 => {
                    return Resolution::Ambiguous { count: many.len() };
                }
                _ => {}
            }
        }

        Resolution::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::relative_to;

    #[test]
    fn relative_paths_climb_only_as_far_as_needed() {
        assert_eq!(
            relative_to("OEBPS/Text/a.xhtml", "OEBPS/Text/b.xhtml"),
            "b.xhtml"
        );
        assert_eq!(
            relative_to("OEBPS/Text/a.xhtml", "OEBPS/Styles/s.css"),
            "../Styles/s.css"
        );
        assert_eq!(
            relative_to("OEBPS/a.xhtml", "OEBPS/Styles/s.css"),
            "Styles/s.css"
        );
        assert_eq!(relative_to("a.xhtml", "b.xhtml"), "b.xhtml");
    }
}
