//! Working out what language a book is in.
//!
//! `dc:language` is required by both EPUB versions, and epubcheck accepts
//! whatever it finds there — `<dc:language>zz</dc:language>` validates clean.
//! So this is a field where being wrong is silent, and being wrong matters:
//! reading systems key hyphenation dictionaries, text-to-speech voice
//! selection and sometimes font choice off it. An absent value falls back to
//! something sensible; a wrong one mis-hyphenates every page.
//!
//! That rules out the obvious shortcut. Calibre writes a default taken from the
//! **system locale** and never reads the book, so a Hungarian novel that passes
//! through Calibre on an English machine comes out declaring `en`. That is a
//! guess wearing the costume of metadata, and it is worse than the error it
//! replaces.
//!
//! # The ladder
//!
//! 1. **What the documents already say.** Many books carry `xml:lang` or `lang`
//!    on every `<html>`. That is a stated fact in the file, the same class of
//!    evidence as the NCX identifier that gets synced to the OPF — not an
//!    inference, and not subject to the write policy below.
//! 2. **Detection from the text**, but only if step 1 found nothing.
//! 3. **Report.** Never a locale, never a default.
//!
//! # Sampling
//!
//! Every rule here comes from a book that broke a simpler version:
//!
//! * **Never classify one document.** *The Complete Works of Aristotle* votes
//!   thirteen English to two Latin, and the two are footnote files that are
//!   nothing but Latin citations. A single unlucky sample declares the book
//!   Latin, confidently.
//! * **Skip the front matter.** Copyright pages are addresses and ISBNs.
//! * **Discard short documents.** Below a few hundred characters a classifier
//!   is guessing.
//! * **Draw adaptively.** *A Supposedly Fun Thing I'll Never Do Again* has 323
//!   documents with a median length of 281 characters, because Calibre split it
//!   into fragments; a fixed "take fifteen documents" yields two usable
//!   samples. Short documents are therefore glued together until they are worth
//!   classifying rather than thrown away.

use std::sync::LazyLock;

use regex::Regex;
use whatlang::Lang;

use crate::book::Book;
use crate::markup::{NodeKind, scan};
use crate::util::re;

/// Below this many characters a sample is not worth classifying.
const MIN_SAMPLE: usize = 400;
/// Fewer classified samples than this and the vote means nothing.
const MIN_SAMPLES: usize = 3;
/// The leading fraction of the spine treated as front matter and skipped.
const FRONT_MATTER: usize = 7;

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<[^>]*>"));
static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| re(r"&[#0-9A-Za-z]+;"));

/// When the tool may write a language it worked out for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    /// Write a detected language only when it is English.
    ///
    /// This is not about accuracy — detection is no better at English than at
    /// anything else. It is about the shape of the failure. Restricted to
    /// English, the worst case is "did nothing, the error remains"; unrestricted,
    /// the worst case is a book confidently labelled the wrong language, which
    /// nobody will ever notice and which mis-hyphenates every page. In a library
    /// that is essentially all English, a French book being reported instead of
    /// fixed costs nothing.
    #[default]
    EnglishOnly,
    /// Write whatever the text says it is.
    Any,
    /// Never detect. Only a language the documents already declare is written.
    Off,
}

/// Where a language came from, which decides whether the policy applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The documents say so. A fact, not a guess.
    Declared,
    /// Worked out from the text.
    Detected { samples: usize, share: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub language: String,
    pub source: Source,
}

/// ISO 639-1 for the languages whatlang knows, which reports 639-3.
///
/// BCP 47 asks for the shortest code that exists, so `en` rather than `eng`.
/// Every whatlang language has a two-letter code except Akan, which keeps its
/// three-letter one — correctly, since ISO 639-1 has none for it.
fn iso_639_1(code: &str) -> &str {
    match code {
        "epo" => "eo",
        "eng" => "en",
        "rus" => "ru",
        "cmn" => "zh",
        "spa" => "es",
        "por" => "pt",
        "ita" => "it",
        "ben" => "bn",
        "fra" => "fr",
        "deu" => "de",
        "ukr" => "uk",
        "kat" => "ka",
        "ara" => "ar",
        "hin" => "hi",
        "jpn" => "ja",
        "heb" => "he",
        "yid" => "yi",
        "pol" => "pl",
        "amh" => "am",
        "jav" => "jv",
        "kor" => "ko",
        "nob" => "nb",
        "dan" => "da",
        "swe" => "sv",
        "fin" => "fi",
        "tur" => "tr",
        "nld" => "nl",
        "hun" => "hu",
        "ces" => "cs",
        "ell" => "el",
        "bul" => "bg",
        "bel" => "be",
        "mar" => "mr",
        "kan" => "kn",
        "ron" => "ro",
        "slv" => "sl",
        "hrv" => "hr",
        "srp" => "sr",
        "mkd" => "mk",
        "lit" => "lt",
        "lav" => "lv",
        "est" => "et",
        "tam" => "ta",
        "vie" => "vi",
        "urd" => "ur",
        "tha" => "th",
        "guj" => "gu",
        "uzb" => "uz",
        "pan" => "pa",
        "aze" => "az",
        "ind" => "id",
        "tel" => "te",
        "pes" => "fa",
        "mal" => "ml",
        "ori" => "or",
        "mya" => "my",
        "nep" => "ne",
        "sin" => "si",
        "khm" => "km",
        "tuk" => "tk",
        "zul" => "zu",
        "sna" => "sn",
        "afr" => "af",
        "lat" => "la",
        "slk" => "sk",
        "cat" => "ca",
        "tgl" => "tl",
        "hye" => "hy",
        "cym" => "cy",
        // Akan has no ISO 639-1 code, so its 639-3 one is the shortest there is.
        other => other,
    }
}

/// The primary subtag, lowercased: `en-GB` and `EN` both become `en`.
///
/// Detection cannot tell `en-GB` from `en-US`, and neither can a book's own
/// `xml:lang` be trusted to have meant the region it wrote. The primary subtag
/// is the part that carries the meaning a reading system acts on.
fn primary_subtag(value: &str) -> Option<String> {
    let tag = value.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    (tag.len() >= 2 && tag.len() <= 3 && tag.chars().all(|c| c.is_ascii_alphabetic()))
        .then_some(tag)
}

/// What each content document declares on its root `<html>`.
fn declared(book: &Book) -> Vec<String> {
    book.markup_names()
        .into_iter()
        .filter_map(|doc| {
            let text = book.text(&doc)?;
            let nodes = scan(text).ok()?;
            let html = nodes
                .iter()
                .find(|n| n.name == "html" && n.kind != NodeKind::End)?;
            html.attr("xml:lang")
                .or_else(|| html.attr("lang"))
                .and_then(|a| primary_subtag(&a.value))
        })
        .collect()
}

/// Visible text, near enough for a classifier: tags out, entities out,
/// whitespace collapsed.
fn plain_text(markup: &str) -> String {
    let body = markup
        .find("<body")
        .and_then(|i| markup[i..].find('>').map(|j| i + j + 1))
        .map_or(markup, |start| &markup[start..]);
    let no_tags = TAG_RE.replace_all(body, " ");
    let no_entities = ENTITY_RE.replace_all(&no_tags, " ");
    no_entities.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Samples worth classifying, drawn from past the front matter.
///
/// Short documents are glued to their neighbours rather than discarded, which
/// is what makes this work on a book Calibre has split into 300 fragments.
fn samples(book: &Book) -> Vec<String> {
    let docs = book.markup_names();
    let skip = docs.len() / FRONT_MATTER;

    let mut out = Vec::new();
    let mut buffer = String::new();
    for doc in docs.into_iter().skip(skip) {
        let Some(text) = book.text(&doc) else {
            continue;
        };
        let plain = plain_text(text);
        if plain.is_empty() {
            continue;
        }
        if !buffer.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(&plain);
        if buffer.chars().count() >= MIN_SAMPLE {
            out.push(std::mem::take(&mut buffer));
        }
    }
    out
}

/// The winner of a majority vote, if it has more than half.
fn majority(votes: &[String]) -> Option<(String, usize)> {
    let mut tally: Vec<(&String, usize)> = Vec::new();
    for v in votes {
        match tally.iter_mut().find(|(k, _)| *k == v) {
            Some((_, c)) => *c += 1,
            None => tally.push((v, 1)),
        }
    }
    let (winner, count) = tally.into_iter().max_by_key(|(_, c)| *c)?;
    (count * 2 > votes.len()).then(|| (winner.clone(), count))
}

/// Work out the book's language, or decide it cannot be worked out.
///
/// Returns `None` when there is no honest answer — the caller reports rather
/// than inventing one.
pub fn detect(book: &Book, policy: Policy) -> Option<Finding> {
    // 1. What the book already says about itself.
    let stated = declared(book);
    if let Some((language, _)) = majority(&stated) {
        return Some(Finding {
            language,
            source: Source::Declared,
        });
    }

    if policy == Policy::Off {
        return None;
    }

    // 2. What the text says.
    let samples = samples(book);
    if samples.len() < MIN_SAMPLES {
        return None;
    }
    let votes: Vec<String> = samples
        .iter()
        .filter_map(|s| whatlang::detect_lang(s))
        .map(|l: Lang| iso_639_1(l.code()).to_string())
        .collect();
    if votes.len() < MIN_SAMPLES {
        return None;
    }
    let (language, share) = majority(&votes)?;

    if policy == Policy::EnglishOnly && language != "en" {
        return None;
    }
    Some(Finding {
        language,
        source: Source::Detected {
            samples: votes.len(),
            share,
        },
    })
}

/// What a book's text says it is, whatever the policy — for reporting when the
/// policy declines to write it.
pub fn observed(book: &Book) -> Option<Finding> {
    detect(book, Policy::Any)
}

#[cfg(test)]
mod tests {
    use super::{iso_639_1, majority, plain_text, primary_subtag};

    #[test]
    fn codes_shorten_to_iso_639_1_where_one_exists() {
        assert_eq!(iso_639_1("eng"), "en");
        assert_eq!(iso_639_1("cmn"), "zh");
        assert_eq!(iso_639_1("pes"), "fa");
        assert_eq!(iso_639_1("nob"), "nb");
        // Akan has no two-letter code, so it keeps the one it has.
        assert_eq!(iso_639_1("aka"), "aka");
    }

    #[test]
    fn a_tag_keeps_only_its_primary_subtag() {
        assert_eq!(primary_subtag("en-GB").as_deref(), Some("en"));
        assert_eq!(primary_subtag("  EN  ").as_deref(), Some("en"));
        assert_eq!(primary_subtag("zh_Hans").as_deref(), Some("zh"));
        assert_eq!(primary_subtag("").as_deref(), None);
        assert_eq!(primary_subtag("english").as_deref(), None);
        assert_eq!(primary_subtag("1").as_deref(), None);
    }

    #[test]
    fn text_extraction_drops_markup_and_the_head() {
        let doc = "<html><head><title>Ignore me</title></head>\
                   <body><p>The quick&nbsp;brown <em>fox</em>.</p></body></html>";
        assert_eq!(plain_text(doc), "The quick brown fox .");
    }

    #[test]
    fn a_majority_needs_more_than_half() {
        let v = |s: &[&str]| s.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
        // The Aristotle shape: two Latin footnote files cannot outvote the book.
        assert_eq!(
            majority(&v(&["en", "en", "en", "la", "la"])),
            Some(("en".to_string(), 3))
        );
        // A tie is not a majority.
        assert_eq!(majority(&v(&["en", "fr"])), None);
        assert_eq!(majority(&v(&["en", "en", "fr", "fr"])), None);
        assert_eq!(majority(&v(&[])), None);
    }
}
