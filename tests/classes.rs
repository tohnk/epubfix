//! The error classes added after the first library sweep.
//!
//! Every expectation here was measured against EPUB Check 5.2.1 first — one
//! construct per book, in both rulesets — and the measurement is quoted in the
//! doc comment of the fixer it belongs to.

mod common;

use std::fmt::Write as _;

use common::verify::verify;
use common::{entry, has, make_epub, read_epub, roundtrip_full, roundtrip_kept};

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
fn a_referenced_duplicate_id_is_reported_rather_than_renamed() {
    let (outcome, after) = fix(&book2(
        r##"<p id="dup">a</p><p id="dup">b</p><p><a href="#dup">link</a></p>"##,
    ));

    assert!(
        outcome.changes.is_empty(),
        "nothing may move: {:?}",
        outcome.changes
    );
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("dup") && f.contains("links to it")),
        "got {:?}",
        outcome.findings
    );
    assert_eq!(ch1(&after).matches(r#"id="dup""#).count(), 2);
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

fn ch1_of(after: &[(String, Vec<u8>)]) -> String {
    entry(after, "OEBPS/ch1.xhtml")
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
            .any(|c| c.contains("removed 1 stylesheet/script")),
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
