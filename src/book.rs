//! In-memory representation of an EPUB container.
//!
//! An EPUB is a zip archive. [`Book`] holds every entry in its original order,
//! plus a UTF-8 decoded copy of each entry a fixer is allowed to rewrite. Fixers
//! mutate the decoded text and record renames; nothing touches the disk until
//! [`Book::save`] repacks the archive.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, Write};
use std::sync::LazyLock;

use regex::Regex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::refs::ReferenceIndex;
use crate::util::{MARKUP, TEXTUAL, ends_with_any};

/// One entry of the source archive, kept verbatim.
#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
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
    opf: Option<String>,
    ncx: Option<String>,
}

impl Book {
    /// Read an entire archive into memory.
    pub fn load<R: Read + Seek>(reader: R) -> zip::result::ZipResult<Self> {
        let mut zip = ZipArchive::new(reader)?;
        let mut entries = Vec::with_capacity(zip.len());
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            let name = f.name().to_string();
            let is_dir = f.is_dir();
            let compression = f.compression();
            let last_modified = f.last_modified();
            let mut data = Vec::new();
            if !is_dir {
                f.read_to_end(&mut data)?;
            }
            entries.push(Entry {
                name,
                data,
                is_dir,
                compression,
                last_modified,
            });
        }

        let order: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();

        // Anything that fails to decode as UTF-8 is left byte-for-byte alone.
        let mut texts = HashMap::new();
        for e in &entries {
            if !e.is_dir
                && ends_with_any(&e.name, TEXTUAL)
                && let Ok(s) = std::str::from_utf8(&e.data)
            {
                texts.insert(e.name.clone(), s.to_owned());
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
            opf,
            ncx,
        })
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
        let declared = self.declared_documents();
        self.order
            .iter()
            .filter(|n| {
                self.texts.contains_key(*n)
                    && (ends_with_any(n, MARKUP) || declared.contains(n.as_str()))
            })
            .cloned()
            .collect()
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

    pub fn text(&self, name: &str) -> Option<&str> {
        self.texts.get(name).map(String::as_str)
    }

    pub fn set_text(&mut self, name: &str, value: String) {
        if let Some(slot) = self.texts.get_mut(name) {
            *slot = value;
        }
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

    /// Visit every content document (`.html` / `.xhtml` / `.htm`) in archive order.
    pub fn for_each_markup<F: FnMut(&str, &mut String)>(&mut self, mut f: F) {
        for name in &self.order {
            if !ends_with_any(name, MARKUP) {
                continue;
            }
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
            data: text.as_bytes().to_vec(),
            is_dir: false,
            compression: CompressionMethod::Deflated,
            last_modified: None,
        });
        self.order.push(name.to_string());
        self.texts.insert(name.to_string(), text);
    }

    /// Schedule `old` to be written out under a new name.
    pub fn rename(&mut self, old: &str, new: String) {
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
