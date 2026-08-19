//! Fixes that apply to the package document (`.opf`).

use std::fmt::Write as _;
use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::filenames::as_url_path;
use crate::fixers::{Fixer, Outcome};
use crate::language::{self, Policy};
use crate::markup::{Edits, Node, NodeKind, line_span, scan};
use crate::paths::{Resolution, Resolver, relative_to};
use crate::refs::resolve_href;
use crate::util::{basename, ends_with_any, is_bad_id, re};

/// Read the OPF, hand it to `f`, and store the result if it changed.
fn edit_opf(book: &mut Book, f: impl FnOnce(&str) -> String) -> bool {
    let Some(name) = book.opf_name().map(str::to_owned) else {
        return false;
    };
    let Some(text) = book.text(&name).map(str::to_owned) else {
        return false;
    };
    let new = f(&text);
    if new == text {
        return false;
    }
    book.set_text(&name, new);
    true
}

/// OPF-001: an OEBPS 1.0 package declaration in a file that is otherwise EPUB 2.
pub struct PackageVersion;

static VERSION_RE: LazyLock<Regex> = LazyLock::new(|| re(r#"(<package\b[^>]*?)version="1\.0""#));

impl Fixer for PackageVersion {
    fn name(&self) -> &'static str {
        "opf-version"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-001"]
    }
    fn description(&self) -> &'static str {
        r#"package version="1.0" -> "2.0""#
    }
    fn apply(&self, book: &mut Book) -> Outcome {
        if edit_opf(book, |t| {
            VERSION_RE
                .replacen(t, 1, r#"${1}version="2.0""#)
                .into_owned()
        }) {
            Outcome::change("package version 1.0 -> 2.0")
        } else {
            Outcome::none()
        }
    }
}

/// RSC-005: `<spine page-map="...">`, an Adobe extension with no place in EPUB 2.
///
/// Removing the attribute validates, and that was all this did. What the
/// attribute points at is a file of *print page numbers* — the ones a reader
/// shows in the margin and a citation refers to — and dropping the only pointer
/// to it silently retires the whole apparatus.
///
/// This is not a rare shape. Across a 179-book library **14 books carry one**,
/// holding between 190 and 2504 `<page>` elements each:
///
/// ```xml
/// <page-map xmlns="http://www.idpf.org/2007/opf">
///   <page name="i"  href="xhtml/fm01.html"/>
///   <page name="17" href="xhtml/ch02.html#page_17"/>
/// </page-map>
/// ```
///
/// EPUB 2 has a standard spelling for exactly this, the NCX `<pageList>`, and
/// the two carry the same information, so the page map is converted into one
/// and only then is the attribute removed.
///
/// # Measured, because almost none of this is guessable
///
/// Against EPUB Check 5.2.1, one construct at a time:
///
/// ```text
/// pageTarget with id/type/value/playOrder      clean
/// pageTarget with no id, or no value           clean
/// pageTarget with no playOrder                 clean
/// pageTarget with no type                      missing required attribute "type"
/// two pageTargets, same type and value         combination of value and type is not unique
/// two type="front", neither carrying a value   clean
/// two type="special"                           clean
/// pageTarget whose content src has no such id  RSC-012 fragment not defined
/// playOrder colliding with a navPoint's        identical playOrder values
/// the page-map item and file, unreferenced     clean
/// ```
///
/// Four rules fall out of that. `type` is the one attribute that must be
/// written. `playOrder` is **omitted**, because the only way to get it wrong is
/// to collide with the navMap and there is no way to get it right that omitting
/// does not also get right. `id` is omitted too, since
/// [`crate::fixers::ncx::NavPointIds`] already gives every entry the id its
/// schema wants and runs later. And `value` is written only for a genuine page
/// *number*, which is what makes the type/value pair unique.
///
/// So a name that is all digits is `type="normal"` carrying that value; one
/// that is all roman numerals is `type="front"` with none, which is how front
/// matter is paginated; anything else — `cover`, or the blank name two of these
/// books use — is `type="special"`, or skipped when there is no name at all.
///
/// The manifest item and the page-map file are both left where they are: they
/// validate clean, and the pagination now has two homes rather than none.
pub struct SpinePageMap;

/// The page labels an NCX `<pageList>` already carries, or `None` if it has no
/// `<pageList>` at all.
///
/// A book with both a page map and a pageList is usually carrying the same
/// pagination twice, and saying so would be noise: of the four in the library,
/// three match to the entry — 189/189, 2503/2503, 231/231. The fourth does not.
/// A Sanderson *Oathbringer* has 1431 pages in its Adobe map and 1221 in its
/// pageList, so 210 page numbers exist in a place no reading system looks. That
/// is worth a person's attention and the other three are not, which is why this
/// compares the labels instead of counting them.
fn listed_pages(ncx: &str) -> Option<std::collections::HashSet<String>> {
    let nodes = scan(ncx).ok()?;
    let list = nodes
        .iter()
        .position(|n| n.name == "pagelist" && n.kind == NodeKind::Start)?;
    let end = nodes[list].close.unwrap_or(nodes.len());
    let mut out = std::collections::HashSet::new();
    for (i, node) in nodes.iter().enumerate() {
        if i <= list || i >= end || node.name != "text" || node.kind != NodeKind::Start {
            continue;
        }
        if let Some(close) = node.close {
            out.insert(
                ncx[node.span.end..nodes[close].span.start]
                    .trim()
                    .to_string(),
            );
        }
    }
    Some(out)
}

/// Every page name a page map gives, in document order.
fn page_names(book: &Book, map_name: &str) -> Vec<String> {
    let Some(text) = book.text(map_name) else {
        return Vec::new();
    };
    let Ok(nodes) = scan(text) else {
        return Vec::new();
    };
    nodes
        .iter()
        .filter(|n| n.name == "page" && n.kind != NodeKind::End)
        .filter_map(|n| n.attr("name"))
        .map(|a| a.value.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The `<pageTarget>` spelling of one `<page>` name.
///
/// Roman numerals are recognised by their alphabet rather than parsed. A page
/// label is a handful of characters and `iii` and `xlii` are the whole of what
/// turns up; a word that happens to be spelled out of the same letters would be
/// filed as front matter instead of special, which changes a `type` and nothing
/// a reader sees.
fn page_kind(name: &str) -> Option<(&'static str, Option<String>)> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if let Ok(n) = name.parse::<u32>()
        && n > 0
    {
        return Some(("normal", Some(n.to_string())));
    }
    if name.chars().all(|c| {
        matches!(
            c.to_ascii_lowercase(),
            'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
        )
    }) {
        return Some(("front", None));
    }
    Some(("special", None))
}

/// The `<pageList>` a page map means, as NCX source, or `None` if it yields
/// nothing worth writing.
///
/// The ids are written here rather than left to [`NavPointIds`], which numbers
/// from the element name and, over a thousand entries, produces
/// `pagetarget-1_2` … `pagetarget-1_1066` once the duplicate pass has been
/// through them. Valid, and no reader would ever see it, but a file someone may
/// open should not look like something went wrong.
fn page_list(book: &Book, map_name: &str, ncx_name: &str) -> Option<(String, usize)> {
    let text = book.text(map_name)?;
    let nodes = scan(text).ok()?;
    let resolver = Resolver::new(book);

    // Unique within the NCX is all an id has to be, so only its own are asked.
    let taken: std::collections::HashSet<String> = book
        .text(ncx_name)
        .and_then(|t| scan(t).ok())
        .map(|ns| {
            ns.iter()
                .filter_map(|n| n.attr("id"))
                .map(|a| a.value.clone())
                .collect()
        })
        .unwrap_or_default();

    let mut rows = String::new();
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut written = 0usize;
    let mut next = 0u32;

    for node in nodes
        .iter()
        .filter(|n| n.name == "page" && n.kind != NodeKind::End)
    {
        let (Some(name), Some(href)) = (node.attr("name"), node.attr("href")) else {
            continue;
        };
        let Some((kind, value)) = page_kind(&name.value) else {
            continue;
        };
        // A page whose document is in the archive under no path at all would
        // only become an RSC-007 in the file it is written into.
        let landing = match resolver.resolve(map_name, &href.value) {
            Resolution::Fine => resolve_href(map_name, &href.value).map(|(t, _)| t),
            Resolution::Moved { target } => Some(target),
            _ => None,
        }?;
        // Uniqueness is on the pair, and only a `normal` carries a value, so
        // this is the only kind that can collide.
        let key = (kind.to_string(), value.clone().unwrap_or_default());
        if value.is_some() && seen.contains(&key) {
            continue;
        }
        seen.push(key);

        let rel = as_url_path(&relative_to(ncx_name, &landing));
        let src = match href.value.split_once('#') {
            Some((_, f)) if !f.is_empty() => format!("{rel}#{f}"),
            _ => rel,
        };
        let id = std::iter::from_fn(|| {
            next += 1;
            Some(format!("page-{next}"))
        })
        .find(|candidate| !taken.contains(candidate))
        .expect("the sequence is unbounded");

        let value = value.map_or(String::new(), |v| format!(" value=\"{v}\""));
        let _ = write!(
            rows,
            "\n    <pageTarget id=\"{id}\" type=\"{kind}\"{value}>\
             <navLabel><text>{}</text></navLabel>\
             <content src=\"{src}\"/></pageTarget>",
            name.value.trim()
        );
        written += 1;
    }

    (written > 0).then(|| (format!("  <pageList>{rows}\n  </pageList>\n"), written))
}

impl Fixer for SpinePageMap {
    fn name(&self) -> &'static str {
        "spine-page-map"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "move an Adobe page-map's page numbers into the NCX pageList, then drop the attribute"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(opf) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&opf) else {
            return Outcome::none();
        };
        let Some(idref) = nodes
            .iter()
            .find(|n| n.name == "spine")
            .and_then(|n| n.attr("page-map"))
            .map(|a| a.value.clone())
        else {
            return Outcome::none();
        };

        let mut outcome = Outcome::none();

        // The page numbers first: the attribute is the only thing that says
        // where they live, so it cannot go until they have somewhere else.
        let map_name = nodes
            .iter()
            .filter(|n| n.name == "item")
            .find(|n| n.attr("id").is_some_and(|a| a.value == idref))
            .and_then(|n| n.attr("href"))
            .and_then(|h| resolve_href(&opf_name, &h.value))
            .map(|(t, _)| t)
            .filter(|t| book.text(t).is_some());

        match (map_name, book.ncx_name().map(str::to_owned)) {
            (Some(map_name), Some(ncx_name)) => {
                let ncx = book.text(&ncx_name).unwrap_or_default().to_string();
                if let Some(already) = listed_pages(&ncx) {
                    // Both spellings present. Only the pages the standard one is
                    // missing are worth anybody's time.
                    let named = page_names(book, &map_name);
                    let mut seen = std::collections::HashSet::new();
                    let mut absent: Vec<String> = named
                        .iter()
                        .filter(|n| !already.contains(*n))
                        .filter(|n| seen.insert((*n).clone()))
                        .cloned()
                        .collect();
                    if absent.is_empty() {
                        // Say so. "removed spine/@page-map" on its own is the
                        // report this fixer used to give when it *did* throw
                        // the pagination away, and a reader has no way to tell
                        // the two apart — which is exactly the question the
                        // line prompted the first time someone ran it on a book
                        // whose NCX already had the lot.
                        outcome.push_change(format!(
                            "the {} print page number(s) in the Adobe page map are already in \
                             the NCX <pageList>, so only the attribute went",
                            named.len()
                        ));
                    } else {
                        let more = absent.len().saturating_sub(3);
                        absent.truncate(3);
                        outcome.push_finding(format!(
                            "{} already has a <pageList> and it is not the whole story: the \
                             Adobe page map names {} page(s) it does not [{}{}]. Merging two \
                             paginations is a judgement about which is right, so the page map \
                             is left where it is",
                            basename(&ncx_name),
                            absent.len() + more,
                            absent.join(", "),
                            if more > 0 {
                                format!(", and {more} more")
                            } else {
                                String::new()
                            }
                        ));
                    }
                } else if let Some(close) = ncx.find("</navMap>")
                    && let Some((list, written)) = page_list(book, &map_name, &ncx_name)
                {
                    let at = ncx[close..].find('\n').map_or(ncx.len(), |i| close + i + 1);
                    let mut edits = Edits::new();
                    edits.insert(at, list);
                    book.set_text(&ncx_name, edits.apply(&ncx));
                    outcome.push_change(format!(
                        "moved {written} print page number(s) from the Adobe page map into the \
                         NCX <pageList>, where a reading system can use them"
                    ));
                }
            }
            (Some(map_name), None) => outcome.push_finding(format!(
                "the book has no NCX, so the print page numbers in {} have nowhere standard to \
                 go and the page map is left where it is",
                basename(&map_name)
            )),
            (None, _) => {}
        }

        let mut edits = Edits::new();
        for attr in nodes
            .iter()
            .filter(|n| n.name == "spine")
            .filter_map(|n| n.attr("page-map"))
        {
            edits.delete(attr.span_with_space.clone());
        }
        book.set_text(&opf_name, edits.apply(&opf));
        outcome.push_change("removed spine/@page-map");
        outcome
    }
}

/// RSC-005: an EPUB 2 `<spine>` with no `toc` attribute.
///
/// EPUB 2 finds the NCX through `spine/@toc`, and a book without one is broken
/// against its own package even when the NCX sits right there in the manifest —
/// *Dune* is exactly this: a full NCX, and no pointer at it. The repair is a
/// single attribute and fully determined: when the manifest declares exactly
/// one NCX, that is the table of contents and the only question epubcheck has
/// an answer to. Zero NCX items or several leave nothing derivable, so those
/// are reported instead.
///
/// EPUB 3 books are deliberately untouched: the attribute was removed there,
/// and writing it would trade the error for a warning.
pub struct SpineToc;

impl Fixer for SpineToc {
    fn name(&self) -> &'static str {
        "spine-toc"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "point EPUB 2 spine/@toc at the NCX the manifest already declares"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() >= 3 {
            return Outcome::none();
        }
        let mut outcome = Outcome::none();
        let Some(name) = book.opf_name().map(str::to_owned) else {
            return outcome;
        };
        let Some(text) = book.text(&name).map(str::to_owned) else {
            return outcome;
        };
        let Ok(nodes) = scan(&text) else {
            return outcome;
        };

        let Some(spine) = nodes
            .iter()
            .find(|n| n.name == "spine" && n.kind != NodeKind::End)
        else {
            return outcome;
        };
        // An empty `toc=""` is not a pointer. Measured, it is worse than the
        // missing attribute it looks like: `value of attribute "toc" is
        // invalid` *and* `OPF-049 Item id "" was not found in the manifest`,
        // two errors where an absent one is a single error. So the two cases
        // are one defect, and differ only in whether the repair writes the
        // attribute or overwrites it.
        let existing = spine.attr("toc").filter(|a| a.value.trim().is_empty());
        if spine.attr("toc").is_some() && existing.is_none() {
            return outcome;
        }

        let ncx_ids: Vec<&str> = nodes
            .iter()
            .filter(|n| n.name == "item" && n.kind != NodeKind::End)
            .filter(|n| {
                n.attr("media-type")
                    .is_some_and(|m| m.value == "application/x-dtbncx+xml")
            })
            .filter_map(|n| n.attr("id").map(|a| a.value.as_str()))
            .collect();
        match ncx_ids.as_slice() {
            [only] => {
                let mut edits = Edits::new();
                match existing {
                    Some(a) => edits.replace(a.span.clone(), format!("toc=\"{only}\"")),
                    None => edits.insert(spine.name_end, format!(" toc=\"{only}\"")),
                }
                book.set_text(&name, edits.apply(&text));
                outcome.push_change(format!("pointed spine/@toc at the NCX (\"{only}\")"));
            }
            [] => outcome.push_finding(
                "spine has no toc attribute and the manifest declares no NCX to point it at"
                    .to_string(),
            ),
            many => outcome.push_finding(format!(
                "spine has no toc attribute and the manifest declares {} NCX entries ({}), so \
                 there is no way to tell which is the table of contents",
                many.len(),
                many.join(", ")
            )),
        }
        outcome
    }
}

/// CSS-007, OPF-035, OPF-037: a manifest `media-type` that is wrong or
/// superseded.
///
/// Three epubcheck codes, one repair — replace the declared type with the one
/// that means the same thing today — so they are one fixer rather than three.
/// The mapping is a fixed table, and every entry is a rename with no judgement
/// in it: the resource is unchanged and only the label the package puts on it
/// moves to the current spelling.
///
/// * **CSS-007** is a doubled prefix, `application/application/x-font-ttf`,
///   which is simply a typo some tool wrote.
/// * **OPF-035** is `text/html` in an EPUB 2 package, where content documents
///   are XHTML and must say so.
/// * **OPF-037** is the OEBPS 1.2 vocabulary — `text/x-oeb1-document` and
///   `text/x-oeb1-css` — left behind by a converter.
///
/// Measured: each of the three validates clean once the type is corrected.
///
/// # A fourth source, which is the file rather than the table
///
/// The table above is a pure relabelling — the same resource under its current
/// name. `OPF-029` is the other direction: the label disagrees with the *bytes*.
/// A Gutenberg *Dracula* declares `image/jpeg` for a `cover.jpg` that is a PNG,
/// and epubcheck reads the signature and objects. So an item whose file sniffs
/// to a known image format and whose declared type is a *different* image type
/// is corrected from the bytes.
///
/// Only image-to-image, deliberately. A font or a stylesheet whose declared
/// type looks odd is not something four magic bytes should be allowed to
/// overrule, and the extension half of the same defect belongs to
/// [`crate::fixers::filenames::UnsafeFilenames`], which is where renames live.
pub struct MediaTypes;

/// `wrong media type -> what it means now`.
const MEDIA_TYPES: &[(&str, &str)] = &[
    // CSS-007: a doubled prefix, and the modern spelling of the font type.
    (
        "application/application/x-font-ttf",
        "application/vnd.ms-opentype",
    ),
    // OPF-037: the OEBPS 1.2 vocabulary.
    ("text/x-oeb1-document", "application/xhtml+xml"),
    ("text/x-oeb1-css", "text/css"),
    // OPF-035: HTML where an EPUB wants XHTML.
    ("text/html", "application/xhtml+xml"),
];

impl Fixer for MediaTypes {
    fn name(&self) -> &'static str {
        "media-types"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["CSS-007", "OPF-035", "OPF-037"]
    }
    fn description(&self) -> &'static str {
        "replace a manifest media-type that is mistyped or superseded"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let mut edits = Edits::new();
        let mut fixed: Vec<String> = Vec::new();

        for node in nodes.iter().filter(|n| n.name == "item") {
            let Some(attr) = node.attr("media-type") else {
                continue;
            };
            // Exact match on the whole attribute value. A substring rewrite
            // would turn "text/html-something" into nonsense.
            let declared = attr.value.trim();
            let from_table = MEDIA_TYPES
                .iter()
                .find(|(wrong, _)| declared == *wrong)
                .map(|(_, right)| *right);
            // Else ask the file itself, for an image whose label its bytes
            // contradict.
            let from_bytes = || {
                if !declared.starts_with("image/") {
                    return None;
                }
                let target = node
                    .attr("href")
                    .and_then(|h| resolve_href(&opf_name, &h.value))
                    .map(|(t, _)| t)?;
                let (_, real) = crate::util::sniff_image(book.bytes(&target)?)?;
                (real != declared).then_some(real)
            };
            let Some(right) = from_table.or_else(from_bytes) else {
                continue;
            };
            edits.replace(attr.span.clone(), format!("media-type=\"{right}\""));
            fixed.push(declared.to_string());
        }

        if fixed.is_empty() {
            return Outcome::none();
        }
        book.set_text(&opf_name, edits.apply(&text));
        let count = fixed.len();
        fixed.sort();
        fixed.dedup();
        Outcome::change(format!(
            "corrected {count} manifest media-type(s) [{}]",
            fixed.join(", ")
        ))
    }
}

/// OPF-091 and OPF-099: manifest items that cannot mean what they say.
///
/// Both are the package document describing itself wrongly, and both have one
/// right answer, so they share a fixer.
///
/// **OPF-091** — `<item href="chapter.xhtml#part2">`. A manifest entry names a
/// *resource*, and a fragment names something inside one, so the two cannot be
/// combined: there is no file called `chapter.xhtml#part2`. Dropping the
/// fragment leaves the item pointing at the file it was always about, and it
/// takes an RSC-001 and an RSC-008 with it, since the phantom filename was also
/// being reported as missing and undeclared.
///
/// **OPF-099** — the manifest listing the package document itself. A manifest
/// is the list of everything *else*; the OPF is what does the listing, and an
/// entry for it describes nothing. Removing that one entry is the whole repair.
pub struct ManifestItems;

impl Fixer for ManifestItems {
    fn name(&self) -> &'static str {
        "manifest-items"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-091", "OPF-099"]
    }
    fn description(&self) -> &'static str {
        "drop a fragment from a manifest href, and the entry a manifest makes for itself"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let mut edits = Edits::new();
        let (mut trimmed, mut dropped) = (0u32, 0u32);

        for node in nodes.iter().filter(|n| n.name == "item") {
            let Some(href) = node.attr("href") else {
                continue;
            };
            let Some((target, fragment)) = resolve_href(&opf_name, &href.value) else {
                continue;
            };
            if target == opf_name {
                edits.delete(line_span(&text, node.element_span(&nodes)));
                dropped += 1;
            } else if fragment.is_some() {
                let path = href.value.split_once('#').map_or("", |(p, _)| p);
                edits.replace(href.span.clone(), format!("href=\"{path}\""));
                trimmed += 1;
            }
        }

        if trimmed == 0 && dropped == 0 {
            return Outcome::none();
        }
        book.set_text(&opf_name, edits.apply(&text));

        let mut outcome = Outcome::none();
        if trimmed > 0 {
            outcome.push_change(format!(
                "dropped the fragment from {trimmed} manifest href(s), which name files"
            ));
        }
        if dropped > 0 {
            outcome.push_change(format!(
                "removed {dropped} manifest entr(ies) for the package document itself"
            ));
        }
        outcome
    }
}

/// OPF-031, RSC-001, RSC-007: a manifest item or guide reference whose file is
/// not where the package says it is.
///
/// `dangling-resources` has resolved this class since the beginning, and could
/// never see it here: it walks [`Book::markup_names`], and the package document
/// is not markup. So the one file whose whole job is to say where things are was
/// the one file nobody checked. Seventeen books in a 200-book library have a
/// defect of this shape, and all of them came back "nothing I can fix".
///
/// The [`Resolver`] does the deciding, exactly as it does for a content
/// document, and the answer is usually that the file is *there* and the path is
/// wrong:
///
/// * An abbyy-to-epub build of *True Hallucinations* keeps its twenty chapters
///   at the archive root. The manifest says `../chapter0001.html` and is right;
///   the guide says `chapter0001.html` and is missing the `../`. Forty errors
///   from one absent prefix.
/// * A Calibre build of *Skylark* points its cover landmark at `cover.xhtml`
///   when the file is `Text/cover.xhtml`.
///
/// This is worth being careful about, and the first version of it was not.
/// Reading the epubcheck log alone, "not declared in manifest" *plus* "could not
/// be found" for twenty chapters reads as a phantom apparatus to be swept away,
/// and deleting the twenty manifest items and spine entries measures as a clean
/// book — with twenty of its twenty-one documents unreachable. Only the archive
/// listing shows what is really wrong. Hence: repointing is tried first and
/// always wins, and removal happens only where [`Resolution::Missing`] says the
/// file is in the archive under no path, spelling or case at all.
///
/// A removed `<item>` takes its `<itemref>` with it, since a spine entry naming
/// an id that no longer exists is a new error in place of the old one. Structural
/// items — the nav document, the NCX the spine names — are reported instead:
/// they are required to exist, so their absence needs a person, not a deletion.
pub struct PackageReferences;

/// Is this manifest item one the book cannot simply lose?
///
/// Three ways to qualify, and the third is the one that matters most.
///
/// The nav document and the NCX the spine names are required to exist, so their
/// absence needs a person rather than a deletion. And **anything in the spine is
/// a document the reader was meant to read**: removing its entry takes it out of
/// the reading order, which is content loss that validates perfectly clean. That
/// is not a hypothetical — the first version of this fixer, working from the
/// epubcheck log alone, would have swept twenty chapters of *True Hallucinations*
/// out of the spine and reported a repaired book.
///
/// So a spine document whose file cannot be found is always reported. A
/// stylesheet, font or script is not in the spine and can go.
fn is_structural(node: &crate::markup::Node, spine_toc: &str, in_spine: &[String]) -> bool {
    node.attr("properties")
        .is_some_and(|p| p.value.split_whitespace().any(|w| w == "nav"))
        || node.attr("id").is_some_and(|a| {
            (!spine_toc.is_empty() && a.value == spine_toc) || in_spine.contains(&a.value)
        })
}

impl Fixer for PackageReferences {
    fn name(&self) -> &'static str {
        "package-references"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-031", "RSC-001", "RSC-007"]
    }
    fn description(&self) -> &'static str {
        "repoint manifest and guide hrefs whose file moved, and drop the ones with no file at all"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };
        let resolver = Resolver::new(book);
        let spine_toc = nodes
            .iter()
            .find(|n| n.name == "spine")
            .and_then(|n| n.attr("toc"))
            .map_or(String::new(), |a| a.value.clone());
        let in_spine: Vec<String> = nodes
            .iter()
            .filter(|n| n.name == "itemref")
            .filter_map(|n| n.attr("idref"))
            .map(|a| a.value.clone())
            .collect();

        let mut outcome = Outcome::none();
        let mut edits = Edits::new();
        let (mut repointed, mut dropped) = (0u32, 0u32);

        for node in nodes.iter().filter(|n| {
            matches!(n.kind, NodeKind::Start | NodeKind::Empty)
                && matches!(n.name.as_str(), "item" | "reference")
        }) {
            let Some(href) = node.attr("href") else {
                continue;
            };
            match resolver.resolve(&opf_name, &href.value) {
                Resolution::Fine | Resolution::NotOurs => {}
                Resolution::Moved { target } => {
                    // A `<reference>` may carry one, and dropping it turns
                    // "the cover page, at this anchor" into "the cover page",
                    // which validates and quietly loses the precision. A
                    // manifest `<item>` may not have one at all, and
                    // `manifest-items` has already stripped those, so keeping
                    // whatever is there is right for both.
                    let rel = as_url_path(&relative_to(&opf_name, &target));
                    let value = match href.value.split_once('#') {
                        Some((_, f)) if !f.is_empty() => format!("{rel}#{f}"),
                        _ => rel,
                    };
                    edits.replace(href.span.clone(), format!("href=\"{value}\""));
                    repointed += 1;
                }
                Resolution::Ambiguous { count } => outcome.push_finding(format!(
                    "<{}> in the package points at \"{}\", and {count} files share that name, \
                     so there is no way to tell which was meant",
                    node.name, href.value
                )),
                Resolution::Missing => {
                    if node.name == "item" && is_structural(node, &spine_toc, &in_spine) {
                        outcome.push_finding(format!(
                            "the package needs \"{}\" — it is in the reading order, or it is the \
                             book's table of contents — and no file of that name is in the book \
                             under any path, spelling or case. Removing the entry would take a \
                             document out of the book and leave a validator with nothing to \
                             complain about, so it needs a person",
                            href.value
                        ));
                        continue;
                    }
                    // No `<itemref>` can be left dangling by this: anything the
                    // spine names took the branch above.
                    edits.delete(line_span(&text, node.element_span(&nodes)));
                    dropped += 1;
                }
            }
        }

        if repointed == 0 && dropped == 0 {
            return outcome;
        }
        book.set_text(&opf_name, edits.apply(&text));
        if repointed > 0 {
            outcome.push_change(format!(
                "repointed {repointed} package reference(s) at the file they name"
            ));
        }
        if dropped > 0 {
            outcome.push_change(format!(
                "removed {dropped} package entr(ies) for files that are not in the book"
            ));
        }
        outcome
    }
}

/// RSC-005: `element "meta" not allowed anywhere` — an element pushed out of
/// the package namespace by `xmlns=""`.
///
/// A Sigil build of *Butcher's Crossing* writes, among ordinary metadata:
///
/// ```xml
/// <meta xmlns="" name="BNContentKind" content="book"/>
/// ```
///
/// `xmlns=""` *un*-declares the default namespace for that element, so this
/// `<meta>` is not in the OPF namespace at all — it is in no namespace, which is
/// why epubcheck says "not allowed anywhere" rather than "not allowed here". It
/// looks identical to its neighbours and is the only one that is wrong.
///
/// Removing the declaration puts the element back in the namespace every element
/// around it is already in, which is plainly what was meant: a vendor metadata
/// entry sitting among other `<meta name content>` entries. Deleting the element
/// measures the same, and keeps less — so the declaration goes and the metadata
/// stays.
///
/// Only inside the package document, and only where the package itself has a
/// default namespace to fall back into. Elsewhere `xmlns=""` may be deliberate.
pub struct NamespaceEscapes;

impl Fixer for NamespaceEscapes {
    fn name(&self) -> &'static str {
        "namespace-escapes"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "put a package element that xmlns=\"\" pushed out of the OPF namespace back into it"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };
        // Nothing to fall back into means nothing to repair.
        if !nodes
            .iter()
            .any(|n| n.name == "package" && n.attr("xmlns").is_some())
        {
            return Outcome::none();
        }

        let mut edits = Edits::new();
        let mut fixed = 0u32;
        for node in nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Start | NodeKind::Empty))
        {
            if let Some(attr) = node.attr("xmlns")
                && attr.value.trim().is_empty()
            {
                edits.delete(attr.span_with_space.clone());
                fixed += 1;
            }
        }

        if fixed == 0 {
            return Outcome::none();
        }
        book.set_text(&opf_name, edits.apply(&text));
        Outcome::change(format!(
            "put {fixed} package element(s) back in the OPF namespace that an empty xmlns had \
             pushed out of it"
        ))
    }
}

/// RSC-008: `Referenced resource … is not declared in the OPF manifest`.
///
/// The mirror of [`PackageReferences`]: there the manifest names a file that is
/// not in the archive, here the archive holds a file the manifest never mentions
/// and something in the book uses. Both are the package document being out of
/// step with the archive, and both are repaired by making it agree.
///
/// *Skylark* is the case, and it is one this tool created. Its stylesheet asks
/// for `..Fonts/AGaramondPro-Regular.otf` — a missing slash — and `css-paths`
/// corrects that to `../Fonts/…`, which exists. The font was undeclared all
/// along; correcting the path is what let epubcheck reach it and say so. A
/// repair that trades RSC-007 for RSC-008 is not a repair, so this finishes it.
///
/// Only files something actually references. A file nothing points at is not an
/// error — epubcheck says nothing about it — and adding manifest entries for
/// stray archive contents would be inventing work.
pub struct UndeclaredResources;

/// Media type by extension, for a file the manifest has to be told about.
///
/// Measured, on both rulesets: epubcheck does not check a font's declared type
/// against its contents, and `application/vnd.ms-opentype`, `font/otf`,
/// `application/x-font-ttf` and `application/font-sfnt` are all accepted for the
/// same `.otf`. So the one entry both EPUB versions have always allowed is the
/// one used, and there is no need for a version-dependent table.
const BY_EXTENSION: &[(&str, &str)] = &[
    (".xhtml", "application/xhtml+xml"),
    (".html", "application/xhtml+xml"),
    (".htm", "application/xhtml+xml"),
    (".css", "text/css"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".png", "image/png"),
    (".gif", "image/gif"),
    (".svg", "image/svg+xml"),
    (".webp", "image/webp"),
    (".otf", "application/vnd.ms-opentype"),
    (".ttf", "application/vnd.ms-opentype"),
    (".woff", "application/font-woff"),
    (".woff2", "font/woff2"),
    (".ncx", "application/x-dtbncx+xml"),
    (".js", "application/javascript"),
    (".mp3", "audio/mpeg"),
    (".m4a", "audio/mp4"),
    (".mp4", "video/mp4"),
    (".smil", "application/smil+xml"),
    (".pls", "application/pls+xml"),
];

impl Fixer for UndeclaredResources {
    fn name(&self) -> &'static str {
        "undeclared-resources"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-008"]
    }
    fn description(&self) -> &'static str {
        "declare a file the book uses that the manifest never mentions"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };
        let Some(manifest) = nodes
            .iter()
            .position(|n| n.name == "manifest" && n.kind == NodeKind::Start)
        else {
            return Outcome::none();
        };
        let Some(close) = nodes[manifest].close else {
            return Outcome::none();
        };

        let declared: std::collections::HashSet<String> = nodes
            .iter()
            .filter(|n| n.name == "item")
            .filter_map(|n| n.attr("href"))
            .filter_map(|a| resolve_href(&opf_name, &a.value).map(|(t, _)| t))
            .collect();
        let mut taken: std::collections::HashSet<String> = nodes
            .iter()
            .filter_map(|n| n.attr("id"))
            .map(|a| a.value.clone())
            .collect();

        let used = crate::refs::referenced_targets(book);
        let mut outcome = Outcome::none();
        let mut added: Vec<String> = Vec::new();
        let mut items = String::new();
        let indent = line_indent(&text, nodes[close].span.start);

        for name in book.names() {
            // The package cannot list itself, the OCF entries are outside it,
            // and anything already declared is done.
            if *name == opf_name
                || name == "mimetype"
                || name.starts_with("META-INF/")
                || declared.contains(name)
                || !used.contains(name)
            {
                continue;
            }
            let lower = name.to_ascii_lowercase();
            let Some((_, media)) = BY_EXTENSION.iter().find(|(ext, _)| lower.ends_with(ext)) else {
                outcome.push_finding(format!(
                    "\"{name}\" is used by the book but not in the manifest, and its extension \
                     names no media type this knows; declaring it with the wrong type would be \
                     a worse error than leaving it undeclared"
                ));
                continue;
            };
            let id = crate::fixers::ids::unique_id(basename(name), &taken);
            taken.insert(id.clone());
            let href = as_url_path(&relative_to(&opf_name, name));
            let _ = writeln!(
                items,
                "{indent}  <item id=\"{id}\" href=\"{href}\" media-type=\"{media}\"/>"
            );
            added.push(basename(name).to_string());
        }

        if added.is_empty() {
            return outcome;
        }
        let mut edits = Edits::new();
        edits.insert(line_span(&text, nodes[close].span.clone()).start, items);
        book.set_text(&opf_name, edits.apply(&text));
        outcome.push_change(format!(
            "declared {} file(s) the book uses but the manifest did not list [{}]",
            added.len(),
            added.join(", ")
        ));
        outcome
    }
}

/// Guide `type` values that say "this file is the book's cover image".
///
/// `cover` is the reserved OPF 2.0.1 spelling. `other.ms-coverimage-standard`
/// is Microsoft Reader's, and it is not a guess: the Penguin *Iliad* carries it
/// beside `other.ms-thumbimage-standard` and `other.ms-thumbimage`, all three
/// pointing straight at JPEGs, which is where this whole class was first seen.
const COVER_TYPES: &[&str] = &["cover", "other.ms-coverimage-standard"];

/// A guide reference's `type`, folded to lower case, or empty if it has none.
fn guide_type(node: &Node) -> String {
    node.attr("type")
        .map_or(String::new(), |a| a.value.to_ascii_lowercase())
}

/// The documents that show `target`, narrowed as far as the archive allows.
///
/// One candidate is an answer and the caller repoints at it; that uniqueness is
/// what makes the retarget a derivation rather than a preference, and it is the
/// same rule the landmarks nav is built with. More than one is a question only a
/// person can settle — a plate that appears both on a cover page and in a list
/// of illustrations gives no way to tell which the guide meant — so the list is
/// returned whole and reported rather than collapsed to a guess.
///
/// The single tie-break is the spine. Where several documents show the file but
/// only one of them is in the reading order, that one is the page a reader can
/// actually be sent to, and the others cannot be.
fn presenters(book: &Book, target: &str, spine: &[String]) -> Vec<String> {
    let showing: Vec<String> = book
        .markup_names()
        .into_iter()
        .filter(|doc| crate::fixers::resources::displays(book, doc, target))
        .collect();
    if showing.len() < 2 {
        return showing;
    }
    let in_spine: Vec<String> = showing
        .iter()
        .filter(|doc| spine.contains(doc))
        .cloned()
        .collect();
    if in_spine.len() == 1 {
        in_spine
    } else {
        showing
    }
}

/// What the guide already says, as `(type, document)` pairs.
///
/// Only the entries that are already doing their job — the ones landing on a
/// content document — since those are the ones a retarget could collide with.
fn destinations(references: &[&Node], opf_name: &str) -> Vec<(String, String)> {
    references
        .iter()
        .filter_map(|n| {
            let (target, _) = n
                .attr("href")
                .and_then(|h| resolve_href(opf_name, &h.value))?;
            ends_with_any(&target, crate::util::MARKUP).then(|| (guide_type(n), target))
        })
        .collect()
}

/// The report for a guide entry that several pages could equally well have
/// meant, naming them so the choice is a glance rather than a search.
fn ambiguous_guide(href: &str, candidates: &[String]) -> String {
    format!(
        "the guide points at \"{href}\", which is not a content document, and {} pages show \
         that file, so there is no way to tell which one it meant to send a reader to — \
         someone who knows the book can, in a second, and the entry is left intact for them \
         to do it [{}]",
        candidates.len(),
        candidates
            .iter()
            .map(|d| basename(d))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Write down which image is the cover, in whatever way this book's version
/// spells that, so dropping the guide entry that said so loses nothing.
///
/// EPUB 2 says it with `<meta name="cover" content="{manifest-id}"/>` in
/// `<metadata>`; EPUB 3 says it with `properties="cover-image"` on the manifest
/// item. Returns the edit and the line to report, or `None` when the book
/// already says it — the usual case, and the reason this is a guard rather than
/// a repair — or when there is nothing to say it about, the image having no
/// manifest entry at all.
fn record_cover(
    book: &Book,
    opf_name: &str,
    text: &str,
    nodes: &[Node],
    image: &str,
) -> Option<(std::ops::Range<usize>, String, String)> {
    let item = nodes.iter().find(|n| {
        n.name == "item"
            && n.attr("href")
                .and_then(|h| resolve_href(opf_name, &h.value))
                .is_some_and(|(t, _)| t == image)
    })?;

    if book.epub_version() >= 3 {
        if nodes.iter().any(|n| {
            n.name == "item"
                && n.attr("properties")
                    .is_some_and(|p| p.value.split_whitespace().any(|w| w == "cover-image"))
        }) {
            return None;
        }
        let note = format!(
            "declared {} the cover image before dropping the guide entry that was the only \
             thing saying so",
            basename(image)
        );
        if let Some(p) = item.attr("properties") {
            return Some((
                p.span.clone(),
                format!("properties=\"{} cover-image\"", p.value),
                note,
            ));
        }
        let at = item.attrs.last()?.span.end;
        return Some((at..at, " properties=\"cover-image\"".to_string(), note));
    }

    if nodes.iter().any(|n| {
        n.name == "meta"
            && n.attr("name")
                .is_some_and(|a| a.value.eq_ignore_ascii_case("cover"))
    }) {
        return None;
    }
    let id = item.attr("id")?.value.clone();
    let metadata = nodes
        .iter()
        .position(|n| n.name == "metadata" && n.kind == NodeKind::Start)?;
    let close = nodes[metadata].close?;

    // Same placement rule as `DcLanguage`: line up with the other children of
    // <metadata>, and insert at the start of the closing tag's line so the
    // indentation already sitting there is not counted twice.
    let close_at = nodes[close].span.start;
    let line_start = text[..close_at].rfind('\n').map_or(0, |i| i + 1);
    let indent = nodes
        .iter()
        .rfind(|n| n.parent == Some(metadata) && n.kind != NodeKind::End)
        .map_or_else(
            || format!("{}  ", line_indent(text, close_at)),
            |last| line_indent(text, last.span.start),
        );
    Some((
        line_start..line_start,
        format!("{indent}<meta name=\"cover\" content=\"{id}\"/>\n"),
        format!(
            "recorded <meta name=\"cover\"> for {} before dropping the guide entry that was \
             the only thing saying so",
            basename(image)
        ),
    ))
}

/// OPF-032: `Guide references "…" which is not a valid "OPS Content Document"`.
///
/// The `guide` is the EPUB 2 predecessor of the landmarks nav, and every
/// `<reference>` in it is required to point at a content document. Conversion
/// tools sometimes point one at the cover *image* instead of the cover page —
/// the Penguin *Iliad* has three `.jpg` references. A reading system cannot
/// navigate to a JPEG, so the entry does nothing but fail validation.
///
/// # Retarget first, remove second
///
/// Deleting the reference validates, and for a long time that was all this did.
/// It is the lossy answer, and on the book it was written for it was the wrong
/// one. `home_9781101153635_msr_cvi_r1.jpg` is not an orphan: the spine's first
/// document, `home_9781101153635_oeb_cover_r1.html`, is a page whose entire
/// body is an `<img>` of exactly that file. The publisher pointed the guide at
/// the picture instead of at the page holding it, and the page is right there.
/// So the reference is repointed at the document that shows the file, and the
/// `type` and `title` are kept — a landmark recovered rather than a landmark
/// thrown away.
///
/// Removal is what happens in the two cases where nothing is lost by it.
///
/// **No document shows the file.** Nothing in the book presents it, so there is
/// no page to send a reader to and the entry cannot be made to mean anything.
/// The *Iliad*'s two Microsoft Reader thumbnails are this: EPUB 2 has nowhere
/// to record "this is the PPC thumbnail", and the file and its manifest item
/// stay regardless.
///
/// **The retarget would land on a `type`/href pair the guide already has.**
/// Measured, that is not a repair but a trade:
///
/// ```text
/// two <reference>, same type, same href       RSC-017 Duplicate "reference"
///                                             elements with the same "type"
///                                             and "href" attributes
/// two <reference>, same type, different href  clean
/// ```
///
/// So the pair is the right granularity, and creating the duplicate would buy a
/// silenced OPF-032 with a new RSC-017. The surviving entry carries the same
/// semantic type to the same destination, so no landmark is lost — only the
/// dropped entry's `title`, which is a second label for one place.
///
/// # Several documents show it, and only a person can say which
///
/// That is not a case for either branch. Repointing at whichever was seen first
/// would be a guess, and a landmark going to the wrong page is worse than one
/// that was never repaired; removing it throws away a recoverable entry to
/// silence a validator. So the entry is left exactly as it is and reported,
/// which is what [`PackageReferences`] and `orphan-links` already do with their
/// own ambiguities. The book keeps its OPF-032 and someone who knows it can
/// settle the question in a second.
///
/// # Nothing may take the last statement of what the cover is
///
/// A `type="cover"` reference pointing at an image is a mangled spelling of a
/// real fact, and removing it can be the moment the book stops saying which
/// picture is its cover. So before that entry goes, [`record_cover`] makes sure
/// the fact is written where its version keeps it — `<meta name="cover">` under
/// EPUB 2, `properties="cover-image"` under EPUB 3 — and only then is the
/// removal a tidy-up instead of a loss.
///
/// This is the general shape of the rule, and it is worth stating for whatever
/// pass comes next: **no fixer may remove the last inbound reference to a
/// manifest item without first checking that what the reference meant is
/// recorded somewhere else.** Nothing here sweeps unreferenced manifest items
/// today. The day something does, this fixer and that one are individually
/// correct and jointly capable of deleting a book's cover.
///
/// A reference to a missing file is a different defect with a different repair
/// and is left to [`PackageReferences`], which runs first; a reference whose
/// fragment does not exist is left to `BrokenFragments`. Both of those can
/// recover the link, and deleting it first would take the chance away.
///
/// # The guide may not be left empty
///
/// `<!ELEMENT guide (reference+)>` — one reference at minimum. So a book whose
/// guide holds nothing but bad entries cannot simply have them removed:
/// measured, that is `element "guide" incomplete`, one error traded for another,
/// and the whole `<guide>` has to go with them. It is optional in EPUB 2 and
/// gone in EPUB 3, so removing it costs nothing.
///
/// The check is unconditional rather than a tidy-up after this fixer's own
/// deletions, because the emptying can happen anywhere — `PackageReferences`
/// removing the last dead entry, or a book that simply arrived with `<guide/>`.
pub struct GuideReferences;

impl Fixer for GuideReferences {
    fn name(&self) -> &'static str {
        "guide-references"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-032", "RSC-005"]
    }
    fn description(&self) -> &'static str {
        "repoint a guide entry naming a file at the document that shows it, and drop the rest"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let present: Vec<String> = book.names().to_vec();
        let spine = crate::refs::spine_documents(book);

        let guide = nodes
            .iter()
            .position(|n| n.name == "guide" && n.kind == NodeKind::Start);
        let references: Vec<&Node> = nodes
            .iter()
            .filter(|n| {
                n.name == "reference" && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
            })
            .collect();

        let mut said = destinations(&references, &opf_name);
        let mut outcome = Outcome::none();
        let mut edits = Edits::new();
        let mut deletions: Vec<std::ops::Range<usize>> = Vec::new();
        let mut retargeted = 0u32;
        let mut cover_recorded = false;

        for node in &references {
            let Some(href) = node.attr("href") else {
                continue;
            };
            let Some((target, _)) = resolve_href(&opf_name, &href.value) else {
                continue;
            };
            // A target that is not in the book at all is somebody else's
            // problem, and one that is markup is doing its job.
            if !present.contains(&target) || ends_with_any(&target, crate::util::MARKUP) {
                continue;
            }

            let kind = guide_type(node);
            let candidates = presenters(book, &target, &spine);
            if let [doc] = candidates.as_slice() {
                // Falls through to removal when the guide already carries this
                // destination under this type: retargeting would only add the
                // duplicate epubcheck reports as RSC-017.
                if !said.contains(&(kind.clone(), doc.clone())) {
                    let rel = as_url_path(&relative_to(&opf_name, doc));
                    edits.replace(href.span.clone(), format!("href=\"{rel}\""));
                    said.push((kind, doc.clone()));
                    retargeted += 1;
                    continue;
                }
            } else if candidates.len() > 1 {
                outcome.push_finding(ambiguous_guide(&href.value, &candidates));
                continue;
            }

            // Only once per book: the second insertion would be a second
            // <meta name="cover">, which is a new defect in place of the old.
            if !cover_recorded
                && COVER_TYPES.contains(&kind.as_str())
                && let Some((span, with, note)) =
                    record_cover(book, &opf_name, &text, &nodes, &target)
            {
                edits.replace(span, with);
                outcome.push_change(note);
                cover_recorded = true;
            }
            deletions.push(line_span(&text, node.element_span(&nodes)));
        }

        let dropped = deletions.len();

        // Nothing would be left inside it, so the element goes too — an empty
        // <guide> is an error of its own.
        if let Some(guide) = guide
            && dropped == references.len()
        {
            edits.delete(line_span(&text, nodes[guide].element_span(&nodes)));
            book.set_text(&opf_name, edits.apply(&text));
            outcome.push_change(if dropped == 0 {
                "removed an empty <guide>, which needs at least one reference".to_string()
            } else {
                format!(
                    "removed the <guide> and all {dropped} of its references, none of which \
                     pointed at a content document"
                )
            });
            return outcome;
        }

        if retargeted == 0 && dropped == 0 {
            return outcome;
        }
        for span in deletions {
            edits.delete(span);
        }
        book.set_text(&opf_name, edits.apply(&text));
        if retargeted > 0 {
            outcome.push_change(format!(
                "repointed {retargeted} guide reference(s) at the document that shows the file \
                 they named"
            ));
        }
        if dropped > 0 {
            outcome.push_change(format!(
                "removed {dropped} guide reference(s) pointing at a non-content file"
            ));
        }
        outcome
    }
}

/// RSC-005: `element "metadata" incomplete; missing required element
/// "dc:language"`.
///
/// Required by both EPUB versions, and position inside `<metadata>` does not
/// matter — measured, inserted first and last, both clean. What epubcheck does
/// *not* do is check the value: `<dc:language>zz</dc:language>` validates
/// without a murmur. So this is a field where being wrong is silent, and it is
/// worth being careful about, since reading systems hyphenate and choose
/// speech voices from it.
///
/// See [`crate::language`] for how the value is arrived at. The short version:
/// what the documents already declare, else what the text says, else nothing.
/// Never the system locale, which is what Calibre does and is how an English
/// machine turns a Hungarian novel into `<dc:language>en</dc:language>`.
pub struct DcLanguage {
    pub policy: Policy,
}

impl Default for DcLanguage {
    fn default() -> Self {
        DcLanguage {
            policy: Policy::EnglishOnly,
        }
    }
}

impl Fixer for DcLanguage {
    fn name(&self) -> &'static str {
        "dc-language"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "add the required <dc:language>, taken from the documents or from the text"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(opf) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&opf) else {
            return Outcome::none();
        };

        // Only an absent element is an error. One that is present but odd is
        // the author's business, and epubcheck does not mind either way.
        if nodes
            .iter()
            .any(|n| n.name == "language" && n.kind != NodeKind::End)
        {
            return Outcome::none();
        }
        let Some(metadata) = nodes
            .iter()
            .position(|n| n.name == "metadata" && n.kind == NodeKind::Start)
        else {
            return Outcome::none();
        };
        let Some(close) = nodes[metadata].close else {
            return Outcome::none();
        };

        let Some(found) = language::detect(book, self.policy) else {
            let mut outcome = Outcome::finding(match language::observed(book) {
                Some(seen) => format!(
                    "no <dc:language>, and the text reads as \"{}\" rather than English, so \
                     nothing was written — set it by hand, or re-run with \
                     --language-detect=any",
                    seen.language
                ),
                None => "no <dc:language>, and neither the documents nor the text say what \
                         language the book is in; it needs setting by hand"
                    .to_string(),
            });
            outcome.push_finding(
                "a wrong <dc:language> is worse than a missing one: reading systems hyphenate \
                 and choose speech voices from it"
                    .to_string(),
            );
            return outcome;
        };

        // Match the prefix the file already uses for Dublin Core rather than
        // assuming `dc:` — it is a namespace binding, not a fixed spelling.
        let prefix = dc_prefix(&opf).unwrap_or_else(|| "dc".to_string());

        // Line up with the other children of <metadata> rather than with the
        // closing tag, and insert at the start of its line so the indentation
        // already sitting there is not counted twice.
        let close_at = nodes[close].span.start;
        let line_start = opf[..close_at].rfind('\n').map_or(0, |i| i + 1);
        let indent = nodes
            .iter()
            .rfind(|n| n.parent == Some(metadata) && n.kind != NodeKind::End)
            .map_or_else(
                || format!("{}  ", line_indent(&opf, close_at)),
                |last| line_indent(&opf, last.span.start),
            );

        let mut edits = Edits::new();
        edits.insert(
            line_start,
            format!(
                "{indent}<{prefix}:language>{}</{prefix}:language>\n",
                found.language
            ),
        );
        book.set_text(&opf_name, edits.apply(&opf));

        let how = match found.source {
            language::Source::Declared => "which is what the content documents declare".to_string(),
            language::Source::Detected { samples, share } => {
                format!("detected from the text ({share} of {samples} samples)")
            }
        };
        Outcome::change(format!(
            "added <dc:language>{}</dc:language> — {how}",
            found.language
        ))
    }
}

/// The Dublin Core elements a package document must have, in both versions.
///
/// The spec that asked for this fixer named only `identifier` and `language`.
/// Measured, `title` belongs here too:
///
/// ```text
/// <dc:title/>  absent  EPUB 2  ERROR   element "metadata" incomplete
/// <dc:title/>  empty   EPUB 2  WARNING title tag is empty
/// <dc:title/>  empty   EPUB 3  ERROR   character content of element invalid
/// ```
///
/// So deleting an empty one trades a warning for a hard error under EPUB 2 and
/// one error for another under EPUB 3 — the section 5e mistake exactly.
const REQUIRED_DC: &[&str] = &["identifier", "title", "language"];

/// OPF-054: `Date value "" is not valid … zero-length string`.
///
/// A Harper Collins build of *The Hobbit* pads its metadata with empty elements:
///
/// ```xml
/// <dc:date/>
/// <dc:subject/>
/// <dc:description/>
/// <dc:rights/>
/// ```
///
/// Only `dc:date` is reported, because it is the only one of the four with a
/// value grammar to violate — the empty string is not a W3C date. The other
/// three are legal and equally meaningless. An element asserting nothing is not
/// information, so removing it cannot lose any.
///
/// The exception is [`REQUIRED_DC`], where the element must exist even when its
/// content is useless. Those are reported instead: an empty `<dc:title/>` is a
/// real defect, but it needs a title, not a deletion, and only a person knows
/// what the title is.
pub struct EmptyMetadata;

impl Fixer for EmptyMetadata {
    fn name(&self) -> &'static str {
        "empty-metadata"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-054"]
    }
    fn description(&self) -> &'static str {
        "remove <dc:*> metadata elements with no content, keeping the required ones"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };
        // No prefix bound to Dublin Core means no `dc:*` elements to weigh up.
        let Some(prefix) = dc_prefix(&text) else {
            return Outcome::none();
        };
        let Some(metadata) = nodes
            .iter()
            .position(|n| n.name == "metadata" && n.kind == NodeKind::Start)
        else {
            return Outcome::none();
        };

        let mut outcome = Outcome::none();
        let mut edits = Edits::new();
        let mut removed: Vec<String> = Vec::new();

        // Start and Empty only: an end tag is the other half of an element
        // already considered, and a comment or processing instruction is not an
        // element at all.
        for node in nodes.iter().filter(|n| {
            n.parent == Some(metadata) && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
        }) {
            let raw = node.raw_name(&text);
            let Some(local) = raw.strip_prefix(&format!("{prefix}:")) else {
                continue;
            };
            // A start tag with no matching end is malformed, not empty; saying
            // otherwise would delete the element and everything after it.
            let empty = match (node.kind, node.close) {
                (NodeKind::Empty, _) => true,
                (NodeKind::Start, Some(close)) => text[node.span.end..nodes[close].span.start]
                    .trim()
                    .is_empty(),
                _ => false,
            };
            if !empty {
                continue;
            }
            if REQUIRED_DC.contains(&local.to_ascii_lowercase().as_str()) {
                outcome.push_finding(format!(
                    "<{raw}> is empty, and both EPUB versions require it — deleting it would \
                     trade one error for another, so it needs a real value"
                ));
                continue;
            }
            edits.delete(line_span(&text, node.element_span(&nodes)));
            removed.push(raw.to_string());
        }

        if !removed.is_empty() {
            book.set_text(&opf_name, edits.apply(&text));
            outcome.push_change(format!(
                "removed {} empty metadata element(s) [{}]",
                removed.len(),
                removed.join(", ")
            ));
        }
        outcome
    }
}

/// OPF-030: `The unique-identifier "X" was not found`.
///
/// `<package unique-identifier="X">` names the id of the `<dc:identifier>` that
/// is the book's identity. A Calibre build of *Either/Or* names
/// `p9781400846931` and its one `<dc:identifier opf:scheme="calibre">` carries
/// no `id` at all, so the package points at nothing. One error, and it takes
/// `ncx-uid` down with it: that fixer finds the identifier *by* this id, so
/// while the pointer dangles the NCX can never be synced either.
///
/// Two repairs, and which one applies depends on what the identifier already
/// has:
///
/// * **No `id`.** Give it the one the package is asking for. Nothing else in
///   the book can be referring to an id that does not exist, so this cannot
///   break a reference — it only creates the target that was always meant to be
///   there.
/// * **A different `id`.** Repoint the package attribute instead. Renaming an
///   id an element already carries would break any `refines="#id"` aimed at it,
///   and EPUB 3 metadata refinement does exactly that.
///
/// Only when the book has exactly one `<dc:identifier>`. With several, which one
/// is the book's identity is a decision with consequences — reading systems key
/// annotations and reading position on it — and the package's own answer has
/// been lost. So that is reported.
///
/// Measured: the real book is `1 ERROR(OPF-030)` before and clean after.
pub struct UniqueIdentifier;

impl Fixer for UniqueIdentifier {
    fn name(&self) -> &'static str {
        "unique-identifier"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-030"]
    }
    fn description(&self) -> &'static str {
        "make the package unique-identifier and the <dc:identifier> id agree"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        // Start and Empty only, here and below: an end tag carries no
        // attributes, and the "absent" branch writes one.
        let Some(package) = nodes
            .iter()
            .find(|n| n.name == "package" && matches!(n.kind, NodeKind::Start | NodeKind::Empty))
        else {
            return Outcome::none();
        };
        // No attribute at all is a schema error with a different repair, and
        // guessing a value for it is not this fixer's business.
        let Some(wanted_attr) = package.attr("unique-identifier") else {
            return Outcome::none();
        };
        let wanted = wanted_attr.value.trim().to_string();
        let wanted_span = wanted_attr.span.clone();

        // Already resolves: the id exists somewhere in the package document.
        if !wanted.is_empty()
            && nodes.iter().any(|n| {
                matches!(n.kind, NodeKind::Start | NodeKind::Empty)
                    && n.attr("id").is_some_and(|a| a.value == wanted)
            })
        {
            return Outcome::none();
        }

        let metadata = nodes
            .iter()
            .position(|n| n.name == "metadata" && n.kind == NodeKind::Start);
        // `Some(i)`, never a bare `metadata`: both sides are `Option`, so with
        // no `<metadata>` element the comparison is `None == None` and every
        // root-level element passes the filter. Nothing in a real package is
        // the root *and* an identifier, so this has never misfired — which is
        // exactly why it is worth spelling out rather than leaving to luck.
        let idents: Vec<&crate::markup::Node> = nodes
            .iter()
            .filter(|n| {
                n.name == "identifier"
                    && metadata.is_some_and(|m| n.parent == Some(m))
                    && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
            })
            .collect();

        let [ident] = idents[..] else {
            // No identifier at all, but the book may already say what its
            // identifier *is* — see [`isbn13`] and the note on this fixer.
            if idents.is_empty()
                && let Some(isbn) = isbn13(&wanted).or_else(|| isbn13(basename(&opf_name)))
                && let Some(edit) = write_identifier(&text, &nodes, &wanted, &isbn)
            {
                let mut edits = Edits::new();
                edits.insert(edit.0, edit.1);
                book.set_text(&opf_name, edits.apply(&text));
                return Outcome::change(format!(
                    "wrote the <dc:identifier> the package was already naming, from the \
                     ISBN the book spells out for itself (urn:isbn:{isbn})"
                ));
            }
            return Outcome::finding(if idents.is_empty() {
                format!(
                    "<package unique-identifier=\"{wanted}\"> names an id no element carries, \
                     and there is no <dc:identifier> to give it to — the book needs an \
                     identifier before it can name one"
                )
            } else {
                format!(
                    "<package unique-identifier=\"{wanted}\"> names an id no element carries, \
                     and the book has {} <dc:identifier> elements; which one is the book's \
                     identity decides what reading systems key annotations and reading \
                     position on, so it needs a person",
                    idents.len()
                )
            });
        };

        let mut edits = Edits::new();
        // Has an id of its own: move the pointer, not the target. Renaming an id
        // an element already carries would break any refines="#id" aimed at it.
        let change = if let Some(existing) = ident.attr("id") {
            let id = existing.value.clone();
            edits.replace(wanted_span, format!("unique-identifier=\"{id}\""));
            format!("pointed the package unique-identifier at the <dc:identifier> id \"{id}\"")
        } else {
            // No id: create the one the package is already asking for. An
            // unusable value gets sanitised and the package attribute follows
            // it, so the two still agree.
            let taken: std::collections::HashSet<String> = nodes
                .iter()
                .filter_map(|n| n.attr("id"))
                .map(|a| a.value.clone())
                .collect();
            let id = if wanted.is_empty() || is_bad_id(&wanted) || taken.contains(&wanted) {
                let new = crate::fixers::ids::unique_id(&wanted, &taken);
                edits.replace(wanted_span, format!("unique-identifier=\"{new}\""));
                new
            } else {
                wanted.clone()
            };
            edits.insert(ident.name_end, format!(" id=\"{id}\""));
            format!("gave the book's <dc:identifier> the id \"{id}\" the package names")
        };

        book.set_text(&opf_name, edits.apply(&text));
        Outcome::change(change)
    }
}

/// The prefix this package document binds Dublin Core to.
fn dc_prefix(opf: &str) -> Option<String> {
    // Asked of the parsed attributes rather than of the bytes. The substring
    // search this replaced looked for `="http://purl.org/dc/elements/1.1/"`
    // exactly, so a book spelling the binding with single quotes or a space
    // around the `=` — both perfectly legal XML — fell through to the "dc"
    // default and would have had an element written under the wrong prefix.
    scan(opf).ok()?.iter().find_map(|node| {
        node.attrs
            .iter()
            .find(|a| a.value.trim() == "http://purl.org/dc/elements/1.1/")
            .and_then(|a| a.name.strip_prefix("xmlns:"))
            .map(str::to_string)
    })
}

/// The whitespace opening the line that byte `at` sits on, so an inserted
/// element lines up with its neighbours.
fn line_indent(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[start..at]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect()
}

/// PKG-005, PKG-006, PKG-007: the OCF `mimetype` entry.
///
/// OCF requires the archive to open with an entry named `mimetype`, stored
/// uncompressed, holding exactly `application/epub+zip` and nothing else — no
/// trailing newline, no BOM, no ZIP extra field.
///
/// [`Book::save`] has always written exactly that, so any book this tool
/// rewrites comes out correct. The gap was that nothing *noticed*: a book whose
/// only defect was its mimetype entry drew "nothing to do", was never
/// rewritten, and kept both errors. The repair existed and could not reach
/// disk. So this fixer changes no bytes itself; it exists to report the
/// condition, which is what causes the archive to be repacked.
///
/// Found by working down epubcheck's message catalogue rather than from a book,
/// which is the argument for doing that: no book in the library has this defect,
/// and one with it would have been quietly returned unrepaired.
pub struct MimetypeEntry;

impl Fixer for MimetypeEntry {
    fn name(&self) -> &'static str {
        "mimetype"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["PKG-005", "PKG-006", "PKG-007"]
    }
    fn description(&self) -> &'static str {
        "put the OCF mimetype entry first, uncompressed, with exactly the required bytes"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let (first, stored, exact) = book.mimetype_state();
        let mut wrong: Vec<&str> = Vec::new();
        if !first {
            wrong.push("was not the first entry");
        }
        if !stored {
            wrong.push("was compressed");
        }
        if !exact {
            wrong.push("did not hold exactly \"application/epub+zip\"");
        }
        if wrong.is_empty() {
            return Outcome::none();
        }
        Outcome::change(format!(
            "rewrote the mimetype entry, which {}",
            wrong.join(" and ")
        ))
    }
}

/// OPF-007: `Re-declaration of reserved prefix "rendition"`.
///
/// The `rendition` vocabulary belongs to the EPUB format itself, so a package
/// that binds the prefix in its own `prefix` attribute is saying again what is
/// already true. *Quantum Mechanics* and *Tales from Shakespeare* both do, and
/// never use the prefix anywhere, so the repair is a deletion and nothing else.
///
/// # Only when the URI disagrees, which is the whole rule
///
/// Re-declaring the prefix is not itself the defect. Measured, epubcheck cares
/// about one thing — whether the URI the package binds is the reserved one:
///
/// ```text
/// rendition: http://www.idpf.org/vocab/rendition/#   clean
/// rendition: http://www.idpf.org/vocab/rendition#    OPF-007  (no slash)
/// rendition: http://example.com/other                OPF-007
/// schema:    http://schema.org/                      clean
/// ```
///
/// Both library books spell it `…/rendition#`, one slash short of the reserved
/// URI, which is exactly why they warn. A Hemingway *In Our Time* binds the
/// reserved URI precisely, epubcheck says nothing about it — and this pass
/// removed the declaration anyway, editing a book that validated 0/0/0/0 for no
/// gain at all. A repair with no defect behind it is not a repair.
///
/// So the URI comparison is the condition, and re-declaration on its own is
/// left alone. Only `rendition` is checked, because it is the only reserved
/// prefix any book in the library rebinds; the rule generalises to the others
/// the day one of them turns up.
///
/// A book that *does* use `rendition:` somewhere keeps its declaration even
/// when the URI is wrong: the prefix is doing real work there, and dropping the
/// binding would silently re-point every use of it at the reserved vocabulary.
pub struct ReservedPrefix;

/// The URI the `rendition` prefix already stands for, per EPUB itself.
const RENDITION_VOCAB: &str = "http://www.idpf.org/vocab/rendition/#";

impl Fixer for ReservedPrefix {
    fn name(&self) -> &'static str {
        "reserved-prefix"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-007"]
    }
    fn description(&self) -> &'static str {
        "drop a rendition prefix declaration that rebinds the reserved vocabulary to another URI"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(&opf_name).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };
        let Some(package) = nodes
            .iter()
            .find(|n| n.name == "package" && n.kind != NodeKind::End)
        else {
            return Outcome::none();
        };
        let Some(prefix) = package.attr("prefix") else {
            return Outcome::none();
        };
        // The attribute is a space-separated list of `name: url` pairs. The
        // declaration we want gone is the one EPUB already makes.
        let tokens: Vec<&str> = prefix.value.split_whitespace().collect();
        let Some(pos) = tokens.iter().position(|t| t.starts_with("rendition:")) else {
            return Outcome::none();
        };

        // Bound to the reserved URI, which is what the prefix already means:
        // saying so twice is not an error and epubcheck does not report it.
        let bound = if tokens[pos] == "rendition:" {
            tokens.get(pos + 1).copied().unwrap_or_default()
        } else {
            tokens[pos].trim_start_matches("rendition:")
        };
        if bound == RENDITION_VOCAB {
            return Outcome::none();
        }

        // Used anywhere beyond this very declaration? If so it must stay.
        let mut used = text.matches("rendition:").count() > 1;
        for name in book.names() {
            if name != &opf_name
                && let Some(other) = book.text(name)
                && other.contains("rendition:")
            {
                used = true;
                break;
            }
        }
        if used {
            return Outcome::finding(
                "the package binds the reserved rendition prefix to a different URI and the \
                 book uses the prefix, so it stays: dropping the binding would re-point every \
                 use of it at the reserved vocabulary, which is a change of meaning and not a \
                 repair"
                    .to_string(),
            );
        }

        // A prefix declaration is `name: url`, so the URL is the next token —
        // but only when the name stands alone. A book that wrote
        // `rendition:http://...` with no space has put the whole mapping in one
        // token, and taking the token after it as well would swallow the *next*
        // declaration's name and leave its URL orphaned. Spelling it that way is
        // already malformed; eating a live prefix on the way past would make it
        // worse rather than better.
        let mapping_is_one_token = tokens[pos] != "rendition:";
        let kept: Vec<&str> = tokens
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != pos && (mapping_is_one_token || *i != pos + 1))
            .map(|(_, t)| *t)
            .collect();
        let new_value = kept.join(" ");
        let mut edits = Edits::new();
        if new_value.is_empty() {
            edits.delete(prefix.span_with_space.clone());
        } else {
            edits.replace(prefix.span.clone(), format!("prefix=\"{new_value}\""));
        }
        book.set_text(&opf_name, edits.apply(&text));
        Outcome::change("removed the redundant rendition prefix declaration".to_string())
    }
}

/// OPF-016 and OPF-017: `<rootfile>` with no usable `full-path`.
///
/// `META-INF/container.xml` is how a reading system finds the package document,
/// and its `full-path` is the only thing in the archive that says where that
/// is. Missing (OPF-016) or empty (OPF-017), nothing can open the book — both
/// come with RSC-003, "no rootfile tag with media type
/// application/oebps-package+xml was found".
///
/// It is repairable because the answer is in the archive: there is a package
/// document, and this tool has already found it — by extension, which is how it
/// can read a book whose container is broken in the first place. Writing that
/// path into the attribute is not a guess, it is copying down where the file
/// actually is.
///
/// Only when exactly one package document exists. A multiple-rendition EPUB has
/// several and which one is primary is a real decision, so that is reported.
pub struct ContainerRootfile;

const CONTAINER: &str = "META-INF/container.xml";

impl Fixer for ContainerRootfile {
    fn name(&self) -> &'static str {
        "container-rootfile"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["OPF-016", "OPF-017"]
    }
    fn description(&self) -> &'static str {
        "point container.xml at the package document when its full-path is missing or empty"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let Some(opf_name) = book.opf_name().map(str::to_owned) else {
            return Outcome::none();
        };
        let Some(text) = book.text(CONTAINER).map(str::to_owned) else {
            return Outcome::none();
        };
        let Ok(nodes) = scan(&text) else {
            return Outcome::none();
        };

        let packages: Vec<&String> = book
            .names()
            .iter()
            .filter(|n| n.to_ascii_lowercase().ends_with(".opf"))
            .collect();
        if packages.len() > 1 {
            return Outcome::finding(format!(
                "container.xml has no usable full-path and the book holds {} package documents, \
                 so which one is the rendition to open is a real choice and not ours",
                packages.len()
            ));
        }

        let mut edits = Edits::new();
        let mut fixed = 0u32;

        // Start and Empty only. An end tag carries no attributes, so it took
        // the "absent" branch and had ` full-path="..."` written into it —
        // `</rootfile full-path="...">`, which is not XML. Two real books hit
        // it, and both were books whose container was *correct*: they simply
        // spell the element `<rootfile ...></rootfile>` rather than
        // self-closing, so an end node exists at all.
        for node in nodes
            .iter()
            .filter(|n| n.name == "rootfile" && matches!(n.kind, NodeKind::Start | NodeKind::Empty))
        {
            match node.attr("full-path") {
                // Present and pointing somewhere: not ours.
                Some(a) if !a.value.trim().is_empty() => continue,
                Some(a) => edits.replace(a.span.clone(), format!("full-path=\"{opf_name}\"")),
                // Absent: add it, just after the element name.
                None => edits.insert(node.name_end, format!(" full-path=\"{opf_name}\"")),
            }
            fixed += 1;
        }

        if fixed == 0 {
            return Outcome::none();
        }
        book.set_text(CONTAINER, edits.apply(&text));
        Outcome::change(format!(
            "pointed {fixed} container rootfile(s) at \"{opf_name}\", where the package document is"
        ))
    }
}

/// CSS-003, CSS-004, RSC-027, RSC-028, HTM-058: a text entry that is not UTF-8.
///
/// EPUB requires UTF-8 for every XML and CSS resource. A tool that wrote UTF-16
/// leaves a file no reading system has to accept, and if the file also *declares*
/// itself UTF-8 — which happens, because the declaration is boilerplate the tool
/// did not update — the mismatch is fatal rather than merely wrong. Measured on
/// that shape: `FATAL(RSC-016)`, and epubcheck stops reading the file.
///
/// [`Book`] decodes UTF-16 on the way in and [`Book::save`] writes every text
/// entry as UTF-8, so the bytes are already right by the time this runs. What is
/// left is the declaration that still says otherwise, and saying so — without a
/// reported change the book is never rewritten and the repair never lands, the
/// same trap as the mimetype entry.
pub struct Encoding;

impl Fixer for Encoding {
    fn name(&self) -> &'static str {
        "encoding"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["CSS-003", "CSS-004", "RSC-027", "RSC-028", "HTM-058"]
    }
    fn description(&self) -> &'static str {
        "re-encode a UTF-16 text entry as UTF-8, and correct what it declares"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let names: Vec<String> = book.transcoded().to_vec();
        if names.is_empty() {
            return Outcome::none();
        }

        for name in &names {
            let Some(text) = book.text(name).map(str::to_owned) else {
                continue;
            };
            let fixed = redeclare_utf8(&text);
            if fixed != text {
                book.set_text(name, fixed);
            }
        }

        Outcome::change(format!(
            "re-encoded {} entr(ies) from UTF-16 to UTF-8 [{}]",
            names.len(),
            names
                .iter()
                .map(|n| crate::util::basename(n).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// Rewrite whatever the file says about its own encoding.
///
/// Two places say it, and both have to move or the file contradicts its bytes:
/// the XML declaration's `encoding=`, and a CSS `@charset`. A declaration that
/// is absent is correct already — UTF-8 is the default in both formats.
fn redeclare_utf8(text: &str) -> String {
    static XML_ENC: LazyLock<Regex> =
        LazyLock::new(|| re(r#"(<\?xml\b[^>]*?encoding=")([^"]*)(")"#));
    static CHARSET: LazyLock<Regex> = LazyLock::new(|| re(r#"(@charset\s+")([^"]*)(")"#));

    let once = XML_ENC.replacen(text, 1, "${1}utf-8${3}");
    CHARSET.replacen(&once, 1, "${1}utf-8${3}").into_owned()
}

/// RSC-005: unnamespaced OPF vocabulary attributes on `dc:` elements in an
/// EPUB 2 package.
///
/// The OPF vocabulary attributes — `scheme`, `file-as`, `role`, `event` — are
/// only legal with the `opf:` prefix, and epubcheck expects exactly that
/// spelling. *Pinocchio* has `scheme="URI"` on its identifier and bare
/// `file-as`/`role` on its contributor, which is four errors in a package
/// that otherwise means everything it says.
///
/// The repair is a prefix, nothing more: the attribute keeps its name, value
/// and position, and only moves into the namespace the package already
/// declares. When no `xmlns:opf` is declared there is nowhere to move it to,
/// and that is reported rather than guessed at.
pub struct LegacyOpfAttrs;

/// The OPF vocabulary attributes this knows to rename, matched on `dc:` elements.
const OPF_ATTRS: &[&str] = &["scheme", "file-as", "role", "event"];

impl Fixer for LegacyOpfAttrs {
    fn name(&self) -> &'static str {
        "opf-attributes"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "prefix EPUB 2 metadata attributes the OPF vocabulary spells with opf:"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() >= 3 {
            return Outcome::none();
        }
        let mut outcome = Outcome::none();
        let Some(name) = book.opf_name().map(str::to_owned) else {
            return outcome;
        };
        let Some(text) = book.text(&name).map(str::to_owned) else {
            return outcome;
        };
        let Ok(nodes) = scan(&text) else {
            return outcome;
        };

        let mut renamed = 0u32;
        let mut edits = Edits::new();
        for node in &nodes {
            // The OPF vocabulary attributes belong on dc: elements; an
            // attribute name like `role` elsewhere means something else.
            if !node.raw_name(&text).to_ascii_lowercase().starts_with("dc:") {
                continue;
            }
            for attr in node
                .attrs
                .iter()
                .filter(|a| OPF_ATTRS.contains(&a.name.as_str()))
            {
                // Only the name moves — the value stays exactly as written.
                edits.replace(
                    attr.span.start..attr.span.start + attr.name.len(),
                    format!("opf:{}", attr.name),
                );
                renamed += 1;
            }
        }

        if renamed == 0 {
            return outcome;
        }
        if !text.contains("xmlns:opf=") {
            outcome.push_finding(
                "the package uses OPF vocabulary attributes without an xmlns:opf declaration to \
                 move them into"
                    .to_string(),
            );
            return outcome;
        }
        book.set_text(&name, edits.apply(&text));
        outcome.push_change(format!(
            "prefixed {renamed} OPF vocabulary attribute(s) with the opf: namespace"
        ));
        outcome
    }
}

/// RSC-005: a `<meta refines="#x">` in the package document whose target id
/// is defined nowhere in it.
///
/// Measured on a Pascal *Pensées*: five refinement elements point at ids the
/// package never defines — `#creator` three times, `#t1`, `#src-id` — while
/// the elements they describe sit right there under other ids. epubcheck
/// reports each missing target, and then a second error on the ones whose
/// property must refine a particular kind of element, because a missing
/// target cannot be checked at all.
///
/// The element to repoint at is decided by the property, measured against
/// EPUB Check 5.2.1 (see [`REFINE_TARGETS`]): `role` refines a creator,
/// contributor or publisher, `title-type` a title, `identifier-type` an
/// identifier or source. The unrestricted properties — `file-as`,
/// `display-seq`, `group-position` — accept any `dc:` element, and for them
/// the only evidence left is the dangling id itself naming one: `#creator` is
/// a plain misspelling of the creator's id when exactly one `dc:creator`
/// exists.
///
/// Repointing is not always the repair. *Pensées*' dangling metas turned out
/// to be duplicates of refinements already present — the converter wrote the
/// same metadata twice, once as `opf:` attributes (converted by the
/// migration) and once as bare `<meta>` — and epubcheck rejects a second
/// `title-type` or `file-as` refining the same element, measured. When
/// another `<meta>` with the same property already refines the chosen
/// element, the dangling one is removed instead, and only when the two
/// disagree about the value — where removing either loses something — is it
/// reported. `role` is the exception: several roles on one creator are legal,
/// so only an exact duplicate is removed.
///
/// Everything ambiguous is reported rather than guessed at.
pub struct DanglingRefines;

/// Which `dc:` elements each `refines` property may describe, measured against
/// EPUB Check 5.2.1. Properties absent from here impose no restriction.
const REFINE_TARGETS: &[(&str, &[&str])] = &[
    ("role", &["creator", "contributor", "publisher"]),
    ("title-type", &["title"]),
    ("identifier-type", &["identifier", "source"]),
];

/// Properties measured to accept any `dc:` element as a target, so only the
/// dangling id itself can say which was meant.
const UNRESTRICTED: &[&str] = &["file-as", "display-seq", "group-position"];

/// Properties measured to reject a second refinement of one element, whatever
/// its value. `role` is absent: measured, several roles on one creator pass.
const UNIQUE_PER_ELEMENT: &[&str] = &[
    "file-as",
    "display-seq",
    "title-type",
    "identifier-type",
    "group-position",
];

impl Fixer for DanglingRefines {
    fn name(&self) -> &'static str {
        "dangling-refines"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "repoint <meta refines> targets that name no element, or remove the ones duplicating \
         a refinement already there"
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one walk deciding four outcomes (repoint, mint, remove, report) is clearer \
                  kept together than split across helpers"
    )]
    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() < 3 {
            return outcome;
        }
        let Some(name) = book.opf_name().map(str::to_owned) else {
            return outcome;
        };
        let Some(text) = book.text(&name).map(str::to_owned) else {
            return outcome;
        };
        let Ok(nodes) = scan(&text) else {
            return outcome;
        };

        let mut taken: std::collections::HashSet<String> = nodes
            .iter()
            .filter_map(|n| n.attr("id"))
            .map(|a| a.value.clone())
            .collect();
        // (property, refined element id) -> what the existing meta says, so a
        // dangling meta can be recognised as a duplicate of a healthy one.
        let mut existing: std::collections::HashMap<(String, String), String> =
            std::collections::HashMap::new();
        for (i, node) in nodes.iter().enumerate() {
            if node.name != "meta" || node.kind == NodeKind::End {
                continue;
            }
            let (Some(attr), Some(property)) = (node.attr("refines"), node.attr("property")) else {
                continue;
            };
            let Some(t) = attr.value.strip_prefix('#') else {
                continue;
            };
            if taken.contains(t) {
                existing.insert(
                    (property.value.clone(), t.to_string()),
                    meta_text(&text, &nodes, i),
                );
            }
        }
        let mut edits = Edits::new();
        let (mut repointed, mut minted, mut removed) = (0u32, 0u32, 0u32);

        for (i, node) in nodes.iter().enumerate() {
            if node.name != "meta" || node.kind == NodeKind::End {
                continue;
            }
            let (Some(attr), Some(property)) = (node.attr("refines"), node.attr("property")) else {
                continue;
            };
            let Some(target) = attr.value.strip_prefix('#') else {
                continue;
            };
            if taken.contains(target) {
                continue;
            }

            let wanted = REFINE_TARGETS
                .iter()
                .find(|(p, _)| *p == property.value)
                .map(|(_, el)| *el);
            if wanted.is_none() && !UNRESTRICTED.contains(&property.value.as_str()) {
                outcome.push_finding(format!(
                    "the <meta property=\"{}\" refines=\"#{target}\"> names no element in the \
                     package, and its property is not one this knows how to repoint",
                    property.value
                ));
                continue;
            }

            let candidates: Vec<&Node> = nodes
                .iter()
                .filter(|n| {
                    n.kind != NodeKind::End
                        && n.raw_name(&text).to_ascii_lowercase().starts_with("dc:")
                        && wanted.is_none_or(|w| w.contains(&n.name.as_str()))
                })
                .collect();

            let chosen: Option<&Node> = match candidates.as_slice() {
                [one] => Some(one),
                [] => {
                    outcome.push_finding(format!(
                        "the <meta property=\"{}\" refines=\"#{target}\"> names no element in the \
                         package, and no dc: element of the kind its property describes exists",
                        property.value
                    ));
                    continue;
                }
                many => {
                    // The dangling id itself naming an element is the last
                    // piece of evidence — and for the unrestricted properties
                    // the only one.
                    let named: Vec<&Node> = many
                        .iter()
                        .filter(|n| n.name.eq_ignore_ascii_case(target))
                        .copied()
                        .collect();
                    if let [one] = named.as_slice() {
                        Some(one)
                    } else {
                        outcome.push_finding(format!(
                            "the <meta property=\"{}\" refines=\"#{target}\"> names no element \
                             in the package, and {} dc: element(s) could be meant, so it is \
                             left alone",
                            property.value, many.len()
                        ));
                        continue;
                    }
                }
            };
            let Some(chosen) = chosen else {
                continue;
            };

            let id = if let Some(id) = chosen.attr("id") {
                id.value.clone()
            } else {
                let mut candidate = chosen.name.clone();
                let mut n = 2;
                while taken.contains(&candidate) {
                    candidate = format!("{}-{n}", chosen.name);
                    n += 1;
                }
                taken.insert(candidate.clone());
                edits.insert(chosen.name_end, format!(" id=\"{candidate}\""));
                minted += 1;
                candidate
            };

            let key = (property.value.clone(), id.clone());
            if let Some(existing_value) = existing.get(&key) {
                let mine = meta_text(&text, &nodes, i);
                if *existing_value == mine {
                    // A byte-for-byte duplicate of a refinement that already
                    // works: deleting the dangling one loses nothing.
                    edits.delete(crate::markup::line_span(&text, node.element_span(&nodes)));
                    removed += 1;
                    continue;
                }
                if UNIQUE_PER_ELEMENT.contains(&property.value.as_str()) {
                    // Only one may stand, and they disagree about the value:
                    // a person picks.
                    outcome.push_finding(format!(
                        "the <meta property=\"{}\" refines=\"#{target}\"> duplicates another \
                         refinement of the same element but says \"{}\" where it says \"{}\", \
                         so both are left alone",
                        property.value, mine, existing_value
                    ));
                    continue;
                }
                // `role`: several roles on one element are legal.
                edits.replace(attr.span.clone(), format!("refines=\"#{id}\""));
                existing.insert(key, mine);
                repointed += 1;
                continue;
            }
            edits.replace(attr.span.clone(), format!("refines=\"#{id}\""));
            existing.insert(key, meta_text(&text, &nodes, i));
            repointed += 1;
        }

        if repointed == 0 && minted == 0 && removed == 0 {
            return outcome;
        }
        book.set_text(&name, edits.apply(&text));
        if repointed > 0 {
            outcome.push_change(format!(
                "repointed {repointed} <meta refines> element(s) at the dc: element their \
                 property describes"
            ));
        }
        if removed > 0 {
            outcome.push_change(format!(
                "removed {removed} <meta refines> element(s) that duplicated a refinement \
                 already there"
            ));
        }
        if minted > 0 {
            outcome.push_change(format!(
                "minted {minted} id(s) on dc: element(s) for them to point at"
            ));
        }
        outcome
    }
}

/// The trimmed text content of node `i`, the element's own markup excluded.
fn meta_text(src: &str, nodes: &[Node], i: usize) -> String {
    nodes[i]
        .close
        .map_or_else(String::new, |c| src[nodes[i].span.end..nodes[c].span.start].trim().to_string())
}

/// The ISBN-13 a string carries, if it carries one beyond doubt.
///
/// Three conditions, and all three matter. Exactly **thirteen** digits, so a
/// year or a page count cannot qualify. A **978 or 979** prefix, which is the
/// whole of the ISBN range and anchors the match to something that is trying to
/// be an ISBN. And a valid **check digit**, which a run of digits passes by
/// accident one time in ten.
///
/// Together those make a false positive something close to impossible: a
/// thirteen-digit string starting 978 whose checksum also lands is not a
/// coincidence, it is an ISBN somebody wrote down.
fn isbn13(text: &str) -> Option<String> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 13 || !(digits.starts_with("978") || digits.starts_with("979")) {
        return None;
    }
    let sum: u32 = digits
        .bytes()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    sum.is_multiple_of(10).then_some(digits)
}

/// Where to put a new `<dc:identifier id="{wanted}">urn:isbn:{isbn}</…>`, and
/// what to write there.
///
/// Same placement rule as [`DcLanguage`]: line up with the other children of
/// `<metadata>` and insert at the start of the closing tag's line, so the
/// indentation already sitting there is not counted twice. The prefix is the
/// one the file actually binds for Dublin Core rather than an assumed `dc`.
fn write_identifier(
    text: &str,
    nodes: &[Node],
    wanted: &str,
    isbn: &str,
) -> Option<(usize, String)> {
    let metadata = nodes
        .iter()
        .position(|n| n.name == "metadata" && n.kind == NodeKind::Start)?;
    let close = nodes[metadata].close?;
    let close_at = nodes[close].span.start;
    let line_start = text[..close_at].rfind('\n').map_or(0, |i| i + 1);
    let indent = nodes
        .iter()
        .rfind(|n| n.parent == Some(metadata) && n.kind != NodeKind::End)
        .map_or_else(
            || format!("{}  ", line_indent(text, close_at)),
            |last| line_indent(text, last.span.start),
        );
    let prefix = dc_prefix(text).unwrap_or_else(|| "dc".to_string());
    Some((
        line_start,
        format!(
            "{indent}<{prefix}:identifier id=\"{wanted}\">urn:isbn:{isbn}</{prefix}:identifier>\n"
        ),
    ))
}

/// RSC-005 / OPF-085: a `dc:identifier` that claims a UUID it does not carry.
///
/// A Kobo storefront writes `<dc:identifier id="unique-id">urn:uuid:</dc:identifier>`
/// and never fills the UUID in, which epubcheck flags as an invalid UUID on
/// every one of the Monogatari books. The identifier is metadata only —
/// generating a real one changes nothing about how the book renders — and the
/// package's `dtb:uid` follows it later in the run through `ncx-uid`.
///
/// Only the invalid part is replaced: an identifier that already carries a
/// well-formed UUID is left alone, which is also what keeps a second run a
/// no-op.
pub struct IdentifierUuid;

impl Fixer for IdentifierUuid {
    fn name(&self) -> &'static str {
        "identifier-uuid"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005", "OPF-085"]
    }
    fn description(&self) -> &'static str {
        "generate a real UUID for an identifier that claims an empty or malformed one"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() < 3 {
            return outcome;
        }
        let Some(name) = book.opf_name().map(str::to_owned) else {
            return outcome;
        };
        let Some(text) = book.text(&name).map(str::to_owned) else {
            return outcome;
        };
        let Ok(nodes) = scan(&text) else {
            return outcome;
        };

        let mut edits = Edits::new();
        let mut replaced = 0u32;
        for node in nodes.iter().filter(|n| {
            n.name == "identifier"
                && n.kind != NodeKind::End
                && n.raw_name(&text).to_ascii_lowercase().starts_with("dc:")
        }) {
            let Some(close) = node.close else {
                continue;
            };
            let value = text[node.span.end..nodes[close].span.start].trim();
            let lower = value.to_ascii_lowercase();
            let Some(suffix) = lower.strip_prefix("urn:uuid:") else {
                continue;
            };
            if valid_uuid(suffix) {
                continue;
            }
            let fresh = fresh_uuid();
            edits.replace(node.span.end..nodes[close].span.start, fresh);
            replaced += 1;
        }

        if replaced == 0 {
            return outcome;
        }
        book.set_text(&name, edits.apply(&text));
        outcome.push_change(format!(
            "replaced {replaced} identifier(s) claiming an empty or malformed UUID with freshly \
             generated ones"
        ));
        outcome
    }
}

/// True for the canonical 8-4-4-4-12 hex shape, which is all epubcheck will
/// accept after a `urn:uuid:` prefix.
fn valid_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && s.chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 8 | 13 | 18 | 23) || c.is_ascii_hexdigit())
}

/// A fresh version-4 UUID, unique per call within a run.
///
/// The tool carries no RNG dependency, and it does not need one: this value
/// only has to be well-formed and never repeated inside one book, so a
/// splitmix stream seeded from the clock and a per-run counter is plenty.
fn fresh_uuid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| {
            // Entropy only: the low 64 bits of the nanosecond clock are
            // plenty, and losing the high ones changes nothing.
            #[allow(clippy::cast_possible_truncation)]
            let n = d.as_nanos() as u64;
            n
        });
    let mut x = nanos ^ COUNTER.fetch_add(1, Ordering::Relaxed).rotate_left(17);
    let mut next = || {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&next().to_le_bytes());
    bytes[8..].copy_from_slice(&next().to_le_bytes());
    bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // RFC 4122 variant
    format!(
        "urn:uuid:{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}
