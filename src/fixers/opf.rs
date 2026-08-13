//! Fixes that apply to the package document (`.opf`).

use std::fmt::Write as _;
use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::language::{self, Policy};
use crate::markup::{Edits, NodeKind, line_span, scan};
use crate::fixers::filenames::as_url_path;
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
pub struct SpinePageMap;

static PAGE_MAP_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(<spine\b[^>]*?)\s*page-map="[^"]*""#));

impl Fixer for SpinePageMap {
    fn name(&self) -> &'static str {
        "spine-page-map"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "remove the Adobe page-map attribute from <spine>"
    }
    fn apply(&self, book: &mut Book) -> Outcome {
        if edit_opf(book, |t| PAGE_MAP_RE.replacen(t, 1, "${1}").into_owned()) {
            Outcome::change("removed spine/@page-map")
        } else {
            Outcome::none()
        }
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
pub struct MediaTypes;

/// `wrong media type -> what it means now`.
const MEDIA_TYPES: &[(&str, &str)] = &[
    // CSS-007: a doubled prefix, and the modern spelling of the font type.
    ("application/application/x-font-ttf", "application/vnd.ms-opentype"),
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
            let Some((_, right)) = MEDIA_TYPES
                .iter()
                .find(|(wrong, _)| attr.value.trim() == *wrong)
            else {
                continue;
            };
            edits.replace(attr.span.clone(), format!("media-type=\"{right}\""));
            fixed.push(attr.value.trim().to_string());
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
                    let rel = as_url_path(&relative_to(&opf_name, &target));
                    edits.replace(href.span.clone(), format!("href=\"{rel}\""));
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

/// OPF-032: `Guide references "…" which is not a valid "OPS Content Document"`.
///
/// The `guide` is the EPUB 2 predecessor of the landmarks nav, and every
/// `<reference>` in it is required to point at a content document. Conversion
/// tools sometimes point one at the cover *image* instead of the cover page —
/// one real book has three `.jpg` references. A reading system cannot navigate
/// to a JPEG, so the entry does nothing but fail validation.
///
/// Only references that resolve to something which is not markup are removed.
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
        "drop guide entries pointing at something that is not a content document"
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
        let mut edits = Edits::new();
        let mut dropped = 0u32;

        let guide = nodes
            .iter()
            .position(|n| n.name == "guide" && n.kind == NodeKind::Start);
        let mut references = 0u32;

        for node in nodes
            .iter()
            .filter(|n| n.name == "reference" && matches!(n.kind, NodeKind::Start | NodeKind::Empty))
        {
            references += 1;
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
            edits.delete(node.element_span(&nodes));
            dropped += 1;
        }

        // Nothing would be left inside it, so the element goes too — an empty
        // <guide> is an error of its own.
        if let Some(guide) = guide
            && references == dropped
        {
            let mut edits = Edits::new();
            edits.delete(line_span(&text, nodes[guide].element_span(&nodes)));
            book.set_text(&opf_name, edits.apply(&text));
            return Outcome::change(if dropped == 0 {
                "removed an empty <guide>, which needs at least one reference".to_string()
            } else {
                format!(
                    "removed the <guide> and all {dropped} of its references, none of which \
                     pointed at a content document"
                )
            });
        }

        if dropped == 0 {
            return Outcome::none();
        }
        book.set_text(&opf_name, edits.apply(&text));
        Outcome::change(format!(
            "removed {dropped} guide reference(s) pointing at a non-content file"
        ))
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
        for node in nodes
            .iter()
            .filter(|n| n.parent == Some(metadata) && matches!(n.kind, NodeKind::Start | NodeKind::Empty))
        {
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
        let idents: Vec<&crate::markup::Node> = nodes
            .iter()
            .filter(|n| {
                n.name == "identifier"
                    && n.parent == metadata
                    && matches!(n.kind, NodeKind::Start | NodeKind::Empty)
            })
            .collect();

        let [ident] = idents[..] else {
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
    let at = opf.find("=\"http://purl.org/dc/elements/1.1/\"")?;
    let decl = &opf[..at];
    let name = decl.rsplit(|c: char| c.is_whitespace()).next()?;
    name.strip_prefix("xmlns:").map(str::to_string)
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
        Outcome::change(format!("rewrote the mimetype entry, which {}", wrong.join(" and ")))
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
