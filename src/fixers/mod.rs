//! The fixer registry.
//!
//! # Adding a new fix
//!
//! 1. Write a unit struct and `impl Fixer for` it, in one of the modules below
//!    (or a new one). `apply` mutates the [`Book`] and returns a human-readable
//!    line per change it made — return an empty `Vec` when there was nothing to do.
//! 2. Add it to the list in [`all`]. Order matters: fixers run top to bottom, so
//!    put anything that rewrites filenames or ids before the passes that read them.
//! 3. Add a case to `tests/fixtures.rs` covering both the broken and the
//!    already-clean input.
//!
//! A fixer must be a no-op on input it does not recognise. Reporting a change it
//! did not make is worse than missing one, because the caller uses a non-empty
//! result to decide whether to rewrite the file at all.

use crate::book::Book;

pub mod filenames;
pub mod ids;
pub mod ncx;
pub mod opf;

pub trait Fixer {
    /// Stable short name, used by `--only` and `--list`.
    fn name(&self) -> &'static str;

    /// The epubcheck message ids this pass is meant to silence.
    fn codes(&self) -> &'static [&'static str];

    /// One-line description of what it does.
    fn description(&self) -> &'static str;

    /// Repair `book`, returning one line per change made.
    fn apply(&self, book: &mut Book) -> Vec<String>;
}

/// Every fixer, in the order they run.
pub fn all() -> Vec<Box<dyn Fixer>> {
    vec![
        Box::new(opf::PackageVersion),
        Box::new(opf::SpinePageMap),
        Box::new(opf::FontMediaType),
        Box::new(ids::XmlIds),
        Box::new(filenames::UnsafeFilenames),
        Box::new(ncx::PlayOrder),
        Box::new(ncx::DtbUid),
    ]
}
