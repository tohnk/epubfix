//! Behaviour at the filesystem level: backups, dry runs, and folder scanning.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{clean_book, make_text_epub, with};
use epubfix::{Options, collect_epubs, fix_file};

/// A book that needs exactly one fix.
fn broken() -> Vec<u8> {
    let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="wrong"/></head>
  <navMap>
    <navPoint id="np1" playOrder="1"><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    make_text_epub(&with(clean_book(), "OEBPS/toc.ncx", ncx))
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn a_backup_is_written_and_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let original = broken();
    let book = write(dir.path(), "book.epub", &original);
    let bak = dir.path().join("book.epub.bak");

    let changes = fix_file(&book, &Options::default()).unwrap();
    assert_eq!(changes, vec!["synced NCX dtb:uid to OPF identifier"]);
    assert_eq!(
        fs::read(&bak).unwrap(),
        original,
        "the .bak is the original"
    );
    assert_ne!(fs::read(&book).unwrap(), original, "the book was rewritten");

    // Force a second round of real changes and check the pristine backup survives.
    fs::write(&book, broken()).unwrap();
    let changes = fix_file(&book, &Options::default()).unwrap();
    assert!(!changes.is_empty());
    assert_eq!(
        fs::read(&bak).unwrap(),
        original,
        "a second run must not clobber the pristine backup"
    );
}

#[test]
fn no_backup_leaves_no_bak() {
    let dir = tempfile::tempdir().unwrap();
    let book = write(dir.path(), "book.epub", &broken());
    let opts = Options {
        backup: false,
        ..Options::default()
    };

    assert!(!fix_file(&book, &opts).unwrap().is_empty());
    assert!(!dir.path().join("book.epub.bak").exists());
}

#[test]
fn dry_run_reports_without_touching_anything() {
    let dir = tempfile::tempdir().unwrap();
    let original = broken();
    let book = write(dir.path(), "book.epub", &original);
    let opts = Options {
        dry_run: true,
        ..Options::default()
    };

    let changes = fix_file(&book, &opts).unwrap();
    assert_eq!(changes, vec!["synced NCX dtb:uid to OPF identifier"]);
    assert_eq!(fs::read(&book).unwrap(), original, "file must be unchanged");
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no .bak and no leftover .tmp"
    );
}

#[test]
fn a_clean_book_is_not_rewritten_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let original = make_text_epub(&clean_book());
    let book = write(dir.path(), "book.epub", &original);

    assert!(fix_file(&book, &Options::default()).unwrap().is_empty());
    assert_eq!(
        fs::read(&book).unwrap(),
        original,
        "an already-clean book must keep its exact bytes"
    );
    assert!(!dir.path().join("book.epub.bak").exists());
}

#[test]
fn a_non_zip_file_fails_without_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let book = write(dir.path(), "book.epub", b"this is not a zip archive");

    let err = fix_file(&book, &Options::default()).unwrap_err();
    assert!(matches!(err, epubfix::Error::Zip(_)), "got {err:?}");
    assert_eq!(fs::read(&book).unwrap(), b"this is not a zip archive");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no leftovers");
}

#[test]
fn a_missing_file_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = fix_file(&dir.path().join("nope.epub"), &Options::default()).unwrap_err();
    assert!(matches!(err, epubfix::Error::Io(_)), "got {err:?}");
}

#[test]
fn scanning_finds_epubs_and_ignores_everything_else() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("sub")).unwrap();
    write(root, "b.epub", b"");
    write(root, "a.EPUB", b"");
    write(root, "notes.txt", b"");
    write(root, "b.epub.bak", b"");
    write(&root.join("sub"), "c.epub", b"");

    let flat = collect_epubs(root, false);
    let flat: Vec<&str> = flat
        .iter()
        .filter_map(|p| p.file_name()?.to_str())
        .collect();
    assert_eq!(
        flat,
        vec!["a.EPUB", "b.epub"],
        "sorted, case-insensitive ext, no .bak"
    );

    let deep = collect_epubs(root, true);
    assert_eq!(deep.len(), 3, "recursive should also pick up sub/c.epub");
    assert!(deep.iter().any(|p| p.ends_with("sub/c.epub")));
}

#[test]
fn only_selects_a_subset_of_fixers() {
    let dir = tempfile::tempdir().unwrap();
    let book = write(dir.path(), "book.epub", &broken());
    let opts = Options {
        only: vec!["opf-version".into()],
        ..Options::default()
    };

    let changes = fix_file(&book, &opts).unwrap();
    assert!(
        changes.is_empty(),
        "the uid fixer was not selected: {changes:?}"
    );
}
