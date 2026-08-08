//! RSC-005: `id` and `name` attributes that are not valid XML Names.
//!
//! Values are collected from the content documents, sanitised, and the mapping is
//! then applied to both the attributes themselves and to every `href`/`src`
//! fragment in the book, so internal links keep pointing at their anchors.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::book::Book;
use crate::fixers::Fixer;
use crate::util::{is_bad_id, re, sanitise_id};

static IDNAME_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"\b(id|name)="([^"]*)""#));
static FRAGMENT_RE: LazyLock<Regex> = LazyLock::new(|| re(r##"\b(href|src)="([^"#]*)#([^"]+)""##));

pub struct XmlIds;

impl Fixer for XmlIds {
    fn name(&self) -> &'static str {
        "xml-ids"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "rewrite id/name values that are not valid XML Names, and the links to them"
    }

    fn apply(&self, book: &mut Book) -> Vec<String> {
        let mut idmap: HashMap<String, String> = HashMap::new();
        book.for_each_markup(|_, text| {
            for c in IDNAME_RE.captures_iter(text) {
                let v = &c[2];
                if is_bad_id(v) {
                    idmap.entry(v.to_string()).or_insert_with(|| sanitise_id(v));
                }
            }
        });

        if idmap.is_empty() {
            return Vec::new();
        }

        book.for_each_markup(|_, text| {
            *text = IDNAME_RE
                .replace_all(text, |c: &Captures| {
                    let v = idmap.get(&c[2]).map_or(&c[2], String::as_str);
                    format!("{}=\"{}\"", &c[1], v)
                })
                .into_owned();
        });

        // Fragments are rewritten everywhere, not just in markup: the NCX and the
        // OPF guide both link into content documents by anchor.
        book.for_each_text(|_, text| {
            *text = FRAGMENT_RE
                .replace_all(text, |c: &Captures| {
                    let f = idmap.get(&c[3]).map_or(&c[3], String::as_str);
                    format!("{}=\"{}#{}\"", &c[1], &c[2], f)
                })
                .into_owned();
        });

        vec![format!("sanitised {} id(s)", idmap.len())]
    }
}
