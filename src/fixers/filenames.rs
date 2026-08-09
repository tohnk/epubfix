//! PKG-009 / PKG-010 / RSC-020: resource filenames a reading system cannot
//! address.
//!
//! The entry is renamed on repack and every textual reference to the old
//! basename — raw or percent-encoded — is pointed at the new one. That is a
//! large, sprawling edit: one rename rewrites the manifest, the spine, and
//! every link in every document that mentions the file. So the bar for doing
//! it has to be an error that actually exists.
//!
//! It was not. This fixer inherited its idea of an "unsafe" name from the
//! Python original, which used `[^A-Za-z0-9_.-]` — a class narrow enough to
//! condemn an apostrophe. On one real book that meant 321 renames for `'` and
//! 42 for `!`, neither of which epubcheck says a single word about: RFC 3986
//! lists both among the sub-delims that are legal unencoded in a path segment.
//! Hundreds of files churned, hundreds of references rewritten, nothing fixed.
//!
//! So the classes below are measured rather than assumed. Each candidate
//! character was put in a filename in an EPUB, referenced both raw and
//! percent-encoded, and run through EPUB Check 5.2.1. Three outcomes fell out,
//! and the difference between the first two decides when a rename is warranted:
//!
//! * [`OCF_FORBIDDEN`] — PKG-009, whatever the reference looks like. The name
//!   itself is illegal, so the only repair is a rename.
//! * [`ENCODE_REQUIRED`] — legal as a name; only the *reference* can be wrong.
//!   `a[b.css` referenced as `a%5Bb.css` is a clean book. Renaming it would be
//!   the same false positive in a smaller size, so these only count when the
//!   book actually spells the name raw somewhere.
//! * Everything else — `! $ & ' ( ) + , - ; = @ ~`, and every non-ASCII
//!   character tried, including accented Latin and CJK — is silent in both
//!   forms and is now left alone.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::util::{basename, dirname};

/// The set `urllib.parse.quote` leaves unescaped: `A-Za-z0-9` and `-._~`.
///
/// Deliberately over-broad, because this is only used to *recognise* an
/// existing encoded reference so it can be repointed. Anything encoded more
/// tightly than a book actually wrote it simply fails to match, which costs a
/// rewrite we would have made; anything looser could match too much.
const QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Characters that are an error in an OCF file name however it is referenced.
///
/// Measured (PKG-009): `"` `*` `:` `<` `>` `?` `\` `|`. Control characters draw
/// the same code and are folded in by [`needs_rename`] rather than listed.
const OCF_FORBIDDEN: &[char] = &['"', '*', ':', '<', '>', '?', '\\', '|'];

/// Characters that are legal in a file name but must be percent-encoded in a
/// URL path.
///
/// Measured: raw in a reference, the bracket-like ones give RSC-020, and `#`
/// and `%` send it somewhere else entirely — `#` splits off a fragment, `%`
/// starts an escape sequence — so the file is reported missing as RSC-001.
/// Percent-encoded, every one of them gives nothing at all.
const ENCODE_REQUIRED: &[char] = &['#', '%', '[', ']', '^', '`', '{', '}'];

/// True if `c` cannot stand in an OCF file name.
///
/// Whitespace covers both halves of the measurement: a tab is PKG-009 as a
/// control character, a space is PKG-010, and either is RSC-020 in a raw
/// reference. `char::is_whitespace` follows the Unicode property, so it also
/// catches the non-breaking space that turns up in names dragged out of word
/// processors.
fn is_forbidden(c: char) -> bool {
    OCF_FORBIDDEN.contains(&c) || c.is_control() || c.is_whitespace()
}

fn needs_encoding(c: char) -> bool {
    ENCODE_REQUIRED.contains(&c)
}

/// Rewrite one path component, replacing only the characters that are actually
/// a problem.
///
/// Note what it no longer does: an accented or CJK stem used to be reduced to
/// underscores, and `_leading_.svg` used to come back as `leading.svg`. Both
/// were losses with nothing bought, so the name now survives everything except
/// the offending characters themselves.
fn safe_filename(base: &str) -> String {
    let (stem, ext) = match base.rfind('.') {
        // Mirrors Python's rpartition('.'): a leading dot is part of the stem.
        Some(i) if i > 0 => (&base[..i], &base[i + 1..]),
        _ => (base, ""),
    };

    let mut s = String::with_capacity(stem.len());
    for c in stem.chars() {
        if is_forbidden(c) || needs_encoding(c) {
            if !s.ends_with('_') {
                s.push('_');
            }
        } else {
            s.push(c);
        }
    }
    if s.is_empty() || s == "_" {
        s = "file".to_string();
    }

    if ext.is_empty() {
        s
    } else {
        format!("{s}.{ext}")
    }
}

/// Whether this entry has to be renamed, and why.
///
/// The second arm is the gate that keeps the encode-required set honest. A book
/// that percent-encodes its references is already valid, so the raw basename
/// never appears in it and nothing happens. A book that spells them raw has a
/// broken reference, and the raw basename is exactly what we would have to find
/// to rewrite it — so the test and the repair look at the same thing.
fn needs_rename(base: &str, raw_reference: impl Fn(&str) -> bool) -> bool {
    base.chars().any(is_forbidden) || (base.chars().any(needs_encoding) && raw_reference(base))
}

pub struct UnsafeFilenames;

impl Fixer for UnsafeFilenames {
    fn name(&self) -> &'static str {
        "filenames"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["PKG-009", "PKG-010", "RSC-020"]
    }
    fn description(&self) -> &'static str {
        "rename resources whose filenames a URL cannot address, and update references"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut planned: Vec<(String, String)> = Vec::new();

        // Snapshot the textual content once: the encode-required gate asks the
        // same question of every candidate, and the answer cannot change while
        // we are only planning.
        let corpus: Vec<&str> = book.names().iter().filter_map(|n| book.text(n)).collect();
        let spelled_raw = |base: &str| corpus.iter().any(|t| t.contains(base));

        for name in book.names() {
            if name.ends_with('/') {
                continue;
            }
            let base = basename(name);
            if base.is_empty() || !needs_rename(base, spelled_raw) {
                continue;
            }
            let dir = dirname(name);
            let mut candidate = format!("{dir}{}", safe_filename(base));
            // Two different originals can sanitise to the same thing, and the
            // sanitised name may already be in use. Either way we must not
            // silently drop an entry by writing two of them to one name.
            if book.name_taken(&candidate) || planned.iter().any(|(_, n)| *n == candidate) {
                candidate = uniquify(&candidate, |c| {
                    book.name_taken(c) || planned.iter().any(|(_, n)| n == c)
                });
            }
            planned.push((name.clone(), candidate));
        }

        if planned.is_empty() {
            return Outcome::none();
        }

        // Longest basename first, so that "ch1 extra.xhtml" is rewritten before a
        // rename of "ch1.xhtml" could chew through the middle of it.
        let mut ordered = planned.clone();
        ordered.sort_by(|a, b| {
            basename(&b.0)
                .len()
                .cmp(&basename(&a.0).len())
                .then_with(|| a.0.cmp(&b.0))
        });

        for (old, new) in &ordered {
            let ob = basename(old);
            let nb = basename(new);
            let quoted = utf8_percent_encode(ob, QUOTE_SET).to_string();
            let mut forms = vec![ob.to_string()];
            if quoted != ob {
                forms.push(quoted);
            }
            book.for_each_text(|_, text| {
                for form in &forms {
                    if text.contains(form.as_str()) {
                        *text = text.replace(form.as_str(), nb);
                    }
                }
            });
        }

        for (old, new) in &planned {
            book.rename(old, new.clone());
        }

        Outcome::change(format!("renamed {} file(s)", planned.len()))
    }
}

/// Append `_2`, `_3`, ... before the extension until `taken` stops complaining.
fn uniquify(candidate: &str, taken: impl Fn(&str) -> bool) -> String {
    let (stem, ext) = match candidate.rfind('.') {
        Some(i) if i > candidate.rfind('/').map_or(0, |s| s + 1) => {
            (&candidate[..i], &candidate[i..])
        }
        _ => (candidate, ""),
    };
    let mut n = 2u32;
    let mut candidate = format!("{stem}_{n}{ext}");
    while taken(&candidate) {
        n += 1;
        candidate = format!("{stem}_{n}{ext}");
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::{needs_rename, safe_filename, uniquify};

    /// The measured PKG-009 set, plus the whitespace and control characters
    /// that draw PKG-009 or PKG-010.
    #[test]
    fn illegal_names_are_renamed_however_they_are_referenced() {
        for base in [
            "a\"b.css",
            "a*b.css",
            "a:b.css",
            "a<b.css",
            "a>b.css",
            "a?b.css",
            "a\\b.css",
            "a|b.css",
            "a b.css",
            "a\tb.css",
            "a\u{a0}b.css",
        ] {
            assert!(
                needs_rename(base, |_| false),
                "{base} is a PKG-009/PKG-010 name and must be renamed even with no raw reference"
            );
        }
    }

    /// The 321-file regression. Every one of these produced no epubcheck
    /// message in either reference form, so no rename is warranted whatever
    /// the book looks like.
    #[test]
    fn legal_names_are_never_renamed() {
        for base in [
            "Don't-Panic.css",
            "hello!.css",
            "a$b.css",
            "a&b.css",
            "a(b).css",
            "a+b.css",
            "a,b.css",
            "a;b.css",
            "a=b.css",
            "a@b.css",
            "a~b.css",
            "Ünïcödé.ttf",
            "中文.xhtml",
        ] {
            assert!(
                !needs_rename(base, |_| true),
                "{base} is legal unencoded and must be left alone"
            );
        }
    }

    /// The encode-required set cuts both ways, so it is gated on evidence.
    #[test]
    fn encode_required_names_wait_for_a_raw_reference() {
        for base in [
            "a#b.css", "a%b.css", "a[b].css", "a^b.css", "a`b.css", "a{b}.css",
        ] {
            assert!(
                !needs_rename(base, |_| false),
                "{base} is a legal name; with every reference encoded there is nothing to fix"
            );
            assert!(
                needs_rename(base, |_| true),
                "{base} spelled raw in a reference is RSC-020 or worse"
            );
        }
    }

    #[test]
    fn only_the_offending_characters_are_replaced() {
        assert_eq!(safe_filename("cover image.jpg"), "cover_image.jpg");
        assert_eq!(safe_filename("a  b   c.png"), "a_b_c.png");
        assert_eq!(safe_filename("chapter.one.xhtml"), "chapter.one.xhtml");
        assert_eq!(safe_filename("no-extension"), "no-extension");
        assert_eq!(safe_filename(".hidden"), ".hidden");
        assert_eq!(safe_filename("a:b.css"), "a_b.css");
        assert_eq!(safe_filename("a[b].css"), "a_b_.css");
        // Legal characters survive, which the old class did not manage.
        assert_eq!(safe_filename("Don't Panic.css"), "Don't_Panic.css");
        assert_eq!(safe_filename("Ünïcödé x.ttf"), "Ünïcödé_x.ttf");
        assert_eq!(safe_filename("_leading_ x.svg"), "_leading_x.svg");
        // A stem made entirely of offending characters still needs a name.
        assert_eq!(safe_filename("???.css"), "file.css");
    }

    #[test]
    fn uniquify_inserts_before_the_extension() {
        assert_eq!(
            uniquify("OEBPS/a.xhtml", |c| c == "OEBPS/a_2.xhtml"),
            "OEBPS/a_3.xhtml"
        );
        assert_eq!(uniquify("OEBPS/a.xhtml", |_| false), "OEBPS/a_2.xhtml");
        assert_eq!(uniquify("OEBPS/noext", |_| false), "OEBPS/noext_2");
        assert_eq!(uniquify(".hidden", |_| false), ".hidden_2");
    }
}
