//! Dangling resources, broken fragments, and the NCX repairs.
//!
//! Every case here came from a real 37-book library sweep, so the fixtures are
//! shaped like what actually turns up rather than what is easy to test.

mod common;

use common::verify::verify;
use common::{entry, has, make_epub, read_epub, roundtrip_full};

const CONTAINER: &str = common::CONTAINER;

fn opf2(manifest: &str, spine: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
{manifest}
  </manifest>
  <spine toc="ncx">{spine}</spine>
</package>"#
    )
}

fn doc(head: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
         <head><title>T</title>{head}</head>\n<body>\n{body}\n</body></html>"
    )
}

fn ncx(entries: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
{entries}
  </navMap>
</ncx>"#
    )
}

fn book(files: &[(&str, &str)]) -> Vec<u8> {
    let owned: Vec<(&str, &[u8])> = files.iter().map(|(n, t)| (*n, t.as_bytes())).collect();
    make_epub(&owned)
}

fn fix(before: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let (outcome, after) = roundtrip_full(before);
    verify(&read_epub(before), &after).assert_sound();
    (outcome, after)
}

// ---------------------------------------------------------------------------
// dangling-resources
// ---------------------------------------------------------------------------

#[test]
fn a_reference_to_a_moved_file_is_repointed() {
    // Calibre writes ../styles/stylesheet.css; the file is at Styles/. Note the
    // case difference, which is why the match is case-insensitive.
    let opf = opf2(
        r#"    <item id="ch1" href="Text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css" href="Styles/stylesheet.css" media-type="text/css"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let ch1 = doc(
        r#"<link rel="stylesheet" type="text/css" href="../styles/stylesheet.css"/>"#,
        "<p>text</p>",
    );
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx("")),
        ("OEBPS/Text/ch1.xhtml", &ch1),
        ("OEBPS/Styles/stylesheet.css", "body{}"),
    ]));

    assert!(
        entry(&after, "OEBPS/Text/ch1.xhtml").contains(r#"href="../Styles/stylesheet.css""#),
        "{}",
        entry(&after, "OEBPS/Text/ch1.xhtml")
    );
    assert!(
        outcome.changes.iter().any(|c| c.contains("repointed 1")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn a_dead_stylesheet_or_script_include_is_removed() {
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let ch1 = doc(
        r#"<link rel="stylesheet" type="text/css" href="../styles/page-template.xpgt"/><script src="js/kobo.js"></script>"#,
        "<p>text</p>",
    );
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx("")),
        ("OEBPS/ch1.xhtml", &ch1),
    ]));

    let fixed = entry(&after, "OEBPS/ch1.xhtml");
    assert!(!fixed.contains("page-template"), "{fixed}");
    assert!(!fixed.contains("kobo.js"), "{fixed}");
    assert!(fixed.contains("<p>text</p>"), "content untouched: {fixed}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("removed 2")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn a_missing_image_is_reported_and_never_deleted() {
    // The one defect that stayed manual across 37 books: genuine content loss
    // in the source. Deleting the element would hide it.
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let ch1 = doc("", r#"<p><img src="images/Art_logo.jpg" alt="logo"/></p>"#);
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx("")),
        ("OEBPS/ch1.xhtml", &ch1),
    ]));

    assert!(entry(&after, "OEBPS/ch1.xhtml").contains("Art_logo.jpg"));
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("Art_logo.jpg") && f.contains("left alone")),
        "got {:?}",
        outcome.findings
    );
}

#[test]
fn an_svg_xlink_href_is_repaired_too() {
    // Full-page illustrations are wrapped in SVG, and the attribute is
    // xlink:href rather than src.
    let opf = opf2(
        r#"    <item id="ch1" href="Text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="Images/plate.jpg" media-type="image/jpeg"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let ch1 = doc(
        "",
        r#"<svg xmlns="http://www.w3.org/2000/svg"><image xlink:href="images/plate.jpg"/></svg>"#,
    );
    let (_, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx("")),
        ("OEBPS/Text/ch1.xhtml", &ch1),
        ("OEBPS/Images/plate.jpg", "\u{ff}\u{d8}"),
    ]));

    assert!(
        entry(&after, "OEBPS/Text/ch1.xhtml").contains(r#"xlink:href="../Images/plate.jpg""#),
        "{}",
        entry(&after, "OEBPS/Text/ch1.xhtml")
    );
}

// ---------------------------------------------------------------------------
// broken-fragments
// ---------------------------------------------------------------------------

#[test]
fn a_mistyped_footnote_link_is_recovered_from_its_backlink() {
    // The case that rules out edit distance: "#1b" has to reach id="Oneb", and
    // only the reciprocal link says so.
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let ch1 = doc(
        "",
        r##"<p><a href="#1b" id="Onet">note</a></p>
<p><a href="#Onet" id="Oneb">back</a></p>"##,
    );
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx("")),
        ("OEBPS/ch1.xhtml", &ch1),
    ]));

    assert!(
        entry(&after, "OEBPS/ch1.xhtml").contains(r##"href="#Oneb""##),
        "{}",
        entry(&after, "OEBPS/ch1.xhtml")
    );
    assert!(
        outcome.changes.iter().any(|c| c.contains("backlinks")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn a_fragment_defined_in_exactly_one_other_document_is_repointed() {
    // A content split moved the anchor without updating the references.
    let opf = opf2(
        r#"    <item id="p" href="v1_preface.html" media-type="application/xhtml+xml"/>
    <item id="a" href="v1_ack.html" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="p"/><itemref idref="a"/>"#,
    );
    let entries = r#"    <navPoint id="n1" playOrder="1"><navLabel><text>Page xii</text></navLabel>
      <content src="v1_preface.html#page_xii"/></navPoint>"#;
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx(entries)),
        ("OEBPS/v1_preface.html", &doc("", "<p>preface</p>")),
        ("OEBPS/v1_ack.html", &doc("", r#"<p id="page_xii">ack</p>"#)),
    ]));

    assert!(
        entry(&after, "OEBPS/toc.ncx").contains(r#"src="v1_ack.html#page_xii""#),
        "{}",
        entry(&after, "OEBPS/toc.ncx")
    );
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("repointed 1 fragment")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn a_fragment_defined_nowhere_is_dropped_leaving_the_document_link() {
    // Calibre split the book *at* those anchors, so the id is gone everywhere.
    // Linking to the document beats linking to nothing.
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="split_004.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/><itemref idref="ch2"/>"#,
    );
    let ch1 = doc("", r#"<p><a href="split_004.xhtml#filepos7994">go</a></p>"#);
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx("")),
        ("OEBPS/ch1.xhtml", &ch1),
        ("OEBPS/split_004.xhtml", &doc("", "<p>chapter</p>")),
    ]));

    let fixed = entry(&after, "OEBPS/ch1.xhtml");
    assert!(fixed.contains(r#"href="split_004.xhtml""#), "{fixed}");
    assert!(!fixed.contains("filepos7994"), "{fixed}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("dropped 1 fragment")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn an_ambiguous_fragment_is_reported_rather_than_guessed_at() {
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>
    <item id="b" href="b.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let (outcome, _) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx("")),
        (
            "OEBPS/ch1.xhtml",
            &doc("", r#"<p><a href="a.xhtml#shared">go</a></p>"#),
        ),
        ("OEBPS/a.xhtml", &doc("", "<p>a</p>")),
        ("OEBPS/b.xhtml", &doc("", r#"<p id="shared">b</p>"#)),
    ]));

    // Defined in exactly one other document, so this one *is* recoverable.
    assert!(
        outcome.changes.iter().any(|c| c.contains("repointed")),
        "got {:?}",
        outcome.changes
    );
}

// ---------------------------------------------------------------------------
// NCX repairs
// ---------------------------------------------------------------------------

#[test]
fn a_nav_entry_for_a_missing_document_is_removed_with_its_children() {
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let entries = r#"    <navPoint id="n1" playOrder="1"><navLabel><text>Real</text></navLabel>
      <content src="ch1.xhtml"/></navPoint>
    <navPoint id="n2" playOrder="2"><navLabel><text>Jacket</text></navLabel>
      <content src="Text/jacket.xhtml"/>
      <navPoint id="n3" playOrder="3"><navLabel><text>Child</text></navLabel>
        <content src="Text/jacket.xhtml"/></navPoint>
    </navPoint>"#;
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx(entries)),
        ("OEBPS/ch1.xhtml", &doc("", "<p>text</p>")),
    ]));

    let fixed = entry(&after, "OEBPS/toc.ncx");
    assert!(fixed.contains("Real"), "the live entry stays: {fixed}");
    assert!(!fixed.contains("jacket.xhtml"), "{fixed}");
    assert!(!fixed.contains("Child"), "nested entry goes too: {fixed}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("removed 1 navigation")),
        "parent and child count as one removal: {:?}",
        outcome.changes
    );
}

#[test]
fn duplicated_ncx_ids_are_made_unique() {
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let entries = r#"    <navPoint id="chap-1" playOrder="1"><navLabel><text>A</text></navLabel>
      <content src="ch1.xhtml"/></navPoint>
    <navPoint id="chap-1" playOrder="2"><navLabel><text>B</text></navLabel>
      <content src="ch1.xhtml"/></navPoint>
    <navPoint id="chap-1" playOrder="3"><navLabel><text>C</text></navLabel>
      <content src="ch1.xhtml"/></navPoint>"#;
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx(entries)),
        ("OEBPS/ch1.xhtml", &doc("", "<p>text</p>")),
    ]));

    let fixed = entry(&after, "OEBPS/toc.ncx");
    assert!(
        fixed.contains(r#"id="chap-1""#),
        "the first one keeps it: {fixed}"
    );
    assert!(fixed.contains(r#"id="chap-1_2""#), "{fixed}");
    assert!(fixed.contains(r#"id="chap-1_3""#), "{fixed}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("made 2")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn a_pagelist_gets_both_required_attributes_or_none() {
    // Supplying only `class` pushed a real book onto the strict validation path
    // and introduced an error epubcheck had not been reporting.
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let toc = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap><navPoint id="n1" playOrder="1"><navLabel><text>A</text></navLabel>
    <content src="ch1.xhtml"/></navPoint></navMap>
  <pageList>
    <pageTarget id="pt1" type="normal" value="1" playOrder="2">
      <navLabel><text>1</text></navLabel><content src="ch1.xhtml"/>
    </pageTarget>
  </pageList>
</ncx>"#;
    let (outcome, after) = fix(&book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", toc),
        ("OEBPS/ch1.xhtml", &doc("", "<p>text</p>")),
    ]));

    let fixed = entry(&after, "OEBPS/toc.ncx");
    assert!(
        fixed.contains(r#"<pageList id="pagelist" class="pagelist">"#),
        "{fixed}"
    );
    assert!(
        outcome.changes.iter().any(|c| c.contains("<pageList>")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn a_clean_book_is_untouched_by_all_of_this() {
    let opf = opf2(
        r#"    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="ch1"/>"#,
    );
    let entries = r#"    <navPoint id="n1" playOrder="1"><navLabel><text>A</text></navLabel>
      <content src="ch1.xhtml"/></navPoint>"#;
    let before = book(&[
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/toc.ncx", &ncx(entries)),
        (
            "OEBPS/ch1.xhtml",
            &doc("", r##"<p id="a">text</p><p><a href="#a">self</a></p>"##),
        ),
    ]);
    let (outcome, after) = fix(&before);
    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    assert_eq!(read_epub(&before), after);
    assert!(has(&after, "OEBPS/ch1.xhtml"));
}
