//! RSC-020 / PKG-010: resource filenames containing spaces or other characters
//! that have to be escaped in a URL.
//!
//! The entry is renamed on repack and every textual reference to the old
//! basename — raw or percent-encoded — is pointed at the new one.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::util::{BAD_CHARS, basename, dirname, safe_filename};

/// The set `urllib.parse.quote` leaves unescaped: `A-Za-z0-9` and `-._~`.
const QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub struct UnsafeFilenames;

impl Fixer for UnsafeFilenames {
    fn name(&self) -> &'static str {
        "filenames"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-020", "PKG-010"]
    }
    fn description(&self) -> &'static str {
        "rename resources whose filenames need URL escaping, and update references"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut planned: Vec<(String, String)> = Vec::new();

        for name in book.names() {
            if name.ends_with('/') {
                continue;
            }
            let base = basename(name);
            if base.is_empty() || !BAD_CHARS.is_match(base) {
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
    use super::uniquify;

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
