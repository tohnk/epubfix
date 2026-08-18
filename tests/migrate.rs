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
    assert!(
        doc.contains("<div>"),
        "and its verse gets a block container: {doc}"
    );
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
        !outcome
            .findings
            .iter()
            .any(|f| f.contains("too few to retag")),
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

/// Inline content inside a `<form>` cannot be repaired as EPUB 2 — measured,
/// wrapping its children changes nothing — so even one run moves the
/// declaration, where the same run in a `<blockquote>` is wrapped instead.
#[test]
fn unwrappable_inline_content_moves_the_declaration_on_its_own() {
    let form = r#"<form action="x"><input type="text" name="q"/></form>"#;
    let (outcome, after) = roundtrip_full(&epub2_ncx(form, "<p>x</p>", NESTED_NCX));

    assert!(entry(&after, "OEBPS/content.opf").contains(r#"version="3.0""#));
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("retagged") && c.contains("requires EPUB 3")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        outcome
            .findings
            .iter()
            .all(|f| !f.contains("only validates under EPUB 3")),
        "nothing left to hand back: {:?}",
        outcome.findings
    );
}

/// The same book under --keep-version stays EPUB 2 and is reported with the
/// right remedy: the declaration is what must move.
#[test]
fn keep_version_reports_unwrappable_content_instead_of_retagging() {
    let form = r#"<form action="x"><input type="text" name="q"/></form>"#;
    let (outcome, after) = common::roundtrip_kept(&epub2_ncx(form, "<p>x</p>", NESTED_NCX));

    assert!(entry(&after, "OEBPS/content.opf").contains(r#"version="2.0""#));
    assert!(
        !outcome.changes.iter().any(|c| c.contains("retagged")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("Run without --keep-version")),
        "got {:?}",
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
    assert!(
        doc.contains("</img>"),
        "the end tag must be untouched: {doc}"
    );
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
// The spine fallback for an NCX that carries no navigation
// ---------------------------------------------------------------------------

/// *The Sound and the Fury*'s shape: an NCX whose `<navMap>` is empty, and
/// spine documents whose headings hold the only table of contents the book
/// has. The nav is generated from those headings, and the same entries are
/// written into the NCX navMap, which epubcheck reports incomplete without
/// them.
#[test]
fn an_empty_navmap_falls_back_to_the_spine_headings() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap/>
</ncx>"#;
    let (outcome, after) = migrate(&epub2_ncx(
        "<h1 id=\"one\">April</h1><p>x</p>",
        "<h2>a note</h2><p>y</p>",
        ncx,
    ));

    let nav = entry(&after, "OEBPS/nav.xhtml");
    assert!(nav.contains("April"), "{nav}");
    assert!(nav.contains(r#"href="ch1.xhtml#one""#), "{nav}");
    assert!(nav.contains("a note"), "{nav}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("from the spine")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        outcome.findings.iter().all(|f| !f.contains("navMap")),
        "got {:?}",
        outcome.findings
    );

    // The NCX navMap is populated with the same entries, so it stops being an
    // epubcheck error itself.
    let ncx_out = entry(&after, "OEBPS/toc.ncx");
    assert!(ncx_out.contains(r#"<navPoint id="np1" playOrder="1">"#), "{ncx_out}");
    assert!(ncx_out.contains("<text>April</text>"), "{ncx_out}");
    assert!(ncx_out.contains(r#"<content src="ch1.xhtml#one"/>"#), "{ncx_out}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("spine entr(ies) into the NCX")),
        "got {:?}",
        outcome.changes
    );
}

/// A spine with no headings at all still gets one entry per document, named
/// by the document title — and by the filename when a converter gave every
/// document the same title, which would fill the whole TOC with one repeated
/// string.
#[test]
fn a_headingless_spine_is_named_by_titles_then_filenames() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap/>
</ncx>"#;
    let (_, after) = migrate(&epub2_ncx("<p>x</p>", "<p>y</p>", ncx));
    let nav = entry(&after, "OEBPS/nav.xhtml");

    assert_eq!(nav.matches(">T</a>").count(), 1, "{nav}");
    assert!(nav.contains(">ch2</a>"), "{nav}");
    assert!(!nav.contains("<ol>\n  </ol>"), "no empty list: {nav}");
}

/// When the spine holds nothing to build from either, the finding stays.
#[test]
fn a_book_with_no_navigation_anywhere_is_reported() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <navMap/>
</ncx>"#;
    // An empty spine: there are no documents whose headings or names could
    // stand in.
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"/>
</package>"#;
    let ch1 = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
        <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
        <head></head>\n<body><p>text</p></body></html>";
    let before = common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
    ]);
    let (outcome, after) = migrate(&before);

    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("no navigation")),
        "got {:?}",
        outcome.findings
    );
    assert!(!has(&after, "OEBPS/nav.xhtml"));
}

// ---------------------------------------------------------------------------
// identifier-uuid
// ---------------------------------------------------------------------------

/// A Kobo package whose identifier is the never-filled `urn:uuid:` gets a
/// real one, and the NCX uid follows it through ncx-uid. A well-formed UUID
/// is left alone, which is also what keeps a second run a no-op.
#[test]
fn an_identifier_claiming_an_empty_uuid_gets_a_real_one() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:</dc:identifier>\n",
        "    <dc:title>T</dc:title><dc:language>en</dc:language>"
    );
    let (outcome, after) = roundtrip_migrated(&book_meta(metadata, PLAIN_NCX.as_bytes()));
    let opf = entry(&after, "OEBPS/content.opf");

    assert!(!opf.contains("urn:uuid:</dc:identifier>"), "{opf}");
    let uuid_re = regex::Regex::new(
        r"urn:uuid:[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}",
    )
    .expect("valid regex");
    let generated = uuid_re.find(&opf).expect("a version-4 UUID").as_str();
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("identifier(s) claiming")),
        "got {:?}",
        outcome.changes
    );

    // ncx-uid syncs dtb:uid to the same value later in the run.
    let ncx = entry(&after, "OEBPS/toc.ncx");
    assert!(ncx.contains(generated), "dtb:uid must follow: {ncx}");

    // Idempotent: a second run changes nothing.
    let repacked = common::make_epub(
        &after
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect::<Vec<_>>(),
    );
    let (second, _) = roundtrip_migrated(&repacked);
    assert!(
        !second.changes.iter().any(|c| c.contains("identifier(s) claiming")),
        "got {:?}",
        second.changes
    );
}

/// A well-formed UUID — and the nil UUID, which epubcheck accepts — is never
/// replaced.
#[test]
fn a_valid_uuid_identifier_is_left_alone() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:12345678-1234-4234-8234-123456789012</dc:identifier>\n",
        "    <dc:title>T</dc:title><dc:language>en</dc:language>"
    );
    let (outcome, after) = roundtrip_migrated(&book_meta(metadata, PLAIN_NCX.as_bytes()));
    let opf = entry(&after, "OEBPS/content.opf");
    assert!(
        opf.contains("urn:uuid:12345678-1234-4234-8234-123456789012"),
        "{opf}"
    );
    assert!(
        !outcome.changes.iter().any(|c| c.contains("identifier(s) claiming")),
        "got {:?}",
        outcome.changes
    );
}

/// A book whose NCX lists the front matter first while the spine holds it
/// last — the shape behind the NAV-011 warnings on the generated navs. The
/// NCX is sorted into spine order before the nav is generated, so the nav
/// inherits the reading order with no second step.
#[test]
fn the_generated_nav_inherits_the_sorted_ncx_order() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="front" playOrder="1"><navLabel><text>Front</text></navLabel><content src="ch2.xhtml"/></navPoint>
    <navPoint id="first" playOrder="2"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let (outcome, after) = migrate(&epub2_ncx(VERSE, "<p>x</p>", ncx));
    let nav = entry(&after, "OEBPS/nav.xhtml");

    let one = nav.find(">One</a>").unwrap();
    let front = nav.find(">Front</a>").unwrap();
    assert!(one < front, "the nav follows the spine order: {nav}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 2 NCX navMap entr(ies)")),
        "got {:?}",
        outcome.changes
    );
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
    assert!(
        opf.contains(r#"property="file-as">Mckenna, Terence</meta>"#),
        "{opf}"
    );
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

// ---------------------------------------------------------------------------
// dangling-refines
// ---------------------------------------------------------------------------

/// The *Pensées* shape: refinements pointing at ids the package never defines,
/// while the elements they describe sit right there under other ids. The
/// property decides which element each means — `role`, `file-as` and
/// `display-seq` all name the lone creator, `title-type` the title — and the
/// ambiguous one is reported rather than guessed at.
#[test]
fn dangling_refines_are_repointed_at_the_element_their_property_names() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:1234-5678</dc:identifier>\n",
        "    <dc:title id=\"t\">Test</dc:title><dc:language>en</dc:language>\n",
        "    <dc:creator id=\"c\">Blaise Pascal</dc:creator>\n",
        "    <dc:identifier>isbn:9780141915647</dc:identifier>\n",
        "    <dc:identifier id=\"isbn_id\">urn:isbn: 9780141915647</dc:identifier>\n",
        "    <meta id=\"role\" property=\"role\" refines=\"#creator\" scheme=\"marc:relators\">aut</meta>\n",
        "    <meta property=\"file-as\" refines=\"#creator\">Pascal, Blaise</meta>\n",
        "    <meta property=\"display-seq\" refines=\"#creator\">1</meta>\n",
        "    <meta property=\"title-type\" refines=\"#t1\">main</meta>\n",
        "    <meta property=\"identifier-type\" refines=\"#src-id\" scheme=\"onix:codelist5\">15</meta>\n",
        "    <meta property=\"identifier-type\" refines=\"#isbn_id\" scheme=\"onix:codelist5\">15</meta>"
    );
    let (outcome, after) = roundtrip_migrated(&book_meta(metadata, PLAIN_NCX.as_bytes()));
    let opf = entry(&after, "OEBPS/content.opf");

    assert_eq!(opf.matches(r##"refines="#c""##).count(), 3, "{opf}");
    assert!(opf.contains(r##"refines="#t""##), "{opf}");
    assert!(opf.contains(r##"refines="#isbn_id""##), "the healthy one is untouched: {opf}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("repointed 4 <meta refines>")),
        "got {:?}",
        outcome.changes
    );
    // Three identifiers could be meant by #src-id, and nothing says which.
    assert!(opf.contains(r##"refines="#src-id""##), "{opf}");
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("identifier-type") && f.contains("3 dc: element(s)")),
        "got {:?}",
        outcome.findings
    );
}

/// The *Pensées* shape in full: the converter wrote the same metadata twice,
/// once as refinements of the real ids and once as dangling ones. Repointing
/// the dangling ones would create the duplicate refinements epubcheck rejects
/// (measured: a second `title-type` or `file-as` on one element is an error),
/// so they are removed instead — and when the two disagree about the value,
/// removing either loses something, so that is reported, not resolved.
#[test]
fn dangling_refines_that_duplicate_a_healthy_one_are_removed() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:1234-5678</dc:identifier>\n",
        "    <dc:title id=\"t\">Test</dc:title><dc:language>en</dc:language>\n",
        "    <dc:creator id=\"c\">Blaise Pascal</dc:creator>\n",
        "    <meta property=\"title-type\" refines=\"#t\">main</meta>\n",
        "    <meta property=\"file-as\" refines=\"#c\">Pascal, Blaise</meta>\n",
        "    <meta property=\"role\" refines=\"#c\">aut</meta>\n",
        "    <meta property=\"title-type\" refines=\"#t1\">main</meta>\n",
        "    <meta property=\"file-as\" refines=\"#creator\">Pascal, Blaise</meta>\n",
        "    <meta property=\"role\" refines=\"#creator\">aut</meta>\n",
        "    <meta property=\"display-seq\" refines=\"#creator\">1</meta>\n",
        "    <meta property=\"title-type\" refines=\"#t2\">subtitle</meta>"
    );
    let (outcome, after) = roundtrip_migrated(&book_meta(metadata, PLAIN_NCX.as_bytes()));
    let opf = entry(&after, "OEBPS/content.opf");

    // The duplicates of the #t and #c refinements are gone...
    assert_eq!(opf.matches(r##"refines="#t""##).count(), 1, "{opf}");
    assert_eq!(
        opf.matches(r##"refines="#c""##).count(),
        3,
        "the two healthy ones, plus the repointed display-seq: {opf}"
    );
    assert!(opf.contains(r##"refines="#c">1"##), "{opf}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("removed 3 <meta refines> element(s) that duplicated")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("repointed 1 <meta refines>")),
        "got {:?}",
        outcome.changes
    );
    // The conflicting value is reported rather than resolved.
    assert!(opf.contains(r##"refines="#t2""##), "left alone: {opf}");
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("subtitle") && f.contains("main")),
        "got {:?}",
        outcome.findings
    );
}

/// The target element has no id at all: one is minted for the repoint to land
/// on. A property outside the measured map is reported, not guessed at.
#[test]
fn a_refines_repoint_mints_the_id_the_target_lacks_and_reports_unknown_properties() {
    let metadata = concat!(
        "    <dc:identifier id=\"BookId\">urn:uuid:1234-5678</dc:identifier>\n",
        "    <dc:title>T</dc:title><dc:language>en</dc:language>\n",
        "    <meta property=\"title-type\" refines=\"#t1\">main</meta>\n",
        "    <meta property=\"something-new\" refines=\"#gone\">v</meta>"
    );
    let (outcome, after) = roundtrip_migrated(&book_meta(metadata, PLAIN_NCX.as_bytes()));
    let opf = entry(&after, "OEBPS/content.opf");

    assert!(opf.contains(r#"<dc:title id="title">T</dc:title>"#), "{opf}");
    assert!(opf.contains(r##"refines="#title""##), "{opf}");
    assert!(
        outcome.changes.iter().any(|c| c.contains("minted 1 id(s)")),
        "got {:?}",
        outcome.changes
    );
    assert!(opf.contains(r##"refines="#gone""##), "left alone: {opf}");
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("something-new") && f.contains("not one this knows")),
        "got {:?}",
        outcome.findings
    );
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
    assert!(
        has(&after, "OEBPS/nav.xhtml"),
        "{:?}",
        common::names(&after)
    );
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



// ---------------------------------------------------------------------------
// The NCX sort that precedes nav generation (deeper shapes)
// ---------------------------------------------------------------------------

/// A book with three spine documents and a front-matter-first NCX, forced
/// through migration: the NCX is sorted before the nav is generated from it,
/// so both come out in reading order.
#[test]
fn an_out_of_order_ncx_is_sorted_before_the_nav_is_built() {
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch3" href="ch3.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/><itemref idref="ch2"/><itemref idref="ch3"/></spine>
</package>"#;
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="c3" playOrder="1"><navLabel><text>Three</text></navLabel><content src="ch3.xhtml"/></navPoint>
    <navPoint id="c1" playOrder="2"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="c2" playOrder="3"><navLabel><text>Two</text></navLabel><content src="ch2.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let doc = |t: &str| {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
             <head><title>{t}</title></head>\n<body><p>{t}</p></body></html>"
        )
    };
    let before = common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("one").as_bytes()),
        ("OEBPS/ch2.xhtml", doc("two").as_bytes()),
        ("OEBPS/ch3.xhtml", doc("three").as_bytes()),
    ]);
    let (outcome, after) = migrate(&before);

    let ncx_out = entry(&after, "OEBPS/toc.ncx");
    let one = ncx_out.find("One").unwrap();
    let two = ncx_out.find("Two").unwrap();
    let three = ncx_out.find("Three").unwrap();
    assert!(one < two && two < three, "{ncx_out}");
    assert!(
        ncx_out.contains(r#"<text>One</text></navLabel><content src="ch1.xhtml""#),
        "labels and targets travel together: {ncx_out}"
    );
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 3 NCX navMap entr(ies)")),
        "got {:?}",
        outcome.changes
    );
    let nav = entry(&after, "OEBPS/nav.xhtml");
    let n1 = nav.find(">One</a>").unwrap();
    let n3 = nav.find(">Three</a>").unwrap();
    assert!(n1 < n3, "the nav inherits the order: {nav}");

    // A second run sorts nothing: the order is already right.
    let repacked = common::make_epub(
        &after
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect::<Vec<_>>(),
    );
    let (second, _) = migrate(&repacked);
    assert!(
        !second.changes.iter().any(|c| c.contains("sorted")),
        "got {:?}",
        second.changes
    );
}

/// The *Green Mile* shape: nested navPoints. Children belong to their parent
/// and travel with it, and their own order is sorted under it.
#[test]
fn nested_navpoints_are_sorted_within_their_parent() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="ch2" playOrder="1"><navLabel><text>Two</text></navLabel><content src="ch2.xhtml"/></navPoint>
    <navPoint id="ch1" playOrder="2"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/>
      <navPoint id="b" playOrder="3"><navLabel><text>B</text></navLabel><content src="ch1.xhtml#b"/></navPoint>
      <navPoint id="a" playOrder="4"><navLabel><text>A</text></navLabel><content src="ch1.xhtml#a"/></navPoint>
    </navPoint>
  </navMap>
</ncx>"#;
    let ch1 = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>one</title></head><body><p id="a">a</p><p id="b">b</p></body></html>"#;
    let ch2 = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>two</title></head><body><p>two</p></body></html>"#;
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/><itemref idref="ch2"/></spine>
</package>"#;
    let before = common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/ch2.xhtml", ch2.as_bytes()),
    ]);
    let (outcome, after) = migrate(&before);

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 4 NCX navMap entr(ies)")),
        "got {:?}",
        outcome.changes
    );
    let ncx = entry(&after, "OEBPS/toc.ncx");
    let one = ncx.find("One").unwrap();
    let two = ncx.find("Two").unwrap();
    assert!(one < two, "the chapter moves as one: {ncx}");
    let a = ncx.find(">A</text>").unwrap();
    let b = ncx.find(">B</text>").unwrap();
    assert!(a < b, "its children sort under it: {ncx}");
    assert!(one < a && b < two, "the children stay nested: {ncx}");
}

/// The *Lolita* shape: two entries in one document, one with a fragment and
/// one without. The bare link lands at the top of the document, so it comes
/// first whatever the NCX said.
#[test]
fn same_document_entries_sort_by_fragment_position() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="n1" playOrder="1"><navLabel><text>Deep</text></navLabel><content src="ch1.xhtml#late"/></navPoint>
    <navPoint id="n2" playOrder="2"><navLabel><text>Top</text></navLabel><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let (outcome, after) = migrate(&epub2_ncx(
        r#"<p id="late">deep</p>"#,
        "<p>x</p>",
        ncx,
    ));

    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 2 NCX navMap entr(ies)")),
        "got {:?}",
        outcome.changes
    );
    let ncx = entry(&after, "OEBPS/toc.ncx");
    assert!(ncx.find("Top").unwrap() < ncx.find("Deep").unwrap(), "{ncx}");
}

/// A navPoint at a document outside the spine cannot be placed, and keeps the
/// place of the entry it followed.
#[test]
fn a_navpoint_outside_the_spine_keeps_its_neighbour() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="n1" playOrder="1"><navLabel><text>Two</text></navLabel><content src="ch2.xhtml"/></navPoint>
    <navPoint id="n2" playOrder="2"><navLabel><text>Extra</text></navLabel><content src="extra.xhtml"/></navPoint>
    <navPoint id="n3" playOrder="3"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="extra" href="extra.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/><itemref idref="ch2"/></spine>
</package>"#;
    let doc = |t: &str| {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
             <head><title>{t}</title></head>\n<body><p>{t}</p></body></html>"
        )
    };
    let before = common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", doc("one").as_bytes()),
        ("OEBPS/ch2.xhtml", doc("two").as_bytes()),
        ("OEBPS/extra.xhtml", doc("extra").as_bytes()),
    ]);
    let (outcome, after) = migrate(&before);

    let ncx = entry(&after, "OEBPS/toc.ncx");
    let one = ncx.find("One").unwrap();
    let two = ncx.find("Two").unwrap();
    let extra = ncx.find("Extra").unwrap();
    assert!(one < two && extra > two, "extra follows Two, as it did: {ncx}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 3 NCX navMap entr(ies)")),
        "got {:?}",
        outcome.changes
    );
}

// ---------------------------------------------------------------------------
// nav-order: an EPUB 3 book shipped with an out-of-order nav
// ---------------------------------------------------------------------------

/// A book that declares EPUB 3 and carries its own nav, whose toc lists the
/// front matter first while the spine holds it last — the *Spice and Wolf*
/// shape. The toc entries are sorted; the landmarks nav is left alone.
#[test]
fn a_shipped_nav_out_of_spine_order_is_sorted_in_place() {
    let nav = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title><meta charset="utf-8"/></head>
<body>
<nav epub:type="toc" id="toc">
  <h1>Contents</h1>
  <ol>
    <li id="front"><a href="titlepage.xhtml">Title Page</a></li>
    <li id="one"><a href="ch1.xhtml">One</a></li>
    <li id="two"><a href="ch2.xhtml">Two</a></li>
  </ol>
</nav>
<nav epub:type="landmarks" hidden="hidden">
  <ol>
    <li><a epub:type="bodymatter" href="ch1.xhtml">Start</a></li>
    <li><a epub:type="titlepage" href="titlepage.xhtml">Title</a></li>
  </ol>
</nav>
</body></html>"#;
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:12345678-1234-4234-8234-123456789012</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="tp" href="titlepage.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/><itemref idref="ch2"/><itemref idref="tp"/></spine>
</package>"#;
    let doc = |t: &str| {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
             <head><title>{t}</title></head>\n<body><p>{t}</p></body></html>"
        )
    };
    let before = common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/titlepage.xhtml", doc("title").as_bytes()),
        ("OEBPS/ch1.xhtml", doc("one").as_bytes()),
        ("OEBPS/ch2.xhtml", doc("two").as_bytes()),
    ]);
    let (outcome, after) = roundtrip_full(&before);

    let nav_out = entry(&after, "OEBPS/nav.xhtml");
    let one = nav_out.find(">One</a>").unwrap();
    let two = nav_out.find(">Two</a>").unwrap();
    let title = nav_out.find(">Title Page</a>").unwrap();
    assert!(one < two && two < title, "toc follows the spine: {nav_out}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 3 toc entr(ies)")),
        "got {:?}",
        outcome.changes
    );
    // The landmarks nav keeps its own order.
    let start = nav_out.find(">Start</a>").unwrap();
    let lm_title = nav_out.rfind(">Title</a>").unwrap();
    assert!(start < lm_title, "{nav_out}");
}

/// A shipped nav already in spine order is not rewritten, and nested entries
/// sort under their parent exactly like nested navPoints do.
#[test]
fn a_shipped_nav_in_order_is_untouched_and_nesting_is_preserved() {
    let nav = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title><meta charset="utf-8"/></head>
<body>
<nav epub:type="toc" id="toc">
  <h1>Contents</h1>
  <ol>
    <li id="one"><a href="ch1.xhtml">One</a>
      <ol>
        <li id="b"><a href="ch1.xhtml#b">B</a></li>
        <li id="a"><a href="ch1.xhtml#a">A</a></li>
      </ol>
    </li>
    <li id="two"><a href="ch2.xhtml">Two</a></li>
  </ol>
</nav>
</body></html>"#;
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:12345678-1234-4234-8234-123456789012</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/><itemref idref="ch2"/></spine>
</package>"#;
    let ch1 = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>one</title></head><body><p id="a">a</p><p id="b">b</p></body></html>"#;
    let ch2 = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>two</title></head><body><p>two</p></body></html>"#;
    let before = common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/ch2.xhtml", ch2.as_bytes()),
    ]);
    let (outcome, after) = roundtrip_full(&before);

    let nav_out = entry(&after, "OEBPS/nav.xhtml");
    let one = nav_out.find(">One</a>").unwrap();
    let a = nav_out.find(">A</a>").unwrap();
    let b = nav_out.find(">B</a>").unwrap();
    let two = nav_out.find(">Two</a>").unwrap();
    assert!(one < a && a < b && b < two, "nested children sort under their parent: {nav_out}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 2 toc entr(ies)")),
        "the nested pair moves: got {:?}",
        outcome.changes
    );
}

/// An EPUB 3 book carrying both an out-of-order NCX and an out-of-order nav:
/// both are sorted, so an old reading system falling back to the NCX shows
/// the same order as one using the nav.
#[test]
fn an_epub3_book_with_both_tocs_gets_both_in_spine_order() {
    let nav = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title><meta charset="utf-8"/></head>
<body>
<nav epub:type="toc" id="toc">
  <h1>Contents</h1>
  <ol>
    <li id="front"><a href="titlepage.xhtml">Title Page</a></li>
    <li id="one"><a href="ch1.xhtml">One</a></li>
    <li id="two"><a href="ch2.xhtml">Two</a></li>
  </ol>
</nav>
</body></html>"#;
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="tp" playOrder="1"><navLabel><text>Title Page</text></navLabel><content src="titlepage.xhtml"/></navPoint>
    <navPoint id="c1" playOrder="2"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="c2" playOrder="3"><navLabel><text>Two</text></navLabel><content src="ch2.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:12345678-1234-4234-8234-123456789012</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="tp" href="titlepage.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/><itemref idref="ch2"/><itemref idref="tp"/></spine>
</package>"#;
    let doc = |t: &str| {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
             <head><title>{t}</title></head>\n<body><p>{t}</p></body></html>"
        )
    };
    let before = common::make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/titlepage.xhtml", doc("title").as_bytes()),
        ("OEBPS/ch1.xhtml", doc("one").as_bytes()),
        ("OEBPS/ch2.xhtml", doc("two").as_bytes()),
    ]);
    let (outcome, after) = roundtrip_full(&before);

    let nav_out = entry(&after, "OEBPS/nav.xhtml");
    let n1 = nav_out.find(">One</a>").unwrap();
    let nt = nav_out.find(">Title Page</a>").unwrap();
    assert!(n1 < nt, "{nav_out}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 3 toc entr(ies)")),
        "got {:?}",
        outcome.changes
    );

    let ncx_out = entry(&after, "OEBPS/toc.ncx");
    let c1 = ncx_out.find(">One</text>").unwrap();
    let ct = ncx_out.find(">Title Page</text>").unwrap();
    assert!(c1 < ct, "{ncx_out}");
    assert!(
        outcome
            .changes
            .iter()
            .any(|c| c.contains("sorted 3 NCX navMap entr(ies)")),
        "got {:?}",
        outcome.changes
    );
}

/// An EPUB 2 book with the same out-of-order NCX: untouched. Its NCX is the
/// only TOC it has, the order is legal as written, and nothing else in the
/// book is invited to move.
#[test]
fn an_epub2_book_with_an_out_of_order_ncx_is_left_alone() {
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="c2" playOrder="1"><navLabel><text>Two</text></navLabel><content src="ch2.xhtml"/></navPoint>
    <navPoint id="c1" playOrder="2"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let before = epub2_ncx("<p>x</p>", "<p>y</p>", ncx);
    let (outcome, after) = roundtrip_full(&before);

    assert!(
        !outcome.changes.iter().any(|c| c.contains("sorted")),
        "got {:?}",
        outcome.changes
    );
    let ncx_out = entry(&after, "OEBPS/toc.ncx");
    assert!(ncx_out.find("Two").unwrap() < ncx_out.find("One").unwrap(), "{ncx_out}");
}
