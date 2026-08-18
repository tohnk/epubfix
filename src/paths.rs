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
//! | 4 | the unique entry ending with the longest run of the written path | the file moved |
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
//! Candidates 4 and 5 require the match to be **unique**, and they match on as
//! much of the written path as they can rather than on the filename alone. That
//! matters whenever a book keeps two copies of a picture: with
//! `OEBPS/assets/Images/plate.jpg` and `OEBPS/Thumbs/plate.jpg` both present, a
//! reference to `Images/plate.jpg` names one of them unambiguously, and a
//! filename-only match would throw that away and report a tie.
//!
//! Suffixes are tried longest-first, and a suffix only matches on segment
//! boundaries — `es/plate.jpg` is not a suffix of `Images/plate.jpg`. Because a
//! shorter suffix always matches at least as many files as a longer one, the
//! first length that matches anything is also the most specific, and if *it* is
//! ambiguous every shorter one is too. So the search stops there and reports.

use std::collections::{HashMap, HashSet};

use crate::book::Book;
use crate::refs::resolve_href;
use crate::util::dirname;

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

/// True if `name`'s trailing path segments are exactly `want`.
///
/// Compared from the right with `rsplit`, so nothing is allocated. The ladder in
/// [`Resolver::resolve`] asks this of candidate entries on every rung, and
/// splitting each name into a fresh `Vec` there was the most expensive thing
/// this module did — a book with 800 entries and 800 broken references spent a
/// fifth of a second in release builds doing nothing else.
fn suffix_matches(name: &str, want: &[&str], fold: bool) -> bool {
    let mut have = name.rsplit('/');
    for w in want.iter().rev() {
        // Fewer segments than asked for: this cannot be a match.
        let Some(h) = have.next() else {
            return false;
        };
        let same = if fold {
            h.eq_ignore_ascii_case(w)
        } else {
            h == *w
        };
        if !same {
            return false;
        }
    }
    true
}

/// Everything needed to resolve a reference, gathered once per book.
pub struct Resolver {
    names: Vec<String>,
    /// Membership, for the question every reference asks first. A linear scan of
    /// every entry answered it before.
    lookup: HashSet<String>,
    /// `lowercased last segment -> indexes into names`.
    ///
    /// Every rung of the suffix ladder requires the *last* segment to match, so
    /// this narrows the candidates from "every entry in the archive" to "the
    /// entries that could possibly match" before a single comparison is made.
    /// Keyed lowercased because the ladder's second pass ignores case; the exact
    /// pass still verifies case through [`suffix_matches`].
    by_leaf: HashMap<String, Vec<usize>>,
    opf_dir: String,
}

impl Resolver {
    pub fn new(book: &Book) -> Self {
        let names = book.names().to_vec();
        let lookup: HashSet<String> = names.iter().cloned().collect();
        let mut by_leaf: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, n) in names.iter().enumerate() {
            by_leaf
                .entry(crate::util::basename(n).to_ascii_lowercase())
                .or_default()
                .push(i);
        }
        Resolver {
            names,
            lookup,
            by_leaf,
            opf_dir: book.opf_name().map(dirname).unwrap_or_default().to_string(),
        }
    }

    fn exists(&self, name: &str) -> bool {
        self.lookup.contains(name)
    }

    /// Entries whose path ends with `want`, matched on segment boundaries, in
    /// archive order.
    fn by_suffix(&self, want: &[&str], fold: bool) -> Vec<&String> {
        // The caller always asks for at least one segment; an empty request
        // would match every entry, which is never what is meant.
        let Some(leaf) = want.last() else {
            return Vec::new();
        };
        let Some(candidates) = self.by_leaf.get(&leaf.to_ascii_lowercase()) else {
            return Vec::new();
        };
        candidates
            .iter()
            .map(|&i| &self.names[i])
            .filter(|n| suffix_matches(n, want, fold))
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

        // The file moved. Match on as much of the written path as still exists
        // somewhere, longest first, exactly before ignoring case — a book whose
        // manifest says `Styles/` and whose markup says `styles/` is common, but
        // the fold must not mask a genuine pair of files differing only in case.
        let segments: Vec<&str> = literal.split('/').collect();
        for fold in [false, true] {
            for k in (1..=segments.len()).rev() {
                let matches = self.by_suffix(&segments[segments.len() - k..], fold);
                match matches.as_slice() {
                    [] => {}
                    [only] => {
                        return Resolution::Moved {
                            target: (*only).clone(),
                        };
                    }
                    // A shorter suffix can only match more files, so there is
                    // nothing better further down.
                    many => return Resolution::Ambiguous { count: many.len() },
                }
            }
        }

        Resolution::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::{relative_to, suffix_matches};

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

    /// The suffix rule the whole ladder rests on: whole segments only, never a
    /// substring. `es/plate.jpg` must not be read as a suffix of
    /// `Images/plate.jpg`, or a reference would resolve to a file that merely
    /// shares the tail of a directory name.
    #[test]
    fn a_suffix_only_matches_on_segment_boundaries() {
        assert!(suffix_matches(
            "OEBPS/Images/plate.jpg",
            &["plate.jpg"],
            false
        ));
        assert!(suffix_matches(
            "OEBPS/Images/plate.jpg",
            &["Images", "plate.jpg"],
            false
        ));
        assert!(suffix_matches(
            "OEBPS/Images/plate.jpg",
            &["OEBPS", "Images", "plate.jpg"],
            false
        ));
        // A partial segment is not a segment.
        assert!(!suffix_matches(
            "OEBPS/Images/plate.jpg",
            &["es", "plate.jpg"],
            false
        ));
        // Asking for more segments than the entry has.
        assert!(!suffix_matches(
            "plate.jpg",
            &["Images", "plate.jpg"],
            false
        ));
        assert!(suffix_matches("plate.jpg", &["plate.jpg"], false));
    }

    /// Case folding is the ladder's second pass and must stay opt-in, so a book
    /// holding two files differing only in case is still reported as a tie
    /// rather than silently resolved to whichever came first.
    #[test]
    fn case_folding_is_only_applied_when_asked_for() {
        assert!(!suffix_matches(
            "OEBPS/Styles/s.css",
            &["styles", "s.css"],
            false
        ));
        assert!(suffix_matches(
            "OEBPS/Styles/s.css",
            &["styles", "s.css"],
            true
        ));
        assert!(!suffix_matches(
            "OEBPS/Styles/s.css",
            &["styles", "S.CSS"],
            false
        ));
        assert!(suffix_matches(
            "OEBPS/Styles/s.css",
            &["styles", "S.CSS"],
            true
        ));
    }
}
