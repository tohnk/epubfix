//! The error classes added after the first library sweep.
//!
//! Every expectation here was measured against EPUB Check 5.2.1 first — one
//! construct per book, in both rulesets — and the measurement is quoted in the
//! doc comment of the fixer it belongs to.

mod common;

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
