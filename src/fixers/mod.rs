//! The fixer registry.
//!
//! # Adding a new fix
//!
//! 1. Write a unit struct and `impl Fixer for` it, in one of the modules below
//!    (or a new one). `apply` mutates the [`Book`] and returns an [`Outcome`].
//! 2. Add it to the list in [`all`]. Order matters: fixers run top to bottom.
//! 3. Add a case to `tests/fixers.rs` covering both the broken and the
//!    already-clean input.
//!
//! A fixer must be a no-op on input it does not recognise. Reporting a change it
//! did not make is worse than missing one, because the caller uses a non-empty
//! change list to decide whether to rewrite the file at all.
//!
//! When a fixer meets something it recognises as wrong but cannot repair safely,
//! it records a *finding* instead of guessing. Findings never modify the book;
//! they surface at the end of a run as "needs manual attention".

use crate::book::Book;

pub mod anchors;
pub mod attrs;
pub mod css_paths;
pub mod filenames;
pub mod ids;
pub mod legacy_html;
pub mod ncx;
pub mod nesting;
pub mod opf;
pub mod resources;
pub mod tables;

/// What a fixer did, and what it decided not to do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// One line per repair actually applied.
    pub changes: Vec<String>,
    /// One line per problem left alone for a human to judge.
    pub findings: Vec<String>,
}

impl Outcome {
    pub fn none() -> Self {
        Self::default()
    }

    /// An outcome carrying a single applied change.
    pub fn change(msg: impl Into<String>) -> Self {
        Outcome {
            changes: vec![msg.into()],
            findings: Vec::new(),
        }
    }

    /// An outcome carrying a single thing that needs a human.
    pub fn finding(msg: impl Into<String>) -> Self {
        Outcome {
            changes: Vec::new(),
            findings: vec![msg.into()],
        }
    }

    pub fn push_change(&mut self, msg: impl Into<String>) {
        self.changes.push(msg.into());
    }

    pub fn push_finding(&mut self, msg: impl Into<String>) {
        self.findings.push(msg.into());
    }

    pub fn merge(&mut self, other: Outcome) {
        self.changes.extend(other.changes);
        self.findings.extend(other.findings);
    }

    /// True if the book was modified. This, not `is_empty`, decides whether the
    /// file is rewritten — a book with only findings must be left untouched.
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.findings.is_empty()
    }
}

pub trait Fixer {
    /// Stable short name, used by `--only` and `--list`.
    fn name(&self) -> &'static str;

    /// The epubcheck message ids this pass is meant to silence.
    fn codes(&self) -> &'static [&'static str];

    /// One-line description of what it does.
    fn description(&self) -> &'static str;

    /// Repair `book`.
    fn apply(&self, book: &mut Book) -> Outcome;
}

/// Every fixer, in the order they run.
///
/// The ordering constraint that matters: everything which reads or rewrites
/// internal links runs *before* `filenames`, which renames the resources those
/// links point at. Reversing that would leave the reference index resolving
/// hrefs against names no longer in the archive.
pub fn all(opts: &crate::Options) -> Vec<Box<dyn Fixer>> {
    vec![
        Box::new(opf::PackageVersion),
        Box::new(opf::SpinePageMap),
        Box::new(opf::FontMediaType),
        Box::new(attrs::XhtmlNamespace),
        Box::new(ids::XmlIds),
        // After xml-ids, so a sanitised id that collided with an existing one
        // is seen as the duplicate it now is.
        Box::new(ids::ContentDuplicateIds),
        Box::new(attrs::DataAttributes),
        Box::new(tables::LegacyTableAttrs {
            mode: opts.presentation,
        }),
        Box::new(legacy_html::ImgAlt),
        Box::new(nesting::NestedAnchors),
        Box::new(nesting::MisplacedBlockquotes),
        Box::new(anchors::MisplacedAnchors),
        // Before dangling-resources, which can recover a link this would only
        // delete.
        Box::new(opf::GuideReferences),
        Box::new(resources::DanglingResources),
        Box::new(css_paths::CssPaths),
        Box::new(resources::BrokenFragments),
        Box::new(resources::DeadSchemes),
        // Before the renumbering, which closes the gaps removal leaves behind.
        Box::new(ncx::DeadNavEntries),
        Box::new(ncx::NcxDuplicateIds),
        Box::new(ncx::PageListAttrs),
        Box::new(filenames::UnsafeFilenames),
        Box::new(ncx::PlayOrder),
        Box::new(ncx::DtbUid),
        // Diagnostic only, and last: it reports on what everything else left behind.
        Box::new(legacy_html::VersionMismatch),
    ]
}
