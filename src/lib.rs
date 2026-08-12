//! Repair common EPUB 2 validation errors in place.
//!
//! The unit of work is a [`Book`], an EPUB loaded entirely into memory. A list of
//! [`Fixer`]s runs over it in order; each one reports the changes it made. If the
//! whole run produces no changes the file on disk is left completely untouched.
//!
//! See [`fixers`] for how to add a new repair.

pub mod book;
pub mod css;
pub mod entities;
pub mod fixers;
pub mod language;
pub mod markup;
pub mod migrate;
pub mod paths;
pub mod refs;
pub mod util;
pub mod verify;
pub mod version;

use std::fmt;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub use book::Book;
pub use fixers::tables::Presentation;
pub use fixers::{Fixer, Outcome};
pub use language::Policy as LanguagePolicy;

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
#[allow(
    clippy::struct_excessive_bools,
    reason = "flat CLI flags, one field per flag"
)]
pub struct Options {
    /// Report what would change without writing anything.
    pub dry_run: bool,
    /// Keep a `.bak` copy of the original next to it.
    pub backup: bool,
    /// Run only these fixers, by [`Fixer::name`]. Empty means all of them.
    pub only: Vec<String>,
    /// Force an upgrade to EPUB 3 even when the content does not require it.
    pub migrate_epub3: bool,
    /// Never change the declared version, only repair against it.
    pub keep_version: bool,
    /// What to do with presentational attributes the ruleset rejects.
    pub presentation: Presentation,
    /// When the tool may write a language it worked out for itself.
    pub language: LanguagePolicy,
    /// Keep an `<img>` whose file is proven absent, reporting it instead of
    /// removing it.
    pub keep_missing_images: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dry_run: false,
            backup: true,
            only: Vec::new(),
            migrate_epub3: false,
            keep_version: false,
            presentation: Presentation::Strip,
            language: LanguagePolicy::EnglishOnly,
            keep_missing_images: false,
        }
    }
}

/// Run every fixer over `book`.
pub fn fix_book(book: &mut Book) -> Outcome {
    fix_book_with(book, &fixers::all(&Options::default()))
}

/// The last phase of a run: everything that can only be decided once every
/// repair has landed.
///
/// Two things live here. **Derived declarations** — the manifest properties —
/// because several fixers can remove the very construct that earned one, so
/// computing them earlier means declaring a property the finished book does
/// not need. And **the closing scan**, which records what is still visibly
/// wrong, because "epubfix changed some bytes" and "this book is in good
/// order" are different claims and only the first is ours to make.
///
/// `derive_declarations` is false for an `--only` run, where writing something
/// the user did not ask for would be wrong. The scan runs either way: it
/// reports, it does not change anything.
///
/// Anything driving the pipeline by hand must call this after [`fix_book`];
/// `fix_file` does it for you. Both times a phase has been added here, the
/// test helpers silently skipped it by calling `fix_book` directly — which is
/// exactly what a library caller would have done.
pub fn finish_book(book: &mut Book, derive_declarations: bool) -> Outcome {
    let mut outcome = if derive_declarations {
        migrate::finalise_properties(book)
    } else {
        Outcome::none()
    };
    outcome.remaining = verify::remaining(book);
    outcome
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
    guarded(book, "EPUB 3 migration", |b| migrate::apply(b, true))
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
    guarded(book, "EPUB 3 conformance repair", |b| {
        migrate::apply(b, false)
    })
}

/// Move a book's declared version to match what it actually contains.
///
/// This is the default behaviour, and it runs in both directions. The
/// declaration is one attribute and the content is thousands of elements, so
/// when they disagree the declaration is what should move — rewriting a book to
/// satisfy a wrong attribute is backwards, and toward EPUB 2 it is not even
/// possible, since there is no XHTML 1.1 spelling of a nav document.
///
/// Only decisive evidence retags. A book whose content genuinely needs EPUB 3
/// is never downgraded, and a book with mixed evidence is left declared as it
/// is and repaired against that declaration instead.
pub fn retag_book(book: &mut Book) -> Outcome {
    let assessment = version::assess(book);
    let retag = version::decide(book, &assessment);
    let why = version::reason(&assessment, retag);

    let mut outcome = match retag {
        version::Retag::Upgrade => guarded(book, "EPUB 3 upgrade", |b| migrate::apply(b, true)),
        version::Retag::Downgrade => guarded(book, "EPUB 2 downgrade", migrate::downgrade),
        // Declaration already fits, or the evidence is mixed. Either way, make
        // the book satisfy whatever it currently claims to be.
        version::Retag::Keep => return conform_book(book),
    };

    if outcome.has_changes() {
        outcome.changes.insert(0, format!("retagged: {why}"));
    }
    outcome
}

/// Run a transformation against a clone, and keep it only if nothing was lost.
fn guarded<F>(book: &mut Book, what: &str, f: F) -> Outcome
where
    F: FnOnce(&mut Book) -> Outcome,
{
    let mut candidate = book.clone();
    let mut outcome = f(&mut candidate);
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
        book.begin_pass();
        let result = f.apply(book);
        let damaged = damaged_by(book);

        if damaged.is_empty() {
            outcome.merge(result);
            continue;
        }
        // The pass broke a document that parsed before it ran. Its other edits
        // may be perfectly good, but nothing here can tell which bytes were the
        // bad ones, so the whole pass is rolled back and its changes are not
        // claimed. The defect it meant to repair is still there, which is the
        // right outcome: a reported error beats a book nothing can open.
        for name in book
            .pass_changes()
            .map(|(n, _)| n.clone())
            .collect::<Vec<_>>()
        {
            book.revert(&name);
        }
        outcome.push_finding(format!(
            "the {} pass would have left {} malformed, so none of it was applied; \
             whatever it was meant to repair is still there",
            f.name(),
            damaged.join(", ")
        ));
    }
    outcome
}

/// Entries this pass turned from well-formed XML into not.
///
/// The check is differential, like every other gate here: a book that arrives
/// with an unclosed tag is not this pass's fault, and blaming it would make the
/// tool refuse exactly the books that most need help. Only XML entries are
/// asked, since a stylesheet is not XML and never was.
fn damaged_by(book: &Book) -> Vec<String> {
    let mut damaged: Vec<String> = book
        .pass_changes()
        .filter(|(name, _)| util::ends_with_any(name, util::XML))
        .filter(|(name, before)| {
            markup::well_formed(before).is_ok()
                && book
                    .text(name)
                    .is_some_and(|after| markup::well_formed(after).is_err())
        })
        .map(|(name, _)| name.clone())
        .collect();
    damaged.sort();
    damaged
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
        fixers::all(opts)
    } else {
        fixers::all(opts)
            .into_iter()
            .filter(|f| opts.only.iter().any(|n| n == f.name()))
            .collect()
    };

    let mut outcome = Outcome::none();
    if opts.migrate_epub3 {
        outcome.merge(migrate_book(&mut book));
        outcome.merge(conform_book(&mut book));
    } else if opts.keep_version {
        outcome.merge(conform_book(&mut book));
    } else {
        outcome.merge(retag_book(&mut book));
    }
    outcome.merge(fix_book_with(&mut book, &selected));
    outcome.merge(finish_book(&mut book, opts.only.is_empty()));
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
