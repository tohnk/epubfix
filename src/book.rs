//! In-memory representation of an EPUB container.
//!
//! An EPUB is a zip archive. [`Book`] holds every entry in its original order,
//! plus a UTF-8 decoded copy of each entry a fixer is allowed to rewrite. Fixers
//! mutate the decoded text and record renames; nothing touches the disk until
//! [`Book::save`] repacks the archive.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::util::{MARKUP, TEXTUAL, ends_with_any};

/// One entry of the source archive, kept verbatim.
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
    pub is_dir: bool,
    pub compression: CompressionMethod,
    pub last_modified: Option<DateTime>,
}

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
