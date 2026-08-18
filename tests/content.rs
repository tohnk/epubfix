//! The content-document fixers: legacy table attributes and stranded anchors.
//!
//! Every case is checked against the structural harness as well as its own
//! assertions, so a fix that silences epubcheck while breaking a link fails here.

mod common;

use common::verify::verify;
use common::{entry, epub2, epub3, read_epub, roundtrip_full};

/// Run a book through every fixer and assert nothing structural broke.
fn fix(before: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let (outcome, after) = roundtrip_full(before);
    verify(&read_epub(before), &after).assert_sound();
    (outcome, after)
}

fn ch1(after: &[(String, Vec<u8>)]) -> String {
    entry(after, "OEBPS/ch1.xhtml")
}

const TABLE: &str = r#"<table border="0" cellpadding="0" valign="top">
  <tr valign="top" align="left"><td valign="top" width="50%" nowrap="nowrap">cell</td></tr>
</table>"#;

// ---------------------------------------------------------------------------
// legacy-table-attrs
// ---------------------------------------------------------------------------

#[test]
fn epub3_strips_the_whole_legacy_set() {
    // Strip mode, for the same reason as the EPUB 2 case: this is about the
    // ruleset's removal list, and the default would rehouse some of these in
    // CSS whose property names contain the attribute names.
    let (outcome, after) = fix_stripping(&epub3(TABLE, "<p>x</p>"));
    let doc = ch1(&after);

    assert!(outcome.has_changes(), "expected changes");
    for attr in ["valign", "cellpadding", "nowrap", "align", "width"] {
        assert!(!doc.contains(attr), "{attr} should be gone from:\n{doc}");
    }
    // border="0" means "no border", which HTML5 expresses by omission.
    assert!(!doc.contains("border"), "{doc}");
    assert!(doc.contains("<table>"), "the tag itself survives: {doc}");
    assert!(doc.contains("epubfix generated presentation"), "{doc}");
    assert!(doc.contains("padding: 0px"), "{doc}");
    assert!(doc.contains("<td class=\"epubfix-presentation-"), "{doc}");
}

#[test]
fn epub3_clamps_a_non_zero_border_rather_than_dropping_it() {
    let (_, after) = fix(&epub3(
        r#"<table border="5"><tr><td>c</td></tr></table>"#,
        "<p>x</p>",
    ));
    assert!(
        ch1(&after).contains(r#"<table border="1">"#),
        "{}",
        ch1(&after)
    );
}

#[test]
fn epub2_keeps_what_xhtml11_allows_and_strips_what_it_does_not() {
    // The regression that matters most: getting this wrong silently degrades
    // every EPUB 2 book in a library sweep. The kept/stripped split was measured
    // against EPUB Check 5.2.1, not read off a spec.
    // Asked in strip mode: this test is about which attributes the *ruleset*
    // allows, and the default now also weighs whether each one is still doing
    // anything, which is a different question with its own tests below.
    let (outcome, after) = fix_stripping(&epub2(TABLE, "<p>x</p>"));
    let doc = ch1(&after);

    // Legal in XHTML 1.1 - must survive untouched.
    assert!(doc.contains(r#"<tr valign="top" align="left">"#), "{doc}");
    assert!(doc.contains(r#"cellpadding="0""#), "{doc}");
    assert!(
        doc.contains(r#"border="0""#),
        "any integer is fine in EPUB 2: {doc}"
    );
    assert!(doc.contains(r#"<td valign="top""#), "{doc}");

    // Not legal even in XHTML 1.1.
    assert!(!doc.contains(r#"<table border="0" cellpadding="0" valign="top">"#));
    assert!(
        !doc.contains(r#"width="50%""#),
        "cell width is Loose-only: {doc}"
    );
    assert!(!doc.contains("nowrap"), "{doc}");
    assert!(outcome.has_changes());
}

#[test]
fn an_epub2_book_with_only_legal_attributes_is_untouched() {
    let legal = r#"<table cellpadding="2" cellspacing="0" border="1" frame="void" rules="none" width="100%">
  <tr valign="top" align="left"><td valign="middle" align="right">cell</td></tr>
</table>"#;
    let before = epub2(legal, "<p>x</p>");
    let (outcome, after) = fix(&before);
    assert!(
        !outcome.has_changes(),
        "EPUB 2 legal markup must not be touched: {:?}",
        outcome.changes
    );
    assert_eq!(read_epub(&before), after, "bytes must be identical");
}

/// Gutenberg's converter leaks its own internals: `tag` holds the qualified
/// name of the element already carrying it, so it says nothing the tag does not.
/// That makes it debris rather than presentation — there is nothing to rehouse,
/// and deleting it is lossless. `xml:space` goes with it: measured in an XHTML
/// document, it changes neither the computed `white-space` nor the rendered
/// height, so no engine is honouring it.
#[test]
fn gutenberg_debris_attributes_are_deleted() {
    let body = r#"<p><a href="ch2.xhtml" tag="{http://www.w3.org/1999/xhtml}a">x</a></p>
<p xml:space="preserve">keep</p>"#;
    let (outcome, after) = fix(&epub2(body, "<p>y</p>"));
    let doc = ch1(&after);

    assert!(!doc.contains("tag="), "{doc}");
    assert!(!doc.contains("xml:space"), "{doc}");
    assert!(doc.contains(r#"<a href="ch2.xhtml">x</a>"#), "{doc}");
    assert!(doc.contains("<p>keep</p>"), "{doc}");
    assert!(
        !doc.contains("style="),
        "nothing was presentation, so nothing is rehoused: {doc}"
    );
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("could not have been doing anything") && c.contains("tagx1")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        outcome.findings.is_empty(),
        "nothing left for a person: {:?}",
        outcome.findings
    );
}

/// Only when it really does name its own element. A `tag` attribute saying
/// anything else is something the book meant, and is left alone.
#[test]
fn a_tag_attribute_that_names_something_else_is_kept() {
    let body = r#"<p><a href="ch2.xhtml" tag="chapter-opening">x</a></p>"#;
    let (outcome, after) = fix(&epub2(body, "<p>y</p>"));
    let doc = ch1(&after);

    assert!(doc.contains(r#"tag="chapter-opening""#), "{doc}");
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("tag") && f.contains("left in place")),
        "got {:?}",
        outcome.findings
    );
}

#[test]
fn a_meta_value_attribute_is_renamed_to_content_under_epub2() {
    // Perfume: <meta name="version" value="1.0"/> on every front-matter page.
    // XHTML 1.1 spells the same thing `content`; HTML5 brought `value` back,
    // so an EPUB 3 book is deliberately left alone.
    let body = r#"<meta name="version" value="1.0"/><p>x</p>"#;
    let (outcome, after) = fix(&epub2(body, "<p>y</p>"));
    let doc = ch1(&after);
    assert!(doc.contains(r#"content="1.0""#), "{doc}");
    assert!(!doc.contains("value="), "{doc}");
    assert!(outcome.has_changes(), "got {:?}", outcome.changes);

    let (outcome3, after3) = fix(&epub3(body, "<p>y</p>"));
    assert!(!outcome3.has_changes(), "got {:?}", outcome3.changes);
    assert!(ch1(&after3).contains("value=\"1.0\""), "{doc}");
}

/// The rename has to check that the name it is moving into is free. Writing a
/// second `content` onto a tag that already has one is not a lesser validation
/// error, it is a well-formedness violation: the document stops parsing, and a
/// book that merely failed epubcheck becomes one no reader will open.
#[test]
fn a_meta_carrying_both_value_and_content_is_left_alone() {
    let body = r#"<meta name="calibre:series" content="A" value="B"/><p>x</p>"#;
    let (outcome, after) = fix(&epub2(body, "<p>y</p>"));
    let doc = ch1(&after);

    assert!(
        doc.contains(r#"<meta name="calibre:series" content="A" value="B"/>"#),
        "the tag must come through untouched rather than gain a second content=: {doc}"
    );
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("both value= and content=")),
        "and it is reported: {:?}",
        outcome.findings
    );
}

#[test]
fn hr_size_and_br_clear_move_to_a_hint_layer_rule_under_epub2() {
    let body = r#"<hr size="0"/><br clear="all"/>rest"#;
    let (outcome, after) = fix(&epub2(body, "<p>y</p>"));
    let doc = ch1(&after);

    assert!(!doc.contains("size="), "{doc}");
    assert!(!doc.contains("clear="), "{doc}");
    assert!(doc.contains("@layer epubfix-hints {"), "{doc}");
    assert!(doc.contains("{ height: 0px; }"), "{doc}");
    assert!(doc.contains("{ clear: all; }"), "{doc}");
    assert!(
        !doc.contains("style="),
        "an inline style would outrank the author rules the attribute sat below: {doc}"
    );
    assert!(outcome.has_changes(), "got {:?}", outcome.changes);
}

/// `align` on a paragraph is rejected by both rulesets — measured, a Calibre
/// *Stranger* whose `<p class="Cite" align="center">` epubcheck flags as
/// EPUB 2 and EPUB 3 alike — and maps to `text-align`, so it joins `clear` in
/// the EPUB 3 removal list rather than surviving a retag as an error.
#[test]
fn epub3_converts_paragraph_align_to_a_hint_layer_rule() {
    let (outcome, after) = fix(&epub3(r#"<p align="center">cite</p>"#, "<p>y</p>"));
    let doc = ch1(&after);

    assert!(!doc.contains("align="), "{doc}");
    assert!(!doc.contains("style="), "{doc}");
    assert!(doc.contains("@layer epubfix-hints {"), "{doc}");
    assert!(doc.contains("{ text-align: center; }"), "{doc}");
    assert!(outcome.has_changes(), "got {:?}", outcome.changes);
}

/// A presentational attribute contributes below every author declaration, so
/// its replacement has to as well. A plain generated class does not — measured,
/// `.gen { border: 0 }` beats an author `img { border: 2px solid }` on
/// specificity, whatever the order — which is why the rule is layered.
#[test]
fn an_image_border_becomes_a_hint_layer_rule() {
    for before in [
        epub2(
            r#"<p><img alt="a" src="a.jpg" border="0"/></p>"#,
            "<p>y</p>",
        ),
        epub3(
            r#"<p><img alt="a" src="a.jpg" border="0"/></p>"#,
            "<p>y</p>",
        ),
    ] {
        let (outcome, after) = fix(&before);
        let doc = ch1(&after);
        assert!(!doc.contains("border="), "{doc}");
        assert!(!doc.contains("style="), "{doc}");
        assert!(doc.contains("@layer epubfix-hints {"), "{doc}");
        assert!(doc.contains("{ border: 0; }"), "{doc}");
        assert!(doc.contains(r#"<img class="epubfix-hint-1""#), "{doc}");
        assert!(outcome.has_changes(), "got {:?}", outcome.changes);
    }
}

/// One class per distinct declaration, not one per attribute occurrence: a book
/// with the same `border="0"` on three hundred images would otherwise carry
/// three hundred identical rules.
#[test]
fn identical_declarations_share_one_generated_class() {
    let body = r#"<p><img alt="a" src="a.jpg" border="0"/><img alt="b" src="b.jpg" border="3"/>
<img alt="c" src="c.jpg" border="0"/></p>"#;
    let (_, after) = fix(&epub2(body, "<p>y</p>"));
    let doc = ch1(&after);

    assert_eq!(doc.matches("{ border: 0; }").count(), 1, "{doc}");
    assert_eq!(
        doc.matches(r#"class="epubfix-hint-1""#).count(),
        2,
        "the two border=\"0\" images share it: {doc}"
    );
    assert_eq!(
        doc.matches(r#"class="epubfix-hint-2""#).count(),
        1,
        "and border=\"3\" gets its own: {doc}"
    );
}

/// In an `<ol>`, `value` numbers the list and must survive. In a `<ul>` under a
/// bullet marker it cannot show, so it goes.
#[test]
fn li_value_goes_from_a_bulleted_ul_and_stays_in_an_ol() {
    let body = r#"<ul><li value="1">a</li></ul><ol><li value="5">b</li></ol>"#;
    let (outcome, after) = fix(&epub3(body, "<p>y</p>"));
    let doc = ch1(&after);

    assert!(doc.contains("<ul><li>a</li></ul>"), "{doc}");
    assert!(doc.contains(r#"<li value="5""#), "the <ol> keeps it: {doc}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("could not have been doing anything") && c.contains("valuex1")),
        "got {:?}",
        outcome.changes
    );
}

/// But a `<ul>` can be restyled to a numbered marker, and then it *can* show —
/// measured: `list-style-type: decimal` renders `5.` `6.` with the attributes
/// and `1.` `2.` without. So a stylesheet that numbers the list is the signal to
/// leave them alone.
#[test]
fn li_value_survives_a_ul_the_stylesheet_numbers() {
    let body = r#"<ul class="num"><li value="5">a</li><li value="6">b</li></ul>"#;
    let (outcome, after) = fix(&common::epub3_with_css(
        body,
        ".num { list-style-type: decimal }\n",
    ));
    let doc = ch1(&after);

    assert!(doc.contains(r#"<li value="5""#), "{doc}");
    assert!(doc.contains(r#"<li value="6""#), "{doc}");
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("value") && f.contains("left in place")),
        "got {:?}",
        outcome.findings
    );
}

#[test]
fn a_style_on_the_html_element_moves_to_the_end_of_head_in_epub2() {
    // XHTML 1.1 has no style attribute on <html>, so the declaration moves to a
    // generated rule — and to the *end* of <head>, because an inline style
    // outranks every normal author declaration and the replacement has to keep
    // beating the book's own stylesheet. First in <head> is for presentational
    // hints, which the original attribute was not.
    let chapter = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">
<html xmlns="http://www.w3.org/1999/xhtml" style="font-size:1.250rem;">
<head><title>T</title></head><body><p>x</p></body></html>"#;
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:12345678-1234-4234-8234-123456789012</dc:identifier>
    <dc:title>T</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>T</text></docTitle>
  <navMap><navPoint id="n1" playOrder="1"><navLabel><text>x</text></navLabel>
    <content src="ch1.xhtml"/></navPoint></navMap>
</ncx>"#;
    let before = common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", chapter.as_bytes()),
    ]);
    let (outcome, after) = fix(&before);

    let doc = ch1(&after);
    assert!(
        !doc.contains("<html xmlns=\"http://www.w3.org/1999/xhtml\" style="),
        "{doc}"
    );
    assert!(doc.contains("<html xmlns="), "{doc}");
    assert!(doc.contains("html { font-size:1.250rem; }"), "{doc}");
    assert!(
        outcome.findings.is_empty(),
        "converted rather than deleted: {:?}",
        outcome.findings
    );

    // Placement is the whole point: after the book's own head content, so an
    // equal-specificity author rule loses to it the way it lost to the
    // attribute.
    let rule = doc.find("html { font-size:1.250rem; }").expect("the rule");
    let title = doc.find("<title>").expect("the book's own head content");
    let head_end = doc.find("</head>").expect("the end of head");
    assert!(
        title < rule && rule < head_end,
        "the rule must sit between the book's head content and </head>: {doc}"
    );
}

/// The two sources sit on opposite sides of the author stylesheet, so one
/// insertion point cannot serve both. A presentational hint has to keep losing
/// to the book's rules and an inline style has to keep beating them, which puts
/// one generated block at each end of `<head>`.
#[test]
fn a_hint_and_an_inline_style_are_generated_at_opposite_ends_of_head() {
    let chapter = r##"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" style="font-size: 32px">
<head><title>T</title><link rel="stylesheet" href="s.css" type="text/css"/></head>
<body link="#00ff00"><p><a href="ch1.xhtml">x</a></p></body></html>"##;
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:12345678-1234-4234-8234-123456789012</dc:identifier>
    <dc:title>T</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css" href="s.css" media-type="text/css"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>T</text></docTitle>
  <navMap><navPoint id="n1" playOrder="1"><navLabel><text>x</text></navLabel>
    <content src="ch1.xhtml"/></navPoint></navMap>
</ncx>"#;
    let (_, after) = fix(&common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", chapter.as_bytes()),
        (
            "OEBPS/s.css",
            b"html { font-size: 8px } a:link { color: red }",
        ),
    ]));
    let doc = ch1(&after);

    let hint = doc
        .find("a:link { color: #00ff00; }")
        .expect("the hint rule");
    let link = doc.find("<link rel=").expect("the book's stylesheet");
    let inline = doc
        .find("html { font-size: 32px }")
        .expect("the inline rule");
    assert!(
        hint < link && link < inline,
        "hint before the book's stylesheet, former inline style after it: {doc}"
    );
}

// ---------------------------------------------------------------------------
// misplaced-anchors
// ---------------------------------------------------------------------------

#[test]
fn an_empty_unreferenced_anchor_between_rows_is_deleted() {
    let body = "<table><tr><td>a</td></tr><a></a><tr><td>b</td></tr></table>";
    let (outcome, after) = fix(&epub2(body, "<p>x</p>"));
    let doc = ch1(&after);

    assert!(!doc.contains("<a></a>"), "{doc}");
    assert!(doc.contains("</tr><tr>"), "the rows close up: {doc}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("1 empty anchor(s) removed")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn a_referenced_anchor_id_moves_to_the_following_row() {
    // The Coleridge case: 11 of 12 stranded anchors looked exactly like this.
    let body = r#"<table><tr><td>a</td></tr><a id="t"></a><tr><td>b</td></tr></table>"#;
    let (outcome, after) = fix(&epub2(body, r#"<p><a href="ch1.xhtml#t">jump</a></p>"#));
    let doc = ch1(&after);

    assert!(!doc.contains("<a id=\"t\"></a>"), "anchor removed: {doc}");
    assert!(
        doc.contains(r#"<tr id="t"><td>b</td></tr>"#),
        "id rehomed: {doc}"
    );
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("1 anchor id(s) moved")),
        "got {:?}",
        outcome.changes
    );
    // verify() inside fix() already proved the inbound link still resolves.
}

#[test]
fn an_anchor_after_the_last_row_moves_its_id_to_the_previous_row() {
    // Relocating it after </table> would be valid in EPUB 3 but not in EPUB 2,
    // so the id goes backwards onto the last legal element instead.
    let body = r#"<table><tr><td>a</td></tr><a id="t"></a></table>"#;
    let (_, after) = fix(&epub2(body, r#"<p><a href="ch1.xhtml#t">jump</a></p>"#));
    let doc = ch1(&after);

    assert!(doc.contains(r#"<tr id="t"><td>a</td></tr>"#), "{doc}");
    assert!(!doc.contains("<a id=\"t\">"), "{doc}");
}

#[test]
fn a_row_that_already_has_an_id_is_never_clobbered() {
    let body =
        r#"<table><tr id="r1"><td>a</td></tr><a id="t"></a><tr id="r2"><td>b</td></tr></table>"#;
    let (_, after) = fix(&epub2(body, r#"<p><a href="ch1.xhtml#t">jump</a></p>"#));
    let doc = ch1(&after);

    assert!(
        doc.contains(r#"<tr id="r1">"#),
        "existing ids intact: {doc}"
    );
    assert!(
        doc.contains(r#"<tr id="r2">"#),
        "existing ids intact: {doc}"
    );
    // Both rows are taken, so the id lands on the table itself.
    assert!(doc.contains(r#"<table id="t">"#), "{doc}");
}

#[test]
fn a_legitimate_anchor_inside_a_cell_is_left_alone() {
    // The false-positive guard: this is valid markup and epubcheck agrees.
    let body = r#"<table><tr><td><a href="ch2.xhtml">link</a></td></tr></table>"#;
    let before = epub2(body, "<p id=\"z\">x</p>");
    let (outcome, after) = fix(&before);

    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    assert_eq!(read_epub(&before), after, "bytes must be identical");
}

#[test]
fn an_anchor_with_visible_text_is_relocated_in_epub3_and_reported_in_epub2() {
    let body = r#"<table><tr><td>a</td></tr><a id="t">visible</a><tr><td>b</td></tr></table>"#;
    let link = r#"<p><a href="ch1.xhtml#t">jump</a></p>"#;

    let (v3, after3) = fix(&epub3(body, link));
    let doc3 = ch1(&after3);
    assert!(
        doc3.contains(r#"<a id="t">visible</a>"#),
        "text preserved: {doc3}"
    );
    assert!(
        doc3.find("</table>").unwrap() < doc3.find(r#"<a id="t">visible"#).unwrap(),
        "the anchor now sits after the table: {doc3}"
    );
    assert!(
        v3.changes
            .iter()
            .any(|c| c.contains("moved out of the table")),
        "got {:?}",
        v3.changes
    );

    // Under XHTML 1.1 an <a> cannot be a child of <body>, so moving it would
    // trade one epubcheck error for another. Report instead of guessing.
    let before2 = epub2(body, link);
    let (v2, after2) = fix(&before2);
    assert!(
        !v2.has_changes(),
        "EPUB 2 must not move it: {:?}",
        v2.changes
    );
    assert!(
        v2.findings.iter().any(|f| f.contains("needs a look")),
        "expected a finding, got {:?}",
        v2.findings
    );
    assert_eq!(
        read_epub(&before2),
        after2,
        "a finding must not modify the book"
    );
}

#[test]
fn a_non_anchor_stranded_in_a_table_is_reported_not_guessed_at() {
    let body = "<table><tr><td>a</td></tr><p>stray paragraph</p></table>";
    let before = epub2(body, "<p>x</p>");
    let (outcome, after) = fix(&before);

    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("<p> is not valid inside <table>")),
        "got {:?}",
        outcome.findings
    );
    assert_eq!(read_epub(&before), after);
}

// ---------------------------------------------------------------------------
// The `name` attribute regression
// ---------------------------------------------------------------------------

#[test]
fn meta_name_is_never_treated_as_an_id() {
    // Every fixture chapter carries <meta name="calibre:cover" content="true"/>.
    // It has a colon, so the old fixer rewrote it to "calibre_cover" and quietly
    // broke Calibre's cover detection. epubcheck never complained, because there
    // was nothing wrong with it.
    let before = epub2("<p>nothing to fix</p>", "<p>x</p>");
    let (outcome, after) = fix(&before);

    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    assert!(ch1(&after).contains(r#"<meta name="calibre:cover" content="true"/>"#));
    assert_eq!(read_epub(&before), after);
}

#[test]
fn form_control_names_are_left_alone_but_anchor_names_are_fixed() {
    let body = r#"<p><input name="user:email"/></p><p><a name="1bad">anchor</a></p>"#;
    let (outcome, after) = fix(&epub2(body, "<p>x</p>"));
    let doc = ch1(&after);

    assert!(doc.contains(r#"<input name="user:email"/>"#), "{doc}");
    // The anchor's `name` is not merely mis-spelled, it is an attribute XHTML
    // 1.1 removed, so it becomes the `id` it stood for and is then sanitised.
    assert!(doc.contains(r#"<a id="id_1bad">"#), "{doc}");
    assert!(!doc.contains("<a name="), "{doc}");
    assert!(
        outcome.changes.iter().any(|c| c == "sanitised 1 id(s)"),
        "{:?}",
        outcome.changes
    );
}

#[test]
fn sanitising_an_id_never_collides_with_an_existing_one() {
    // "a:b" sanitises to "a_b", which is already taken in this document.
    let body = r#"<p id="a:b">one</p><p id="a_b">two</p>"#;
    let (_, after) = fix(&epub2(body, r#"<p><a href="ch1.xhtml#a:b">jump</a></p>"#));
    let doc = ch1(&after);

    assert!(doc.contains(r#"id="a_b_2""#), "{doc}");
    assert!(doc.contains(r#"id="a_b""#), "{doc}");
    // The link followed the renamed anchor, not the pre-existing one.
    assert!(entry(&after, "OEBPS/ch2.xhtml").contains("#a_b_2"));
}

#[test]
fn ids_in_different_documents_do_not_interfere() {
    let before = epub2(
        r#"<p id="x:1">one</p>"#,
        r#"<p id="x:1">two</p><p><a href="ch1.xhtml#x:1">to one</a></p>"#,
    );
    let (_, after) = fix(&before);
    // Both are renamed, and the cross-document link still resolves — checked by
    // verify() inside fix().
    assert!(ch1(&after).contains(r#"id="x_1""#));
    assert!(entry(&after, "OEBPS/ch2.xhtml").contains(r#"id="x_1""#));
    assert!(entry(&after, "OEBPS/ch2.xhtml").contains(r#"href="ch1.xhtml#x_1""#));
}

#[test]
fn everything_at_once_stays_idempotent() {
    let body = r#"<table border="0" valign="top">
  <tr valign="top"><td width="10">a</td></tr><a id="t"></a><tr><td>b</td></tr><a></a>
</table>
<p id="1bad">text</p>"#;
    let link = r#"<p><a href="ch1.xhtml#t">jump</a> <a href="ch1.xhtml#1bad">bad</a></p>"#;

    for before in [epub2(body, link), epub3(body, link)] {
        let (outcome, after) = fix(&before);
        assert!(outcome.has_changes());

        // Feeding the result back in must find nothing left to do.
        let repacked = common::make_epub(
            &after
                .iter()
                .map(|(n, d)| (n.as_str(), d.as_slice()))
                .collect::<Vec<_>>(),
        );
        let (second, _) = fix(&repacked);
        assert!(
            !second.has_changes(),
            "second pass should be a no-op: {:?}",
            second.changes
        );
    }
}

// ---------------------------------------------------------------------------
// --preserve-presentation, and the CSS-override report
// ---------------------------------------------------------------------------

/// Run with `--strip-presentation` for supported conversions. Unsupported
/// attributes remain report-only even in this mode. This was the default until
/// the Ars Arcanum table of *The Hero of Ages* showed 75 of 75 cell widths
/// holding up a layout no stylesheet mentioned.
fn fix_stripping(before: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let opts = epubfix::Options {
        presentation: epubfix::Presentation::Strip,
        ..epubfix::Options::default()
    };
    common::roundtrip_with(before, &opts)
}

fn fix_preserving(before: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let opts = epubfix::Options {
        presentation: epubfix::Presentation::Preserve,
        ..epubfix::Options::default()
    };
    common::roundtrip_with(before, &opts)
}

#[test]
fn preserve_presentation_converts_attributes_to_inline_style() {
    let (outcome, after) = fix_preserving(&epub3(TABLE, "<p>x</p>"));
    let doc = ch1(&after);

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("to inline style")),
        "got {:?}",
        outcome.changes
    );
    assert!(!doc.contains("valign="), "the attribute still goes: {doc}");
    assert!(doc.contains("vertical-align: top"), "{doc}");
    assert!(doc.contains("text-align: left"), "{doc}");
    assert!(
        doc.contains("width: 50%"),
        "a percentage passes through: {doc}"
    );
    assert!(doc.contains("white-space: nowrap"), "{doc}");
}

#[test]
fn stylesheet_presentation_converts_table_cell_padding() {
    let (outcome, after) = fix_preserving(&epub3(TABLE, "<p>x</p>"));
    let doc = ch1(&after);
    assert!(!doc.contains("cellpadding"), "{doc}");
    assert!(doc.contains("epubfix generated presentation"), "{doc}");
    assert!(doc.contains("padding: 0px"), "{doc}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("generated CSS")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn preserve_presentation_appends_to_an_existing_style_attribute() {
    let body = r#"<table><tr><td style="color: red" valign="top">cell</td></tr></table>"#;
    let (_, after) = fix_preserving(&epub3(body, "<p>x</p>"));
    let doc = ch1(&after);
    assert!(
        doc.contains(r#"style="color: red; vertical-align: top""#),
        "{doc}"
    );
}

#[test]
fn preserve_presentation_floats_an_aligned_image_rather_than_centring_its_text() {
    // `align` means different things on different elements, and getting that
    // wrong would move the picture rather than leave it be.
    let body = r#"<p><img src="x.jpg" alt="x" align="left" hspace="8"/></p>"#;
    let (_, after) = fix_preserving(&epub3(body, "<p>y</p>"));
    let doc = ch1(&after);
    assert!(doc.contains("float: left"), "{doc}");
    assert!(doc.contains("margin-left: 8px; margin-right: 8px"), "{doc}");
    assert!(!doc.contains("text-align"), "{doc}");
}

#[test]
fn stripping_reports_only_the_attributes_no_stylesheet_was_overriding() {
    // The Calibre case: a generated stylesheet already sets vertical-align on
    // the rows, so those attributes had been doing nothing for years and
    // removing them cannot change the page. `nowrap` has no such rule.
    let body = r#"<table class="calibre1">
  <tr class="calibre7" valign="top"><td class="calibre8" nowrap="nowrap">cell</td></tr>
</table>"#;
    let css = ".calibre7 { vertical-align: middle }\n.calibre8 { vertical-align: inherit }\n";
    let before = common::epub3_with_css(body, css);
    let (outcome, _) = fix_stripping(&before);

    let finding = outcome
        .findings
        .iter()
        .find(|f| f.contains("not already overridden"))
        .unwrap_or_else(|| panic!("expected a report, got {:?}", outcome.findings));
    assert!(finding.contains("nowrapx1"), "{finding}");
    assert!(
        !finding.contains("valign"),
        "valign was overridden: {finding}"
    );
}

#[test]
fn a_book_whose_stylesheet_covers_everything_draws_no_report() {
    let body =
        r#"<table class="t"><tr class="r" valign="top"><td class="c">cell</td></tr></table>"#;
    let css = ".r { vertical-align: middle }\n";
    let (outcome, _) = fix(&common::epub3_with_css(body, css));
    assert!(
        !outcome
            .findings
            .iter()
            .any(|f| f.contains("not already overridden")),
        "got {:?}",
        outcome.findings
    );
}

// ---------------------------------------------------------------------------
// The default: keep whatever the attribute was actually doing
// ---------------------------------------------------------------------------

/// An attribute no stylesheet was overriding is the only thing holding that
/// layout up, so it moves into CSS rather than being dropped.
///
/// *The Hero of Ages* is why: its Ars Arcanum table sets `width` on 75 cells,
/// the stylesheet declares no width at all, and stripping all 75 reflows a
/// reference table people actually consult.
#[test]
fn an_attribute_nothing_overrides_is_rehoused_in_css() {
    let body = r#"<table class="t"><tr class="r"><td class="c" width="191">cell</td></tr></table>"#;
    let (outcome, after) = fix(&common::epub3_with_css(body, ".c { color: black }\n"));
    let doc = ch1(&after);

    assert!(doc.contains("width: 191px"), "{doc}");
    assert!(!doc.contains(r#"width="191""#), "{doc}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("into generated CSS")),
        "got {:?}",
        outcome.changes
    );
}

/// One a stylesheet *was* overriding has been inert for as long as the book has
/// existed, so it is simply removed — converting it would raise it above the
/// author rule and change the page in the other direction.
#[test]
fn an_attribute_the_stylesheet_overrides_is_stripped() {
    let body =
        r#"<table class="t"><tr class="r" valign="top"><td class="c">cell</td></tr></table>"#;
    let (outcome, after) = fix(&common::epub3_with_css(
        body,
        ".r { vertical-align: middle }\n",
    ));
    let doc = ch1(&after);

    assert!(!doc.contains("valign"), "{doc}");
    assert!(
        !doc.contains("vertical-align"),
        "it must not be rehoused: {doc}"
    );
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("already overrode")),
        "got {:?}",
        outcome.changes
    );
}

/// The two answers in one document, which is the ordinary case.
#[test]
fn a_mixed_table_gets_both_treatments_and_the_report_says_so() {
    let body = r#"<table class="t">
  <tr class="r" valign="top"><td class="c" width="191">cell</td></tr>
</table>"#;
    let (outcome, after) = fix(&common::epub3_with_css(
        body,
        ".r { vertical-align: middle }\n",
    ));
    let doc = ch1(&after);

    assert!(
        doc.contains("width: 191px"),
        "the width was load-bearing: {doc}"
    );
    assert!(!doc.contains("vertical-align"), "the valign was not: {doc}");
    let line = outcome
        .changes
        .iter()
        .find(|c| c.contains("legacy attribute"))
        .unwrap_or_else(|| panic!("got {:?}", outcome.changes));
    assert!(line.contains("moved 1"), "{line}");
    assert!(line.contains("stripped 1"), "{line}");
}

/// Nothing to warn about any more: the default no longer removes anything that
/// was doing something, so the old "may change how the page looks" finding has
/// nothing to report unless stripping was asked for explicitly.
#[test]
fn the_default_draws_no_rendering_warning() {
    let body = r#"<table class="t"><tr class="r"><td class="c" width="191">cell</td></tr></table>"#;
    let (outcome, _) = fix(&common::epub3_with_css(body, ".c { color: black }\n"));
    assert!(
        !outcome
            .findings
            .iter()
            .any(|f| f.contains("not already overridden")),
        "got {:?}",
        outcome.findings
    );
}

/// An attribute with no faithful CSS spelling still cannot be converted, and
/// saying so is the honest outcome rather than inventing a declaration.
#[test]
fn an_attribute_with_no_css_equivalent_is_still_reported() {
    let body = r#"<table class="t" frame="box"><tr class="r"><td class="c">cell</td></tr></table>"#;
    let (outcome, after) = fix(&common::epub3_with_css(body, ".c { color: black }\n"));
    assert!(ch1(&after).contains("frame=\"box\""), "{}", ch1(&after));
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("no conversion implemented") && f.contains("frame")),
        "got {:?}",
        outcome.findings
    );
}
