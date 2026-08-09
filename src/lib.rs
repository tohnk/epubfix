//! Repair common EPUB 2 validation errors in place.
//!
//! The unit of work is a [`Book`], an EPUB loaded entirely into memory. A list of
//! [`Fixer`]s runs over it in order; each one reports the changes it made. If the
//! whole run produces no changes the file on disk is left completely untouched.
//!
//! See [`fixers`] for how to add a new repair.

pub mod book;
pub mod entities;
pub mod fixers;
pub mod markup;
pub mod migrate;
pub mod refs;
pub mod util;
pub mod verify;

use std::fmt;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub use book::Book;
pub use fixers::{Fixer, Outcome};

#[derive(Debug)]
pub enum Error {
    /// Reading, writing, or replacing the file on disk.
    Io(std::io::Error),
    /// The file is not a readable zip archive.
    Zip(zip::result::ZipError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Zip(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Zip(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::Zip(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Options {
    /// Report what would change without writing anything.
    pub dry_run: bool,
    /// Keep a `.bak` copy of the original next to it.
    pub backup: bool,
    /// Run only these fixers, by [`Fixer::name`]. Empty means all of them.
    pub only: Vec<String>,
    /// Convert an EPUB 2 book to EPUB 3 before running the fixers.
    ///
    /// Off by default: migration changes the file's format identity, and some
    /// older reading systems are EPUB 2 only. That is a policy choice, not
    /// something the tool can compute.
    pub migrate_epub3: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dry_run: false,
            backup: true,
            only: Vec::new(),
            migrate_epub3: false,
        }
    }
}

/// Run every fixer over `book`.
pub fn fix_book(book: &mut Book) -> Outcome {
    fix_book_with(book, &fixers::all())
}

/// Convert `book` to EPUB 3, but only if the result preserves everything.
///
/// Runs on a clone and keeps it only when [`verify::check`] is satisfied, so a
/// migration that would drop a link, lose an id or move text aborts and leaves
/// the book exactly as it was. Returns the outcome either way.
///
/// This runs *before* the fixer registry, because the EPUB version decides what
/// several of the fixers should do — `legacy-table-attrs` in particular strips a
/// much larger set under HTML5 rules.
pub fn migrate_book(book: &mut Book) -> Outcome {
    if book.epub_version() >= 3 {
        return Outcome::none();
    }
    guarded(book, true, "EPUB 3 migration")
}

/// Repair a book that already *declares* EPUB 3 but was written as EPUB 2.
///
/// This is the mirror of [`migrate_book`], and it runs by default rather than
/// behind a flag. The distinction is deliberate: changing a book's declared
/// version is a policy choice, but a book carrying an XHTML 1.1 DOCTYPE, bare
/// `&mdash;`, `opf:role` attributes and no nav document while claiming to be
/// EPUB 3 is simply broken against its own declaration. Repairing that is
/// ordinary work, and one of those errors — the undeclared entity — is fatal.
///
/// The version itself is never touched here.
pub fn conform_book(book: &mut Book) -> Outcome {
    if book.epub_version() < 3 {
        return Outcome::none();
    }
    guarded(book, false, "EPUB 3 conformance repair")
}

/// Run the EPUB 3 work against a clone, and keep it only if nothing was lost.
fn guarded(book: &mut Book, bump_version: bool, what: &str) -> Outcome {
    let mut candidate = book.clone();
    let mut outcome = migrate::apply(&mut candidate, bump_version);
    if !outcome.has_changes() {
        return outcome;
    }

    let problems = verify::check(book, &candidate);
    if problems.is_empty() {
        *book = candidate;
        return outcome;
    }

    // Something would have been lost. Keep the findings, drop the changes.
    let mut aborted = Outcome::none();
    aborted.push_finding(format!(
        "{what} was abandoned because it would not have preserved the book ({})",
        problems.join("; ")
    ));
    aborted.findings.append(&mut outcome.findings);
    aborted
}

/// Run a chosen set of fixers over `book`, in the order given.
pub fn fix_book_with(book: &mut Book, fixers: &[Box<dyn Fixer>]) -> Outcome {
    let mut outcome = Outcome::none();
    for f in fixers {
        outcome.merge(f.apply(book));
    }
    outcome
}

/// Repair one EPUB in place.
///
/// Returns what was changed and what needs a human. No changes means the file
/// was already clean and was not rewritten — a book that produced only findings
/// is left untouched too. The replacement is staged in a sibling temporary file
/// and moved into place only once it has been written in full, so an interrupted
/// run cannot leave a truncated book behind.
pub fn fix_file(path: &Path, opts: &Options) -> Result<Outcome> {
    let mut book = Book::load(File::open(path)?)?;

    let selected: Vec<Box<dyn Fixer>> = if opts.only.is_empty() {
        fixers::all()
    } else {
        fixers::all()
            .into_iter()
            .filter(|f| opts.only.iter().any(|n| n == f.name()))
            .collect()
    };

    let mut outcome = Outcome::none();
    if opts.migrate_epub3 {
        outcome.merge(migrate_book(&mut book));
    }
    // Always, flag or not: a book must at least satisfy the version it claims.
    outcome.merge(conform_book(&mut book));
    outcome.merge(fix_book_with(&mut book, &selected));
    if !outcome.has_changes() || opts.dry_run {
        return Ok(outcome);
    }

    let tmp = sibling(path, ".epubfix.tmp");
    let result = write_and_replace(&book, path, &tmp, opts.backup);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(outcome)
}

fn write_and_replace(book: &Book, path: &Path, tmp: &Path, backup: bool) -> Result<()> {
    book.save(BufWriter::new(File::create(tmp)?))?;

    if backup {
        let bak = sibling(path, ".bak");
        // Never overwrite an existing backup: on a second run that would replace
        // the pristine original with an already-modified copy.
        if !bak.exists() {
            fs::copy(path, &bak)?;
        }
    }
    fs::rename(tmp, path)?;
    Ok(())
}

/// `path` with `suffix` appended to the full filename.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

/// Every `.epub` in `dir`, sorted. Descends into subdirectories when `recursive`.
pub fn collect_epubs(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_into(dir, recursive, &mut out);
    out.sort();
    out
}

fn collect_into(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if recursive {
                collect_into(&p, true, out);
            }
        } else if is_epub(&p) {
            out.push(p);
        }
    }
}

fn is_epub(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("epub"))
}
