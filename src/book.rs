//! In-memory representation of an EPUB container.
//!
//! An EPUB is a zip archive. [`Book`] holds every entry in its original order,
//! plus a UTF-8 decoded copy of each entry a fixer is allowed to rewrite. Fixers
//! mutate the decoded text and record renames; nothing touches the disk until
//! [`Book::save`] repacks the archive.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, Write};
use std::sync::{Arc, LazyLock};

use regex::Regex;
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::refs::ReferenceIndex;
use crate::util::{MARKUP, TEXTUAL, ends_with_any};

/// One entry of the source archive, kept verbatim.
/// Ceilings on what one book may unpack to.
///
/// Generous by design: the largest book in a 179-book library is under 30 MB,
/// and the point is not to police size but to fail with a message instead of
/// an out-of-memory kill. A hostile archive can declare a compression ratio of
/// a thousand to one, and nothing downstream gets a chance to object.
const MAX_ENTRY: u64 = 100 * 1024 * 1024;
const MAX_TOTAL: u64 = 500 * 1024 * 1024;

/// And on how many of them there may be.
///
/// The byte ceilings above do not bound an archive of a million empty entries:
/// nothing decompresses, so nothing counts against them, while each one still
/// costs a name and a record. The largest book in a 179-book library has 480
/// entries.
const MAX_ENTRIES: usize = 100_000;

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    /// The entry's bytes exactly as they arrived.
    ///
    /// Shared rather than owned, because [`Book`] is cloned wholesale by
    /// [`crate::guarded`] to try a migration on a copy and keep it only if
    /// nothing was lost. A book is mostly images, and every one of those clones
    /// duplicated every image: on a 100 MB illustrated book that is 100 MB of
    /// memcpy per gated pass, to compare text nobody has touched. An `Arc`
    /// makes the clone a pointer bump. The bytes are never mutated in place —
    /// a rewritten entry is a new `String` in `texts` — so there is nothing to
    /// make copy-on-write.
    pub data: Arc<[u8]>,
    pub is_dir: bool,
    pub compression: CompressionMethod,
    pub last_modified: Option<DateTime>,
}

#[derive(Clone)]
pub struct Book {
    entries: Vec<Entry>,
    /// Entry names in archive order, so every pass is deterministic.
    order: Vec<String>,
    /// Decoded contents of the textual entries, keyed by *original* name.
    texts: HashMap<String, String>,
    /// Pending `old name -> new name`, applied when the archive is repacked.
    renames: BTreeMap<String, String>,
    /// The same mapping read backwards, so a lookup by the name a document
    /// *will* have finds the entry it is still stored under. See [`Book::text`].
    renamed_from: HashMap<String, String>,
    opf: Option<String>,
    ncx: Option<String>,
    /// What the current pass overwrote, so it can be rolled back alone.
    undo: HashMap<String, String>,
    /// Entries that arrived as UTF-16 and were decoded on the way in.
    transcoded: Vec<String>,
    /// References a fixer has deliberately reserved for a person.
    reserved: HashSet<(String, String)>,
    markup_names_cache: std::cell::RefCell<Option<Vec<String>>>,
}

/// Decode UTF-16 with a byte-order mark, which is the only form that turns up:
/// an EPUB entry without one is UTF-8 by definition, so a BOM is what makes
/// this unambiguous rather than a guess about odd-looking bytes.
fn from_utf16(data: &[u8]) -> Option<String> {
    let (rest, big) = match data.get(..2)? {
        [0xFE, 0xFF] => (&data[2..], true),
        [0xFF, 0xFE] => (&data[2..], false),
        _ => return None,
    };
    if rest.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = rest
        .chunks_exact(2)
        .map(|c| {
            if big {
                u16::from_be_bytes([c[0], c[1]])
            } else {
                u16::from_le_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16(&units).ok()
}

impl Book {
    /// Read an entire archive into memory.
    pub fn load<R: Read + Seek>(reader: R) -> zip::result::ZipResult<Self> {
        let mut zip = ZipArchive::new(reader)?;
        // The entry count comes from the archive's own central directory and
        // nothing has verified it yet, so it is a hint, not a promise. Reserve
        // for a big book and let the vector grow if the archive really is one.
        let mut entries = Vec::with_capacity(zip.len().min(16_384));
        if zip.len() > MAX_ENTRIES {
            return Err(ZipError::InvalidArchive(
                format!("the archive declares more than {MAX_ENTRIES} entries").into(),
            ));
        }
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            let name = f.name().to_string();
            let is_dir = f.is_dir();
            let compression = f.compression();
            let last_modified = f.last_modified();
            let mut data = Vec::new();
            if !is_dir {
                // A ratio-bomb entry is the one input that can take the whole
                // process down: `read_to_end` on a 50 GB payload declared by a
                // 2 MB archive is an allocation failure, not an error anyone
                // can catch, and it ends a whole library sweep rather than one
                // book. Reading one byte past the ceiling is enough to know.
                let mut limited = (&mut f).take(MAX_ENTRY + 1);
                limited.read_to_end(&mut data)?;
                if data.len() as u64 > MAX_ENTRY {
                    return Err(ZipError::InvalidArchive(
                        format!("{name} unpacks to more than {} MB", MAX_ENTRY >> 20).into(),
                    ));
                }
                total += data.len() as u64;
                if total > MAX_TOTAL {
                    return Err(ZipError::InvalidArchive(
                        format!("the book unpacks to more than {} MB", MAX_TOTAL >> 20).into(),
                    ));
                }
            }
            entries.push(Entry {
                name,
                data: data.into(),
                is_dir,
                compression,
                last_modified,
            });
        }

        let order: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();

        // UTF-8 first, then UTF-16 by its byte-order mark. Anything that is
        // neither is left byte-for-byte alone.
        //
        // # The UTF-8 byte-order mark has to come off
        //
        // Not for tidiness. quick-xml reports buffer positions relative to the
        // text *after* the mark, so with a `\u{FEFF}` still on the front every
        // span the scanner produces is three bytes out and `dissect` reads the
        // element name from the wrong offset. The result is a node list of the
        // right length in which every name is the empty string — so the file
        // matches nothing, and **every fixer silently skips it**.
        //
        // That is how a Kodansha *Wild Sheep Chase* and three Dune books came to
        // report "the NCX has no navMap": their NCX has 58 navPoints and a
        // perfectly good `<navMap>`, and not one element of it was visible.
        //
        // Removing it is safe and measured: epubcheck scores both books
        // identically with the mark and without it. And because the mark is
        // stripped on the way in, `verify` compares two texts that never had
        // one, so the preservation gate is unaffected.
        let mut texts = HashMap::new();
        let mut transcoded = Vec::new();
        for e in &entries {
            if e.is_dir || !ends_with_any(&e.name, TEXTUAL) {
                continue;
            }
            if let Ok(s) = std::str::from_utf8(&e.data) {
                texts.insert(e.name.clone(), s.trim_start_matches('\u{FEFF}').to_owned());
            } else if let Some(s) = from_utf16(&e.data) {
                texts.insert(e.name.clone(), s);
                transcoded.push(e.name.clone());
            }
        }

        let find = |ext: &str| {
            order
                .iter()
                .find(|n| n.to_ascii_lowercase().ends_with(ext) && texts.contains_key(*n))
                .cloned()
        };
        let opf = find(".opf");
        let ncx = find(".ncx");

        Ok(Book {
            entries,
            order,
            texts,
            renames: BTreeMap::new(),
            renamed_from: HashMap::new(),
            opf,
            ncx,
            undo: HashMap::new(),
            transcoded,
            reserved: HashSet::new(),
            markup_names_cache: std::cell::RefCell::new(None),
        })
    }

    /// Reserve one reference for a person to decide about.
    ///
    /// A *finding* says "I can see a repair here and will not guess at it". The
    /// trap is that a later, blunter pass can then take the choice away — and
    /// the blunter the repair, the more likely it silences exactly the thing the
    /// person was being asked to look at.
    ///
    /// The case that made this necessary: `orphan-links` can repoint a link
    /// whose anchor was discarded at the heading its own text names, and
    /// declines when *two* headings carry those words, because a contents page
    /// pointing at the wrong chapter is worse than one pointing nowhere. It
    /// reports, and the person now knows which two headings clash — they can
    /// open the book and tell which was meant in a second. If
    /// `dangling-resources` then unlinks it for being unresolvable, the report
    /// still arrives but the `href` recording the intent is gone, and the repair
    /// a person could have made has been made impossible.
    ///
    /// So the distinction is not "can this be resolved" but **"could anyone
    /// resolve it"**. Where candidate targets exist and only a person can choose
    /// between them, the construct is left exactly as it is.
    pub fn reserve(&mut self, doc: &str, reference: &str) {
        self.reserved
            .insert((doc.to_string(), reference.to_string()));
    }

    /// Has some earlier pass reserved this reference for a person?
    pub fn is_reserved(&self, doc: &str, reference: &str) -> bool {
        // Scanned rather than hashed: the set is empty on almost every book and
        // never larger than a handful, and this way the check allocates nothing.
        self.reserved
            .iter()
            .any(|(d, r)| d == doc && r == reference)
    }

    /// Major EPUB version from the package document, e.g. `2` or `3`.
    ///
    /// This decides which ruleset a content document is validated against, and
    /// so which legacy markup is actually an error. Defaults to 2 when there is
    /// no readable OPF, matching what epubcheck assumes for a bare EPUB.
    pub fn epub_version(&self) -> u32 {
        static VER: LazyLock<Regex> =
            LazyLock::new(|| crate::util::re(r#"<package\b[^>]*?\bversion="(\d+)"#));
        self.opf_text()
            .and_then(|t| VER.captures(t))
            .and_then(|c| c[1].parse().ok())
            .unwrap_or(2)
    }

    /// Names of the content documents, in archive order.
    ///
    /// The extension is a good guess and the manifest is the fact, so both
    /// count. A Kobo build of *Essays and Aphorisms* ships an empty XHTML
    /// document called `page-map.xml`, declared `application/xhtml+xml` and
    /// listed in the spine: epubcheck validates it as a content document and
    /// reports `element "body" incomplete`, while every fixer here skipped it
    /// on sight of the extension. Deciding this the way epubcheck decides it is
    /// the only way the two can agree.
    pub fn markup_names(&self) -> Vec<String> {
        if let Some(names) = self.markup_names_cache.borrow().as_ref() {
            return names.clone();
        }
        let declared = self.declared_documents();
        let names: Vec<String> = self
            .order
            .iter()
            .filter(|n| {
                self.texts.contains_key(*n)
                    && (ends_with_any(n, MARKUP) || declared.contains(n.as_str()))
            })
            .cloned()
            .collect();
        *self.markup_names_cache.borrow_mut() = Some(names.clone());
        names
    }

    /// Archive names the manifest declares to be XHTML content documents.
    fn declared_documents(&self) -> std::collections::HashSet<String> {
        static ITEM: LazyLock<Regex> = LazyLock::new(|| crate::util::re(r"<item\b[^>]*>"));
        let Some(opf) = self.opf_name() else {
            return std::collections::HashSet::new();
        };
        let Some(text) = self.opf_text() else {
            return std::collections::HashSet::new();
        };
        ITEM.find_iter(text)
            .filter(|m| m.as_str().contains("application/xhtml+xml"))
            .filter_map(|m| {
                let tag = m.as_str();
                let at = tag.find("href=\"")? + 6;
                let href = &tag[at..at + tag[at..].find('"')?];
                crate::refs::resolve_href(opf, href).map(|(target, _)| target)
            })
            .collect()
    }

    /// Index every internal link in the book, so fixers can tell whether an
    /// anchor is live before touching it.
    pub fn reference_index(&self) -> ReferenceIndex {
        let mut idx = ReferenceIndex::default();
        for name in &self.order {
            if let Some(text) = self.texts.get(name) {
                idx.add_document(name, text);
            }
        }
        idx
    }

    /// Name of the package document, if one was found and is readable.
    pub fn opf_name(&self) -> Option<&str> {
        self.opf.as_deref()
    }

    /// Name of the NCX table of contents, if one was found and is readable.
    pub fn ncx_name(&self) -> Option<&str> {
        self.ncx.as_deref()
    }

    pub fn opf_text(&self) -> Option<&str> {
        self.opf.as_deref().and_then(|n| self.text(n))
    }

    pub fn ncx_text(&self) -> Option<&str> {
        self.ncx.as_deref().and_then(|n| self.text(n))
    }

    /// The decoded contents of an entry, by either of its names.
    ///
    /// A rename is *pending* until the archive is repacked: `texts` stays keyed
    /// by the name the entry arrived under, because that is what [`Book::save`]
    /// writes from. But `filenames` rewrites the references at the same time,
    /// so every pass after it resolves an href to the name the file is *going*
    /// to have — and asking for that name found nothing at all.
    ///
    /// Measured, on an EPUB 3 book whose `ch 1.xhtml` holds an `<svg>`: the
    /// rename fixes four RSC-020s, and `finalise_properties` then resolves
    /// `ch_1.xhtml`, gets `None`, and silently skips the document, leaving
    /// `OPF-014 The property "svg" should be declared`. The identical book
    /// under a safe filename validates clean. Three passes had the same blind
    /// spot — `finalise_properties`, `ncx-spine-order` and `ncx-nav-order` —
    /// and none of them could have known, so the lookup is what has to give.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.texts
            .get(name)
            .or_else(|| self.renamed_from.get(name).and_then(|o| self.texts.get(o)))
            .map(String::as_str)
    }

    pub fn set_text(&mut self, name: &str, value: String) {
        // Written back under the name the entry is stored as, for the same
        // reason [`Book::text`] reads through one. `texts.get_mut` on the new
        // name simply missed, so the edit was dropped without a word.
        let name = &self
            .renamed_from
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())[..];
        if let Some(slot) = self.texts.get_mut(name) {
            if !self.undo.contains_key(name) {
                self.undo.insert(name.to_string(), slot.clone());
            }
            *slot = value;
            if Some(name) == self.opf.as_deref() {
                self.markup_names_cache.replace(None);
            }
        }
    }

    /// Start a new pass, forgetting how to undo the previous one.
    ///
    /// The undo log exists so a pass that damages a document can be rolled
    /// back on its own, without discarding the work of the passes before it.
    pub fn begin_pass(&mut self) {
        self.undo.clear();
    }

    /// Entries this pass has rewritten, with what they held before it started.
    pub fn pass_changes(&self) -> impl Iterator<Item = (&String, &String)> {
        self.undo.iter()
    }

    /// Put one entry back to what it held at the start of the pass.
    ///
    /// Restoring the OPF drops the markup-name cache for the same reason
    /// writing it does: the cache is derived from the manifest, and a pass that
    /// edited the manifest, read the cache back, and was then rolled back would
    /// otherwise leave the derived answer describing a package that no longer
    /// exists.
    pub fn revert(&mut self, name: &str) {
        // Through the rename mapping, as [`Book::text`] and [`Book::set_text`]
        // both are. Every caller today passes a storage name — `pass_changes`
        // hands back `undo`'s own keys — so this is a landmine rather than a
        // live bug, and the asymmetry is exactly the kind that stops being
        // theoretical the moment someone reverts a document by the name the
        // markup calls it.
        let name = &self
            .renamed_from
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())[..];
        if let Some(before) = self.undo.remove(name)
            && let Some(slot) = self.texts.get_mut(name)
        {
            *slot = before;
            if Some(name) == self.opf.as_deref() {
                self.markup_names_cache.replace(None);
            }
        }
    }

    /// What the OCF `mimetype` entry looks like in the archive as it arrived:
    /// `(is it first, is it stored uncompressed, are its bytes exactly right)`.
    ///
    /// [`Book::save`] always writes a correct one, so this exists only so a
    /// fixer can notice that it *needed* writing. Without that the book is
    /// never rewritten and the repair never reaches disk.
    pub fn mimetype_state(&self) -> (bool, bool, bool) {
        let first = self.entries.first().is_some_and(|e| e.name == "mimetype");
        let entry = self.entries.iter().find(|e| e.name == "mimetype");
        let stored = entry.is_some_and(|e| e.compression == CompressionMethod::Stored);
        let exact = entry.is_some_and(|e| &*e.data == b"application/epub+zip");
        (first, stored, exact)
    }

    /// Entries that arrived as UTF-16 and are now held as text.
    ///
    /// [`Book::save`] writes every text entry as UTF-8, so these are already
    /// repaired in memory — but as with the mimetype, nothing would notice, and
    /// a book whose only defect was its encoding would never be rewritten.
    pub fn transcoded(&self) -> &[String] {
        &self.transcoded
    }

    /// Entry names in archive order.
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// Visit every rewritable text in archive order.
    pub fn for_each_text<F: FnMut(&str, &mut String)>(&mut self, mut f: F) {
        for name in &self.order {
            if let Some(t) = self.texts.get_mut(name) {
                f(name, t);
            }
        }
    }

    /// Add a new entry to the archive, after the existing ones.
    ///
    /// Used by the EPUB 3 migration to introduce a nav document; ordinary
    /// fixers have no business creating files.
    pub fn add_text_entry(&mut self, name: &str, text: String) {
        if self.texts.contains_key(name) {
            self.set_text(name, text);
            return;
        }
        self.entries.push(Entry {
            name: name.to_string(),
            data: Arc::from(text.as_bytes()),
            is_dir: false,
            compression: CompressionMethod::Deflated,
            last_modified: None,
        });
        self.order.push(name.to_string());
        self.texts.insert(name.to_string(), text);
        self.markup_names_cache.replace(None);
    }

    /// Schedule `old` to be written out under a new name.
    pub fn rename(&mut self, old: &str, new: String) {
        self.renamed_from.insert(new.clone(), old.to_string());
        self.renames.insert(old.to_string(), new);
    }

    pub fn renames(&self) -> &BTreeMap<String, String> {
        &self.renames
    }

    /// True if any entry (after pending renames) is already called `name`.
    pub fn name_taken(&self, name: &str) -> bool {
        self.order.iter().any(|n| n == name) || self.renames.values().any(|n| n == name)
    }

    /// Repack the archive.
    ///
    /// `mimetype` is written first and uncompressed, as OCF requires; every other
    /// entry keeps its original order, compression method and timestamp.
    pub fn save<W: Write + Seek>(&self, writer: W) -> zip::result::ZipResult<()> {
        let mut out = ZipWriter::new(writer);

        let mimetype = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(DateTime::default());
        out.start_file("mimetype", mimetype)?;
        out.write_all(b"application/epub+zip")?;

        for e in &self.entries {
            if e.is_dir || e.name == "mimetype" {
                continue;
            }
            let mut options = SimpleFileOptions::default().compression_method(e.compression);
            if let Some(ts) = e.last_modified {
                options = options.last_modified_time(ts);
            }
            let name = self.renames.get(&e.name).unwrap_or(&e.name);
            out.start_file(name.as_str(), options)?;
            match self.texts.get(&e.name) {
                Some(t) => out.write_all(t.as_bytes())?,
                None => out.write_all(&e.data)?,
            }
        }

        out.finish()?;
        Ok(())
    }
}
