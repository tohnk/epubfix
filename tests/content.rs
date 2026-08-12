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
    let (outcome, after) = fix(&epub3(TABLE, "<p>x</p>"));
    let doc = ch1(&after);

    assert!(outcome.has_changes(), "expected changes");
    for attr in ["valign", "cellpadding", "nowrap", "align", "width"] {
        assert!(!doc.contains(attr), "{attr} should be gone from:\n{doc}");
    }
    // border="0" means "no border", which HTML5 expresses by omission.
    assert!(!doc.contains("border"), "{doc}");
    assert!(doc.contains("<table>"), "the tag itself survives: {doc}");
    assert!(doc.contains("<td>cell</td>"), "{doc}");
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
    let (outcome, after) = fix(&epub2(TABLE, "<p>x</p>"));
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

/// Run with `--preserve-presentation` instead of the default strip.
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
            .any(|c| c.contains("converted to inline style")),
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
fn preserve_presentation_reports_what_it_could_not_convert() {
    // cellpadding describes the cells, not the table, so there is no
    // single-property CSS equivalent to write on the element carrying it.
    let (outcome, _) = fix_preserving(&epub3(TABLE, "<p>x</p>"));
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("no single-property CSS equivalent") && f.contains("cellpadding")),
        "got {:?}",
        outcome.findings
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
    let (outcome, _) = fix(&before);

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
