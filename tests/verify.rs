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
