//! The error classes added after the first library sweep.
//!
//! Every expectation here was measured against EPUB Check 5.2.1 first — one
//! construct per book, in both rulesets — and the measurement is quoted in the
//! doc comment of the fixer it belongs to.

mod common;

use std::fmt::Write as _;

use common::verify::verify;
use common::{entry, has, make_epub, names, read_epub, roundtrip_full, roundtrip_kept};

const CONTAINER: &str = common::CONTAINER;

fn opf(version: &str, manifest: &str, spine: &str, guide: &str) -> String {
    let (ncx_item, toc_attr) = if version == "2.0" {
        (
            r#"    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#,
            r#" toc="ncx""#,
        )
    } else {
        ("", "")
    };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
{ncx_item}
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
{manifest}
  </manifest>
  <spine{toc_attr}><itemref idref="ch1"/>{spine}</spine>
{guide}</package>"#
    )
}

const NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap><navPoint id="n1" playOrder="1"><navLabel><text>A</text></navLabel>
    <content src="ch1.xhtml"/></navPoint></navMap>
</ncx>"#;

fn doc(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
         <head><title>T</title></head>\n<body>\n{body}\n</body></html>"
    )
}

/// An EPUB 2 book whose first chapter has `body`.
fn book2(body: &str) -> Vec<u8> {
    let opf = opf("2.0", "", "", "");
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc(body).as_bytes()),
    ])
}

/// Repair, and prove nothing structural was lost on the way.
fn fix(before: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let (outcome, after) = roundtrip_full(before);
    verify(&read_epub(before), &after).assert_sound();
    (outcome, after)
}

fn ch1_of(after: &[(String, Vec<u8>)]) -> String {
    entry(after, "OEBPS/ch1.xhtml")
}

fn ch1(after: &[(String, Vec<u8>)]) -> String {
    entry(after, "OEBPS/ch1.xhtml")
}

// ---------------------------------------------------------------------------
// content-duplicate-ids
// ---------------------------------------------------------------------------

#[test]
fn an_unreferenced_duplicate_id_is_renamed_and_the_first_one_is_not() {
    // Kobo's injected reading-location spans, twelve to a chapter in the book
    // this came from.
    let (outcome, after) = fix(&book2(
        r#"<p id="kobo.40.N">a</p><p id="kobo.40.N">b</p><p id="kobo.40.N">c</p>"#,
    ));
    let fixed = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("made 2")),
        "got {:?}",
        outcome.changes
    );
    // The first keeps the name, so anything that resolves today still resolves.
    assert!(fixed.contains(r#"<p id="kobo.40.N">a</p>"#), "{fixed}");
    assert!(fixed.contains(r#"id="kobo.40.N_2""#), "{fixed}");
    assert!(fixed.contains(r#"id="kobo.40.N_3""#), "{fixed}");
}

#[test]
fn a_referenced_duplicate_id_is_renamed_after_the_first() {
    // This used to stop at a duplicate something linked to, for fear of
    // breaking the link. It cannot: a fragment resolves to the *first* element
    // with that id in tree order, so `#dup` already lands on the first and
    // renaming the second cannot move it. Nothing can be pointing at the second
    // either -- it is unreachable by definition. Measured: two `id="dup"` with
    // an `<a href="#dup">` is 2 errors, and renaming only the second is 0.
    let (outcome, after) = fix(&book2(
        r##"<p id="dup">a</p><p id="dup">b</p><p><a href="#dup">link</a></p>"##,
    ));
    let got = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("duplicated id")),
        "got {:?}",
        outcome.changes
    );
    assert_eq!(got.matches(r#"id="dup""#).count(), 1, "got {got}");
    assert!(got.contains(r#"id="dup_2""#), "got {got}");
    assert!(
        got.contains(r##"href="#dup""##),
        "the link is untouched and still lands on the first: {got}"
    );
    assert!(outcome.findings.is_empty(), "got {:?}", outcome.findings);
}

// ---------------------------------------------------------------------------
// nested-anchors
// ---------------------------------------------------------------------------

#[test]
fn a_nested_anchor_is_unwrapped_and_its_text_kept() {
    let (outcome, after) = fix(&book2(
        r#"<p><a href="ch1.xhtml">outer <a href="ch1.xhtml">inner</a> tail</a></p>"#,
    ));
    let fixed = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("unwrapped 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(fixed.contains("outer inner tail"), "{fixed}");
    assert_eq!(fixed.matches("<a ").count(), 1, "one anchor left: {fixed}");
}

#[test]
fn a_nested_anchor_with_an_id_becomes_a_span_so_links_still_land() {
    let (outcome, after) = fix(&book2(
        r#"<p><a href="ch1.xhtml">outer <a id="note1">inner</a> tail</a></p>"#,
    ));
    let fixed = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("into <span>s")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        fixed.contains(r#"<span id="note1">inner</span>"#),
        "{fixed}"
    );
}

#[test]
fn an_anchor_nested_through_a_span_is_caught_too() {
    // The rule is about ancestry, not parentage.
    let (outcome, _) = fix(&book2(
        r#"<p><a href="ch1.xhtml"><span><a href="ch1.xhtml">in</a></span></a></p>"#,
    ));
    assert!(
        outcome.changes.iter().any(|c| c.contains("unwrapped 1")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn sibling_anchors_are_left_alone() {
    let (outcome, _) = fix(&book2(
        r#"<p><a href="ch1.xhtml">one</a> and <a href="ch1.xhtml">two</a></p>"#,
    ));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
}

// ---------------------------------------------------------------------------
// misplaced-blockquotes
// ---------------------------------------------------------------------------

#[test]
fn a_paragraph_is_split_around_the_blockquote_it_swallowed() {
    let (outcome, after) = fix(&book2(
        "<p>before<blockquote><p>quote</p></blockquote>after</p>",
    ));
    let fixed = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("split 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        fixed.contains("<p>before</p><blockquote><p>quote</p></blockquote><p>after</p>"),
        "{fixed}"
    );
}

#[test]
fn splitting_keeps_the_paragraphs_attributes_and_skips_empty_halves() {
    // Nothing precedes the quotation, so the leading paragraph would be empty;
    // an empty <p> is valid but renders as a blank line, which is a change
    // made for nothing.
    let (_, after) = fix(&book2(
        r#"<p class="lead"><blockquote><p>q</p></blockquote>after</p>"#,
    ));
    let fixed = ch1(&after);

    assert!(!fixed.contains(r#"<p class="lead"></p>"#), "{fixed}");
    assert!(
        fixed.contains(r#"<blockquote><p>q</p></blockquote><p class="lead">after</p>"#),
        "attributes carry to the reopened half: {fixed}"
    );
}

#[test]
fn a_paragraph_holding_nothing_but_a_blockquote_loses_both_its_tags() {
    let (_, after) = fix(&book2("<p><blockquote><p>q</p></blockquote></p>"));
    let fixed = ch1(&after);
    assert!(
        fixed.contains("<blockquote><p>q</p></blockquote>"),
        "{fixed}"
    );
    assert!(!fixed.contains("<p></p>"), "no empty paragraph: {fixed}");
}

#[test]
fn two_blockquotes_in_one_paragraph_are_both_cut_out() {
    let (outcome, after) = fix(&book2(
        "<p>a<blockquote><p>q</p></blockquote>b<blockquote><p>r</p></blockquote>c</p>",
    ));
    let fixed = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("split 2")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        fixed.contains(
            "<p>a</p><blockquote><p>q</p></blockquote><p>b</p>\
             <blockquote><p>r</p></blockquote><p>c</p>"
        ),
        "{fixed}"
    );
}

#[test]
fn a_blockquote_in_a_span_is_demoted_rather_than_split() {
    // A <span> does not auto-close the way a <p> does, so splitting it would
    // be an invention. Demoting the quotation is not.
    let (outcome, after) = fix(&book2(
        r#"<p><span class="q"><blockquote>words</blockquote></span></p>"#,
    ));
    let fixed = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("demoted 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        fixed.contains(r#"<span class="blockquote">words</span>"#),
        "{fixed}"
    );
}

#[test]
fn demotion_merges_into_an_existing_class_rather_than_adding_a_second() {
    let (_, after) = fix(&book2(
        r#"<p><span><blockquote class="verse" lang="la">dixit</blockquote></span></p>"#,
    ));
    let fixed = ch1(&after);

    assert!(
        fixed.contains(r#"<span class="blockquote verse" lang="la">dixit</span>"#),
        "{fixed}"
    );
    assert_eq!(fixed.matches("class=").count(), 1, "{fixed}");
}

#[test]
fn a_blockquote_that_can_be_neither_split_nor_demoted_is_reported() {
    let (outcome, after) = fix(&book2(
        "<p><span><blockquote><p>q</p></blockquote></span></p>",
    ));

    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("blockquote") && f.contains("needs a look")),
        "got {:?}",
        outcome.findings
    );
    assert!(ch1(&after).contains("<blockquote>"));
}

#[test]
fn a_blockquote_where_it_belongs_is_untouched() {
    let (outcome, _) = fix(&book2("<blockquote><p>q</p></blockquote>"));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
}

// ---------------------------------------------------------------------------
// data-attributes
// ---------------------------------------------------------------------------

#[test]
fn data_attribute_names_html5_rejects_are_removed() {
    // Kindle leaves these behind. The scanner lowercases attribute names, and
    // uppercase is half of what makes one invalid, so this also pins that the
    // check reads the raw source.
    let opf = opf("3.0", "", "", "");
    let before = make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        (
            "OEBPS/ch1.xhtml",
            doc(r#"<p data-AmznRemoved="1" data-AmznRemoved-M8="2" data-2foo="3" data-keep="4">x</p>"#)
                .as_bytes(),
        ),
    ]);
    let (outcome, after) = roundtrip_kept(&before);
    let fixed = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("removed 3")),
        "got {:?}",
        outcome.changes
    );
    assert!(!fixed.contains("AmznRemoved"), "{fixed}");
    assert!(!fixed.contains("data-2foo"), "{fixed}");
    assert!(
        fixed.contains(r#"data-keep="4""#),
        "valid ones stay: {fixed}"
    );
}

#[test]
fn well_formed_data_attributes_in_an_epub2_book_are_reported_not_deleted() {
    // Under XHTML 1.1 every data-* attribute is an error, valid name or not.
    // Deleting them all would be data loss to satisfy a declaration that is
    // probably the thing at fault.
    let (outcome, after) = fix(&book2(r#"<p data-foo="1">x</p>"#));

    assert!(ch1(&after).contains(r#"data-foo="1""#));
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("data-*") && f.contains("EPUB 2")),
        "got {:?}",
        outcome.findings
    );
}

// ---------------------------------------------------------------------------
// xhtml-namespace
// ---------------------------------------------------------------------------

#[test]
fn a_root_html_with_no_namespace_gets_one() {
    let opf = opf("2.0", "", "", "");
    let before = make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        (
            "OEBPS/ch1.xhtml",
            b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
              <html><head><title>T</title></head><body><p>x</p></body></html>",
        ),
    ]);
    let (outcome, after) = fix(&before);

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("declared the XHTML namespace")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        ch1(&after).contains(r#"<html xmlns="http://www.w3.org/1999/xhtml">"#),
        "{}",
        ch1(&after)
    );
}

#[test]
fn a_document_that_already_declares_the_namespace_is_untouched() {
    let (outcome, _) = fix(&book2("<p>x</p>"));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
}

// ---------------------------------------------------------------------------
// guide-references
// ---------------------------------------------------------------------------

#[test]
fn a_guide_reference_to_an_image_is_dropped_and_the_rest_kept() {
    let guide = r#"  <guide>
    <reference type="cover" title="Cover" href="cover.jpg"/>
    <reference type="text" title="Text" href="ch1.xhtml"/>
  </guide>
"#;
    let opf = opf(
        "2.0",
        r#"    <item id="cov" href="cover.jpg" media-type="image/jpeg"/>"#,
        "",
        guide,
    );
    let before = make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>x</p>").as_bytes()),
        ("OEBPS/cover.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]),
    ]);
    let (outcome, after) = fix(&before);
    let fixed = entry(&after, "OEBPS/content.opf");

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("removed 1 guide")),
        "got {:?}",
        outcome.changes
    );
    let guide_section = &fixed[fixed.find("<guide>").unwrap()..];
    assert!(!guide_section.contains("cover.jpg"), "{fixed}");
    assert!(guide_section.contains(r#"href="ch1.xhtml""#), "{fixed}");
    // Only the guide entry goes: the image is still the cover, and its
    // manifest item is what says so.
    assert!(
        fixed.contains(r#"<item id="cov" href="cover.jpg""#),
        "{fixed}"
    );
    assert!(has(&after, "OEBPS/cover.jpg"), "the image itself stays");
}

// ---------------------------------------------------------------------------
// dangling-resources, in CSS
// ---------------------------------------------------------------------------

#[test]
fn a_font_face_url_with_a_doubled_directory_is_repointed() {
    let css = "@font-face {\n  font-family: \"Bembo\";\n  \
               src: url(\"OEBPS/Fonts/bembo.otf\");\n}\n";
    let opf = opf(
        "2.0",
        r#"    <item id="css" href="Styles/s.css" media-type="text/css"/>
    <item id="fnt" href="Fonts/bembo.otf" media-type="font/otf"/>"#,
        "",
        "",
    );
    let before = make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>x</p>").as_bytes()),
        ("OEBPS/Styles/s.css", css.as_bytes()),
        ("OEBPS/Fonts/bembo.otf", b"OTTO\x00"),
    ]);
    let (outcome, after) = fix(&before);
    let fixed = entry(&after, "OEBPS/Styles/s.css");

    assert!(
        outcome.changes.iter().any(|c| c.contains("repointed 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(fixed.contains(r#"url("../Fonts/bembo.otf")"#), "{fixed}");
}

#[test]
fn a_css_url_that_already_resolves_is_left_alone() {
    let css = "@font-face { src: url(\"../Fonts/bembo.otf\"); }\n\
               p { background: url(data:image/gif;base64,AA); }\n";
    let opf = opf(
        "2.0",
        r#"    <item id="css" href="Styles/s.css" media-type="text/css"/>
    <item id="fnt" href="Fonts/bembo.otf" media-type="font/otf"/>"#,
        "",
        "",
    );
    let before = make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>x</p>").as_bytes()),
        ("OEBPS/Styles/s.css", css.as_bytes()),
        ("OEBPS/Fonts/bembo.otf", b"OTTO\x00"),
    ]);
    let (outcome, after) = fix(&before);

    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert_eq!(entry(&after, "OEBPS/Styles/s.css"), css);
}

// ---------------------------------------------------------------------------
// css-paths, and the candidate ladder
// ---------------------------------------------------------------------------

/// A book with a stylesheet at `OEBPS/Styles/s.css` and whatever else is given.
fn css_book(css: &str, extra: &[(&str, &[u8])], extra_manifest: &str) -> Vec<u8> {
    let opf = opf(
        "2.0",
        &format!(
            "    <item id=\"css\" href=\"Styles/s.css\" media-type=\"text/css\"/>\n{extra_manifest}"
        ),
        "",
        "",
    );
    let ch1 = doc("<p>text</p>");
    let mut files: Vec<(&str, &[u8])> = vec![
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/Styles/s.css", css.as_bytes()),
    ];
    files.extend_from_slice(extra);
    make_epub(&files)
}

fn css_of(after: &[(String, Vec<u8>)]) -> String {
    entry(after, "OEBPS/Styles/s.css")
}

/// The *Butcher's Crossing* case: CSS resolves against the stylesheet, not the
/// document that links it, so `url(OEBPS/Fonts/x.otf)` inside
/// `OEBPS/Styles/s.css` reads as `OEBPS/Styles/OEBPS/Fonts/x.otf`. Reading the
/// path from the archive root instead finds the font.
#[test]
fn a_url_written_from_the_archive_root_is_repointed() {
    let css = "@font-face {\n  font-family: \"G\";\n  src: url(OEBPS/Fonts/g.otf);\n}\n";
    let (outcome, after) = fix(&css_book(
        css,
        &[("OEBPS/Fonts/g.otf", b"OTTO\x00")],
        "    <item id=\"f\" href=\"Fonts/g.otf\" media-type=\"font/otf\"/>\n",
    ));

    assert!(
        outcome.changes.iter().any(|c| c.contains("repointed 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        css_of(&after).contains(r#"url("../Fonts/g.otf")"#),
        "{}",
        css_of(&after)
    );
    assert!(css_of(&after).contains("@font-face"), "the rule stays");
}

/// Deleting a dead `@font-face` is behaviour-preserving: the font could never
/// load, so the fallback is what has been rendering all along.
#[test]
fn a_font_face_with_no_font_behind_it_is_removed_whole() {
    let css = "@font-face {\n  font-family: \"Adobe Garamond Pro\";\n  \
               src: url(OEBPS/Fonts/gone.otf);\n}\n\
               p { font-family: \"Adobe Garamond Pro\", serif; }\n";
    let (outcome, after) = fix(&css_book(css, &[], ""));
    let fixed = css_of(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("@font-face")),
        "got {:?}",
        outcome.changes
    );
    assert!(!fixed.contains("@font-face"), "{fixed}");
    assert!(!fixed.contains("gone.otf"), "{fixed}");
    // The fallback that was already rendering must survive untouched.
    assert!(
        fixed.contains(r#"p { font-family: "Adobe Garamond Pro", serif; }"#),
        "{fixed}"
    );
}

#[test]
fn a_dead_import_loses_the_statement_and_a_moved_one_is_repointed() {
    let css = "@import url(\"gone.css\");\n@import url(\"OEBPS/Styles/other.css\");\n\
               p { margin: 0 }\n";
    let (_, after) = fix(&css_book(
        css,
        &[("OEBPS/Styles/other.css", b"em { font-style: italic }")],
        "    <item id=\"o\" href=\"Styles/other.css\" media-type=\"text/css\"/>\n",
    ));
    let fixed = css_of(&after);

    assert!(!fixed.contains("gone.css"), "{fixed}");
    assert!(fixed.contains(r#"@import url("other.css");"#), "{fixed}");
    assert!(fixed.contains("p { margin: 0 }"), "{fixed}");
}

#[test]
fn a_dead_url_in_an_ordinary_rule_costs_only_its_own_declaration() {
    let css = "p {\n  color: red;\n  background: url(missing.png);\n  margin: 0;\n}\n";
    let (_, after) = fix(&css_book(css, &[], ""));
    let fixed = css_of(&after);

    assert!(!fixed.contains("missing.png"), "{fixed}");
    assert!(fixed.contains("color: red"), "the rule survives: {fixed}");
    assert!(fixed.contains("margin: 0"), "{fixed}");
    assert!(fixed.contains("p {"), "{fixed}");
}

#[test]
fn two_files_sharing_a_basename_are_reported_rather_than_guessed_at() {
    let css = "p { background: url(dup.png) }\n";
    let (outcome, after) = fix(&css_book(
        css,
        &[
            ("OEBPS/A/dup.png", b"\x89PNG"),
            ("OEBPS/B/dup.png", b"\x89PNG"),
        ],
        "    <item id=\"d1\" href=\"A/dup.png\" media-type=\"image/png\"/>\n\
             \x20   <item id=\"d2\" href=\"B/dup.png\" media-type=\"image/png\"/>\n",
    ));

    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("matches 2 files")),
        "got {:?}",
        outcome.findings
    );
    assert_eq!(css_of(&after), css, "nothing may move");
}

#[test]
fn a_stylesheet_whose_references_all_resolve_is_untouched() {
    let css = "@font-face { src: url(\"../Fonts/g.otf\"); }\n\
               p { background: url(data:image/gif;base64,AA); }\n\
               a { color: blue }\n";
    let (outcome, after) = fix(&css_book(
        css,
        &[("OEBPS/Fonts/g.otf", b"OTTO\x00")],
        "    <item id=\"f\" href=\"Fonts/g.otf\" media-type=\"font/otf\"/>\n",
    ));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert_eq!(css_of(&after), css);
}

/// A markup reference gets the same ladder: `styles/` for `Styles/`.
#[test]
fn a_link_whose_directory_differs_only_in_case_is_repointed() {
    let opf = opf(
        "2.0",
        r#"    <item id="css" href="Styles/s.css" media-type="text/css"/>"#,
        "",
        "",
    );
    let ch1 = doc("<p>text</p>").replace(
        "</head>",
        r#"<link rel="stylesheet" type="text/css" href="styles/S.css"/></head>"#,
    );
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/Styles/s.css", b"p { margin: 0 }"),
    ]));

    assert!(
        outcome.changes.iter().any(|c| c.contains("repointed 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        ch1_of(&after).contains(r#"href="Styles/s.css""#),
        "{}",
        ch1_of(&after)
    );
}


// ---------------------------------------------------------------------------
// Manifest properties are computed from final state (§5j)
// ---------------------------------------------------------------------------

/// The ordering bug: `scripted` was declared for a `<script>` that
/// `dangling-resources` then deleted, so the tool removed one error and
/// introduced OPF-015 in the same run.
#[test]
fn a_property_is_not_declared_for_a_construct_a_later_fixer_removes() {
    let opf = opf("3.0", "", "", "");
    let ch1 = doc("<p>text</p>").replace("</head>", r#"<script src="js/kobo.js"></script></head>"#);
    let (outcome, after) = roundtrip_kept(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
    ]));
    let fixed = entry(&after, "OEBPS/content.opf");

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("removed 1 dead resource include")),
        "got {:?}",
        outcome.changes
    );
    assert!(!fixed.contains("scripted"), "{fixed}");
}

/// The same rule in the other direction: a property that is no longer earned
/// is withdrawn, not left behind.
#[test]
fn a_property_nothing_still_needs_is_withdrawn() {
    let opf = opf("3.0", "", "", "").replace(
        r#"<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml" properties="scripted svg"/>"#,
    );
    let ch1 = doc(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#);
    let (outcome, after) = roundtrip_kept(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
    ]));
    let fixed = entry(&after, "OEBPS/content.opf");

    assert!(
        outcome.changes.iter().any(|c| c.contains("withdrew 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        fixed.contains(r#"properties="svg""#),
        "svg is still earned: {fixed}"
    );
    assert!(!fixed.contains("scripted"), "{fixed}");
}

/// Properties this module does not derive say something about the item's role
/// that no content scan could work out, so they are carried through.
#[test]
fn the_nav_property_is_never_touched_by_the_property_sync() {
    let opf = opf("3.0", "", "", "").replace(
        r#"<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml" properties="nav scripted"/>"#,
    );
    let ch1 = doc("<p>text</p>");
    let (_, after) = roundtrip_kept(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
    ]));
    let fixed = entry(&after, "OEBPS/content.opf");

    assert!(fixed.contains(r#"properties="nav""#), "{fixed}");
    assert!(!fixed.contains("scripted"), "{fixed}");
}

/// A reference names a directory as well as a file, and that is evidence worth
/// using: with two copies of a picture in the book, `Images/plate.jpg` picks out
/// one of them unambiguously. Matching on the filename alone would throw the
/// directory away and report a tie it did not have to.
#[test]
fn a_directory_that_disambiguates_is_used_rather_than_reporting_a_tie() {
    let css = "p { background: url(Images/plate.jpg) }\n";
    let (outcome, after) = fix(&css_book(
        css,
        &[
            ("OEBPS/assets/Images/plate.jpg", b"\xff\xd8"),
            ("OEBPS/Thumbs/plate.jpg", b"\xff\xd8"),
        ],
        "    <item id=\"i1\" href=\"assets/Images/plate.jpg\" media-type=\"image/jpeg\"/>\n\
         \x20   <item id=\"i2\" href=\"Thumbs/plate.jpg\" media-type=\"image/jpeg\"/>\n",
    ));
    assert!(
        css_of(&after).contains("../assets/Images/plate.jpg"),
        "expected the Images/ one, got {:?} / {}",
        outcome.findings,
        css_of(&after)
    );
}

// ---------------------------------------------------------------------------
// dead-schemes
// ---------------------------------------------------------------------------

/// The Kindle leftover: `kindle:pos:…` names a position in a different file
/// format, so the link is dead for every reader of the EPUB.
#[test]
fn a_reader_private_scheme_loses_its_href_and_keeps_everything_else() {
    let (outcome, after) = fix(&book2(
        r#"<p><a id="k1" class="lnk" href="kindle:pos:fid:0001:off:0000000000">Genesis</a></p>"#,
    ));
    let fixed = ch1(&after);

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("dropped 1 dead kindle:")),
        "got {:?}",
        outcome.changes
    );
    assert!(!fixed.contains("kindle:"), "{fixed}");
    // An <a> with no href is valid in both rulesets, so the element, its text,
    // its class and — importantly — its id all stay where they were.
    assert!(
        fixed.contains(r#"<a id="k1" class="lnk">Genesis</a>"#),
        "{fixed}"
    );
}

/// A warning is not a licence to guess. `sms:` draws the same HTM-025 but is
/// somebody's real intention, so it is left exactly as written.
#[test]
fn a_scheme_that_might_mean_something_is_left_alone() {
    let before = book2(r#"<p><a href="sms:+15551234">text</a> <a href="x-custom:thing">x</a></p>"#);
    let (outcome, after) = fix(&before);
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert!(ch1(&after).contains("sms:+15551234"));
    assert!(ch1(&after).contains("x-custom:thing"));
}

#[test]
fn ordinary_links_are_untouched_by_the_scheme_check() {
    let (outcome, _) = fix(&book2(
        r##"<p><a href="ch1.xhtml#top">a</a> <a href="https://example.org">b</a>
        <a href="mailto:x@example.org">c</a> <a href="#top">d</a></p><p id="top">t</p>"##,
    ));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
}

/// When nothing in the package says where a navigation entry should point, the
/// link goes and the loss is named rather than buried in a count. This book has
/// no cover-image and no nav, so there is nothing to derive from.
#[test]
fn a_navigation_entry_with_nothing_to_derive_from_is_named() {
    let (outcome, after) = fix(&book2(
        r#"<nav epub:type="landmarks"><ol>
        <li><a epub:type="cover" href="kindle:embed:0001?mime=image/jpg">Cover</a></li>
        </ol></nav>"#,
    ));

    assert!(!ch1(&after).contains("kindle:"));
    let finding = outcome
        .findings
        .iter()
        .find(|f| f.contains("navigation entry"))
        .unwrap_or_else(|| panic!("expected a report, got {:?}", outcome.findings));
    assert!(finding.contains("\"Cover\""), "names the entry: {finding}");
}

/// A book with the standard EPUB 3 landmarks: a `cover-image` in the manifest,
/// one document displaying it, and a nav document.
fn landmark_book(body: &str) -> Vec<u8> {
    let opf = opf(
        "3.0",
        r#"    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="cov" href="Images/cover.jpeg" media-type="image/jpeg" properties="cover-image"/>
    <item id="tp" href="titlepage.xhtml" media-type="application/xhtml+xml" properties="svg"/>"#,
        r#"<itemref idref="tp"/>"#,
        "",
    );
    let titlepage = doc(r#"<div><svg xmlns="http://www.w3.org/2000/svg"
        xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 10 10">
        <image width="10" height="10" xlink:href="Images/cover.jpeg"/></svg></div>"#);
    let nav = doc(r#"<nav epub:type="toc"><ol><li><a href="ch1.xhtml">One</a></li></ol></nav>"#)
        .replace(
            r#"<html xmlns="http://www.w3.org/1999/xhtml">"#,
            r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">"#,
        );
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", doc(body).as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/titlepage.xhtml", titlepage.as_bytes()),
        ("OEBPS/Images/cover.jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]),
    ])
}

/// The NASB case. `epub:type="cover"` is not a hint to interpret — it is a
/// declaration, and EPUB 3 says where a cover lives: the manifest names the
/// cover *image*, and the cover *document* is whichever one displays it. So the
/// link can be repaired rather than merely silenced.
#[test]
fn a_cover_landmark_is_repointed_at_the_document_showing_the_cover() {
    let (outcome, after) = roundtrip_kept(&landmark_book(
        r#"<nav epub:type="landmarks"><ol>
        <li><a epub:type="cover" href="kindle:embed:0001?mime=image/jpg">Cover</a></li>
        </ol></nav>"#,
    ));
    let fixed = ch1(&after);

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("repointed 1 dead link")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        fixed.contains(r#"<a epub:type="cover" href="titlepage.xhtml">Cover</a>"#),
        "{fixed}"
    );
    // Repaired, not merely silenced: nothing to report.
    assert!(outcome.findings.is_empty(), "got {:?}", outcome.findings);
}

#[test]
fn a_toc_landmark_is_repointed_at_the_nav_document() {
    let (_, after) = roundtrip_kept(&landmark_book(
        r#"<nav epub:type="landmarks"><ol>
        <li><a epub:type="toc" href="kindle:pos:fid:0000">Contents</a></li>
        </ol></nav>"#,
    ));
    assert!(
        ch1(&after).contains(r#"href="nav.xhtml">Contents</a>"#),
        "{}",
        ch1(&after)
    );
}

/// An `epub:type` the package says nothing about stays a removal, because
/// there is no derivation to make — only a preference.
#[test]
fn a_landmark_type_the_package_cannot_locate_is_not_invented() {
    let (outcome, after) = roundtrip_kept(&landmark_book(
        r#"<nav epub:type="landmarks"><ol>
        <li><a epub:type="bodymatter" href="kindle:pos:fid:0009">Start</a></li>
        </ol></nav>"#,
    ));

    assert!(!ch1(&after).contains("kindle:"));
    assert!(!ch1(&after).contains("href="), "{}", ch1(&after));
    assert!(
        outcome.findings.iter().any(|f| f.contains("\"Start\"")),
        "got {:?}",
        outcome.findings
    );
}

/// `epub:type` is a space-separated list, so `"cover frontmatter"` is a cover.
#[test]
fn a_compound_epub_type_still_names_the_cover() {
    let (_, after) = roundtrip_kept(&landmark_book(
        r#"<nav epub:type="landmarks"><ol>
        <li><a epub:type="cover frontmatter" href="kindle:embed:0001">Cover</a></li>
        </ol></nav>"#,
    ));
    assert!(
        ch1(&after).contains(r#"href="titlepage.xhtml""#),
        "{}",
        ch1(&after)
    );
}

/// An ordinary body link losing a dead scheme is not a navigation loss, so it
/// draws no report.
#[test]
fn an_ordinary_dead_link_is_fixed_without_a_report() {
    let (outcome, _) = fix(&book2(
        r#"<p><a href="kindle:pos:fid:0001">Genesis</a></p>"#,
    ));
    assert!(outcome.findings.is_empty(), "got {:?}", outcome.findings);
}

// ---------------------------------------------------------------------------
// dc-language
// ---------------------------------------------------------------------------

const EN: &str = "It was a bright cold day in April, and the clocks were striking \
    thirteen. Winston Smith, his chin nuzzled into his breast in an effort to escape \
    the vile wind, slipped quickly through the glass doors of Victory Mansions, though \
    not quickly enough to prevent a swirl of gritty dust from entering along with him. ";
const LA: &str = "Gallia est omnis divisa in partes tres, quarum unam incolunt Belgae, \
    aliam Aquitani, tertiam qui ipsorum lingua Celtae, nostra Galli appellantur. Hi \
    omnes lingua, institutis, legibus inter se differunt. Gallos ab Aquitanis Garumna \
    flumen, a Belgis Matrona et Sequana dividit. ";

/// A book with no `dc:language`, and the given documents.
fn language_book(docs: &[(String, String)], lang_attr: &str) -> Vec<u8> {
    let mut items = String::new();
    let mut spine = String::new();
    for (i, (f, _)) in docs.iter().enumerate() {
        let _ = writeln!(
            items,
            "    <item id=\"d{i}\" href=\"{f}\" media-type=\"application/xhtml+xml\"/>"
        );
        let _ = write!(spine, "<itemref idref=\"d{i}\"/>");
    }
    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
{items}  </manifest>
  <spine toc="ncx">{spine}</spine>
</package>"#
    );
    let pages: Vec<(String, String)> = docs
        .iter()
        .map(|(f, text)| {
            (
                format!("OEBPS/{f}"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
                     <html xmlns=\"http://www.w3.org/1999/xhtml\"{lang_attr}>\
                     <head><title>T</title></head><body><p>{text}</p></body></html>"
                ),
            )
        })
        .collect();
    let mut files: Vec<(&str, &[u8])> = vec![
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
    ];
    files.extend(pages.iter().map(|(n, t)| (n.as_str(), t.as_bytes())));
    make_epub(&files)
}

fn chapters(prefix: &str, text: &str, n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| (format!("{prefix}{i}.xhtml"), text.repeat(3)))
        .collect()
}

fn opf_of(after: &[(String, Vec<u8>)]) -> String {
    entry(after, "OEBPS/content.opf")
}

/// What the documents declare is a stated fact, not an inference, so it is
/// used whatever the detection policy says — and only the primary subtag is
/// written, since nothing can tell `en-GB` from `en-US`.
#[test]
fn a_declared_xml_lang_is_used_and_reduced_to_its_primary_subtag() {
    let docs = chapters("ch", EN, 6);
    for policy in [
        epubfix::LanguagePolicy::EnglishOnly,
        epubfix::LanguagePolicy::Off,
    ] {
        let opts = epubfix::Options {
            language: policy,
            ..epubfix::Options::default()
        };
        let (outcome, after) =
            common::roundtrip_with(&language_book(&docs, r#" xml:lang="en-GB""#), &opts);
        assert!(
            outcome
                .changes
                .iter()
                .any(|c| c.contains("<dc:language>en</dc:language>") && c.contains("declare")),
            "got {:?} for {policy:?}",
            outcome.changes
        );
        assert!(opf_of(&after).contains("<dc:language>en</dc:language>"));
    }
}

/// The Aristotle shape: thirteen English chapters and two Latin footnote files.
/// One unlucky sample declares the book Latin, with confidence.
#[test]
fn latin_footnote_files_cannot_outvote_the_book() {
    let mut docs = vec![(
        "copyright.xhtml".to_string(),
        "Copyright 2011. All rights reserved. ISBN 978-0-00-000000-0.".to_string(),
    )];
    docs.extend(chapters("ch", EN, 13));
    docs.extend(chapters("notes", LA, 2));

    let (outcome, after) = roundtrip_full(&language_book(&docs, ""));
    assert!(
        outcome.changes.iter().any(|c| c.contains("detected")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        opf_of(&after).contains("<dc:language>en</dc:language>"),
        "{}",
        opf_of(&after)
    );
}

/// A Calibre-fragmented book: sixty documents of 280 characters each. A fixed
/// "take fifteen documents" yields almost nothing classifiable, so short
/// documents are glued together rather than discarded.
#[test]
fn documents_below_the_floor_are_glued_rather_than_discarded() {
    let docs: Vec<(String, String)> = (0..60)
        .map(|i| (format!("f{i:03}.xhtml"), EN[..280].to_string()))
        .collect();
    let (_, after) = roundtrip_full(&language_book(&docs, ""));
    assert!(
        opf_of(&after).contains("<dc:language>en</dc:language>"),
        "{}",
        opf_of(&after)
    );
}

/// The default policy writes only English — not because detection is worse in
/// other languages, but because the worst case becomes "did nothing" instead of
/// "confidently wrong".
#[test]
fn a_book_that_is_not_english_is_reported_by_default_and_written_on_request() {
    let fr = "Longtemps, je me suis couché de bonne heure. Parfois, à peine ma bougie \
        éteinte, mes yeux se fermaient si vite que je n'avais pas le temps de me dire: \
        Je m'endors. Et, une demi-heure après, la pensée qu'il était temps de chercher \
        le sommeil m'éveillait. ";
    let docs = chapters("ch", fr, 6);

    let (outcome, after) = roundtrip_full(&language_book(&docs, ""));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert!(!opf_of(&after).contains("dc:language"));
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("\"fr\"") && f.contains("--language-detect=any")),
        "the report must name what it saw: {:?}",
        outcome.findings
    );

    let opts = epubfix::Options {
        language: epubfix::LanguagePolicy::Any,
        ..epubfix::Options::default()
    };
    let (_, after) = common::roundtrip_with(&language_book(&docs, ""), &opts);
    assert!(opf_of(&after).contains("<dc:language>fr</dc:language>"));
}

/// Never a locale, never a default. A book with nothing to go on is reported.
#[test]
fn a_book_with_nothing_to_go_on_is_never_guessed_at() {
    let docs = vec![
        ("a.xhtml".to_string(), "Short.".to_string()),
        ("b.xhtml".to_string(), "Also short.".to_string()),
    ];
    let (outcome, after) = roundtrip_full(&language_book(&docs, ""));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("dc:language")),
        "got {:?}",
        outcome.changes
    );
    assert!(!opf_of(&after).contains("dc:language"));
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("neither the documents nor the text")),
        "got {:?}",
        outcome.findings
    );
}

#[test]
fn a_book_that_already_declares_a_language_is_untouched() {
    let (outcome, _) = fix(&book2("<p>text</p>"));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("dc:language")),
        "got {:?}",
        outcome.changes
    );
}

/// The Dublin Core prefix is a namespace binding, not a fixed spelling.
#[test]
fn the_prefix_the_package_actually_binds_is_the_one_written() {
    let docs = chapters("ch", EN, 6);
    let book = language_book(&docs, "");
    let rebound = {
        let files = read_epub(&book);
        let opf = entry(&files, "OEBPS/content.opf")
            .replace("xmlns:dc=", "xmlns:dcterms=")
            .replace("<dc:", "<dcterms:")
            .replace("</dc:", "</dcterms:");
        let owned: Vec<(String, Vec<u8>)> = files
            .into_iter()
            .map(|(n, d)| {
                if n == "OEBPS/content.opf" {
                    (n, opf.clone().into_bytes())
                } else {
                    (n, d)
                }
            })
            .collect();
        make_epub(
            &owned
                .iter()
                .map(|(n, d)| (n.as_str(), d.as_slice()))
                .collect::<Vec<_>>(),
        )
    };
    let (_, after) = roundtrip_full(&rebound);
    assert!(
        opf_of(&after).contains("<dcterms:language>en</dcterms:language>"),
        "{}",
        opf_of(&after)
    );
}

// ---------------------------------------------------------------------------
// The final scan
// ---------------------------------------------------------------------------

/// A document that will not parse is fatal to EPUB Check and invisible to every
/// fixer, since they all skip what they cannot scan. It is exactly the case
/// where reporting "nothing to do" is worst, so the run says so instead.
#[test]
fn a_document_that_will_not_parse_is_reported() {
    let opf = opf("2.0", "", "", "");
    let broken = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
        <html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\
        <body><p>Tom <b>and Jerry</p></body></html>";
    let (outcome, _) = roundtrip_full(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", broken.as_bytes()),
    ]));

    assert!(
        outcome
            .remaining
            .iter()
            .any(|r| r.contains("not well-formed XML") && r.contains("ch1.xhtml")),
        "got {:?}",
        outcome.remaining
    );
}

/// The lenient scanner is what lets fixers work on sloppy files, so it must not
/// be the thing asked about well-formedness — a distinction that hid this whole
/// class of defect until the two were separated.
#[test]
fn the_lenient_scanner_and_the_strict_check_disagree_on_purpose() {
    let sloppy = "<html><body><p>Tom <b>and Jerry</p></body></html>";
    assert!(
        epubfix::markup::scan(sloppy).is_ok(),
        "the editor must still be able to work here"
    );
    assert!(
        epubfix::markup::well_formed(sloppy).is_err(),
        "but the report must not call it fine"
    );
}

#[test]
fn a_well_formed_book_draws_no_parse_complaint() {
    let (outcome, _) = fix(&book2("<p>text</p>"));
    assert!(outcome.remaining.is_empty(), "got {:?}", outcome.remaining);
}

/// Renames are pending until the archive is repacked, so the book still knows
/// its files by their old names while the markup already points at the new
/// ones. Checking one against the other reports every renamed file as missing.
#[test]
fn a_renamed_file_is_not_reported_missing_by_the_closing_scan() {
    let opf = opf(
        "2.0",
        r#"    <item id="img" href="img/cover image.gif" media-type="image/gif"/>"#,
        "",
        "",
    );
    let ch1 = doc(r#"<p><img src="img/cover image.gif" alt=""/></p>"#);
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/img/cover image.gif", b"GIF89a"),
    ]));

    assert!(
        outcome.changes.iter().any(|c| c.contains("renamed 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(has(&after, "OEBPS/img/cover_image.gif"));
    assert!(
        outcome.remaining.is_empty(),
        "the rename is the repair, not a defect: {:?}",
        outcome.remaining
    );
}

// ---------------------------------------------------------------------------
// fragment-documents
// ---------------------------------------------------------------------------

/// An EPUB 2 book whose `ch1.xhtml` holds `raw` verbatim — no wrapping, no
/// assumptions — next to a well-formed sibling to copy the house style from.
fn book_raw(raw: &str) -> Vec<u8> {
    let opf = opf(
        "2.0",
        concat!(
            r#"    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>"#,
            "\n",
            r#"    <item id="css" href="s.css" media-type="text/css"/>"#,
            "\n",
            r#"    <item id="img" href="cover.gif" media-type="image/gif"/>"#
        ),
        r#"<itemref idref="ch2"/>"#,
        "",
    );
    let ch2 = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"en\">\n\
         <head><title>Two</title>\
         <link href=\"s.css\" rel=\"stylesheet\" type=\"text/css\"/></head>\n\
         <body>\n<p>prose</p>\n</body></html>";
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", raw.as_bytes()),
        ("OEBPS/ch2.xhtml", ch2.as_bytes()),
        ("OEBPS/s.css", b"p { margin: 0 }"),
        ("OEBPS/cover.gif", b"GIF89a"),
    ])
}

/// The Keats cover: a 45-byte file with no root element at all.
#[test]
fn a_bare_markup_fragment_becomes_a_document() {
    let (outcome, after) = fix(&book_raw(r#"<img alt="Image" src="cover.gif" />"#));
    let got = ch1(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("bare markup")),
        "got {:?}",
        outcome.changes
    );
    assert!(got.starts_with("<?xml version=\"1.0\""), "got {got}");
    assert!(got.contains(r#"<html xmlns="http://www.w3.org/1999/xhtml""#), "got {got}");
    assert!(got.contains("<body>"), "got {got}");
    assert!(got.contains(r#"<img alt="Image" src="cover.gif" />"#), "got {got}");
    assert!(outcome.remaining.is_empty(), "got {:?}", outcome.remaining);
}

/// Wrapping alone trades one epubcheck error for two: XHTML 1.1 wants block
/// content directly inside `<body>`, and an `<img>` is inline.
#[test]
fn a_wrapped_fragment_also_gets_the_block_container() {
    let (_, after) = fix(&book_raw(r#"<img alt="Image" src="cover.gif" />"#));
    let got = ch1(&after);
    let body = got.split_once("<body>").unwrap().1;
    assert!(
        body.trim_start().starts_with("<div>"),
        "the img must not sit bare in the body: {got}"
    );
}

/// The `<title>` is not optional — a head without one is itself an RSC-005 —
/// so it is taken from what the NCX already calls the document.
#[test]
fn the_invented_title_comes_from_the_navigation_label() {
    let (_, after) = fix(&book_raw(r#"<img alt="Image" src="cover.gif" />"#));
    assert!(ch1(&after).contains("<title>A</title>"), "got {}", ch1(&after));
}

/// A wrapped cover that lost its stylesheet would render unstyled, which is a
/// visible regression even though epubcheck is satisfied either way.
#[test]
fn the_house_style_is_copied_from_a_sibling_document() {
    let (_, after) = fix(&book_raw(r#"<img alt="Image" src="cover.gif" />"#));
    let got = ch1(&after);
    assert!(got.contains(r#"href="s.css""#), "got {got}");
    assert!(
        !got.contains("<title>Two</title>"),
        "the sibling's title belongs to the sibling: {got}"
    );
}

/// The other half of the class: a real document, correctly wrapped, whose body
/// still holds nothing a body may directly hold.
#[test]
fn a_document_whose_body_holds_only_inline_content_gets_a_container() {
    let raw = doc(r#"<img alt="Image" src="cover.gif"/>"#);
    let (outcome, after) = fix(&book_raw(&raw));

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("block container")),
        "got {:?}",
        outcome.changes
    );
    let got = ch1(&after);
    let body = got.split_once("<body>").unwrap().1;
    assert!(body.trim_start().starts_with("<div>"), "got {got}");
    assert!(
        got.contains("<title>T</title>"),
        "an existing head is left alone: {got}"
    );
}

/// The question is "does this body have *any* block content", never "only".
/// A chapter with one stray inline child among its paragraphs is the version
/// mismatch of section 5a, and rewriting the markup to paper over a wrong
/// package attribute is precisely what this tool must not do.
#[test]
fn a_body_with_block_content_is_left_alone_however_stray_its_siblings() {
    let raw = doc("<p>prose</p>\n<span>stray</span>");
    let (outcome, after) = fix(&book_raw(&raw));

    assert!(
        !outcome
            .changes
            .iter()
            .any(|c| c.contains("block container")),
        "got {:?}",
        outcome.changes
    );
    assert_eq!(ch1(&after), raw, "the document must come out untouched");
}

#[test]
fn an_ordinary_document_is_not_mistaken_for_a_fragment() {
    let raw = doc("<p>prose</p>");
    let (outcome, after) = fix(&book_raw(&raw));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert_eq!(ch1(&after), raw);
}

// ---------------------------------------------------------------------------
// empty-metadata
// ---------------------------------------------------------------------------

/// An EPUB 2 book whose `<metadata>` carries `extra` verbatim.
fn book_meta(extra: &str) -> Vec<u8> {
    let opf = opf("2.0", "", "", "").replace(
        "  </metadata>",
        &format!("{extra}\n  </metadata>"),
    );
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
    ])
}

/// The Hobbit's metadata block: four empty elements, only one of them flagged.
#[test]
fn empty_dc_elements_are_removed() {
    let (outcome, after) = fix(&book_meta(
        "    <dc:date/>\n    <dc:subject/>\n    <dc:description/>\n    <dc:rights/>",
    ));
    let opf = opf_of(&after);

    assert!(
        outcome.changes.iter().any(|c| c.contains("dc:date")),
        "got {:?}",
        outcome.changes
    );
    for gone in ["<dc:date", "<dc:subject", "<dc:description", "<dc:rights"] {
        assert!(!opf.contains(gone), "{gone} survived: {opf}");
    }
}

/// Four deletions in a row must not leave four blank lines behind.
#[test]
fn removing_metadata_takes_the_whole_line_with_it() {
    let (_, after) = fix(&book_meta(
        "    <dc:date/>\n    <dc:subject/>\n    <dc:description/>\n    <dc:rights/>",
    ));
    let opf = opf_of(&after);
    let metadata = opf
        .split_once("<metadata")
        .and_then(|(_, r)| r.split_once("</metadata>"))
        .map(|(m, _)| m)
        .unwrap();
    assert!(!metadata.contains("\n\n"), "blank lines left behind: {metadata}");
}

/// An element with content is information, however uninteresting.
#[test]
fn metadata_with_content_is_left_alone() {
    let (outcome, after) = fix(&book_meta("    <dc:date>2011-09-29</dc:date>"));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert!(opf_of(&after).contains("<dc:date>2011-09-29</dc:date>"));
}

/// Measured: `<dc:title/>` empty is a warning under EPUB 2 and an error under
/// EPUB 3, but `<dc:title>` *absent* is a hard `metadata incomplete` error in
/// both. Deleting it trades one complaint for a worse one — the 5e mistake.
#[test]
fn an_empty_required_element_is_reported_rather_than_deleted() {
    let (outcome, after) = fix(&book_meta("    <dc:title/>\n    <dc:date/>"));

    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("dc:title") && f.contains("require")),
        "got {:?}",
        outcome.findings
    );
    assert!(
        opf_of(&after).contains("<dc:title/>"),
        "the title must survive: {}",
        opf_of(&after)
    );
    assert!(
        !opf_of(&after).contains("<dc:date"),
        "the optional one still goes"
    );
}

/// Only `dc:*` elements are in scope. A `<meta>` with no content is the
/// package's business and often meaningful.
#[test]
fn empty_non_dc_metadata_is_not_touched() {
    let (outcome, after) = fix(&book_meta(r#"    <meta name="cover" content=""/>"#));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert!(opf_of(&after).contains(r#"<meta name="cover" content=""/>"#));
}

// ---------------------------------------------------------------------------
// dangling-resources: missing images
// ---------------------------------------------------------------------------

/// An EPUB 2 book whose chapter body is `body`, with one image that exists so
/// the archive is not trivially empty of them.
fn book_img(body: &str) -> Vec<u8> {
    let opf = opf(
        "2.0",
        r#"    <item id="real" href="images/real.jpg" media-type="image/jpeg"/>"#,
        "",
        "",
    );
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc(body).as_bytes()),
        ("OEBPS/images/real.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]),
    ])
}

/// The Hobbit case: the image is alone in a `<p>`, and both go.
#[test]
fn a_proven_missing_image_and_its_empty_wrapper_are_removed() {
    let (outcome, after) = fix(&book_img(
        "<h1>About the Publisher</h1>\n\
         <p class=\"ct-2\"><img alt=\"\" src=\"images/Art_logo.jpg\"/></p>\n\
         <p>HarperCollins</p>",
    ));
    let got = ch1(&after);

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("Art_logo.jpg") && c.contains("wrapper")),
        "the removal must be named in full: {:?}",
        outcome.changes
    );
    assert!(!got.contains("Art_logo"), "got {got}");
    assert!(!got.contains("ct-2"), "the empty wrapper goes too: {got}");
    assert!(got.contains("<p>HarperCollins</p>"), "got {got}");
    assert!(outcome.remaining.is_empty(), "got {:?}", outcome.remaining);
}

/// The wrapper is only removed when the image was its *only* content.
#[test]
fn a_wrapper_holding_other_content_survives_the_image() {
    let (_, after) = fix(&book_img(
        "<h1>Publisher</h1>\n<p class=\"ct-2\"><img alt=\"\" src=\"gone.jpg\"/> HarperCollins</p>",
    ));
    let got = ch1(&after);
    assert!(!got.contains("gone.jpg"), "got {got}");
    assert!(got.contains("HarperCollins"), "the text stays: {got}");
    assert!(got.contains("ct-2"), "the wrapper stays: {got}");
}

/// Measured: emptying the `<body>` produces `element "body" incomplete`, which
/// is a worse error than the RSC-007 being fixed. So the wrapper stays.
#[test]
fn the_wrapper_stays_when_removing_it_would_empty_the_body() {
    let (outcome, after) = fix(&book_img(r#"<p class="ct-2"><img alt="" src="gone.jpg"/></p>"#));
    let got = ch1(&after);

    assert!(!got.contains("gone.jpg"), "the image still goes: {got}");
    assert!(
        got.contains(r#"<p class="ct-2">"#),
        "the body must keep some block content: {got}"
    );
    assert!(outcome.remaining.is_empty(), "got {:?}", outcome.remaining);
}

/// Only one level. A `<div>` holding other material is not walked up into.
#[test]
fn removal_stops_at_the_immediate_wrapper() {
    let (_, after) = fix(&book_img(
        "<div class=\"keep\"><p><img alt=\"\" src=\"gone.jpg\"/></p><p>other</p></div>",
    ));
    let got = ch1(&after);
    assert!(got.contains(r#"<div class="keep">"#), "got {got}");
    assert!(got.contains("<p>other</p>"), "got {got}");
}

/// An image whose file is merely misfiled is repointed, never deleted — the
/// deletion is reserved for a proven absence, after all five candidates fail.
#[test]
fn an_image_that_resolves_elsewhere_is_repointed_not_removed() {
    let (outcome, after) = fix(&book_img(r#"<p><img alt="" src="../pics/real.jpg"/></p>"#));
    let got = ch1(&after);
    assert!(got.contains("images/real.jpg"), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("repointed")),
        "got {:?}",
        outcome.changes
    );
}

/// An `<a>` is never removed — the text and any id stay — but its href goes.
/// By the time the resolver says `Missing`, five candidate resolutions have
/// failed and the file is in the archive under no path, spelling or case, so
/// the link is dead whatever anyone does with it. Measured: `<a>` with no href
/// is clean in both rulesets.
#[test]
fn a_dead_hyperlink_keeps_its_text_and_loses_its_href() {
    let (outcome, after) = fix(&book_img(r#"<p><a href="gone.xhtml">chapter</a></p>"#));
    let got = ch1(&after);
    assert!(!got.contains("gone.xhtml"), "got {got}");
    assert!(got.contains("<a>chapter</a>"), "got {got}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("unlinked") && c.contains("gone.xhtml")),
        "the target is named, so nothing goes quietly: {:?}",
        outcome.changes
    );
}

/// A target that could never have been a filename is unlinked for a different
/// reason, and says so. Two Eddings volumes carry a placeholder a converter
/// never filled in; *Rise of the Horde* has a name that was already mojibake
/// when it was written.
#[test]
fn a_link_whose_target_was_never_a_filename_says_so() {
    for (href, text) in [
        ("XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX", "Chapter One"),
        ("%EF%BF%BD%EF%BF%BD", "Prologue"),
    ] {
        let (outcome, after) = fix(&book_img(&format!(
            r#"<p><a href="{href}" class="c">{text}</a></p>"#
        )));
        let got = ch1(&after);
        assert!(!got.contains(href), "{href}: got {got}");
        assert!(got.contains(&format!(r#"<a class="c">{text}</a>"#)), "{href}: got {got}");
        assert!(
            outcome
                .changes
                .iter()
                .any(|c| c.contains("never a filename")),
            "{href}: got {:?}",
            outcome.changes
        );
    }
}

#[test]
fn keep_missing_images_restores_the_report() {
    let opts = epubfix::Options {
        keep_missing_images: true,
        ..Default::default()
    };
    let before = book_img(r#"<h1>T</h1><p class="ct-2"><img alt="" src="gone.jpg"/></p>"#);
    let (outcome, after) = common::roundtrip_with(&before, &opts);

    assert!(ch1(&after).contains("gone.jpg"), "got {}", ch1(&after));
    assert!(
        outcome.findings.iter().any(|f| f.contains("gone.jpg")),
        "got {:?}",
        outcome.findings
    );
}

/// An `<svg>` cover page is a real document, not a fragment.
///
/// Both cover documents of a real *Hobbit* hold an `<svg>` directly in
/// `<body>`. epubcheck accepts that — `ns:svg` is in the permitted set it
/// prints — and an earlier `BLOCK` list omitted it, so the fixer wrapped two
/// documents that had nothing wrong with them. The same shape as every other
/// false positive this tool has had: an inferred content model, not an
/// observed error.
#[test]
fn an_svg_cover_page_is_not_wrapped() {
    let body = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 800">
<image width="600" height="800" xlink:href="cover.gif"/>
</svg>"#;
    let raw = doc(body);
    let (outcome, after) = fix(&book_raw(&raw));

    assert!(
        !outcome
            .changes
            .iter()
            .any(|c| c.contains("block container")),
        "got {:?}",
        outcome.changes
    );
    assert_eq!(ch1(&after), raw, "the cover must come out untouched");
}

/// Same for `MathML`, the other foreign-namespace root a body may hold.
///
/// Unlike SVG, `MathML` really is an error under EPUB 2, so this book retags —
/// and that is the repair. Wrapping the `<math>` in a `<div>` would not have
/// made it legal there, so it would have been an edit that fixed nothing.
#[test]
fn a_math_element_is_not_wrapped() {
    let raw = doc(r#"<math xmlns="http://www.w3.org/1998/Math/MathML"><mi>x</mi></math>"#);
    let (outcome, after) = fix(&book_raw(&raw));

    assert!(
        !outcome
            .changes
            .iter()
            .any(|c| c.contains("block container")),
        "got {:?}",
        outcome.changes
    );
    assert!(ch1(&after).contains("<math"), "got {}", ch1(&after));
    assert!(
        !ch1(&after).contains("<div>"),
        "no container was needed: {}",
        ch1(&after)
    );
}

/// A comment inside `<metadata>` crashed the whole run on a real book.
///
/// `<!-- … -->` has no element name, and the scanner records that as
/// `name_end == span.start`. Reading the raw name from one byte past the `<`
/// then asks for an inverted byte range, which panics — and because the sweep
/// had no guard, one book aborted the entire library.
#[test]
fn a_comment_in_the_metadata_does_not_crash_the_run() {
    let (outcome, after) = fix(&book_meta(
        "    <!-- calibre metadata -->\n    <dc:date/>",
    ));

    assert!(
        outcome.changes.iter().any(|c| c.contains("dc:date")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        opf_of(&after).contains("<!-- calibre metadata -->"),
        "the comment is not ours to remove: {}",
        opf_of(&after)
    );
}

/// The same shape for the other two nameless node kinds.
#[test]
fn a_processing_instruction_in_the_metadata_is_harmless() {
    let (_, after) = fix(&book_meta(
        "    <?calibre version=\"1\"?>\n    <dc:rights/>",
    ));
    assert!(opf_of(&after).contains("<?calibre"), "got {}", opf_of(&after));
    assert!(!opf_of(&after).contains("<dc:rights"));
}

// ---------------------------------------------------------------------------
// head-content
// ---------------------------------------------------------------------------

/// A book whose chapter head holds `extra` on top of the usual title.
fn book_head(extra: &str) -> Vec<u8> {
    let ch1 = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
         <head><title>T</title>\n{extra}\n</head>\n<body>\n<p>text</p>\n</body></html>"
    );
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/s.css", b"p { margin: 0 }"),
    ])
}

/// Calibre leaves three of these in the head of every document it writes.
#[test]
fn an_empty_paragraph_in_the_head_is_removed() {
    let (outcome, after) = fix(&book_head("<p> </p>\n<p> </p>"));
    assert!(
        outcome.changes.iter().any(|c| c.contains("<head>")),
        "got {:?}",
        outcome.changes
    );
    assert!(!ch1(&after).contains("<p> </p>"), "got {}", ch1(&after));
    assert!(ch1(&after).contains("<p>text</p>"), "the body is untouched");
}

/// Everything a head may legally hold stays, including a `<meta>` between the
/// paragraphs that were removed.
#[test]
fn legal_head_content_is_untouched() {
    let (_, after) = fix(&book_head(
        "<meta name=\"author\" content=\"me\"/>\n<p> </p>\n<link href=\"s.css\" rel=\"stylesheet\"/>",
    ));
    let got = ch1(&after);
    assert!(got.contains(r#"<meta name="author" content="me"/>"#), "got {got}");
    assert!(got.contains("s.css"), "got {got}");
    let head = got.split_once("<head>").unwrap().1.split_once("</head>").unwrap().0;
    assert!(!head.contains("<p>"), "got {head}");
    assert!(got.contains("<p>text</p>"), "the body keeps its own: {got}");
}

/// Text in a head is text nobody can see, and where it should go is a
/// judgement about the book rather than about the markup.
#[test]
fn a_head_element_carrying_text_is_reported_not_deleted() {
    let (outcome, after) = fix(&book_head("<p>a stray sentence</p>"));
    assert!(
        ch1(&after).contains("a stray sentence"),
        "got {}",
        ch1(&after)
    );
    assert!(
        outcome.findings.iter().any(|f| f.contains("<head>")),
        "got {:?}",
        outcome.findings
    );
}

/// A comment is not an element, and "not permitted here" must not reach it.
#[test]
fn a_comment_in_the_head_survives() {
    let (_, after) = fix(&book_head("<!-- calibre -->"));
    assert!(ch1(&after).contains("<!-- calibre -->"), "got {}", ch1(&after));
}

// ---------------------------------------------------------------------------
// orphan-links
// ---------------------------------------------------------------------------

/// A contents page linking at Word bookmarks Calibre discarded, plus the two
/// story documents whose headings those links name.
fn book_toc(contents: &str, h1: &str, h2: &str) -> Vec<u8> {
    let opf = opf(
        "2.0",
        concat!(
            r#"    <item id="s1" href="s1.xhtml" media-type="application/xhtml+xml"/>"#,
            "\n",
            r#"    <item id="s2" href="s2.xhtml" media-type="application/xhtml+xml"/>"#
        ),
        r#"<itemref idref="s1"/><itemref idref="s2"/>"#,
        "",
    );
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc(contents).as_bytes()),
        ("OEBPS/s1.xhtml", doc(h1).as_bytes()),
        ("OEBPS/s2.xhtml", doc(h2).as_bytes()),
    ])
}

/// The Girl With Curious Hair case: the link text is the destination.
#[test]
fn a_link_whose_anchor_was_discarded_is_repointed_at_its_heading() {
    let (outcome, after) = fix(&book_toc(
        r#"<p><a href="_Toc73360389"><span>LYNDON</span></a></p>"#,
        r#"<h1 id="pb_3">LYNDON</h1><p>Hello down there.</p>"#,
        r#"<h1 id="pb_4">JOHN BILLY</h1><p>Was me supposed to tell.</p>"#,
    ));

    assert!(
        outcome.changes.iter().any(|c| c.contains("repointed 1")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        ch1(&after).contains(r#"href="s1.xhtml#pb_3""#),
        "got {}",
        ch1(&after)
    );
    assert!(
        ch1(&after).contains("<span>LYNDON</span>"),
        "the link text is untouched: {}",
        ch1(&after)
    );
    assert!(outcome.remaining.is_empty(), "got {:?}", outcome.remaining);
}

/// A contents page pointing at the wrong chapter is worse than one that points
/// nowhere, so two headings with the same words resolve to neither.
#[test]
fn an_ambiguous_heading_is_reported_rather_than_guessed_at() {
    let (outcome, after) = fix(&book_toc(
        r#"<p><a href="_Toc1">LYNDON</a></p>"#,
        r#"<h1 id="a">LYNDON</h1><p>one</p>"#,
        r#"<h1 id="b">LYNDON</h1><p>two</p>"#,
    ));

    // orphan-links declines, so the reader is told which two headings clash —
    // and the dead href still goes, since nothing can make it resolve.
    assert!(
        outcome.findings.iter().any(|f| f.contains("2 headings")),
        "got {:?}",
        outcome.findings
    );
    assert!(!ch1(&after).contains(r#"href="_Toc1""#), "got {}", ch1(&after));
    assert!(ch1(&after).contains(">LYNDON</a>"), "got {}", ch1(&after));
}

/// Fire on the observed error. A link that already lands somewhere is not this
/// fixer's business, whatever its text says.
#[test]
fn a_working_link_is_never_repointed_by_its_text() {
    let (outcome, after) = fix(&book_toc(
        r#"<p><a href="s2.xhtml">LYNDON</a></p>"#,
        r#"<h1 id="pb_3">LYNDON</h1><p>one</p>"#,
        r#"<h1 id="pb_4">JOHN BILLY</h1><p>two</p>"#,
    ));
    assert!(ch1(&after).contains(r#"href="s2.xhtml""#), "got {}", ch1(&after));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
}

/// No heading says what the link says, so there is nothing to aim at and the
/// existing report stands.
#[test]
fn a_broken_link_matching_no_heading_is_still_reported() {
    let (outcome, after) = fix(&book_toc(
        r#"<p><a href="_Toc1">SOMETHING ELSE ENTIRELY</a></p>"#,
        r#"<h1 id="a">LYNDON</h1><p>one</p>"#,
        r#"<h1 id="b">JOHN BILLY</h1><p>two</p>"#,
    ));
    // orphan-links has nothing to aim at, so the bookmark is unlinked and
    // named — the contents line still reads, it just no longer pretends to go
    // anywhere.
    assert!(
        outcome.changes.iter().any(|c| c.contains("_Toc1")),
        "got {:?}",
        outcome.changes
    );
    assert!(!ch1(&after).contains(r#"href="_Toc1""#), "got {}", ch1(&after));
    assert!(
        ch1(&after).contains(">SOMETHING ELSE ENTIRELY</a>"),
        "got {}",
        ch1(&after)
    );
}

/// An empty `<body>` is `element "body" incomplete`, not "nothing to do".
///
/// A Kobo build of *Essays and Aphorisms* carries one error through a whole
/// run because of this: an empty XHTML document in the spine, which the
/// container step skipped on the grounds that there was nothing to wrap.
#[test]
fn an_empty_body_gets_the_container_too() {
    let raw = doc("");
    let (outcome, after) = fix(&book_raw(&raw));

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("block container")),
        "got {:?}",
        outcome.changes
    );
    assert!(ch1(&after).contains("<div>"), "got {}", ch1(&after));
    assert!(outcome.remaining.is_empty(), "got {:?}", outcome.remaining);
}

/// HTML5 accepts an empty body and accepts bare inline content, so under
/// EPUB 3 the container repairs nothing and the edit would be noise on a book
/// with nothing wrong with it. Measured in both rulesets.
#[test]
fn epub3_bodies_are_left_alone_however_thin_their_content() {
    for body in ["", r#"<img alt="x" src="cover.gif"/>"#] {
        let before = common::epub3(body, "<p>x</p>");
        let (outcome, after) = roundtrip_full(&before);
        assert!(
            !outcome
                .changes
                .iter()
                .any(|c| c.contains("block container")),
            "body {body:?} got {:?}",
            outcome.changes
        );
        assert!(
            !entry(&after, "OEBPS/ch1.xhtml").contains("<div>"),
            "body {body:?} was wrapped anyway"
        );
    }
}

/// A content document is whatever the manifest says it is. This one is called
/// `.xml`, which every fixer used to skip on sight while epubcheck validated
/// it as XHTML and reported the error nobody could fix.
#[test]
fn a_content_document_is_found_by_its_media_type_not_its_extension() {
    let opf = opf(
        "2.0",
        r#"    <item id="pm" href="page-map.xml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="pm"/>"#,
        "",
    );
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
        ("OEBPS/page-map.xml", doc("").as_bytes()),
    ]));

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("block container")),
        "got {:?}",
        outcome.changes
    );
    assert!(entry(&after, "OEBPS/page-map.xml").contains("<div>"));
}

/// The NCX is XML in the package too, and is emphatically not a content
/// document. Its media type is what keeps it out.
#[test]
fn the_ncx_is_not_mistaken_for_a_content_document() {
    let (outcome, after) = fix(&book2("<p>text</p>"));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert_eq!(entry(&after, "OEBPS/toc.ncx"), NCX);
}

// ---------------------------------------------------------------------------
// nested anchors: the self-closing inner anchor
// ---------------------------------------------------------------------------

/// A self-closing inner anchor has no end tag to rewrite, so the replacement
/// has to be a whole element rather than an opening tag.
///
/// The real markup, from *A New History of Western Philosophy*:
///
/// ```html
/// <a class="nounder" href="ch02.html#x"><a id="page_viii"/>Stoicism</a>
/// ```
///
/// Emitting `<span id="page_viii">` there left it unclosed and turned two
/// content-model errors into two *fatal* ones — a book no reader can open.
#[test]
fn a_self_closing_nested_anchor_becomes_a_whole_span() {
    let (outcome, after) = fix(&book2(
        r#"<p><a class="nounder" href="ch1.xhtml#x"><a id="page_viii"/>Stoicism</a></p>"#,
    ));
    let got = ch1(&after);

    assert!(
        got.contains(r#"<span id="page_viii"></span>"#),
        "got {got}"
    );
    assert!(got.contains("Stoicism"), "the text stays: {got}");
    assert!(
        epubfix::markup::well_formed(&got).is_ok(),
        "the document must still parse: {got}"
    );
    assert!(outcome.remaining.is_empty(), "got {:?}", outcome.remaining);
}

/// The same shape with no id: nothing worth keeping, so the tag goes entirely
/// and leaves no stray `</span>` behind.
#[test]
fn a_self_closing_nested_anchor_with_no_id_leaves_nothing() {
    let (_, after) = fix(&book2(
        r#"<p><a href="ch1.xhtml#x"><a/>Stoicism</a></p>"#,
    ));
    let got = ch1(&after);
    assert!(!got.contains("</span>"), "got {got}");
    assert!(got.contains("Stoicism"), "got {got}");
    assert!(epubfix::markup::well_formed(&got).is_ok(), "got {got}");
}

// ---------------------------------------------------------------------------
// duplicate-ids: one algorithm, both files
// ---------------------------------------------------------------------------

/// A replacement name must be checked against every id in the document, not
/// just the ones seen so far.
///
/// This was two fixers with one algorithm between them, and the NCX copy had
/// drifted: on `np`, `np`, `np_2` it renamed the duplicate onto the innocent
/// `np_2`, then renamed *that* to `np_2_2` to resolve the collision it had
/// just created. Two elements changed where one was wrong.
#[test]
fn a_rename_skips_past_a_name_that_is_already_in_use() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="np" playOrder="1"><navLabel><text>A</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="np" playOrder="2"><navLabel><text>B</text></navLabel><content src="ch1.xhtml#b"/></navPoint>
    <navPoint id="np_2" playOrder="3"><navLabel><text>C</text></navLabel><content src="ch1.xhtml#c"/></navPoint>
  </navMap>
</ncx>"#;
    let (_, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        (
            "OEBPS/ch1.xhtml",
            doc(r#"<p id="b">one</p><p id="c">two</p>"#).as_bytes(),
        ),
    ]));

    let got = entry(&after, "OEBPS/toc.ncx");
    let ids: Vec<&str> = got
        .match_indices(r#"navPoint id=""#)
        .map(|(i, m)| {
            let rest = &got[i + m.len()..];
            &rest[..rest.find('"').unwrap()]
        })
        .collect();
    assert_eq!(ids, ["np", "np_3", "np_2"], "got {got}");
}

/// The same shape in a content document, which is where the correct
/// implementation already lived.
#[test]
fn the_same_rule_holds_in_a_content_document() {
    let (_, after) = fix(&book2(
        r#"<p id="x">one</p><p id="x">two</p><p id="x_2">three</p>"#,
    ));
    let got = ch1(&after);
    assert!(got.contains(r#"id="x_3""#), "got {got}");
    assert!(got.contains(r#"id="x_2">three"#), "the innocent one stays: {got}");
}

// ---------------------------------------------------------------------------
// mimetype
// ---------------------------------------------------------------------------

/// Build an archive with a deliberately wrong `mimetype` entry: last in the
/// archive, deflated, and with a trailing newline.
fn book_bad_mimetype() -> Vec<u8> {
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    let mut buf = Cursor::new(Vec::new());
    {
        let mut w = ZipWriter::new(&mut buf);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, data) in [
            ("META-INF/container.xml", CONTAINER.to_string()),
            ("OEBPS/content.opf", opf("2.0", "", "", "")),
            ("OEBPS/toc.ncx", NCX.to_string()),
            ("OEBPS/ch1.xhtml", doc("<p>text</p>")),
        ] {
            w.start_file(name, deflated).unwrap();
            w.write_all(data.as_bytes()).unwrap();
        }
        w.start_file("mimetype", deflated).unwrap();
        w.write_all(b"application/epub+zip\n").unwrap();
        w.finish().unwrap();
    }
    buf.into_inner()
}

/// The repair lived in the archive writer all along; nothing reported it, so a
/// book whose only defect was its mimetype entry drew "nothing to do" and was
/// never rewritten.
#[test]
fn a_wrong_mimetype_entry_is_reported_so_the_book_is_rewritten() {
    let (outcome, after) = roundtrip_full(&book_bad_mimetype());

    assert!(
        outcome.changes.iter().any(|c| c.contains("mimetype")),
        "got {:?}",
        outcome.changes
    );
    assert_eq!(after[0].0, "mimetype", "it must come first");
    assert_eq!(after[0].1, b"application/epub+zip", "and hold exactly that");
}

/// It must be silent on every book that already has this right, which is
/// almost all of them.
#[test]
fn a_correct_mimetype_entry_draws_no_comment() {
    let (outcome, _) = fix(&book2("<p>text</p>"));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("mimetype")),
        "got {:?}",
        outcome.changes
    );
}

// ---------------------------------------------------------------------------
// Derived from epubcheck's message catalogue rather than from a broken book.
// Every trigger below was confirmed against EPUB Check 5.2.1 first.
// ---------------------------------------------------------------------------

/// A book with `extra` appended to its manifest.
fn book_manifest(extra: &str) -> Vec<u8> {
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", extra, "", "").as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
        ("OEBPS/s.css", b"p { margin: 0 }"),
        ("OEBPS/i.gif", b"GIF89a"),
    ])
}

/// OPF-035 and OPF-037: media types that have been superseded.
#[test]
fn superseded_media_types_are_replaced() {
    let (outcome, after) = fix(&book_manifest(
        r#"    <item id="c" href="s.css" media-type="text/x-oeb1-css"/>"#,
    ));
    assert!(
        outcome.changes.iter().any(|c| c.contains("media-type")),
        "got {:?}",
        outcome.changes
    );
    assert!(opf_of(&after).contains(r#"media-type="text/css""#));
}

/// The match is on the whole value: a type that merely starts the same way is
/// a different type and must not be rewritten.
#[test]
fn a_media_type_that_only_looks_similar_is_left_alone() {
    let (outcome, after) = fix(&book_manifest(
        r#"    <item id="c" href="s.css" media-type="text/html-fragment"/>"#,
    ));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert!(opf_of(&after).contains(r#"media-type="text/html-fragment""#));
}

/// OPF-091: a manifest entry names a file, so it cannot carry a fragment.
#[test]
fn a_fragment_on_a_manifest_href_is_dropped() {
    let (outcome, after) = fix(&book_manifest(
        r#"    <item id="i" href="i.gif#part" media-type="image/gif"/>"#,
    ));
    assert!(
        outcome.changes.iter().any(|c| c.contains("fragment")),
        "got {:?}",
        outcome.changes
    );
    assert!(opf_of(&after).contains(r#"href="i.gif""#), "{}", opf_of(&after));
}

/// OPF-099: a manifest is the list of everything else.
#[test]
fn the_manifest_entry_for_the_package_document_is_removed() {
    let (outcome, after) = fix(&book_manifest(
        r#"    <item id="self" href="content.opf" media-type="application/oebps-package+xml"/>"#,
    ));
    assert!(
        outcome.changes.iter().any(|c| c.contains("package document")),
        "got {:?}",
        outcome.changes
    );
    assert!(!opf_of(&after).contains("content.opf"), "{}", opf_of(&after));
}

/// RSC-013 and RSC-009: a fragment on a target that has nothing inside it.
#[test]
fn inert_fragments_on_stylesheets_and_rasters_are_dropped() {
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        (
            "OEBPS/content.opf",
            opf(
                "2.0",
                concat!(
                    r#"    <item id="c" href="s.css" media-type="text/css"/>"#,
                    "\n",
                    r#"    <item id="i" href="i.gif" media-type="image/gif"/>"#
                ),
                "",
                "",
            )
            .as_bytes(),
        ),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        (
            "OEBPS/ch1.xhtml",
            doc(concat!(
                r#"<p><img src="i.gif#x" alt="x"/></p>"#,
                r#"<p><a href="s.css#top">s</a></p>"#
            ))
            .as_bytes(),
        ),
        ("OEBPS/s.css", b"p { margin: 0 }"),
        ("OEBPS/i.gif", b"GIF89a"),
    ]));

    let got = ch1(&after);
    assert!(got.contains(r#"src="i.gif""#), "got {got}");
    assert!(got.contains(r#"href="s.css""#), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("inert fragment")),
        "got {:?}",
        outcome.changes
    );
}

/// An SVG really is a document with ids in it, so its fragments stay.
#[test]
fn a_fragment_on_an_svg_is_kept() {
    let svg = r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect id="r" width="10" height="10"/></svg>"#;
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        (
            "OEBPS/content.opf",
            opf(
                "2.0",
                r#"    <item id="i" href="i.svg" media-type="image/svg+xml"/>"#,
                "",
                "",
            )
            .as_bytes(),
        ),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        (
            "OEBPS/ch1.xhtml",
            doc(r#"<p><img src="i.svg#r" alt="x"/></p>"#).as_bytes(),
        ),
        ("OEBPS/i.svg", svg.as_bytes()),
    ]));
    assert!(ch1(&after).contains(r#"src="i.svg#r""#), "got {}", ch1(&after));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
}

/// OPF-016 and OPF-017: container.xml with no usable `full-path`. The answer is
/// in the archive — the package document is right there.
#[test]
fn a_container_with_no_rootfile_path_is_pointed_at_the_package() {
    for rootfile in [
        r#"<rootfile media-type="application/oebps-package+xml"/>"#,
        r#"<rootfile full-path="" media-type="application/oebps-package+xml"/>"#,
        // Written as a pair rather than self-closing, which is how both real
        // books that hit this spell it. Without a kind filter the `</rootfile>`
        // end node carries no attributes, takes the "absent" branch, and comes
        // out as `</rootfile full-path="...">` — not XML.
        r#"<rootfile media-type="application/oebps-package+xml"></rootfile>"#,
    ] {
        let container = CONTAINER.replace(
            r#"<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>"#,
            rootfile,
        );
        let (outcome, after) = fix(&make_epub(&[
            ("META-INF/container.xml", container.as_bytes()),
            ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
            ("OEBPS/toc.ncx", NCX.as_bytes()),
            ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
        ]));

        assert!(
            outcome.changes.iter().any(|c| c.contains("rootfile")),
            "{rootfile}: got {:?}",
            outcome.changes
        );
        let container = entry(&after, "META-INF/container.xml");
        assert!(
            container.contains(r#"full-path="OEBPS/content.opf""#),
            "{rootfile}: got {container}"
        );
        // The attribute landing *somewhere* is not the test. It has to land in
        // a start tag, and only parsing the result says so.
        assert!(
            epubfix::markup::well_formed(&container).is_ok(),
            "{rootfile}: got {container}"
        );
    }
}

/// PKG-011: a name may not end with a full stop.
#[test]
fn a_filename_ending_in_a_dot_is_renamed_and_its_references_follow() {
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        (
            "OEBPS/content.opf",
            opf(
                "2.0",
                r#"    <item id="i" href="pic." media-type="image/gif"/>"#,
                "",
                "",
            )
            .as_bytes(),
        ),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        (
            "OEBPS/ch1.xhtml",
            doc(r#"<p><img src="pic." alt="x"/></p>"#).as_bytes(),
        ),
        ("OEBPS/pic.", b"GIF89a"),
    ]));

    assert!(
        outcome.changes.iter().any(|c| c.contains("renamed")),
        "got {:?}",
        outcome.changes
    );
    assert!(has(&after, "OEBPS/pic"), "got {:?}", names(&after));
    assert!(ch1(&after).contains(r#"src="pic""#), "got {}", ch1(&after));
}

/// OPF-060: two entries that differ only by case are one name to OCF. The
/// first in archive order keeps it.
#[test]
fn entries_differing_only_by_case_are_made_distinct() {
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        (
            "OEBPS/content.opf",
            opf(
                "2.0",
                concat!(
                    r#"    <item id="i" href="Pic.gif" media-type="image/gif"/>"#,
                    "\n",
                    r#"    <item id="j" href="pic.gif" media-type="image/gif"/>"#
                ),
                "",
                "",
            )
            .as_bytes(),
        ),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        (
            "OEBPS/ch1.xhtml",
            doc(r#"<p><img src="Pic.gif" alt="x"/><img src="pic.gif" alt="y"/></p>"#).as_bytes(),
        ),
        ("OEBPS/Pic.gif", b"GIF89a"),
        ("OEBPS/pic.gif", b"GIF89b"),
    ]));

    assert!(
        outcome.changes.iter().any(|c| c.contains("renamed")),
        "got {:?}",
        outcome.changes
    );
    assert!(has(&after, "OEBPS/Pic.gif"), "the first one keeps its name");
    assert!(!has(&after, "OEBPS/pic.gif"), "got {:?}", names(&after));
    assert!(ch1(&after).contains(r#"src="Pic.gif""#), "got {}", ch1(&after));
}

/// CSS-003 and the fatal shape: a UTF-16 entry, re-encoded and re-declared.
#[test]
fn a_utf16_entry_is_re_encoded_as_utf8() {
    let css: Vec<u8> = {
        let s = "@charset \"utf-16\";\np { margin: 0 }";
        let mut v = vec![0xFF, 0xFE];
        v.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        v
    };
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        (
            "OEBPS/content.opf",
            opf(
                "2.0",
                r#"    <item id="c" href="s.css" media-type="text/css"/>"#,
                "",
                "",
            )
            .as_bytes(),
        ),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
        ("OEBPS/s.css", &css),
    ]));

    assert!(
        outcome.changes.iter().any(|c| c.contains("UTF-16")),
        "got {:?}",
        outcome.changes
    );
    let out = entry(&after, "OEBPS/s.css");
    assert!(out.contains("@charset \"utf-8\";"), "got {out}");
    assert!(out.contains("margin: 0"), "the rules survive: {out}");
}

/// A UTF-8 book must not be re-encoded, or every book would be rewritten.
#[test]
fn a_utf8_book_is_not_re_encoded() {
    let (outcome, _) = fix(&book2("<p>text</p>"));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("UTF-16")),
        "got {:?}",
        outcome.changes
    );
}

// ---------------------------------------------------------------------------
// The Hero of Ages: 1049 errors from six habits
// ---------------------------------------------------------------------------

/// `<a name="x" id="x">` is the legacy anchor pattern — one identity written
/// twice on purpose. Counting the two attributes separately saw a duplicate
/// that was not there and renamed the id to `x_2`, breaking the pair. One real
/// book had 303 of them.
#[test]
fn an_anchors_matching_name_and_id_are_not_a_duplicate() {
    let (_, after) = fix(&book2(r#"<p><a name="x" id="x">anchor</a></p>"#));
    let got = ch1(&after);
    assert!(!got.contains("x_2"), "the pair is not a duplicate: {got}");
    assert!(got.contains(r#"id="x""#), "got {got}");
}

/// XHTML 1.1 removed `name` from `<a>`, and it almost always duplicates the id.
#[test]
fn an_anchor_name_that_repeats_the_id_is_dropped() {
    let (outcome, after) = fix(&book2(r#"<p><a name="x" id="x">anchor</a></p>"#));
    let got = ch1(&after);
    assert!(!got.contains("name="), "got {got}");
    assert!(got.contains(r#"<a id="x">anchor</a>"#), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("<a name>")),
        "got {:?}",
        outcome.changes
    );
}

/// With no id, the name *is* the id and becomes one, so links keep landing.
#[test]
fn an_anchor_name_with_no_id_becomes_the_id() {
    let (_, after) = fix(&book2(
        r##"<p><a name="x">anchor</a></p><p><a href="#x">to it</a></p>"##,
    ));
    let got = ch1(&after);
    assert!(got.contains(r#"<a id="x">"#), "got {got}");
    assert!(got.contains(r##"href="#x""##), "the link still resolves: {got}");
}

/// HTML5 accepts `name` on an anchor, so an EPUB 3 book is left alone.
#[test]
fn anchor_names_are_untouched_in_epub3() {
    let before = common::epub3(r#"<p><a name="x" id="x">anchor</a></p>"#, "<p>y</p>");
    let (outcome, after) = roundtrip_full(&before);
    assert!(
        entry(&after, "OEBPS/ch1.xhtml").contains(r#"name="x""#),
        "got {}",
        entry(&after, "OEBPS/ch1.xhtml")
    );
    assert!(
        !outcome.changes.iter().any(|c| c.contains("<a name>")),
        "got {:?}",
        outcome.changes
    );
}

/// `<body text link>` is an error under *both* rulesets, unlike the table
/// attributes it sits beside.
#[test]
fn presentational_body_attributes_are_stripped() {
    for v3 in [false, true] {
        let before = if v3 {
            common::epub3("<p>text</p>", "<p>y</p>")
        } else {
            common::epub2("<p>text</p>", "<p>y</p>")
        };
        // Put the attributes on the body of the built book.
        let patched: Vec<(String, Vec<u8>)> = read_epub(&before)
            .into_iter()
            .map(|(n, d)| {
                if n == "OEBPS/ch1.xhtml" {
                    let t = String::from_utf8(d).unwrap().replace(
                        "<body>",
                        r##"<body text="#000000" link="#0000ff" bgcolor="#fff">"##,
                    );
                    (n, t.into_bytes())
                } else {
                    (n, d)
                }
            })
            .collect();
        let rebuilt: Vec<(&str, &[u8])> =
            patched.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        let (outcome, after) = roundtrip_full(&make_epub(&rebuilt));

        let got = entry(&after, "OEBPS/ch1.xhtml");
        for gone in ["text=", "link=", "bgcolor="] {
            assert!(!got.contains(gone), "v3={v3} {gone} survived: {got}");
        }
        assert!(
            outcome.changes.iter().any(|c| c.contains("legacy attribute")),
            "v3={v3} got {:?}",
            outcome.changes
        );
    }
}

/// XHTML 1.1 spells its enumerated keywords in lower case, so `dir="LTR"` is a
/// bad *value*, not a bad attribute.
#[test]
fn an_uppercase_keyword_value_is_lowercased() {
    let (outcome, after) = fix(&book2(
        r#"<table><tr><td valign="TOP">c</td></tr></table>"#,
    ));
    assert!(ch1(&after).contains(r#"valign="top""#), "got {}", ch1(&after));
    assert!(
        outcome.changes.iter().any(|c| c.contains("lower-cased")),
        "got {:?}",
        outcome.changes
    );
}

/// A value that is not one of the keywords at all is a different defect, and
/// lower-casing it would dress the error up rather than repair it.
#[test]
fn a_value_that_is_not_a_keyword_is_left_for_the_report() {
    let (outcome, after) = fix(&book2(r#"<p><span dir="sideways">x</span></p>"#));
    assert!(ch1(&after).contains(r#"dir="sideways""#), "got {}", ch1(&after));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("lower-cased")),
        "got {:?}",
        outcome.changes
    );
}

/// HTML5 matches enumerated values case-insensitively, so EPUB 3 is untouched.
#[test]
fn keyword_case_is_not_touched_in_epub3() {
    let before = common::epub3(r#"<p><span dir="LTR">x</span></p>"#, "<p>y</p>");
    let (outcome, after) = roundtrip_full(&before);
    assert!(entry(&after, "OEBPS/ch1.xhtml").contains(r#"dir="LTR""#));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("lower-cased")),
        "got {:?}",
        outcome.changes
    );
}

/// `<u>` is gone from XHTML 1.1. A `<span>` is legal in its place, but only
/// keeps the underline if something still says so.
#[test]
fn a_u_element_becomes_a_span_that_still_underlines() {
    let (outcome, after) = fix(&book2("<p><u>underlined</u></p>"));
    let got = ch1(&after);
    assert!(!got.contains("<u>"), "got {got}");
    assert!(got.contains("text-decoration: underline"), "got {got}");
    assert!(got.contains("underlined</span>"), "the text stays: {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("<u>")),
        "got {:?}",
        outcome.changes
    );
}

/// When the book's own stylesheet already underlines the class, the `<span>`
/// needs no inline style — the same question `legacy-table-attrs` asks.
#[test]
fn a_u_whose_class_already_underlines_gets_no_inline_style() {
    let (_, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        (
            "OEBPS/content.opf",
            opf(
                "2.0",
                r#"    <item id="c" href="s.css" media-type="text/css"/>"#,
                "",
                "",
            )
            .as_bytes(),
        ),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        (
            "OEBPS/ch1.xhtml",
            doc(r#"<p><u class="ul">underlined</u></p>"#)
                .replace("</head>", r#"<link rel="stylesheet" type="text/css" href="s.css"/></head>"#)
                .as_bytes(),
        ),
        ("OEBPS/s.css", b".ul { text-decoration: underline }"),
    ]));
    let got = ch1(&after);
    assert!(got.contains(r#"<span class="ul">"#), "got {got}");
    assert!(!got.contains("style="), "got {got}");
}

/// The NCX has ids too, and they must be XML Names. One real book names every
/// navPoint with a UUID; 61 of 96 start with a digit.
#[test]
fn ncx_ids_that_are_not_xml_names_are_sanitised() {
    let ncx = NCX.replace(r#"id="n1""#, r#"id="96bb9689-dcf8-4189""#);
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
    ]));
    let got = entry(&after, "OEBPS/toc.ncx");
    assert!(got.contains(r#"id="id_96bb9689"#), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("sanitised")),
        "got {:?}",
        outcome.changes
    );
}

/// A `<link>` is a void element: everything it does, it does through its
/// `href`. One whose target is proven absent does nothing, whatever its `rel`
/// says — a Gutenberg *Dracula* has fourteen `<link rel="coverpage">` and they
/// used to be reported as "left alone because it carries content", which is
/// true of an `<a>` and not of these.
#[test]
fn a_dead_link_is_removed_whatever_its_rel_says() {
    let page = doc("<p>text</p>").replace(
        "</head>",
        r#"<link rel="coverpage" href="images/cover.jpg"/></head>"#,
    );
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", page.as_bytes()),
    ]));

    assert!(!ch1(&after).contains("coverpage"), "{}", ch1(&after));
    assert!(
        outcome.changes.iter().any(|c| c.contains("resource include")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        !outcome.findings.iter().any(|f| f.contains("carries content")),
        "a <link> carries nothing: {:?}",
        outcome.findings
    );
}

/// Neither element is reported as "carrying content" any more — a `<link>`
/// never did, and an `<a>` is now repaired rather than handed back. What has to
/// stay true is that the `<a>`'s content survives the repair.
#[test]
fn a_dead_hyperlink_is_repaired_rather_than_handed_back() {
    let (outcome, after) = fix(&book2(r#"<p><a href="gone.xhtml">chapter</a></p>"#));
    assert!(
        !outcome.findings.iter().any(|f| f.contains("carries content")),
        "got {:?}",
        outcome.findings
    );
    assert!(ch1(&after).contains(">chapter</a>"), "got {}", ch1(&after));
}


/// A short run of verse gets the block container XHTML 1.1 wants, rather than
/// being handed back to be wrapped by hand.
#[test]
fn a_short_run_of_inline_content_in_a_blockquote_is_wrapped() {
    let (outcome, after) = fix(&book2(
        "<blockquote>bare verse<br/>second line</blockquote>",
    ));
    let got = ch1(&after);

    assert!(
        got.contains("<blockquote><div>bare verse<br/>second line</div></blockquote>"),
        "got {got}"
    );
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("run(s) of inline content")),
        "got {:?}",
        outcome.changes
    );
}

/// Only the runs that need it: a quotation already holding a block keeps its
/// shape, and a loose run beside it is wrapped on its own.
#[test]
fn each_run_is_wrapped_separately_and_blocks_are_left_where_they_are() {
    let (_, after) = fix(&book2(
        "<blockquote><p>proper</p>loose verse<p>also proper</p></blockquote>",
    ));
    let got = ch1(&after);
    assert!(got.contains("<p>proper</p><div>loose verse</div><p>also proper</p>"), "got {got}");
}

/// A blockquote that was already correct must come out untouched.
#[test]
fn a_blockquote_that_is_already_block_only_is_not_wrapped() {
    let raw = doc("<blockquote><div>bare verse</div></blockquote>");
    let (outcome, after) = fix(&book_raw(&raw));
    assert!(outcome.changes.is_empty(), "got {:?}", outcome.changes);
    assert_eq!(ch1(&after), raw);
}

/// Above the threshold the book has been retagged and declares EPUB 3, so the
/// wrapping must not also fire and rewrite a third of the markup.
#[test]
fn a_book_past_the_threshold_is_retagged_rather_than_wrapped() {
    let verse = "<blockquote>verse<br/>more</blockquote>".repeat(12);
    let (outcome, after) = fix(&book2(&verse));

    assert!(
        opf_of(&after).contains(r#"version="3.0""#),
        "got {}",
        opf_of(&after)
    );
    assert!(
        !outcome
            .changes
            .iter()
            .any(|c| c.contains("run(s) of inline content")),
        "the declaration moved instead of the markup: {:?}",
        outcome.changes
    );
}

/// `<a name="x" id="x">` is one identity, and the closing scan must count it
/// that way too.
///
/// Under EPUB 3 the `name` is legal and stays, so this pattern survives a run —
/// and counting the two attributes separately reported 22 duplicate ids in a
/// book epubcheck calls clean. A report that sends someone looking for a defect
/// that is not there is worse than no report.
#[test]
fn a_matching_name_and_id_are_not_reported_as_a_duplicate() {
    let before = common::epub3(
        r#"<p><a name="TOC_id754016" id="TOC_id754016">anchor</a></p>"#,
        "<p>y</p>",
    );
    let (outcome, _) = roundtrip_full(&before);
    assert!(
        !outcome.remaining.iter().any(|r| r.contains("duplicate id")),
        "got {:?}",
        outcome.remaining
    );
}

/// A body's text colour does have a single-property equivalent, so it is
/// rehoused rather than reported as unconvertible.
#[test]
fn a_body_text_colour_is_moved_into_css() {
    let before = common::epub3_with_css("<p>text</p>", ".c { color: red }\n");
    let patched: Vec<(String, Vec<u8>)> = read_epub(&before)
        .into_iter()
        .map(|(n, d)| {
            if n == "OEBPS/ch1.xhtml" {
                let t = String::from_utf8(d)
                    .unwrap()
                    .replace("<body>", r##"<body text="#123456">"##);
                (n, t.into_bytes())
            } else {
                (n, d)
            }
        })
        .collect();
    let rebuilt: Vec<(&str, &[u8])> = patched
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let (outcome, after) = roundtrip_full(&make_epub(&rebuilt));

    let got = entry(&after, "OEBPS/ch1.xhtml");
    assert!(got.contains("color: #123456"), "got {got}");
    assert!(!got.contains("text="), "got {got}");
    assert!(
        !outcome
            .findings
            .iter()
            .any(|f| f.contains("no single-property") && f.contains("text")),
        "got {:?}",
        outcome.findings
    );
}

// ---------------------------------------------------------------------------
// unique-identifier
// ---------------------------------------------------------------------------

/// The OPF of `opf()` with its `<dc:identifier>` line replaced.
fn opf_with_identifier(identifiers: &str) -> String {
    opf("2.0", "", "", "").replace(
        r#"    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>"#,
        identifiers,
    )
}

fn book_with_identifier(identifiers: &str) -> Vec<u8> {
    let opf = opf_with_identifier(identifiers);
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
    ])
}

/// OPF-030: the package names an id its only identifier does not carry, and the
/// identifier has no id at all. Give it the one being asked for.
#[test]
fn an_identifier_with_no_id_is_given_the_one_the_package_names() {
    let (outcome, after) = fix(&book_with_identifier(
        r#"    <dc:identifier opf:scheme="calibre" xmlns:opf="http://www.idpf.org/2007/opf">urn:uuid:9-9</dc:identifier>"#,
    ));

    let got = entry(&after, "OEBPS/content.opf");
    assert!(got.contains(r#"<dc:identifier id="BookId""#), "got {got}");
    assert!(got.contains(r#"unique-identifier="BookId""#), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("BookId")),
        "got {:?}",
        outcome.changes
    );
    // And with the pointer resolving, the NCX can finally be synced to it.
    assert!(
        entry(&after, "OEBPS/toc.ncx").contains("urn:uuid:9-9"),
        "got {}",
        entry(&after, "OEBPS/toc.ncx")
    );
}

/// The same defect, but the identifier already has an id of its own. Move the
/// pointer instead — renaming the id would break any `refines="#id"` aimed at it.
#[test]
fn an_identifier_that_already_has_an_id_keeps_it_and_the_package_moves() {
    let (outcome, after) = fix(&book_with_identifier(
        r#"    <dc:identifier id="pub-id">urn:uuid:9-9</dc:identifier>"#,
    ));

    let got = entry(&after, "OEBPS/content.opf");
    assert!(got.contains(r#"unique-identifier="pub-id""#), "got {got}");
    assert!(got.contains(r#"<dc:identifier id="pub-id""#), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("pub-id")),
        "got {:?}",
        outcome.changes
    );
}

/// With several identifiers and no pointer into any of them, which one is the
/// book's identity is a decision with consequences. Report it.
#[test]
fn several_identifiers_and_a_dangling_pointer_are_reported_not_guessed() {
    let (outcome, after) = fix(&book_with_identifier(
        "    <dc:identifier>urn:uuid:9-9</dc:identifier>\n\
         \x20   <dc:identifier opf:scheme=\"ISBN\" xmlns:opf=\"http://www.idpf.org/2007/opf\">1234567890</dc:identifier>",
    ));

    assert!(
        outcome.findings.iter().any(|f| f.contains("identity")),
        "got {:?}",
        outcome.findings
    );
    assert!(
        !entry(&after, "OEBPS/content.opf").contains("id=\"BookId\""),
        "nothing should have been assigned"
    );
}

/// A pointer that already resolves is not this fixer's business.
#[test]
fn a_unique_identifier_that_resolves_is_left_alone() {
    let (outcome, _) = fix(&book_with_identifier(
        r#"    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>"#,
    ));
    assert!(
        !outcome
            .changes
            .iter()
            .any(|c| c.contains("unique-identifier") || c.contains("<dc:identifier>")),
        "got {:?}",
        outcome.changes
    );
}

// ---------------------------------------------------------------------------
// content-type-meta
// ---------------------------------------------------------------------------

const BAD_CT: &str =
    r#"<meta content="http://www.w3.org/1999/xhtml; charset=utf-8" http-equiv="Content-Type"/>"#;

/// RSC-005: HTML5 fixes the value of the encoding-declaration `<meta>`, and a
/// Doubleday *Robot Dreams* puts a namespace URI where the media type goes.
#[test]
fn a_wrong_encoding_declaration_is_corrected_under_epub3() {
    let opf = opf("3.0", "", "", "");
    let body = doc("<p>text</p>").replace("<head>", &format!("<head>{BAD_CT}"));
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", body.as_bytes()),
    ]));

    let got = ch1(&after);
    assert!(
        got.contains(r#"content="text/html; charset=utf-8""#),
        "got {got}"
    );
    assert!(!got.contains("1999/xhtml; charset"), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("encoding")),
        "got {:?}",
        outcome.changes
    );
}

/// XHTML 1.1 does not check the value — measured, the same markup is clean as
/// EPUB 2 — so a book that stays EPUB 2 is not touched.
#[test]
fn a_wrong_encoding_declaration_is_left_alone_under_epub2() {
    let body = doc("<p>text</p>").replace("<head>", &format!("<head>{BAD_CT}"));
    let (_, after) = roundtrip_kept(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", body.as_bytes()),
    ]));
    assert!(ch1(&after).contains("1999/xhtml; charset"), "got {}", ch1(&after));
}

/// `charset=` is a claim about the bytes of the file. Relabelling one that says
/// something else would turn a wrong declaration into a wrong document.
#[test]
fn an_encoding_declaration_naming_another_charset_is_reported() {
    let meta = r#"<meta http-equiv="Content-Type" content="text/html; charset=iso-8859-1"/>"#;
    let body = doc("<p>text</p>").replace("<head>", &format!("<head>{meta}"));
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("3.0", "", "", "").as_bytes()),
        ("OEBPS/ch1.xhtml", body.as_bytes()),
    ]));

    assert!(
        outcome.findings.iter().any(|f| f.contains("iso-8859-1")),
        "got {:?}",
        outcome.findings
    );
    assert!(ch1(&after).contains("iso-8859-1"), "got {}", ch1(&after));
}

// ---------------------------------------------------------------------------
// broken-fragments: the same-document case
// ---------------------------------------------------------------------------

/// RSC-012 where the link is same-document and the anchor is nowhere in the
/// book. There is no document to fall back on, so the href goes and the element,
/// its text and its class stay.
#[test]
fn a_same_document_link_to_nothing_loses_its_href_and_keeps_its_text() {
    let (outcome, after) = fix(&book2(
        r##"<p><a href="#nowhere" class="c">Isaac Asimov</a></p>"##,
    ));

    let got = ch1(&after);
    assert!(!got.contains("#nowhere"), "got {got}");
    assert!(got.contains(r#"<a class="c">Isaac Asimov</a>"#), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("same-document")),
        "got {:?}",
        outcome.changes
    );
}

/// The same link, but the anchor exists: nothing to do.
#[test]
fn a_same_document_link_that_resolves_keeps_its_href() {
    let (outcome, after) = fix(&book2(
        r##"<p id="here"><a href="#here" class="c">x</a></p>"##,
    ));
    assert!(ch1(&after).contains(r##"href="#here""##), "got {}", ch1(&after));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("same-document")),
        "got {:?}",
        outcome.changes
    );
}

// ---------------------------------------------------------------------------
// package-references
// ---------------------------------------------------------------------------

/// The OPF of a book whose chapter really is at the archive root, with the
/// manifest and guide spelling the path however the caller says.
fn root_chapter_book(manifest_href: &str, guide_href: &str) -> Vec<u8> {
    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="{manifest_href}" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
  <guide><reference href="{guide_href}" title="Text" type="text"/></guide>
</package>"#
    );
    let ncx = NCX.replace(r#"src="ch1.xhtml""#, r#"src="../ch1.xhtml""#);
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("ch1.xhtml", doc("<p>text</p>").as_bytes()),
    ])
}

/// OPF-031 and RSC-007 against the package document. The file is in the book;
/// the guide is simply missing the `../` the manifest has.
#[test]
fn a_guide_href_missing_a_directory_step_is_repointed_at_the_real_file() {
    let (outcome, after) = fix(&root_chapter_book("../ch1.xhtml", "ch1.xhtml"));

    let opf = entry(&after, "OEBPS/content.opf");
    assert!(opf.contains(r#"<reference href="../ch1.xhtml""#), "got {opf}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("repointed")),
        "got {:?}",
        outcome.changes
    );
    // And the document itself is untouched: this is a path repair, not a purge.
    assert!(has(&after, "ch1.xhtml"), "got {:?}", names(&after));
    assert!(opf.contains(r#"<itemref idref="ch1"/>"#), "got {opf}");
}

/// The regression that matters most here. Reading the epubcheck log alone, a
/// manifest entry that is "not declared" *and* "could not be found" reads as a
/// phantom to sweep away — and sweeping it away measures as a clean book with
/// the chapter gone. Anything in the spine is therefore reported, never removed.
#[test]
fn a_spine_document_that_cannot_be_found_is_reported_and_never_removed() {
    let (outcome, after) = fix(&root_chapter_book("nowhere/gone.xhtml", "../ch1.xhtml"));

    let opf = entry(&after, "OEBPS/content.opf");
    assert!(opf.contains(r#"<itemref idref="ch1"/>"#), "got {opf}");
    assert!(opf.contains(r#"id="ch1""#), "the manifest entry stays: {opf}");
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("reading order") && f.contains("gone.xhtml")),
        "got {:?}",
        outcome.findings
    );
}

/// A manifest item that is *not* in the spine carries no reading order with it,
/// so a dead one can go. A Kobo build of *BAKEMONOGATARI* lists a `kobo.js`
/// that is not in the archive.
#[test]
fn a_dead_manifest_item_outside_the_spine_is_removed() {
    let opf = opf("2.0", r#"    <item id="js" href="js/kobo.js" media-type="application/javascript"/>"#, "", "");
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
    ]));

    assert!(!entry(&after, "OEBPS/content.opf").contains("kobo.js"), "got {}", entry(&after, "OEBPS/content.opf"));
    assert!(
        outcome.changes.iter().any(|c| c.contains("not in the book")),
        "got {:?}",
        outcome.changes
    );
}

/// `<!ELEMENT guide (reference+)>` — emptying it is an error of its own, so the
/// element goes with its last entry.
#[test]
fn a_guide_whose_every_entry_goes_is_removed_with_them() {
    let guide = "  <guide><reference href=\"cover.jpg\" type=\"cover\"/></guide>\n";
    let opf = opf(
        "2.0",
        r#"    <item id="cov" href="cover.jpg" media-type="image/jpeg"/>"#,
        "",
        guide,
    );
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc(r#"<p><img src="cover.jpg" alt="c"/></p>"#).as_bytes()),
        ("OEBPS/cover.jpg", b"\xFF\xD8\xFF\xE0"),
    ]));

    let got = entry(&after, "OEBPS/content.opf");
    assert!(!got.contains("<guide"), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("<guide>")),
        "got {:?}",
        outcome.changes
    );
}

// ---------------------------------------------------------------------------
// undeclared-resources
// ---------------------------------------------------------------------------

/// RSC-008: the archive holds a file the book uses and the manifest never
/// mentions. In *Skylark* it is a font, and it only became visible once
/// `css-paths` corrected the `..Fonts/` typo that was hiding it.
#[test]
fn a_font_the_stylesheet_uses_is_added_to_the_manifest() {
    let css = "@font-face { font-family: G; src: url(../Fonts/G.otf); }";
    let opf = opf(
        "2.0",
        r#"    <item id="css" href="Styles/s.css" media-type="text/css"/>"#,
        "",
        "",
    );
    let ch1 = doc("<p>text</p>").replace(
        "</head>",
        r#"<link rel="stylesheet" type="text/css" href="Styles/s.css"/></head>"#,
    );
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/Styles/s.css", css.as_bytes()),
        ("OEBPS/Fonts/G.otf", b"\x00\x01\x00\x00"),
    ]));

    let got = entry(&after, "OEBPS/content.opf");
    assert!(got.contains(r#"href="Fonts/G.otf""#), "got {got}");
    assert!(
        got.contains(r#"media-type="application/vnd.ms-opentype""#),
        "got {got}"
    );
    assert!(
        outcome.changes.iter().any(|c| c.contains("G.otf")),
        "got {:?}",
        outcome.changes
    );
}

/// A file nothing points at is not an error, and declaring it would be
/// inventing work.
#[test]
fn an_unreferenced_stray_file_is_left_undeclared() {
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
        ("OEBPS/leftover.png", b"\x89PNG"),
    ]));

    assert!(
        !entry(&after, "OEBPS/content.opf").contains("leftover"),
        "got {}",
        entry(&after, "OEBPS/content.opf")
    );
    assert!(
        !outcome.changes.iter().any(|c| c.contains("leftover")),
        "got {:?}",
        outcome.changes
    );
}

// ---------------------------------------------------------------------------
// truncated-documents
// ---------------------------------------------------------------------------

/// FATAL RSC-016: a document that simply stops, with everything still open.
#[test]
fn a_document_that_stops_mid_air_has_its_elements_closed() {
    let truncated = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
         <head><title>T</title></head>\n<body>\n<div>\n<p>the last words</p>";
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", truncated.as_bytes()),
    ]));

    let got = ch1(&after);
    assert!(epubfix::markup::well_formed(&got).is_ok(), "got {got}");
    assert!(got.contains("the last words"), "nothing is lost: {got}");
    assert!(got.trim_end().ends_with("</html>"), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("left open")),
        "got {:?}",
        outcome.changes
    );
}

/// A bare `<br>` is a void element written without its slash, not a document
/// that stops early — closing it would swallow everything after it.
#[test]
fn an_unclosed_void_element_is_reported_rather_than_closed() {
    let bad = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
         <head><title>T</title></head>\n<body>\n<div><p>one<br>two</p></div>\n</body></html>";
    let (outcome, after) = roundtrip_full(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", bad.as_bytes()),
    ]));

    assert!(
        outcome.findings.iter().any(|f| f.contains("void element")),
        "got {:?}",
        outcome.findings
    );
    assert!(ch1(&after).contains("one<br>two"), "got {}", ch1(&after));
}

/// A well-formed document is never touched, however deeply nested.
#[test]
fn a_complete_document_is_not_given_extra_closing_tags() {
    let (outcome, after) = fix(&book2("<div><p>one</p><p>two</p></div>"));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("left open")),
        "got {:?}",
        outcome.changes
    );
    assert!(!ch1(&after).contains("</html>\n</html>"), "got {}", ch1(&after));
}

// ---------------------------------------------------------------------------
// ncx-entry-ids
// ---------------------------------------------------------------------------

/// RSC-005: the NCX schema requires an `id` on every navigation entry, and a
/// Kobo build of *BAKEMONOGATARI* writes none at all.
#[test]
fn nav_points_without_an_id_are_given_one_and_the_rest_keep_theirs() {
    let ncx = NCX.replace(
        r#"<navPoint id="n1" playOrder="1">"#,
        r#"<navPoint playOrder="1">"#,
    );
    let ncx = ncx.replace(
        "</navMap>",
        r#"<navPoint id="n1" playOrder="2"><navLabel><text>B</text></navLabel>
    <content src="ch1.xhtml"/></navPoint></navMap>"#,
    );
    let (outcome, after) = fix(&make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf("2.0", "", "", "").as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("<p>text</p>").as_bytes()),
    ]));

    let got = entry(&after, "OEBPS/toc.ncx");
    assert_eq!(got.matches("<navPoint").count(), 2, "got {got}");
    assert_eq!(got.matches(" id=\"").count(), 2, "both have one now: {got}");
    // The one that already had an id keeps it, so nothing that linked to it moves.
    assert!(got.contains(r#"id="n1""#), "got {got}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("the schema requires")),
        "got {:?}",
        outcome.changes
    );
}
