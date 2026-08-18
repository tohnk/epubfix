//! Proof that the shared verification harness actually catches things.

mod common;

use common::verify::{verify, visible_text};
use common::{epub2, read_epub, roundtrip_full};

#[test]
fn a_clean_book_passes_every_invariant_and_changes_nothing() {
    let before = epub2(
        "<p id=\"a\">text</p>",
        "<p><a href=\"ch1.xhtml#a\">to a</a></p>",
    );
    let (outcome, after) = roundtrip_full(&before);

    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    let r = verify(&read_epub(&before), &after);
    r.assert_sound();
    assert!(
        r.changed.is_empty(),
        "nothing should differ: {:?}",
        r.changed
    );
    assert!(r.text_changed.is_empty());
}

#[test]
fn the_harness_notices_a_newly_broken_link() {
    // The check must fire on a link that *stopped* working.
    let good = read_epub(&epub2(
        "<p id=\"a\">text</p>",
        "<p><a href=\"ch1.xhtml#a\">good</a></p>",
    ));
    let bad = read_epub(&epub2(
        "<p id=\"a\">text</p>",
        "<p><a href=\"ch1.xhtml#nope\">bad</a></p>",
    ));
    let r = verify(&good, &bad);
    assert!(
        r.problems.iter().any(|p| p.contains("no such id exists")),
        "expected a dangling-fragment problem, got {:?}",
        r.problems
    );
}

#[test]
fn the_harness_notices_a_newly_duplicated_id() {
    let good = read_epub(&epub2("<p id=\"a\">one</p><p id=\"b\">two</p>", "<p>x</p>"));
    let bad = read_epub(&epub2("<p id=\"a\">one</p><p id=\"a\">two</p>", "<p>x</p>"));
    let r = verify(&good, &bad);
    assert!(
        r.problems.iter().any(|p| p.contains("duplicate id")),
        "expected a duplicate-id problem, got {:?}",
        r.problems
    );
}

#[test]
fn the_harness_ignores_a_defect_the_book_arrived_with() {
    // The correction that matters: an absolute gate refused nine books out of a
    // real 37-book library over defects that were already in the input.
    let broken = read_epub(&epub2(
        "<p id=\"a\">text</p>",
        "<p><a href=\"ch1.xhtml#nope\">bad</a></p>",
    ));
    let r = verify(&broken, &broken);
    assert!(
        r.problems.is_empty(),
        "nothing got worse, so nothing to report: {:?}",
        r.problems
    );
}

/// The gap that let two real bugs through.
///
/// A deleted table-of-contents entry keeps the page, its text, its ids and
/// every link — the whole of what the rest of this file can see — and takes an
/// epubcheck error with it, so it measures as an improvement. Only a gate that
/// knows what a table of contents *is* can tell that a way into the book was
/// thrown away.
#[test]
fn the_harness_notices_a_navigation_entry_that_was_deleted() {
    const NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="n1" playOrder="1"><navLabel><text>One</text></navLabel>
      <content src="ch1.xhtml"/></navPoint>
    <navPoint id="n2" playOrder="2"><navLabel><text>Two</text></navLabel>
      <content src="ch2.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let full = read_epub(&common::epub2_ncx("<p>one</p>", "<p>two</p>", NCX));
    let pruned = read_epub(&common::epub2_ncx(
        "<p>one</p>",
        "<p>two</p>",
        &NCX.replace(
            r#"    <navPoint id="n2" playOrder="2"><navLabel><text>Two</text></navLabel>
      <content src="ch2.xhtml"/></navPoint>
"#,
            "",
        ),
    ));

    let r = verify(&full, &pruned);
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("navigation no longer reaches") && p.contains("ch2.xhtml")),
        "expected the lost destination to be named, got {:?}",
        r.problems
    );
    // Everything the old gate looked at is untouched — which is exactly why it
    // saw nothing.
    assert!(
        !r.problems.iter().any(|p| p.contains("visible text")
            || p.contains("was lost")
            || p.contains("disappeared")),
        "got {:?}",
        r.problems
    );
    // And it must not fire in reverse: adding navigation is not losing it.
    assert!(
        verify(&pruned, &full)
            .problems
            .iter()
            .all(|p| !p.contains("navigation")),
        "gaining an entry is not a loss"
    );
}

/// A stylesheet is a reference-bearing file, and the harness could not see one.
/// `filenames` and `css-paths` both rewrite `url()`, so a pass that renamed a
/// font and left the stylesheet pointing at the old name would have been called
/// sound by every test in the suite.
#[test]
fn the_harness_notices_a_stylesheet_left_pointing_at_nothing() {
    const OPF: &str = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="css" href="s.css" media-type="text/css"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    const CH1: &str = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>x</p></body></html>"#;

    let book = |css: &str| {
        common::make_epub(&[
            ("META-INF/container.xml", common::CONTAINER.as_bytes()),
            ("OEBPS/content.opf", OPF.as_bytes()),
            ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
            ("OEBPS/s.css", css.as_bytes()),
            ("OEBPS/bg.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]),
        ])
    };

    let good = read_epub(&book(r#"body { background: url("bg.jpg"); }"#));
    let bad = read_epub(&book(r#"body { background: url("gone.jpg"); }"#));

    assert!(
        verify(&good, &bad)
            .problems
            .iter()
            .any(|p| p.contains("url() to missing file") && p.contains("gone.jpg")),
        "expected the dead stylesheet reference to be reported, got {:?}",
        verify(&good, &bad).problems
    );
    // And it stays quiet when the book arrived that way.
    assert!(
        verify(&bad, &bad).problems.is_empty(),
        "a defect the book came with is not this run's fault"
    );
}

#[test]
fn visible_text_ignores_markup_and_head() {
    let a =
        "<html><head><title>T</title></head><body><p class=\"x\">Hello   world</p></body></html>";
    let b = "<html><head><title>Different</title></head><body><p>Hello world</p></body></html>";
    assert_eq!(visible_text(a), visible_text(b));
    assert_ne!(visible_text(a), visible_text("<body><p>Hello</p></body>"));
}

/// A pass that would leave a document malformed is rolled back whole.
///
/// This is the guarantee that matters most, because the tool knew: the closing
/// scan reported the damage and the book was written anyway. A reported error
/// beats a book nothing can open, so now the pass is undone and says so.
#[test]
fn a_pass_that_breaks_a_document_is_rolled_back() {
    use epubfix::book::Book;
    use epubfix::fixers::{Fixer, Outcome};
    use std::io::Cursor;

    /// Deletes the end tag of every paragraph, which is exactly the shape of
    /// the real bug: a replacement that is not a whole element.
    struct Vandal;
    impl Fixer for Vandal {
        fn name(&self) -> &'static str {
            "vandal"
        }
        fn codes(&self) -> &'static [&'static str] {
            &[]
        }
        fn description(&self) -> &'static str {
            "test double"
        }
        fn apply(&self, book: &mut Book) -> Outcome {
            for doc in book.markup_names() {
                if let Some(text) = book.text(&doc).map(str::to_owned) {
                    book.set_text(&doc, text.replace("</p>", ""));
                }
            }
            Outcome::change("broke everything")
        }
    }

    let before = common::epub2("<p>text</p>", "<p>more</p>");
    let mut book = Book::load(Cursor::new(before.clone())).unwrap();
    let outcome = epubfix::fix_book_with(&mut book, &[Box::new(Vandal)]);

    assert!(
        !outcome.has_changes(),
        "a rolled-back pass must not claim a change: {:?}",
        outcome.changes
    );
    assert!(
        outcome.findings.iter().any(|f| f.contains("vandal")),
        "got {:?}",
        outcome.findings
    );
    assert!(
        book.text("OEBPS/ch1.xhtml").unwrap().contains("</p>"),
        "the document must be back as it was"
    );
}

/// A book that arrives malformed is not this run's fault, and refusing to work
/// on it would refuse exactly the books that most need help.
#[test]
fn a_pass_touching_an_already_malformed_document_still_applies() {
    use epubfix::book::Book;
    use epubfix::fixers::{Fixer, Outcome};
    use std::io::Cursor;

    struct Tidy;
    impl Fixer for Tidy {
        fn name(&self) -> &'static str {
            "tidy"
        }
        fn codes(&self) -> &'static [&'static str] {
            &[]
        }
        fn description(&self) -> &'static str {
            "test double"
        }
        fn apply(&self, book: &mut Book) -> Outcome {
            for doc in book.markup_names() {
                if let Some(text) = book.text(&doc).map(str::to_owned) {
                    book.set_text(&doc, text.replace("bogus", "fixed"));
                }
            }
            Outcome::change("did a thing")
        }
    }

    // Arrives with an unclosed <b>, and stays that way: not ours to blame.
    let before = common::epub2("<p>bogus <b>text</p>", "<p>more</p>");
    let mut book = Book::load(Cursor::new(before)).unwrap();
    let outcome = epubfix::fix_book_with(&mut book, &[Box::new(Tidy)]);

    assert!(outcome.has_changes(), "got {:?}", outcome.findings);
    assert!(book.text("OEBPS/ch1.xhtml").unwrap().contains("fixed"));
}
