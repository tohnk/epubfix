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

use std::fmt::Write as _;

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::book::Book;
use crate::fixers::css_paths::{IMPORT_RE, URL_RE, unquote};
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, scan};
use crate::paths::{Resolution, Resolver, relative_to};
use crate::refs::resolve_href;
use crate::util::{basename, dirname, ends_with_any};

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

/// Write an archive path as it must appear inside an `href`.
///
/// Anything generating a reference has to go through this. A manifest entry
/// written for `a[1].css` with the bracket raw is RSC-020 — and worse here,
/// because [`UnsafeFilenames`] would then see a raw reference, take it as the
/// evidence it waits for, and rename a file that never needed renaming.
///
/// The set is [`ENCODE_REQUIRED`] plus whitespace, which is exactly what that
/// constant and [`needs_rename`] were measured for.
pub(crate) fn as_url_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if ENCODE_REQUIRED.contains(&c) || c.is_whitespace() {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).as_bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

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
    // PKG-011: a trailing FULL STOP is illegal outright, and repeated ones are
    // still one name ending in a dot, so they all go before anything else runs.
    let base = base.trim_end_matches('.');
    if base.is_empty() {
        return "file".to_string();
    }
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

/// One directory component, sanitised with the same character rules as a file
/// name but with no stem/extension split — a directory is not expected to have
/// an extension to protect, and splitting on a dot would reattach unsafe
/// characters it already has no business touching.
fn safe_dir_component(dir: &str) -> String {
    let dir = dir.trim_end_matches('.');
    let mut s = String::with_capacity(dir.len());
    for c in dir.chars() {
        if is_forbidden(c) || needs_encoding(c) {
            if !s.ends_with('_') {
                s.push('_');
            }
        } else {
            s.push(c);
        }
    }
    if s.is_empty() || s == "_" {
        "file".to_string()
    } else {
        s
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
    base.ends_with('.')
        || base.chars().any(is_forbidden)
        || (base.chars().any(needs_encoding) && raw_reference(base))
}

/// The key OCF compares two entry names by: OPF-060 requires them to be unique
/// "after Unicode canonical normalization and full case folding", so `Pic.gif`
/// and `pic.gif` are the same name however different they look in a listing.
///
/// `to_lowercase` is Unicode-aware and is the same comparison for every name a
/// book is likely to hold; it is used only to *find* a collision, and the
/// repair is a rename that removes any doubt.
fn folded(name: &str) -> String {
    name.to_lowercase()
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

    #[allow(
        clippy::too_many_lines,
        reason = "planning and applying one rename is one pass, and splitting it \
                  would spread the directory and basename rules further apart"
    )]
    fn apply(&self, book: &mut Book) -> Outcome {
        let mut planned: Vec<(String, String)> = Vec::new();

        // Snapshot the textual content once: the encode-required gate asks the
        // same question of every candidate, and the answer cannot change while
        // we are only planning.
        let corpus: Vec<&str> = book.names().iter().filter_map(|n| book.text(n)).collect();
        let spelled_raw = |base: &str| corpus.iter().any(|t| t.contains(base));

        // A directory component is renamed under the same rules as a file
        // name: any reference that crosses the directory names it, so a raw
        // reference is the same evidence here as it is for a basename. Renaming
        // the whole component at once keeps references *inside* the directory
        // valid for free — a sibling link spells no directory at all.
        let dir_unsafe = |dir: &str| {
            dir.split('/').any(|c| {
                !c.is_empty()
                    && (c.ends_with('.')
                        || c.chars().any(is_forbidden)
                        || (c.chars().any(needs_encoding) && spelled_raw(c)))
            })
        };
        let safe_path = |name: &str, base_needs: bool| {
            let dir = dirname(name);
            let safe_dir: String = dir
                .split('/')
                .map(|c| {
                    if c.is_empty() {
                        String::new()
                    } else {
                        safe_dir_component(c)
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            let base = if base_needs {
                safe_filename(basename(name))
            } else {
                basename(name).to_string()
            };
            format!("{safe_dir}{base}")
        };

        for name in book.names() {
            if name.ends_with('/') {
                continue;
            }
            let base = basename(name);
            let base_needs = !base.is_empty() && needs_rename(base, spelled_raw);
            if !base_needs && !dir_unsafe(dirname(name)) {
                continue;
            }
            let mut candidate = safe_path(name, base_needs);
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

        // OPF-060: two entries that differ only by case are one name as far as
        // OCF is concerned. This is a condition on a *pair*, so it cannot live
        // in needs_rename with the rest; the first entry in archive order keeps
        // its name and any later one that folds onto it is renamed.
        let mut seen: HashMap<String, String> = HashMap::new();
        for name in book.names() {
            if name.ends_with('/') {
                continue;
            }
            let after = planned
                .iter()
                .find(|(old, _)| old == name)
                .map_or(name.clone(), |(_, new)| new.clone());
            match seen.entry(folded(&after)) {
                Entry::Vacant(v) => {
                    v.insert(after);
                }
                Entry::Occupied(_) => {
                    let candidate = uniquify(&after, |c| {
                        book.name_taken(c)
                            || seen.contains_key(&folded(c))
                            || planned.iter().any(|(_, n)| n == c)
                    });
                    seen.insert(folded(&candidate), candidate.clone());
                    match planned.iter_mut().find(|(old, _)| old == name) {
                        Some(slot) => slot.1 = candidate,
                        None => planned.push((name.clone(), candidate)),
                    }
                }
            }
        }

        if planned.is_empty() {
            return Outcome::none();
        }

        let moved: HashMap<&str, &str> = planned
            .iter()
            .map(|(o, n)| (o.as_str(), n.as_str()))
            .collect();
        for (name, text) in rewritten_references(book, &moved) {
            book.set_text(&name, text);
        }

        for (old, new) in &planned {
            book.rename(old, new.clone());
        }

        Outcome::change(format!("renamed {} file(s)", planned.len()))
    }
}

/// Every text entry whose references have to change, with its new contents.
///
/// # Why this is not a string replace
///
/// It used to be one. For each renamed path component the old spelling was
/// searched for across every text entry in the book and swapped for the new
/// one, which is short, general, and finds a reference written from any
/// directory. It also cannot tell a reference from anything else that happens
/// to contain those bytes, and a filename is a very ordinary run of bytes.
///
/// Measured, on a book with `B.xhtml`, `b.xhtml` and `ab.xhtml` — the first two
/// collide under OCF's case folding, so `b.xhtml` is renamed to `b_2.xhtml`:
///
/// ```text
/// before   <a href="ab.xhtml">   <item href="ab.xhtml">   <content src="ab.xhtml"/>
/// after    <a href="ab_2.xhtml"> <item href="ab_2.xhtml"> <content src="ab_2.xhtml"/>
///          ERROR(RSC-001) File "OEBPS/ab_2.xhtml" could not be found.
/// ```
///
/// Three references broken and a file invented, because `b.xhtml` is a
/// substring of `ab.xhtml`. The same mechanism turns a renamed `img` directory
/// into `<img_2 src=…>` and the word `imagination` into `img_2agination`. The
/// ordering heuristic that used to guard this — longest changed component
/// first — only helps when one rename is a prefix of another, and does nothing
/// here.
///
/// So a reference is found by *resolving* it, not by matching bytes. Each one
/// is asked where it lands; if that is a file being renamed, it is rewritten
/// relative to wherever the referring document is itself about to live, which
/// is the one calculation that stays correct when a directory moves and takes
/// its documents with it. Anything that does not resolve to a renamed file is
/// not touched at all.
fn rewritten_references(book: &Book, moved: &HashMap<&str, &str>) -> Vec<(String, String)> {
    let resolver = Resolver::new(book);
    let mut out = Vec::new();

    for name in book.names() {
        let Some(text) = book.text(name) else {
            continue;
        };
        // The referring document may be moving too, and a relative reference is
        // written from where the document ends up.
        let here = moved.get(name.as_str()).map_or(name.as_str(), |n| *n);

        // Where a reference in this file lands, leniently: `package-references`
        // and `dangling-resources` have already run, but a path that is merely
        // misspelled still points at a real file and must not be left behind
        // pointing at its old name.
        let lands = |raw: &str| match resolver.resolve(name, raw) {
            Resolution::Fine => resolve_href(name, raw).map(|(t, _)| t),
            Resolution::Moved { target } => Some(target),
            _ => None,
        };
        let repointed = |raw: &str| -> Option<String> {
            let target = lands(raw)?;
            let new = moved.get(target.as_str())?;
            let rel = as_url_path(&relative_to(here, new));
            Some(match raw.split_once('#') {
                Some((_, f)) if !f.is_empty() => format!("{rel}#{f}"),
                _ => rel,
            })
        };
        // `rootfile/@full-path` is the exception, and it is the one that cannot
        // be got wrong: OCF resolves it from the *container root* rather than
        // from `META-INF/`, and writes it the same way. Treating it like an
        // ordinary relative reference produced `full-path="../OEBPS/…"`, which
        // is a book no reading system can open.
        let from_root = |raw: &str| -> Option<String> {
            let (target, _) = resolve_href("x", raw)?;
            moved.get(target.as_str()).map(|new| as_url_path(new))
        };

        let mut edits = Edits::new();
        if ends_with_any(name, &[".css"]) {
            for c in IMPORT_RE.captures_iter(text) {
                let Some(arg) = c.get(1).or_else(|| c.get(2)) else {
                    continue;
                };
                if let Some(value) = repointed(unquote(arg.as_str())) {
                    edits.replace(arg.range(), format!("\"{value}\""));
                }
            }
            let imported: Vec<_> = IMPORT_RE.find_iter(text).map(|m| m.range()).collect();
            for c in URL_RE.captures_iter(text) {
                let arg = c.get(1).expect("group 1 always matches");
                if imported.iter().any(|r| r.contains(&arg.start())) {
                    continue;
                }
                if let Some(value) = repointed(unquote(arg.as_str())) {
                    edits.replace(arg.range(), format!("\"{value}\""));
                }
            }
        } else {
            let Ok(nodes) = scan(text) else { continue };
            for attr in nodes
                .iter()
                .flat_map(|n| n.attrs.iter())
                .filter(|a| is_path_attr(&a.name))
            {
                let rewritten = if attr.name == "full-path" {
                    from_root(&attr.value)
                } else {
                    repointed(&attr.value)
                };
                if let Some(value) = rewritten {
                    edits.replace(attr.span.clone(), format!("{}=\"{value}\"", attr.name));
                }
            }
        }

        if !edits.is_empty() {
            out.push((name.clone(), edits.apply(text)));
        }
    }
    out
}

/// Attributes whose value is a path into the container.
///
/// `full-path` is the one that is easy to forget and expensive to miss: it is
/// how `container.xml` names the package document, it is spelled like nothing
/// else in EPUB, and a renamed OPF whose only pointer still says the old name
/// is a book no reading system can open at all.
fn is_path_attr(name: &str) -> bool {
    matches!(name, "src" | "href" | "full-path") || name.ends_with(":href")
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
