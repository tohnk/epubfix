//! Repair common EPUB 2 validation errors in place.
//!
//! The unit of work is a [`Book`], an EPUB loaded entirely into memory. A list of
//! [`Fixer`]s runs over it in order; each one reports the changes it made. If the
//! whole run produces no changes the file on disk is left completely untouched.
//!
//! See [`fixers`] for how to add a new repair.

pub mod book;
pub mod fixers;
pub mod util;

use std::fmt;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub use book::Book;
pub use fixers::Fixer;

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
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dry_run: false,
            backup: true,
            only: Vec::new(),
        }
    }
}

/// Run every fixer over `book`, returning one line per change.
pub fn fix_book(book: &mut Book) -> Vec<String> {
    fix_book_with(book, &fixers::all())
}

/// Run a chosen set of fixers over `book`, in the order given.
pub fn fix_book_with(book: &mut Book, fixers: &[Box<dyn Fixer>]) -> Vec<String> {
    let mut changes = Vec::new();
    for f in fixers {
        changes.extend(f.apply(book));
    }
    changes
}

/// Repair one EPUB in place.
///
/// Returns the list of changes made — empty means the file was already clean and
/// was not rewritten. The replacement is staged in a sibling temporary file and
/// moved into place only once it has been written in full, so an interrupted run
/// cannot leave a truncated book behind.
pub fn fix_file(path: &Path, opts: &Options) -> Result<Vec<String>> {
    let mut book = Book::load(File::open(path)?)?;

    let selected: Vec<Box<dyn Fixer>> = if opts.only.is_empty() {
        fixers::all()
    } else {
        fixers::all()
            .into_iter()
            .filter(|f| opts.only.iter().any(|n| n == f.name()))
            .collect()
    };

    let changes = fix_book_with(&mut book, &selected);
    if changes.is_empty() || opts.dry_run {
        return Ok(changes);
    }

    let tmp = sibling(path, ".epubfix.tmp");
    let result = write_and_replace(&book, path, &tmp, opts.backup);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(changes)
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
