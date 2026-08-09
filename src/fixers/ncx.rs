//! Fixes that apply to the NCX table of contents (`.ncx`).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Captures, NoExpand, Regex};

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, NodeKind, scan};
use crate::refs::resolve_href;
use crate::util::basename;
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

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.ncx_text().map(str::to_owned) else {
            return Outcome::none();
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
            return Outcome::none();
        }
        book.set_text(&name, new);
        Outcome::change(format!("renumbered playOrder ({counter} target(s))"))
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

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.ncx_text().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(opf) = book.opf_text().map(str::to_owned) else {
            return Outcome::none();
        };

        let Some(um) = UNIQUE_ID_RE.captures(&opf) else {
            return Outcome::none();
        };
        let ident_re = re(&format!(
            r#"(?s)<dc:identifier[^>]*\bid="{}"[^>]*>\s*([^<]*?)\s*<"#,
            regex::escape(&um[1])
        ));
        let Some(im) = ident_re.captures(&opf) else {
            return Outcome::none();
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
            return Outcome::none();
        }
        book.set_text(&name, new);
        Outcome::change("synced NCX dtb:uid to OPF identifier")
    }
}

/// RSC-007, reported against the NCX: a table-of-contents entry pointing at a
/// document that is not in the book.
///
/// Removing the whole entry is safe and unambiguous — an entry for a document
/// that does not exist has no content to lose. Two things have to be right:
/// removal is nesting-aware, since deleting a parent takes its children with it,
/// and it runs before `ncx-play-order`, which closes the gaps left behind.
pub struct DeadNavEntries;

impl Fixer for DeadNavEntries {
    fn name(&self) -> &'static str {
        "ncx-dead-entries"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-007"]
    }
    fn description(&self) -> &'static str {
        "remove navigation entries pointing at documents that are not in the book"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        let present: Vec<String> = book.names().to_vec();

        // Content documents carry nav lists too, in EPUB 3.
        let mut targets: Vec<String> = book.markup_names();
        if let Some(ncx) = book.ncx_name() {
            targets.push(ncx.to_string());
        }

        for name in targets {
            let Some(text) = book.text(&name).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();
            let mut removed = 0u32;
            // Byte ranges already scheduled, so a nested entry inside a removed
            // parent is not deleted twice.
            let mut covered: Vec<std::ops::Range<usize>> = Vec::new();

            for (i, node) in nodes.iter().enumerate() {
                // navPoint/navTarget/pageTarget in an NCX; <li> in an EPUB 3
                // nav document. All of them wrap a single destination.
                let entry = match node.name.as_str() {
                    "navpoint" | "navtarget" | "pagetarget" | "li" => node,
                    _ => continue,
                };
                if entry.kind == NodeKind::End {
                    continue;
                }

                // The document this entry names, from <content src> or <a href>.
                let dest = nodes.iter().filter(|c| c.parent == Some(i)).find_map(|c| {
                    match c.name.as_str() {
                        "content" => c.attr("src"),
                        "a" => c.attr("href"),
                        _ => None,
                    }
                });
                let Some(dest) = dest else { continue };
                let Some((target, _)) = resolve_href(&name, &dest.value) else {
                    continue;
                };
                if present.contains(&target) {
                    continue;
                }

                let span = entry.element_span(&nodes);
                if covered
                    .iter()
                    .any(|c| c.start <= span.start && span.end <= c.end)
                {
                    continue; // inside a parent already going
                }
                covered.push(span.clone());
                edits.delete(span);
                removed += 1;
            }

            if removed > 0 {
                book.set_text(&name, edits.apply(&text));
                outcome.push_change(format!(
                    "{}: removed {removed} navigation entr(ies) for missing documents",
                    basename(&name)
                ));
            }
        }

        outcome
    }
}

/// RSC-005: `The "id" attribute does not have a unique value`, in the NCX.
///
/// Safe to renumber because `navPoint` ids are not link targets — nothing
/// addresses them by fragment. That is verified per book rather than assumed:
/// any id something does reference is left exactly as it is.
pub struct NcxDuplicateIds;

impl Fixer for NcxDuplicateIds {
    fn name(&self) -> &'static str {
        "ncx-duplicate-ids"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "make duplicated NCX ids unique, leaving any that are referenced alone"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let index = book.reference_index();
        let mut outcome = Outcome::none();
        let mut edits = Edits::new();
        let mut seen: Vec<String> = Vec::new();
        let mut renamed = 0u32;

        for node in nodes.iter().filter(|n| n.kind != NodeKind::End) {
            let Some(id) = node.attr("id") else { continue };
            if !seen.contains(&id.value) {
                seen.push(id.value.clone());
                continue;
            }
            // A duplicate. If anything links to it, renaming would break that.
            if index.is_referenced(&name, &id.value) {
                outcome.push_finding(format!(
                    "{}: id \"{}\" is duplicated but something references it, so it was left alone",
                    basename(&name),
                    id.value
                ));
                continue;
            }
            let mut n = 2;
            let mut candidate = format!("{}_{n}", id.value);
            while seen.contains(&candidate) {
                n += 1;
                candidate = format!("{}_{n}", id.value);
            }
            edits.replace(id.span.clone(), format!("id=\"{candidate}\""));
            seen.push(candidate);
            renamed += 1;
        }

        if renamed > 0 {
            book.set_text(&name, edits.apply(&text));
            outcome.push_change(format!("made {renamed} duplicated NCX id(s) unique"));
        }
        outcome
    }
}

/// RSC-005: `<pageList>` missing the attributes its DTD requires.
///
/// The NCX DTD requires *both* `id` and `class`. Supplying only one is worse
/// than supplying neither: on a book whose `<pageList>` had neither, adding just
/// `class` pushed the element onto the strict validation path and introduced an
/// error epubcheck had not been reporting. When a schema requires a set, supply
/// the whole set or none of it.
pub struct PageListAttrs;

impl Fixer for PageListAttrs {
    fn name(&self) -> &'static str {
        "ncx-pagelist-attrs"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "give <pageList> both of the attributes its DTD requires, or neither"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(name) = book.ncx_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let mut edits = Edits::new();
        let mut fixed = 0u32;
        for node in nodes
            .iter()
            .filter(|n| n.name == "pagelist" && n.kind != NodeKind::End)
        {
            let (has_id, has_class) = (node.attr("id").is_some(), node.attr("class").is_some());
            if has_id && has_class {
                continue;
            }
            let mut add = String::new();
            if !has_id {
                add.push_str(" id=\"pagelist\"");
            }
            if !has_class {
                add.push_str(" class=\"pagelist\"");
            }
            edits.insert(node.name_end, add);
            fixed += 1;
        }

        if fixed == 0 {
            return Outcome::none();
        }
        book.set_text(&name, edits.apply(&text));
        Outcome::change(format!(
            "completed the required attributes on {fixed} <pageList> element(s)"
        ))
    }
}
