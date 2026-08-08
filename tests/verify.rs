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
fn the_harness_notices_a_broken_link() {
    // Not a fixer bug — a hand-built book that is already broken, proving the
    // link check actually fires rather than passing vacuously.
    let broken = epub2(
        "<p id=\"a\">text</p>",
        "<p><a href=\"ch1.xhtml#nope\">bad</a></p>",
    );
    let files = read_epub(&broken);
    let r = verify(&files, &files);
    assert!(
        r.problems.iter().any(|p| p.contains("no such id exists")),
        "expected a dangling-fragment problem, got {:?}",
        r.problems
    );
}

#[test]
fn the_harness_notices_a_duplicate_id() {
    let dup = epub2("<p id=\"a\">one</p><p id=\"a\">two</p>", "<p>x</p>");
    let files = read_epub(&dup);
    let r = verify(&files, &files);
    assert!(
        r.problems.iter().any(|p| p.contains("duplicate id")),
        "expected a duplicate-id problem, got {:?}",
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
