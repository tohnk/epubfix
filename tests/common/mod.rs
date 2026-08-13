//! Helpers for building and inspecting throwaway EPUBs in memory.
//!
//! Each integration test file compiles this module separately, so any helper
//! a given file does not use reads as dead code there.
#![allow(dead_code)]

pub mod verify;

use std::fmt::Write as _;
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
    let (outcome, files) = roundtrip_full(bytes);
    (outcome.changes, files)
}

/// As [`roundtrip`], but keeps the findings too.
///
/// Mirrors what `fix_file` does on a default run: version retagging, then the
/// fixers. If these drift apart the tests stop testing the real pipeline.
pub fn roundtrip_full(bytes: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let mut book = Book::load(Cursor::new(bytes.to_vec())).unwrap();
    let mut outcome = epubfix::retag_book(&mut book);
    outcome.merge(epubfix::fix_book(&mut book));
    outcome.merge(epubfix::finish_book(&mut book, true));
    let mut out = Cursor::new(Vec::new());
    book.save(&mut out).unwrap();
    (outcome, read_epub(&out.into_inner()))
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

/// Add an entry to a book description.
pub fn plus(
    files: Vec<(&'static str, &'static str)>,
    name: &'static str,
    text: &'static str,
) -> Vec<(&'static str, &'static str)> {
    let mut out = files;
    out.push((name, text));
    out
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

// ---------------------------------------------------------------------------
// Version-specific book builders, for the content-document fixers.
// ---------------------------------------------------------------------------

fn opf2() -> String {
    r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/><itemref idref="ch2"/></spine>
</package>"#
        .to_string()
}

fn opf3() -> String {
    r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="nav"/><itemref idref="ch1"/><itemref idref="ch2"/></spine>
</package>"#
        .to_string()
}

const NAV3: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title><meta charset="utf-8"/></head>
<body><nav epub:type="toc"><ol><li><a href="ch1.xhtml">One</a></li></ol></nav></body></html>"#;

fn chapter(body: &str, v3: bool) -> String {
    // A <meta name="calibre:cover"> rides along in every chapter: it is valid
    // markup that an earlier version of the id fixer corrupted.
    if v3 {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
             <head><title>T</title><meta charset=\"utf-8\"/>\
             <meta name=\"calibre:cover\" content=\"true\"/></head>\n\
             <body>\n{body}\n</body></html>"
        )
    } else {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \
             \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n\
             <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
             <head><title>T</title>\
             <meta name=\"calibre:cover\" content=\"true\"/></head>\n\
             <body>\n{body}\n</body></html>"
        )
    }
}

/// An EPUB 2 book with two content documents carrying the given bodies.
pub fn epub2(ch1_body: &str, ch2_body: &str) -> Vec<u8> {
    build(false, ch1_body, ch2_body)
}

/// As [`roundtrip_full`], but with non-default options — the presentation
/// mode, in practice.
pub fn roundtrip_with(
    bytes: &[u8],
    opts: &epubfix::Options,
) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let mut book = Book::load(Cursor::new(bytes.to_vec())).unwrap();
    let mut outcome = epubfix::retag_book(&mut book);
    outcome.merge(epubfix::fix_book_with(
        &mut book,
        &epubfix::fixers::all(opts),
    ));
    outcome.merge(epubfix::finish_book(&mut book, true));
    let mut out = Cursor::new(Vec::new());
    book.save(&mut out).unwrap();
    (outcome, read_epub(&out.into_inner()))
}

/// An EPUB 3 book whose chapter links a stylesheet, for the CSS-override
/// report: whether stripping an attribute is visible depends on what the
/// book's own styles already say.
pub fn epub3_with_css(ch1_body: &str, css: &str) -> Vec<u8> {
    let opf = opf3().replace(
        r#"    <item id="ch1""#,
        concat!(
            r#"    <item id="css" href="s.css" media-type="text/css"/>"#,
            "\n",
            r#"    <item id="ch1""#
        ),
    );
    let ch1 = chapter(ch1_body, true).replace(
        "</head>",
        r#"<link rel="stylesheet" type="text/css" href="s.css"/></head>"#,
    );
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", NAV3.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/ch2.xhtml", chapter("<p>x</p>", true).as_bytes()),
        ("OEBPS/s.css", css.as_bytes()),
    ])
}

/// The same book declared as EPUB 3, so version-gated fixers can be compared.
pub fn epub3(ch1_body: &str, ch2_body: &str) -> Vec<u8> {
    build(true, ch1_body, ch2_body)
}

/// An EPUB 2 book with a caller-supplied NCX, for exercising nav generation.
pub fn epub2_ncx(ch1_body: &str, ch2_body: &str, ncx: &str) -> Vec<u8> {
    let ch1 = chapter(ch1_body, false);
    let ch2 = chapter(ch2_body, false);
    let opf = opf2();
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/ch2.xhtml", ch2.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
    ])
}

/// An EPUB 2 book with a caller-supplied NCX and OPF `guide`, for landmarks.
pub fn epub2_ncx_guide(ch1_body: &str, ch2_body: &str, ncx: &str, guide: &str) -> Vec<u8> {
    let opf = opf2().replace("</package>", &format!("{guide}</package>"));
    let opf = opf.replace(
        "  <spine",
        concat!(
            r#"    <item id="cov" href="cover.jpg" media-type="image/jpeg"/>"#,
            "\n  <spine"
        ),
    );
    // The manifest line above lands inside <manifest>, which closes just before
    // the spine; putting it there keeps the guide's image reference resolvable.
    let opf = opf.replace(
        concat!(
            r#"    <item id="cov" href="cover.jpg" media-type="image/jpeg"/>"#,
            "\n  <spine"
        ),
        concat!(
            r#"    <item id="cov" href="cover.jpg" media-type="image/jpeg"/>"#,
            "\n  </manifest>\n  <spine"
        ),
    );
    let opf = opf.replacen(
        "  </manifest>\n    <item id=\"cov\"",
        "    <item id=\"cov\"",
        1,
    );
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", chapter(ch1_body, false).as_bytes()),
        ("OEBPS/ch2.xhtml", chapter(ch2_body, false).as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/cover.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]),
    ])
}

/// The `--keep-version` path: repair against the declared version, never retag.
pub fn roundtrip_kept(bytes: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let mut book = Book::load(Cursor::new(bytes.to_vec())).unwrap();
    let mut outcome = epubfix::conform_book(&mut book);
    outcome.merge(epubfix::fix_book(&mut book));
    outcome.merge(epubfix::finish_book(&mut book, true));
    let mut out = Cursor::new(Vec::new());
    book.save(&mut out).unwrap();
    (outcome, read_epub(&out.into_inner()))
}

/// Load, migrate, then run the fixers, exactly as `--migrate-epub3` does.
pub fn roundtrip_migrated(bytes: &[u8]) -> (epubfix::Outcome, Vec<(String, Vec<u8>)>) {
    let mut book = Book::load(Cursor::new(bytes.to_vec())).unwrap();
    let mut outcome = epubfix::migrate_book(&mut book);
    outcome.merge(epubfix::conform_book(&mut book));
    outcome.merge(epubfix::fix_book(&mut book));
    outcome.merge(epubfix::finish_book(&mut book, true));
    let mut out = Cursor::new(Vec::new());
    book.save(&mut out).unwrap();
    (outcome, read_epub(&out.into_inner()))
}

/// Every image a body refers to by a plain relative path.
///
/// Without this a fixture asking about, say, `align` on an `<img>` is also
/// silently asking about an image whose file is not in the archive — and since
/// a proven-missing image is now removed rather than reported, the subject of
/// the test disappears before the assertion runs. A book that mentions a
/// picture should contain it; a test that wants a *missing* one builds its own
/// archive rather than going through here.
fn referenced_images(bodies: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for body in bodies {
        for (_, rest) in body.match_indices("src=\"").map(|(i, _)| body.split_at(i + 5)) {
            let Some(path) = rest.split('"').next() else {
                continue;
            };
            if path.contains(':') || path.starts_with('/') || path.contains("..") {
                continue;
            }
            if ends_with_any(&path.to_ascii_lowercase(), IMAGES) && !out.contains(&path.to_string())
            {
                out.push(path.to_string());
            }
        }
    }
    out
}

const IMAGES: &[&str] = &[".jpg", ".jpeg", ".png", ".gif", ".svg"];

fn ends_with_any(name: &str, exts: &[&str]) -> bool {
    exts.iter().any(|e| name.ends_with(e))
}

fn build(v3: bool, ch1_body: &str, ch2_body: &str) -> Vec<u8> {
    // Every image a body names goes into the archive *and* the manifest. A file
    // the book uses and the package does not list is RSC-008 -- a real defect
    // that `undeclared-resources` now repairs, so a fixture that skipped the
    // manifest half was quietly asking for a change on every image test.
    let mut declared = String::new();
    for (i, p) in referenced_images(&[ch1_body, ch2_body]).iter().enumerate() {
        {
            let media = if p.to_ascii_lowercase().ends_with(".png") {
                "image/png"
            } else if p.to_ascii_lowercase().ends_with(".gif") {
                "image/gif"
            } else {
                "image/jpeg"
            };
            let _ = writeln!(
                declared,
                "    <item id=\"img{i}\" href=\"{p}\" media-type=\"{media}\"/>"
            );
        }
    }
    let opf = if v3 { opf3() } else { opf2() };
    let opf = opf.replace("  </manifest>", &format!("{declared}  </manifest>"));
    let ch1 = chapter(ch1_body, v3);
    let ch2 = chapter(ch2_body, v3);
    let mut files: Vec<(&str, &[u8])> = vec![
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/ch2.xhtml", ch2.as_bytes()),
    ];
    if v3 {
        files.push(("OEBPS/nav.xhtml", NAV3.as_bytes()));
    } else {
        files.push(("OEBPS/toc.ncx", CLEAN_NCX.as_bytes()));
    }
    let images: Vec<String> = referenced_images(&[ch1_body, ch2_body])
        .iter()
        .map(|p| format!("OEBPS/{p}"))
        .collect();
    files.extend(images.iter().map(|p| (p.as_str(), b"\xFF\xD8\xFF\xE0".as_slice())));
    make_epub(&files)
}

/// A book that *declares* EPUB 3 but is written as EPUB 2 throughout: XHTML 1.1
/// DOCTYPEs, a named entity, `opf:` attributes, no nav document and no
/// `dcterms:modified`. The mirror of the mis-declared EPUB 2 case.
pub fn epub3_written_as_epub2(ch1_body: &str) -> Vec<u8> {
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="BookId" opf:scheme="UUID">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
    <dc:creator opf:role="aut" opf:file-as="Coleridge, Samuel">Samuel Coleridge</dc:creator>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    let ch1 = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \
         \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
         <head><title>T</title></head>\n<body>\n{ch1_body}\n</body></html>"
    );
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/toc.ncx", CLEAN_NCX.as_bytes()),
    ])
}

/// A book that arrives already broken: EPUB 2-shaped content declared EPUB 3
/// (so it wants retagging), carrying a dangling stylesheet link, a dangling
/// fragment and a duplicate id — none of which the retag causes or cures.
pub fn epub3_written_as_epub2_but_already_broken() -> Vec<u8> {
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="BookId" opf:scheme="UUID">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    let ch1 = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \
         \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
         <head><title>T</title>\
         <link rel=\"stylesheet\" type=\"text/css\" href=\"../styles/page-template.xpgt\"/>\
         </head>\n<body>\n\
         <p id=\"dup\">one</p><p id=\"dup\">two</p>\n\
         <p><a href=\"#nowhere\">dangling</a></p>\n\
         </body></html>";
    make_epub(&[
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/toc.ncx", CLEAN_NCX.as_bytes()),
    ])
}
