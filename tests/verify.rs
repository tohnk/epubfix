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
