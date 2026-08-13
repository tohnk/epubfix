//! EPUB 2 → EPUB 3 migration, the diagnostic that recommends it, and `img-alt`.

mod common;

use common::verify::verify;
use common::{
    entry, epub2, epub2_ncx, epub3, epub3_written_as_epub2, has, read_epub, roundtrip_full,
    roundtrip_migrated,
};

/// Migrate, then assert nothing structural was lost.
fn migrate(before: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let (outcome, after) = roundtrip_migrated(before);
    verify(&read_epub(before), &after).assert_sound();
    (outcome, after)
}

/// Verse in a blockquote: legal HTML5 flow content, illegal XHTML 1.1.
const VERSE: &str = "<blockquote>\nThe Frost performs its secret ministry,<br/>\n\
     Unhelped by any wind. <span>The owlet's cry</span><br/>\n\
     Came loud&mdash;and hark, again!\n</blockquote>";

const NESTED_NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="np1" playOrder="1">
      <navLabel><text>Part One</text></navLabel>
      <content src="ch1.xhtml"/>
      <navPoint id="np2" playOrder="2">
        <navLabel><text>Kubla Khan &amp; Others</text></navLabel>
        <content src="ch2.xhtml"/>
      </navPoint>
    </navPoint>
  </navMap>
  <pageList>
    <pageTarget id="pt1" type="normal" value="1" playOrder="3">
      <navLabel><text>1</text></navLabel><content src="ch1.xhtml"/>
    </pageTarget>
  </pageList>
</ncx>"#;

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

#[test]
fn keep_version_suppresses_retagging_entirely() {
    // A book whose verse plainly needs EPUB 3, held at EPUB 2 on request.
    let (outcome, after) = common::roundtrip_kept(&epub2(&VERSE.repeat(8), "<p>x</p>"));
    assert!(entry(&after, "OEBPS/content.opf").contains(r#"version="2.0""#));
    assert!(!has(&after, "OEBPS/nav.xhtml"));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("retagged")),
        "got {:?}",
        outcome.changes
    );
}

#[test]
fn a_few_stray_inline_runs_are_wrapped_rather_than_retagging_the_book() {
    // One or two of these mean a couple of paragraphs need a <div>, not that
    // the whole book is the wrong format. Changing a book's format identity on
    // that evidence would be wildly out of proportion — and so would handing it
    // back to be wrapped by hand, which is what this used to do.
    let (outcome, after) = roundtrip_full(&epub2(VERSE, "<p>x</p>"));
    let doc = entry(&after, "OEBPS/ch1.xhtml");

    assert!(
        entry(&after, "OEBPS/content.opf").contains(r#"version="2.0""#),
        "must not retag on weak evidence"
    );
    assert!(doc.contains("<blockquote>"), "the quotation stays: {doc}");
    assert!(doc.contains("<div>"), "and its verse gets a block container: {doc}");
    assert!(
        doc.contains("secret ministry") && doc.contains("owlet"),
        "every word survives: {doc}"
    );
    assert!(
        outcome.changes.iter().any(|c| c.contains("inline content")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        !outcome.findings.iter().any(|f| f.contains("too few to retag")),
        "there is nothing left to hand back: {:?}",
        outcome.findings
    );
}

#[test]
fn enough_of_them_and_the_declaration_is_what_moves() {
    // At scale the balance flips: repairing would mean hundreds of edits, so
    // the one wrong attribute is overwhelmingly the likelier error.
    let (outcome, after) = roundtrip_full(&epub2_ncx(&VERSE.repeat(8), "<p>x</p>", NESTED_NCX));

    assert!(entry(&after, "OEBPS/content.opf").contains(r#"version="3.0""#));
    assert!(has(&after, "OEBPS/nav.xhtml"));
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("retagged") && c.contains("requires EPUB 3")),
        "got {:?}",
        outcome.changes
    );
    // The verse itself is untouched - only the declaration moved.
    assert!(entry(&after, "OEBPS/ch1.xhtml").contains("<blockquote>"));
    assert!(!entry(&after, "OEBPS/ch1.xhtml").contains("<div>"));
}

/// A book whose only absolute URLs are hyperlinks needs no extra properties.
///
/// The regression: a radio-series book with a link to each programme's page had
/// `remote-resources` proposed for three of its manifest items, on a package
/// that validated clean. A hyperlink is somewhere the reader may go, not
/// something the document loads.
#[test]
fn hyperlinks_to_the_open_web_are_not_remote_resources() {
    let linky = r#"<p>See <a href="https://www.bbc.co.uk/programmes/b006qykl">the programme
    page</a> and <a href="http://example.org/notes">the notes</a>.</p>"#;
    let (_, after) = migrate(&epub2_ncx(&format!("{VERSE}{linky}"), linky, NESTED_NCX));
    let opf = entry(&after, "OEBPS/content.opf");

    assert!(!opf.contains("remote-resources"), "{opf}");
    // The nav document still gets its own property, so this is not just an
    // assertion that nothing was declared at all.
    assert!(opf.contains(r#"properties="nav""#), "{opf}");
    assert_eq!(opf.matches("properties=").count(), 1, "{opf}");
}

#[test]
fn migration_produces_everything_epub3_requires() {
    let (outcome, after) = migrate(&epub2_ncx(VERSE, "<p>x</p>", NESTED_NCX));
    let opf = entry(&after, "OEBPS/content.opf");
    let ch1 = entry(&after, "OEBPS/ch1.xhtml");

    assert!(opf.contains(r#"version="3.0""#), "{opf}");
    assert!(opf.contains("dcterms:modified"), "{opf}");
    assert!(opf.contains(r#"properties="nav""#), "{opf}");
    assert!(has(&after, "OEBPS/nav.xhtml"), "nav document created");

    // Found by testing, not from the spec: HTML5 needs its own DOCTYPE, and
    // declares no named entities at all.
    assert!(ch1.contains("<!DOCTYPE html>"), "{ch1}");
    assert!(!ch1.contains("XHTML 1.1"), "{ch1}");
    assert!(
        ch1.contains("&#8212;"),
        "&mdash; must become numeric: {ch1}"
    );
    assert!(!ch1.contains("&mdash;"), "{ch1}");

    assert!(outcome.has_changes());
}

#[test]
fn the_nav_document_mirrors_the_ncx() {
    let (_, after) = migrate(&epub2_ncx("<p id=\"a\">x</p>", "<p>y</p>", NESTED_NCX));
    let nav = entry(&after, "OEBPS/nav.xhtml");

    assert!(nav.contains(r#"epub:type="toc""#), "{nav}");
    assert!(
        nav.contains(r#"epub:type="page-list""#),
        "pageList kept: {nav}"
    );
    assert!(nav.contains(r#"<a href="ch1.xhtml">Part One</a>"#), "{nav}");
    // The child navPoint becomes a nested <ol>, not a sibling entry.
    let outer = nav.find("Part One").unwrap();
    let inner = nav.find("Kubla").unwrap();
    let nested_ol = nav[outer..inner].matches("<ol>").count();
    assert_eq!(nested_ol, 1, "child navPoint should nest: {nav}");
    // Markup in a label survives as markup, not as a literal ampersand.
    assert!(nav.contains("Kubla Khan &amp; Others"), "{nav}");
    // verify() already proved every nav href resolves.
}

#[test]
fn migration_is_idempotent_and_does_not_churn_the_timestamp() {
    let (first, after) = migrate(&epub2_ncx(VERSE, "<p>x</p>", NESTED_NCX));
    assert!(first.has_changes());

    let repacked = common::make_epub(
        &after
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect::<Vec<_>>(),
    );
    let (second, again) = migrate(&repacked);
    assert!(
        !second.has_changes(),
        "a migrated book must migrate to nothing: {:?}",
        second.changes
    );
    assert_eq!(after, again, "including the dcterms:modified timestamp");
}

#[test]
fn an_epub3_book_keeps_its_version_and_its_nav() {
    // The fixture already declares EPUB 3 and has a nav document, so migration
    // has nothing to do. Its verse does use `&mdash;`, which is a *fatal* error
    // in EPUB 3, so the conformance pass rewrites that and nothing else.
    let before = epub3(VERSE, "<p>x</p>");
    let (outcome, after) = migrate(&before);

    assert!(
        !outcome.changes.iter().any(|c| c.contains("3.0")),
        "version untouched: {:?}",
        outcome.changes
    );
    assert!(
        !outcome.changes.iter().any(|c| c.contains("generated")),
        "no second nav: {:?}",
        outcome.changes
    );
    assert_eq!(
        entry(&after, "OEBPS/nav.xhtml"),
        entry(&read_epub(&before), "OEBPS/nav.xhtml"),
        "the existing nav must be left exactly as it was"
    );
}

#[test]
fn a_genuinely_clean_epub3_book_comes_through_byte_identical() {
    let before = epub3("<p id=\"a\">plain prose</p>", "<p>x</p>");
    let (outcome, after) = migrate(&before);
    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    assert_eq!(read_epub(&before), after);
}

// ---------------------------------------------------------------------------
// The mirror case: declared EPUB 3, written as EPUB 2
// ---------------------------------------------------------------------------

#[test]
fn a_book_declaring_epub3_but_written_as_epub2_is_downgraded() {
    // Nothing here needs EPUB 3, and the book is EPUB 2 throughout. Moving the
    // declaration down is one attribute; conforming it upward would mean
    // rewriting every DOCTYPE, every entity, the metadata, and adding a file.
    let before = epub3_written_as_epub2("<p>Came loud&mdash;and hark, again!</p>");
    let (outcome, after) = roundtrip_full(&before);
    let opf = entry(&after, "OEBPS/content.opf");
    let ch1 = entry(&after, "OEBPS/ch1.xhtml");

    assert!(opf.contains(r#"version="2.0""#), "{opf}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("retagged") && c.contains("written as EPUB 2")),
        "got {:?}",
        outcome.changes
    );

    // The content is already right for EPUB 2, so none of it should move.
    assert!(ch1.contains("XHTML 1.1"), "DOCTYPE kept: {ch1}");
    assert!(ch1.contains("&mdash;"), "the entity is legal again: {ch1}");
    assert!(
        opf.contains("opf:role"),
        "legacy metadata is legal again: {opf}"
    );
    assert!(!has(&after, "OEBPS/nav.xhtml"), "no nav needed in EPUB 2");
}

#[test]
fn a_downgrade_strips_the_epub3_only_package_constructs() {
    // These would be errors under an EPUB 2 declaration, so retagging has to
    // take them with it.
    let before = epub3_written_as_epub2("<p>plain</p>");
    let (_, after) = roundtrip_full(&before);
    let opf = entry(&after, "OEBPS/content.opf");

    assert!(!opf.contains("dcterms:modified"), "{opf}");
    assert!(!opf.contains("properties="), "{opf}");
    assert!(
        opf.contains(r#"toc="ncx""#),
        "spine must point at the NCX: {opf}"
    );
}

#[test]
fn a_book_that_needs_epub3_is_never_downgraded_however_epub2_it_looks() {
    // The two-sided test. This book has every EPUB 2 marker there is, but its
    // verse only validates as HTML5, so downgrading would trade a handful of
    // errors for a great many. It stays EPUB 3 and is repaired forward.
    let before = epub3_written_as_epub2(&VERSE.repeat(8));
    let (outcome, after) = roundtrip_full(&before);
    let opf = entry(&after, "OEBPS/content.opf");

    assert!(opf.contains(r#"version="3.0""#), "{opf}");
    assert!(
        !outcome.changes.iter().any(|c| c.contains("2.0")),
        "no downgrade: {:?}",
        outcome.changes
    );
    assert!(has(&after, "OEBPS/nav.xhtml"), "conformed forward instead");
    assert!(entry(&after, "OEBPS/ch1.xhtml").contains("<!DOCTYPE html>"));
}

#[test]
fn conformance_repair_is_idempotent() {
    let before = epub3_written_as_epub2("<p>Came loud&mdash;and hark!</p>");
    let (first, after) = roundtrip_full(&before);
    assert!(first.has_changes());

    let repacked = common::make_epub(
        &after
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect::<Vec<_>>(),
    );
    let (second, again) = roundtrip_full(&repacked);
    assert!(
        !second.has_changes(),
        "second pass should be a no-op: {:?}",
        second.changes
    );
    assert_eq!(after, again);
}

#[test]
fn keep_version_never_moves_the_declaration() {
    // A book declaring EPUB 3 stays EPUB 3 even when everything about it looks
    // like EPUB 2. Downgrading would be lossy and, for markup relying on HTML5
    // content models, would create far more errors than it removed.
    let before = epub3_written_as_epub2("<p>plain prose</p>");
    let (_, after) = common::roundtrip_kept(&before);
    assert!(entry(&after, "OEBPS/content.opf").contains(r#"version="3.0""#));
}

#[test]
fn an_undeclared_entity_aborts_the_migration_and_changes_nothing() {
    // XHTML 1.1 declares the HTML 4 entity set and nothing else, so this
    // document was already broken. Migrating would turn a parse warning into a
    // fatal error, so the whole migration is abandoned.
    let before = epub2_ncx("<p>a &bogusentity; b</p>", "<p>x</p>", NESTED_NCX);
    let (outcome, after) = roundtrip_migrated(&before);

    assert!(
        outcome.findings.iter().any(|f| f.contains("bogusentity")),
        "expected a finding naming it, got {:?}",
        outcome.findings
    );
    assert!(!has(&after, "OEBPS/nav.xhtml"), "nothing should be created");
    assert!(
        entry(&after, "OEBPS/content.opf").contains(r#"version="2.0""#),
        "the book must stay as it was"
    );
}

#[test]
fn migration_unblocks_the_table_fixer() {
    // The point of running migration first: the version decides what the
    // content fixers do. Under EPUB 2 `valign` on a row is legal and stays;
    // once migrated it is an error and goes.
    let table = r#"<table><tr valign="top"><td valign="top">c</td></tr></table>"#;

    let (_, kept) = roundtrip_full(&epub2_ncx(table, "<p>x</p>", NESTED_NCX));
    assert!(
        entry(&kept, "OEBPS/ch1.xhtml").contains(r#"<tr valign="top">"#),
        "legal in EPUB 2, so untouched"
    );

    let (_, stripped) = migrate(&epub2_ncx(table, "<p>x</p>", NESTED_NCX));
    assert!(
        !entry(&stripped, "OEBPS/ch1.xhtml").contains("valign"),
        "an error once migrated, so stripped"
    );
}

// ---------------------------------------------------------------------------
// version-mismatch diagnostic
// ---------------------------------------------------------------------------

#[test]
fn an_epub2_book_with_html5_only_markup_keeps_its_declaration() {
    // The declaration is the thing that must not move on this evidence. What
    // *does* move is the markup, by the smallest edit that makes it legal where
    // it stands: a <div> around the run, and the book is still EPUB 2.
    let before = epub2(VERSE, "<p>x</p>");
    let (_, after) = roundtrip_full(&before);
    let doc = entry(&after, "OEBPS/ch1.xhtml");

    assert!(entry(&after, "OEBPS/content.opf").contains(r#"version="2.0""#));
    assert!(doc.contains("<blockquote>"), "{doc}");
    assert!(doc.contains("<div>"), "{doc}");
}

#[test]
fn the_same_markup_in_an_epub3_book_is_not_reported() {
    let (outcome, _) = roundtrip_full(&epub3(VERSE, "<p>x</p>"));
    assert!(
        !outcome
            .findings
            .iter()
            .any(|f| f.contains("only validates under EPUB 3")),
        "valid here, so nothing to say: {:?}",
        outcome.findings
    );
}

// ---------------------------------------------------------------------------
// img-alt
// ---------------------------------------------------------------------------

#[test]
fn decorative_images_get_empty_alt_and_content_images_are_reported() {
    let body = r#"<p><img src="orn.png"/></p>
<p><img src="rule-01.png"/></p>
<p><img src="portrait-of-coleridge.jpg"/></p>
<p><img src="plate.jpg" alt="A plate"/></p>"#;
    let (outcome, after) = roundtrip_full(&epub2(body, "<p>x</p>"));
    let doc = entry(&after, "OEBPS/ch1.xhtml");

    assert!(doc.contains(r#"<img alt="" src="orn.png"/>"#), "{doc}");
    assert!(doc.contains(r#"<img alt="" src="rule-01.png"/>"#), "{doc}");
    // A photograph gets no invented caption - writing alt="" would lie to a
    // screen reader to please the validator.
    assert!(
        doc.contains(r#"<img src="portrait-of-coleridge.jpg"/>"#),
        "{doc}"
    );
    assert!(
        doc.contains(r#"alt="A plate""#),
        "existing alt untouched: {doc}"
    );
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("added alt=\"\" to 2 decorative")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("1 image(s) have no alt text")),
        "got {:?}",
        outcome.findings
    );
}

/// An `<img>` written as a pair rather than self-closing. The `</img>` end node
/// carries no attributes, so without a kind filter it reads as an image with no
/// alt and no src -- a phantom "needs a caption" finding at best, and ` alt=""`
/// written into a closing tag at worst.
#[test]
fn a_paired_img_tag_is_counted_once_and_its_end_tag_left_alone() {
    let body = r#"<p><img src="orn.png"></img></p>"#;
    let (outcome, after) = roundtrip_full(&epub2(body, "<p>x</p>"));
    let doc = entry(&after, "OEBPS/ch1.xhtml");

    assert!(doc.contains(r#"<img alt="" src="orn.png">"#), "{doc}");
    assert!(doc.contains("</img>"), "the end tag must be untouched: {doc}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("added alt=\"\" to 1 decorative")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        outcome.findings.iter().all(|f| !f.contains("no alt text")),
        "got {:?}",
        outcome.findings
    );
}

#[test]
fn img_alt_does_nothing_in_epub3_where_it_is_not_an_error() {
    let body = r#"<p><img src="orn.png"/></p><p><img src="portrait.jpg"/></p>"#;
    let before = epub3(body, "<p>x</p>");
    let (outcome, after) = roundtrip_full(&before);

    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    assert_eq!(read_epub(&before), after, "bytes must be identical");
}

// ---------------------------------------------------------------------------
// The preservation gate
// ---------------------------------------------------------------------------

#[test]
fn a_book_that_arrives_broken_is_still_retagged() {
    // The regression that matters most here. An absolute gate — "every internal
    // reference must resolve" — refused nine books out of a real 37-book
    // library, every one for a defect already present in the input. The gate
    // asks whether the operation made anything *worse*, not whether the result
    // is perfect.
    let before = common::epub3_written_as_epub2_but_already_broken();
    let (outcome, after) = roundtrip_full(&before);

    assert!(
        entry(&after, "OEBPS/content.opf").contains(r#"version="2.0""#),
        "the pre-existing defects must not block the retag: {:?}",
        outcome.findings
    );
    assert!(
        !outcome.findings.iter().any(|f| f.contains("abandoned")),
        "got {:?}",
        outcome.findings
    );

    // The defects it arrived with are separately handled on their own merits:
    // the dead stylesheet include goes, and the same-document link to an anchor
    // that is nowhere in the book loses its href but keeps its text. Neither has
    // any bearing on the gate.
    let ch1 = entry(&after, "OEBPS/ch1.xhtml");
    assert!(!ch1.contains("page-template.xpgt"), "{ch1}");
    assert!(!ch1.contains("#nowhere"), "{ch1}");
    assert!(ch1.contains(">dangling</a>"), "{ch1}");
}

#[test]
fn the_gate_still_stops_an_operation_that_would_break_something() {
    // The differential gate must not have become a no-op: a transformation that
    // drops a document is still refused.
    use epubfix::Book;
    use std::io::Cursor;

    let before = Book::load(Cursor::new(epub2("<p id=\"a\">x</p>", "<p>y</p>"))).unwrap();
    let mut after = before.clone();
    after.set_text("OEBPS/ch1.xhtml", "<html><body></body></html>".to_string());

    let problems = epubfix::verify::check(&before, &after);
    assert!(
        problems.iter().any(|p| p.contains("was lost")),
        "losing an id must still be caught: {problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("visible text changed")),
        "losing text must still be caught: {problems:?}"
    );
}

#[test]
fn a_newly_broken_reference_is_caught_even_when_others_were_already_broken() {
    use epubfix::Book;
    use std::io::Cursor;

    let before = Book::load(Cursor::new(
        common::epub3_written_as_epub2_but_already_broken(),
    ))
    .unwrap();
    let mut after = before.clone();
    // Break something that used to work, on top of what was already broken.
    let text = after
        .text("OEBPS/toc.ncx")
        .unwrap()
        .replace("ch1.xhtml", "gone.xhtml");
    after.set_text("OEBPS/toc.ncx", text);

    let problems = epubfix::verify::check(&before, &after);
    assert!(
        problems.iter().any(|p| p.contains("gone.xhtml")),
        "the new breakage must surface: {problems:?}"
    );
    assert!(
        !problems.iter().any(|p| p.contains("page-template")),
        "the pre-existing one must not: {problems:?}"
    );
}

// ---------------------------------------------------------------------------
// Triggering on violations rather than features
// ---------------------------------------------------------------------------

#[test]
fn an_epub2_book_with_an_inline_svg_cover_is_never_retagged() {
    // The regression that matters most here. An earlier version listed `svg`
    // among the constructs that "require EPUB 3" and fired on five books in a
    // row, three of which validated with zero errors. SVG is legal in EPUB 2 —
    // OPS 2.0.1 lists it among the core media types, and EPUB Check agrees.
    let cover = r#"<div><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" width="10" height="10">
      <image width="10" height="10" xlink:href="cover.png"/></svg></div>"#;
    let before = epub2_ncx(cover, "<p>plain prose</p>", NESTED_NCX);
    let (outcome, after) = roundtrip_full(&before);

    assert!(
        !outcome.changes.iter().any(|c| c.contains("retagged")),
        "an SVG cover is not evidence of anything: {:?}",
        outcome.changes
    );
    assert!(entry(&after, "OEBPS/content.opf").contains(r#"version="2.0""#));
    assert!(!has(&after, "OEBPS/nav.xhtml"));
}

#[test]
fn a_book_with_no_evidence_of_a_problem_is_left_completely_alone() {
    // The hard precondition: no violations means nothing to fix, so every
    // change is downside. Holds whichever version the book declares.
    let svg = r#"<div><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg></div>"#;
    let before = epub2(svg, "<p>prose</p>");
    let (outcome, after) = roundtrip_full(&before);

    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    assert_eq!(read_epub(&before), after, "byte-identical");
}

#[test]
fn an_element_that_really_is_illegal_in_epub2_still_triggers_an_upgrade() {
    // The trigger has to keep working for constructs that genuinely are errors.
    // <section> has no XHTML 1.1 equivalent and EPUB Check rejects it outright.
    let before = epub2_ncx(
        "<section><p>modern markup</p></section>",
        "<p>x</p>",
        NESTED_NCX,
    );
    let (outcome, after) = roundtrip_full(&before);

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("retagged") && c.contains("section")),
        "got {:?}",
        outcome.changes
    );
    assert!(entry(&after, "OEBPS/content.opf").contains(r#"version="3.0""#));
}

/// The `guide` is the EPUB 2 spelling of landmarks. Migration adds the modern
/// form alongside it rather than replacing it — `guide` stays legal in EPUB 3.
#[test]
fn the_opf_guide_becomes_a_landmarks_nav() {
    let guide = r#"  <guide>
    <reference type="cover" title="Cover" href="ch1.xhtml"/>
    <reference type="text" title="Begin Reading" href="ch2.xhtml"/>
    <reference type="title-page" title="Title" href="ch1.xhtml"/>
  </guide>
"#;
    let before = common::epub2_ncx_guide(VERSE, "<p>x</p>", NESTED_NCX, guide);
    let (outcome, after) = migrate(&before);
    let nav = entry(&after, "OEBPS/nav.xhtml");

    assert!(nav.contains(r#"epub:type="landmarks""#), "{nav}");
    // epubcheck requires an epub:type on every landmarks anchor.
    assert!(
        nav.contains(r#"<a epub:type="cover" href="ch1.xhtml">Cover</a>"#),
        "{nav}"
    );
    // Guide types that have a different EPUB 3 spelling are translated, so a
    // reading system looking for the start of the book actually finds it.
    assert!(
        nav.contains(r#"epub:type="bodymatter" href="ch2.xhtml">Begin Reading"#),
        "text -> bodymatter: {nav}"
    );
    assert!(nav.contains(r#"epub:type="titlepage""#), "{nav}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("3 landmarks")),
        "got {:?}",
        outcome.changes
    );
    // The guide itself is untouched: still valid, and EPUB 2 readers use it.
    assert!(entry(&after, "OEBPS/content.opf").contains("<guide>"));
}

#[test]
fn a_guide_entry_pointing_at_an_image_never_reaches_the_landmarks_nav() {
    // It is an OPF-032 error where it stands; copying it into the nav would
    // move the error rather than fix it.
    let guide = r#"  <guide>
    <reference type="cover" title="Cover" href="cover.jpg"/>
    <reference type="text" title="Text" href="ch1.xhtml"/>
  </guide>
"#;
    let before = common::epub2_ncx_guide(VERSE, "<p>x</p>", NESTED_NCX, guide);
    let (_, after) = migrate(&before);
    let nav = entry(&after, "OEBPS/nav.xhtml");

    assert!(!nav.contains("cover.jpg"), "{nav}");
    assert!(nav.contains(r#"epub:type="bodymatter""#), "{nav}");
}

#[test]
fn a_book_with_no_guide_gets_no_landmarks_nav() {
    // An empty <ol> is itself an error, so the section has to be omitted
    // entirely rather than emitted blank.
    let (_, after) = migrate(&epub2_ncx(VERSE, "<p>x</p>", NESTED_NCX));
    assert!(!entry(&after, "OEBPS/nav.xhtml").contains("landmarks"));
}

// ---------------------------------------------------------------------------
// Package metadata the converter used to miss
// ---------------------------------------------------------------------------

/// An EPUB 2 book with caller-supplied `<metadata>` contents and NCX bytes.
fn book_meta(metadata: &str, ncx: &[u8]) -> Vec<u8> {
    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
{metadata}
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#
    );
    let ch1 = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
        <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
        <head><title>T</title></head>\n<body><p>text</p></body></html>";
    common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
    ])
}

const PLAIN_NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap><navPoint id="n1" playOrder="1"><navLabel><text>One</text></navLabel>
    <content src="ch1.xhtml"/></navPoint></navMap>
</ncx>"#;

/// A prefix is a local name for a namespace, and Calibre binds one *per
/// element* — `ns0`, `ns1`, `ns2` for three attributes in one file, none of
/// them `opf`. Matching the literal string `opf:` saw none of them.
#[test]
fn legacy_opf_attributes_are_found_under_whatever_prefix_a_book_binds() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:1234-5678</dc:identifier>\n",
        "    <dc:title>T</dc:title><dc:language>en</dc:language>\n",
        "    <dc:creator xmlns:ns0=\"http://www.idpf.org/2007/opf\" ns0:role=\"aut\" ",
        "ns0:file-as=\"Mckenna, Terence\">Terence Mckenna</dc:creator>\n",
        "    <dc:contributor xmlns:ns1=\"http://www.idpf.org/2007/opf\" ns1:role=\"bkp\">calibre</dc:contributor>"
    );
    let (outcome, after) = roundtrip_migrated(&book_meta(metadata, PLAIN_NCX.as_bytes()));
    let opf = entry(&after, "OEBPS/content.opf");

    assert!(!opf.contains("ns0:role"), "{opf}");
    assert!(!opf.contains("ns0:file-as"), "{opf}");
    assert!(!opf.contains("ns1:role"), "{opf}");
    assert!(opf.contains(r#"property="role">aut</meta>"#), "{opf}");
    assert!(opf.contains(r#"property="file-as">Mckenna, Terence</meta>"#), "{opf}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("3 legacy opf:")),
        "got {:?}",
        outcome.changes
    );
}

/// EPUB 3 has no `opf:event` and permits at most one `<dc:date>`. A book can
/// break both at once, so both rules are applied — and the survivor is the one
/// that says it is the publication date, not merely the first.
#[test]
fn legacy_dates_are_reduced_to_the_one_epub3_allows() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:1234-5678</dc:identifier>\n",
        "    <dc:title>T</dc:title><dc:language>en</dc:language>\n",
        "    <dc:date xmlns:opf=\"http://www.idpf.org/2007/opf\" opf:event=\"converted\">2009-12-10</dc:date>\n",
        "    <dc:date xmlns:opf=\"http://www.idpf.org/2007/opf\" opf:event=\"publication\">2010-10-25</dc:date>"
    );
    let (outcome, after) = roundtrip_migrated(&book_meta(metadata, PLAIN_NCX.as_bytes()));
    let opf = entry(&after, "OEBPS/content.opf");

    assert!(!opf.contains("opf:event"), "{opf}");
    assert_eq!(opf.matches("<dc:date").count(), 1, "{opf}");
    // The publication date is the one kept, not the first one in the file.
    assert!(opf.contains("2010-10-25"), "{opf}");
    assert!(!opf.contains("2009-12-10"), "{opf}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("<dc:date>")),
        "got {:?}",
        outcome.changes
    );
}

/// A single date carrying the attribute keeps its value and loses the attribute.
#[test]
fn a_lone_dated_element_keeps_its_value() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:1234-5678</dc:identifier>\n",
        "    <dc:title>T</dc:title><dc:language>en</dc:language>\n",
        "    <dc:date xmlns:opf=\"http://www.idpf.org/2007/opf\" opf:event=\"modification\">2011-01-01</dc:date>"
    );
    let (_, after) = roundtrip_migrated(&book_meta(metadata, PLAIN_NCX.as_bytes()));
    let opf = entry(&after, "OEBPS/content.opf");
    assert!(!opf.contains("opf:event"), "{opf}");
    assert!(opf.contains("2011-01-01"), "{opf}");
}

/// A UTF-8 byte-order mark left every span the scanner produced three bytes
/// out, so every element name came back empty and the file matched nothing.
/// Four books reported "the NCX has no navMap" for an NCX that plainly had one.
#[test]
fn a_byte_order_mark_does_not_make_a_file_invisible() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:1234-5678</dc:identifier>\n",
        "    <dc:title>T</dc:title><dc:language>en</dc:language>"
    );
    let mut ncx = vec![0xEF, 0xBB, 0xBF];
    ncx.extend_from_slice(PLAIN_NCX.as_bytes());

    let (outcome, after) = roundtrip_migrated(&book_meta(metadata, &ncx));
    assert!(
        !outcome.findings.iter().any(|f| f.contains("no navMap")),
        "got {:?}",
        outcome.findings
    );
    assert!(has(&after, "OEBPS/nav.xhtml"), "{:?}", common::names(&after));
    assert!(
        entry(&after, "OEBPS/nav.xhtml").contains(r#"<a href="ch1.xhtml">One</a>"#),
        "got {}",
        entry(&after, "OEBPS/nav.xhtml")
    );
    // And the mark itself does not survive into the repaired book.
    assert!(
        !entry(&after, "OEBPS/toc.ncx").starts_with('\u{FEFF}'),
        "the mark should be gone"
    );
}
