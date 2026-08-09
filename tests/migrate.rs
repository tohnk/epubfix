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
fn a_few_stray_inline_runs_are_reported_rather_than_retagging_the_book() {
    // One or two of these mean a couple of paragraphs need a <div>, not that
    // the whole book is the wrong format. Changing a book's format identity on
    // that evidence would be wildly out of proportion.
    let (outcome, after) = roundtrip_full(&epub2(VERSE, "<p>x</p>"));

    assert!(
        entry(&after, "OEBPS/content.opf").contains(r#"version="2.0""#),
        "must not retag on weak evidence"
    );
    let finding = outcome
        .findings
        .iter()
        .find(|f| f.contains("too few to retag"))
        .unwrap_or_else(|| panic!("expected a report, got {:?}", outcome.findings));
    assert!(finding.contains("--migrate-epub3"), "{finding}");
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
fn an_epub2_book_with_html5_only_markup_is_reported_not_rewritten() {
    let before = epub2(VERSE, "<p>x</p>");
    let (outcome, after) = roundtrip_full(&before);

    let finding = outcome
        .findings
        .iter()
        .find(|f| f.contains("only validates under EPUB 3"))
        .unwrap_or_else(|| {
            panic!(
                "expected a version-mismatch finding, got {:?}",
                outcome.findings
            )
        });
    assert!(finding.contains("--migrate-epub3"), "{finding}");
    // Diagnosis only: the verse itself must be untouched.
    assert!(entry(&after, "OEBPS/ch1.xhtml").contains("<blockquote>"));
    assert!(!entry(&after, "OEBPS/ch1.xhtml").contains("<div>"));
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

    // And the defects it arrived with are still there, untouched and unhidden.
    let ch1 = entry(&after, "OEBPS/ch1.xhtml");
    assert!(ch1.contains("page-template.xpgt"));
    assert!(ch1.contains(r##"href="#nowhere""##));
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
