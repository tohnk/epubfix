//! Helpers for building and inspecting throwaway EPUBs in memory.
//!
//! Each integration test file compiles this module separately, so any helper
//! a given file does not use reads as dead code there.
#![allow(dead_code)]

use std::io::{Cursor, Write};

use epubfix::Book;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Build a zip from `(name, bytes)` pairs, `mimetype` first and stored.
pub fn make_epub(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut w = ZipWriter::new(&mut buf);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        w.start_file("mimetype", stored).unwrap();
        w.write_all(b"application/epub+zip").unwrap();
        for (name, data) in files {
            if *name == "mimetype" {
                continue;
            }
            w.start_file(*name, deflated).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

/// Same, for the common case where every entry is text.
pub fn make_text_epub(files: &[(&str, &str)]) -> Vec<u8> {
    let owned: Vec<(&str, &[u8])> = files.iter().map(|(n, t)| (*n, t.as_bytes())).collect();
    make_epub(&owned)
}

/// Read an archive back, preserving entry order.
pub fn read_epub(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        out.push((name, data));
    }
    out
}

/// Load, fix, repack. Returns the change list and the resulting archive.
pub fn roundtrip(bytes: &[u8]) -> (Vec<String>, Vec<(String, Vec<u8>)>) {
    let mut book = Book::load(Cursor::new(bytes.to_vec())).unwrap();
    let changes = epubfix::fix_book(&mut book);
    let mut out = Cursor::new(Vec::new());
    book.save(&mut out).unwrap();
    (changes, read_epub(&out.into_inner()))
}

/// Text of one entry of a repacked archive.
pub fn entry(files: &[(String, Vec<u8>)], name: &str) -> String {
    let (_, data) = files
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no entry {name}; have {:?}", names(files)));
    String::from_utf8(data.clone()).unwrap()
}

pub fn names(files: &[(String, Vec<u8>)]) -> Vec<String> {
    files.iter().map(|(n, _)| n.clone()).collect()
}

pub fn has(files: &[(String, Vec<u8>)], name: &str) -> bool {
    files.iter().any(|(n, _)| n == name)
}

pub const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

/// A well-formed package document with nothing wrong with it.
pub const CLEAN_OPF: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;

/// A well-formed NCX whose uid already matches [`CLEAN_OPF`].
pub const CLEAN_NCX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="np1" playOrder="1"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;

pub const CLEAN_CH1: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><h1 id="intro">One</h1></body></html>"#;

/// A book with no defects, as a base to introduce one into.
pub fn clean_book() -> Vec<(&'static str, &'static str)> {
    vec![
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", CLEAN_OPF),
        ("OEBPS/toc.ncx", CLEAN_NCX),
        ("OEBPS/ch1.xhtml", CLEAN_CH1),
    ]
}

/// Replace the contents of one entry in a book description.
pub fn with(
    files: Vec<(&'static str, &'static str)>,
    name: &str,
    text: &'static str,
) -> Vec<(&'static str, &'static str)> {
    let mut out = files;
    for f in &mut out {
        if f.0 == name {
            f.1 = text;
            return out;
        }
    }
    panic!("no such entry {name}");
}
