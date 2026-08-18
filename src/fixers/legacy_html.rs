//! Fixes and diagnostics for markup that XHTML 1.1 rejects but HTML5 accepts.
//!
//! Both of these are EPUB 2 only, and both run the opposite way round from
//! `legacy-table-attrs`: there it is HTML5 that is stricter, here it is
//! XHTML 1.1.

use std::sync::LazyLock;

use regex::Regex;

use crate::book::Book;
use crate::fixers::{Fixer, Outcome};
use crate::markup::{Edits, NodeKind, scan};
use crate::util::{basename, re};

// ---------------------------------------------------------------------------
// img-alt
// ---------------------------------------------------------------------------

/// Filenames that read as ornament rather than content.
static DECORATIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)(^|[^a-z])(orn|ornament|rule|deco|divider|sep|separator|dingbat|flourish|swash|border|spacer|blank|line|star|asterism|fleuron|glyph)([^a-z]|$)",
    )
});

/// RSC-005: `element "img" missing required attribute "alt"`.
///
/// XHTML 1.1 requires `alt`; HTML5 does not make its absence a validation
/// error. So this is EPUB 2 only.
///
/// The mechanical fix is `alt=""`, which is correct for an ornament and a lie
/// for a photograph — and writing it onto a content image actively harms a
/// screen-reader user while making the validator happy. That is the wrong trade
/// to make silently, so `alt=""` is written only where the filename reads as
/// decorative; everything else is reported for a human to caption.
pub struct ImgAlt;

impl Fixer for ImgAlt {
    fn name(&self) -> &'static str {
        "img-alt"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "add alt=\"\" to decorative images in EPUB 2, and report the rest"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() >= 3 {
            return outcome;
        }

        let mut added = 0u32;
        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();
            let mut needs_caption = 0u32;

            // Start and Empty only. An end tag has no attributes, so a book
            // written `<img ...></img>` would offer `</img>` as an image with
            // no alt and no src — either a phantom "needs a caption" finding or,
            // if the empty name matched, ` alt=""` written into a closing tag.
            for node in nodes
                .iter()
                .filter(|n| n.name == "img" && matches!(n.kind, NodeKind::Start | NodeKind::Empty))
            {
                if node.attr("alt").is_some() {
                    continue;
                }
                let src = node.attr("src").map_or("", |a| a.value.as_str());
                // A caption or title means somebody already described it.
                let described = node.attr("title").is_some() || node.attr("aria-label").is_some();
                if !described && DECORATIVE_RE.is_match(basename(src)) {
                    edits.insert(node.name_end, " alt=\"\"");
                    added += 1;
                } else {
                    needs_caption += 1;
                }
            }

            if needs_caption > 0 {
                outcome.push_finding(format!(
                    "{}: {needs_caption} image(s) have no alt text and do not look decorative; \
                     they need a real caption, or migrate the book to EPUB 3 where alt is not \
                     a validation error",
                    basename(&doc)
                ));
            }
            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if added > 0 {
            outcome.push_change(format!("added alt=\"\" to {added} decorative image(s)"));
        }
        outcome
    }
}

// ---------------------------------------------------------------------------
// version-mismatch (diagnostic only)
// ---------------------------------------------------------------------------

/// Reports a book whose markup does not match its declared version, when there
/// is not enough evidence to retag it automatically.
///
/// The retagging in [`crate::version`] handles the clear-cut cases in both
/// directions, and on a default run there is nothing left to say: the wrappable
/// runs get a `<div>`, the unwrappable ones retag. This fires only when
/// retagging was refused with `--keep-version`, to say what was found and what
/// the declaration still hides.
pub struct VersionMismatch;

impl Fixer for VersionMismatch {
    fn name(&self) -> &'static str {
        "version-mismatch"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "report markup that does not match the declared version (never rewrites)"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let mut outcome = Outcome::none();
        if book.epub_version() >= 3 {
            return outcome;
        }
        // This runs last, so a mismatch still standing here is one that nothing
        // acted on: either the evidence was too weak to retag on, or retagging
        // was suppressed or refused. A book that *was* retagged now declares
        // EPUB 3 and returns above, which is what keeps this quiet on the
        // ordinary path.
        let assessment = crate::version::assess(book);
        if assessment.suggestive_only() {
            outcome.push_finding(format!(
                "declares EPUB 2, but {} element(s) still hold inline content that only \
                 validates under EPUB 3 rules. A short run inside a <blockquote> is wrapped in \
                 a <div> automatically and these are what is left, typically inside a <body>, \
                 <form> or <fieldset>, where wrapping does not help. Run without \
                 --keep-version to have the declaration moved instead.",
                assessment.inline_in_block
            ));
        } else if !assessment.epub3_only.is_empty() {
            outcome.push_finding(format!(
                "declares EPUB 2, but the content requires EPUB 3 ({}). The declaration was \
                 left as it is — run without --keep-version to have it corrected, since \
                 rewriting the markup to suit the declaration would be the larger change by \
                 far.",
                assessment.epub3_only.join("; ")
            ));
        }
        outcome
    }
}

/// RSC-005: `element "u" not allowed anywhere`.
///
/// XHTML 1.1 dropped the purely presentational `<u>`; HTML5 brought it back
/// with a meaning, so this is EPUB 2 only — measured, the same document is
/// clean under EPUB 3.
///
/// The element becomes a `<span>`, which is legal exactly where `<u>` was and
/// carries the same content model, so nothing moves. What matters is that the
/// text stays underlined: `<u>` had that behaviour from the user-agent
/// stylesheet, and a `<span>` has none. So the book's own stylesheet is asked
/// first — the same question `legacy-table-attrs` asks before stripping. If a
/// rule already underlines the element through one of its classes, the `<span>`
/// needs nothing; if not, the declaration is written inline, because losing the
/// underline is a visible change and this repair is meant to be invisible.
pub struct UnderlineElements;

impl Fixer for UnderlineElements {
    fn name(&self) -> &'static str {
        "underline-elements"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "turn the removed <u> element into a <span> that still underlines"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        if book.epub_version() >= 3 {
            return Outcome::none();
        }
        let sheet = crate::fixers::tables::author_styles(book);
        let mut converted = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for (i, node) in nodes.iter().enumerate() {
                if node.name != "u" || node.kind == NodeKind::End {
                    continue;
                }
                let classes = node.attr("class").map_or("", |a| a.value.as_str());
                let styled = sheet.declares("u", classes, "text-decoration")
                    || sheet.declares("span", classes, "text-decoration");

                // Retag rather than rebuild: only the name moves, so every
                // other attribute comes through byte-for-byte. An existing
                // style attribute is extended instead of being shadowed by a
                // second one.
                edits.replace(node.span.start..node.name_end, "<span".to_string());
                if styled {
                    // nothing to add
                } else if let Some(existing) = node.attr("style") {
                    edits.replace(
                        existing.span.clone(),
                        format!(
                            "style=\"{}; text-decoration: underline\"",
                            existing.value.trim().trim_end_matches(';')
                        ),
                    );
                } else {
                    edits.insert(node.name_end, " style=\"text-decoration: underline\"");
                }
                // A self-closing `<u/>` or `<center/>` has no close tag to
                // rewrite, and `<span/>`/`<div/>` are not void elements: an XML
                // parser reads the shorthand, an HTML one reads an *unclosed*
                // start tag and swallows the rest of the document into it.
                // `nesting::rewrap` learned this on a real `<a id="page_viii"/>`
                // and turned two content-model errors into two fatal ones; the
                // answer is the same here, emit both tags.
                match (node.kind, node.close) {
                    (_, Some(close)) => {
                        edits.replace(nodes[close].span.clone(), "</span>".to_string());
                    }
                    // The `/>` is what has to go: replacing only the close tag
                    // leaves `<span/></span>`, which is not well-formed either.
                    (NodeKind::Empty, None) => {
                        let end = node.span.end;
                        edits.replace(end - 2..end, "></span>".to_string());
                    }
                    // Opened and never closed: the input is already malformed
                    // and there is no honest place to put a close tag.
                    _ => {}
                }
                let _ = i;
                converted += 1;
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if converted == 0 {
            return Outcome::none();
        }
        Outcome::change(format!(
            "turned {converted} <u> element(s) into underlined <span>s"
        ))
    }
}

/// RSC-005: `element "center" not allowed here`.
///
/// `<center>` is gone from both rulesets — XHTML 1.1 never had it, and HTML5
/// removed it — but every converter from the Word era emitted it. The
/// element's whole effect is `text-align: center` on its contents, so the
/// replacement is a `<div>` carrying that one declaration: same content model
/// for everything a `<center>` ever legally held, same rendering, legal in
/// both rulesets.
///
/// Like the `<u>` repair, the book's own stylesheet is asked first: if a rule
/// already centers this element's class, the `<div>` needs nothing inline.
pub struct CenterElements;

impl Fixer for CenterElements {
    fn name(&self) -> &'static str {
        "center-elements"
    }
    fn codes(&self) -> &'static [&'static str] {
        &["RSC-005"]
    }
    fn description(&self) -> &'static str {
        "turn the removed <center> element into a centered <div>"
    }

    fn apply(&self, book: &mut Book) -> Outcome {
        let sheet = crate::fixers::tables::author_styles(book);
        let mut converted = 0u32;

        for doc in book.markup_names() {
            let Some(text) = book.text(&doc).map(str::to_owned) else {
                continue;
            };
            let Ok(nodes) = scan(&text) else { continue };
            let mut edits = Edits::new();

            for node in nodes
                .iter()
                .filter(|n| n.name == "center" && n.kind != NodeKind::End)
            {
                let classes = node.attr("class").map_or("", |a| a.value.as_str());
                let styled = sheet.declares("center", classes, "text-align")
                    || sheet.declares("div", classes, "text-align");

                // Retag rather than rebuild: only the name moves, so every
                // other attribute comes through byte-for-byte. An existing
                // style attribute is extended instead of being shadowed by a
                // second one.
                edits.replace(node.span.start..node.name_end, "<div".to_string());
                if styled {
                    // nothing to add
                } else if let Some(existing) = node.attr("style") {
                    edits.replace(
                        existing.span.clone(),
                        format!(
                            "style=\"{}; text-align: center\"",
                            existing.value.trim().trim_end_matches(';')
                        ),
                    );
                } else {
                    edits.insert(node.name_end, " style=\"text-align: center\"");
                }
                // A self-closing `<u/>` or `<center/>` has no close tag to
                // rewrite, and `<span/>`/`<div/>` are not void elements: an XML
                // parser reads the shorthand, an HTML one reads an *unclosed*
                // start tag and swallows the rest of the document into it.
                // `nesting::rewrap` learned this on a real `<a id="page_viii"/>`
                // and turned two content-model errors into two fatal ones; the
                // answer is the same here, emit both tags.
                match (node.kind, node.close) {
                    (_, Some(close)) => {
                        edits.replace(nodes[close].span.clone(), "</div>".to_string());
                    }
                    // The `/>` is what has to go: replacing only the close tag
                    // leaves `<div/></div>`, which is not well-formed either.
                    (NodeKind::Empty, None) => {
                        let end = node.span.end;
                        edits.replace(end - 2..end, "></div>".to_string());
                    }
                    // Opened and never closed: the input is already malformed
                    // and there is no honest place to put a close tag.
                    _ => {}
                }
                converted += 1;
            }

            if !edits.is_empty() {
                book.set_text(&doc, edits.apply(&text));
            }
        }

        if converted == 0 {
            return Outcome::none();
        }
        Outcome::change(format!(
            "turned {converted} <center> element(s) into centered <div>s"
        ))
    }
}
