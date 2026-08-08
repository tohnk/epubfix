//! Fixes that apply to the NCX table of contents (`.ncx`).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Captures, NoExpand, Regex};

use crate::book::Book;
use crate::fixers::Fixer;
use crate::util::re;

static NAV_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"<(?:navPoint|navTarget|pageTarget)\b[^>]*>"));
static CONTENT_SRC_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"<content\s+src="([^"]+)""#));
static PLAY_ORDER_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"playOrder="\d*""#));
static UNIQUE_ID_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"unique-identifier="([^"]+)""#));
static META_RE: LazyLock<Regex> = LazyLock::new(|| re(r"<meta\b[^>]*/?>"));
static CONTENT_ATTR_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"content="[^"]*""#));

/// RSC-005: `playOrder` values that are 0-based, duplicated, gapped, or that
/// disagree between navPoints pointing at the same target.
///
/// Renumbers from 1 in document order, giving every distinct `<content src>` a
/// single number — which is what the spec means by "consistent".
pub struct PlayOrder;

impl Fixer for PlayOrder {
    fn name(&self) -> &'static str {
        "ncx-play-order"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "renumber toc.ncx playOrder consecutively from 1"
    }

    fn apply(&self, book: &mut Book) -> Vec<String> {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Vec::new();
        };
        let Some(text) = book.ncx_text().map(str::to_owned) else {
            return Vec::new();
        };

        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut counter: usize = 0;
        let new = NAV_TAG_RE
            .replace_all(&text, |c: &Captures| {
                let tag = &c[0];
                if !tag.contains("playOrder") {
                    return tag.to_string();
                }
                let m = c.get(0).expect("group 0 always matches");
                // The target is the first <content src> after the opening tag;
                // a nav element without its own inherits the next one, matching
                // the numbering readers actually apply.
                let key = match CONTENT_SRC_RE.captures(&text[m.end()..]) {
                    Some(s) => s[1].to_string(),
                    None => format!("@{}", m.start()),
                };
                let n = *seen.entry(key).or_insert_with(|| {
                    counter += 1;
                    counter
                });
                PLAY_ORDER_RE
                    .replace(tag, NoExpand(&format!("playOrder=\"{n}\"")))
                    .into_owned()
            })
            .into_owned();

        if new == text {
            return Vec::new();
        }
        book.set_text(&name, new);
        vec![format!("renumbered playOrder ({counter} target(s))")]
    }
}

/// NCX-001: `dtb:uid` must equal the OPF unique-identifier byte for byte.
pub struct DtbUid;

impl Fixer for DtbUid {
    fn name(&self) -> &'static str {
        "ncx-uid"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["NCX-001"]
    }
    fn description(&self) -> &'static str {
        "sync toc.ncx dtb:uid to the OPF unique-identifier"
    }

    fn apply(&self, book: &mut Book) -> Vec<String> {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Vec::new();
        };
        let Some(text) = book.ncx_text().map(str::to_owned) else {
            return Vec::new();
        };
        let Some(opf) = book.opf_text().map(str::to_owned) else {
            return Vec::new();
        };

        let Some(um) = UNIQUE_ID_RE.captures(&opf) else {
            return Vec::new();
        };
        let ident_re = re(&format!(
            r#"(?s)<dc:identifier[^>]*\bid="{}"[^>]*>\s*([^<]*?)\s*<"#,
            regex::escape(&um[1])
        ));
        let Some(im) = ident_re.captures(&opf) else {
            return Vec::new();
        };
        let ident = im[1].to_string();

        let new = META_RE
            .replace_all(&text, |c: &Captures| {
                let tag = &c[0];
                if !tag.contains("dtb:uid") {
                    return tag.to_string();
                }
                CONTENT_ATTR_RE
                    .replace(tag, NoExpand(&format!("content=\"{ident}\"")))
                    .into_owned()
            })
            .into_owned();

        if new == text {
            return Vec::new();
        }
        book.set_text(&name, new);
        vec!["synced NCX dtb:uid to OPF identifier".into()]
    }
}
