//! One test per fixer: it repairs the defect, and it leaves a clean book alone.

mod common;

use std::io::Cursor;

use common::{clean_book, entry, has, make_epub, make_text_epub, names, roundtrip, with};
use epubfix::Book;

#[test]
fn clean_book_reports_no_changes() {
    let (changes, _) = roundtrip(&make_text_epub(&clean_book()));
    assert!(changes.is_empty(), "unexpected changes: {changes:?}");
}

#[test]
fn package_version_is_upgraded() {
    let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="1.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest>
  <spine toc="ncx"/>
</package>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    assert_eq!(changes, vec!["package version 1.0 -> 2.0"]);
    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(fixed.contains(r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0""#));
    // The XML declaration is also version="1.0" and must be left alone.
    assert!(fixed.starts_with(r#"<?xml version="1.0" encoding="utf-8"?>"#));
}

#[test]
fn spine_page_map_is_removed() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest>
  <spine toc="ncx" page-map="pm"><itemref idref="ch1"/></spine>
</package>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    assert_eq!(changes, vec!["removed spine/@page-map"]);
    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(!fixed.contains("page-map"));
    // The rest of the tag must survive intact.
    assert!(fixed.contains(r#"<spine toc="ncx">"#));
}

#[test]
fn doubled_font_media_type_is_corrected() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="f1" href="f.ttf" media-type="application/application/x-font-ttf"/>
  </manifest>
  <spine toc="ncx"/>
</package>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    assert_eq!(changes, vec!["corrected font media-type"]);
    assert!(
        entry(&out, "OEBPS/content.opf").contains(r#"media-type="application/vnd.ms-opentype""#)
    );
}

#[test]
fn invalid_ids_are_sanitised_and_links_follow() {
    let ch1 = r##"<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <h1 id="1intro">One</h1>
  <a name="ch:two">two</a>
  <a href="#1intro">back to intro</a>
  <a href="ch1.xhtml#ch:two">to two</a>
</body></html>"##;
    let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <navMap>
    <navPoint id="np1" playOrder="1"><content src="ch1.xhtml#1intro"/></navPoint>
  </navMap>
</ncx>"#;
    let files = with(
        with(clean_book(), "OEBPS/ch1.xhtml", ch1),
        "OEBPS/toc.ncx",
        ncx,
    );
    let (changes, out) = roundtrip(&make_text_epub(&files));

    assert!(
        changes.iter().any(|c| c == "sanitised 2 id(s)"),
        "got {changes:?}"
    );
    let fixed = entry(&out, "OEBPS/ch1.xhtml");
    assert!(fixed.contains(r#"id="id_1intro""#));
    assert!(fixed.contains(r#"name="ch_two""#));
    assert!(fixed.contains(r##"href="#id_1intro""##));
    assert!(fixed.contains(r#"href="ch1.xhtml#ch_two""#));
    // A fragment in the NCX points at the same anchor and must be updated too.
    assert!(entry(&out, "OEBPS/toc.ncx").contains(r#"src="ch1.xhtml#id_1intro""#));
}

#[test]
fn valid_ids_are_left_alone() {
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <h1 id="intro">One</h1><p><a name="_x-1.2">ok</a></p><p id="">empty</p>
</body></html>"#;
    let (changes, _) = roundtrip(&make_text_epub(&with(clean_book(), "OEBPS/ch1.xhtml", ch1)));
    assert!(changes.is_empty(), "got {changes:?}");
}

#[test]
fn unsafe_filenames_are_renamed_and_references_updated() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="img" href="cover%20image.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine toc="ncx"/>
</package>"#;
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <img src="cover image.jpg" alt=""/>
</body></html>"#;

    // Real JPEG bytes: not valid UTF-8, so the fixer must not try to decode it.
    let jpeg: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
    let epub = make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/cover image.jpg", jpeg),
    ]);
    let (changes, out) = roundtrip(&epub);

    assert!(
        changes.iter().any(|c| c == "renamed 1 file(s)"),
        "got {changes:?}"
    );
    assert!(has(&out, "OEBPS/cover_image.jpg"), "{:?}", names(&out));
    assert!(!has(&out, "OEBPS/cover image.jpg"));
    // Both the percent-encoded and the raw reference are rewritten.
    assert!(entry(&out, "OEBPS/content.opf").contains(r#"href="cover_image.jpg""#));
    assert!(entry(&out, "OEBPS/ch1.xhtml").contains(r#"src="cover_image.jpg""#));

    let (_, data) = out
        .iter()
        .find(|(n, _)| n == "OEBPS/cover_image.jpg")
        .unwrap();
    assert_eq!(data, jpeg, "binary payload must survive byte for byte");
}

#[test]
fn colliding_renames_do_not_lose_an_entry() {
    // "a b.txt" and "a-b.txt" both sanitise toward the same stem shape; two
    // distinct originals must still end up as two distinct entries.
    let epub = make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", common::CLEAN_OPF.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", common::CLEAN_CH1.as_bytes()),
        ("OEBPS/a b.css", b"a{}"),
        ("OEBPS/a+b.css", b"b{}"),
        ("OEBPS/a_b.css", b"c{}"),
    ]);
    let (changes, out) = roundtrip(&epub);

    assert!(
        changes.iter().any(|c| c == "renamed 2 file(s)"),
        "got {changes:?}"
    );
    let css: Vec<&String> = out
        .iter()
        .map(|(n, _)| n)
        .filter(|n| {
            std::path::Path::new(n)
                .extension()
                .is_some_and(|e| e == "css")
        })
        .collect();
    assert_eq!(css.len(), 3, "no entry may be dropped: {css:?}");
    let mut sorted = css.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 3, "names must stay distinct: {css:?}");
    assert!(css.iter().any(|n| n.as_str() == "OEBPS/a_b.css"));
}

#[test]
fn play_order_is_renumbered_from_one() {
    let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <navMap>
    <navPoint id="np1" playOrder="0"><content src="ch1.xhtml"/></navPoint>
    <navPoint id="np2" playOrder="0"><content src="ch2.xhtml"/></navPoint>
    <navPoint id="np3" playOrder="7"><content src="ch3.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(clean_book(), "OEBPS/toc.ncx", ncx)));

    assert!(
        changes
            .iter()
            .any(|c| c == "renumbered playOrder (3 target(s))"),
        "got {changes:?}"
    );
    let fixed = entry(&out, "OEBPS/toc.ncx");
    assert!(fixed.contains(r#"id="np1" playOrder="1""#), "{fixed}");
    assert!(fixed.contains(r#"id="np2" playOrder="2""#), "{fixed}");
    assert!(fixed.contains(r#"id="np3" playOrder="3""#), "{fixed}");
}

#[test]
fn nav_points_sharing_a_target_share_a_play_order() {
    let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <navMap>
    <navPoint id="a" playOrder="1"><content src="ch1.xhtml"/></navPoint>
    <navPoint id="b" playOrder="2"><content src="ch1.xhtml"/></navPoint>
    <navPoint id="c" playOrder="3"><content src="ch2.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(clean_book(), "OEBPS/toc.ncx", ncx)));

    assert!(
        changes
            .iter()
            .any(|c| c == "renumbered playOrder (2 target(s))"),
        "got {changes:?}"
    );
    let fixed = entry(&out, "OEBPS/toc.ncx");
    assert!(fixed.contains(r#"id="a" playOrder="1""#), "{fixed}");
    assert!(fixed.contains(r#"id="b" playOrder="1""#), "{fixed}");
    assert!(fixed.contains(r#"id="c" playOrder="2""#), "{fixed}");
}

#[test]
fn nav_points_without_play_order_are_untouched() {
    let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <navMap>
    <navPoint id="np1"><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let (changes, _) = roundtrip(&make_text_epub(&with(clean_book(), "OEBPS/toc.ncx", ncx)));
    assert!(changes.is_empty(), "got {changes:?}");
}

#[test]
fn dtb_uid_is_synced_to_the_opf_identifier() {
    let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head>
    <meta name="dtb:uid" content="something-else"/>
    <meta name="dtb:depth" content="1"/>
  </head>
  <navMap>
    <navPoint id="np1" playOrder="1"><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(clean_book(), "OEBPS/toc.ncx", ncx)));

    assert_eq!(changes, vec!["synced NCX dtb:uid to OPF identifier"]);
    let fixed = entry(&out, "OEBPS/toc.ncx");
    assert!(
        fixed.contains(r#"name="dtb:uid" content="urn:uuid:1234-5678""#),
        "{fixed}"
    );
    // Other <meta> tags must be left exactly as they were.
    assert!(fixed.contains(r#"name="dtb:depth" content="1""#), "{fixed}");
}

#[test]
fn mimetype_is_first_stored_and_bare() {
    let (_, out) = roundtrip(&make_text_epub(&clean_book()));
    assert_eq!(out[0].0, "mimetype");
    assert_eq!(out[0].1, b"application/epub+zip");

    // OCF also constrains the raw local header: stored, no extra field.
    let mut book = Book::load(Cursor::new(make_text_epub(&clean_book()))).unwrap();
    epubfix::fix_book(&mut book);
    let mut buf = Cursor::new(Vec::new());
    book.save(&mut buf).unwrap();
    let raw = buf.into_inner();

    assert_eq!(&raw[0..4], b"PK\x03\x04");
    let method = u16::from_le_bytes([raw[8], raw[9]]);
    let name_len = u16::from_le_bytes([raw[26], raw[27]]) as usize;
    let extra_len = u16::from_le_bytes([raw[28], raw[29]]) as usize;
    assert_eq!(method, 0, "mimetype must be stored, not deflated");
    assert_eq!(extra_len, 0, "mimetype must carry no extra field");
    assert_eq!(&raw[30..30 + name_len], b"mimetype");
    let start = 30 + name_len;
    assert_eq!(&raw[start..start + 20], b"application/epub+zip");
}

#[test]
fn a_book_with_no_opf_or_ncx_is_handled() {
    let epub = make_epub(&[("OEBPS/stray.txt", b"hello")]);
    let (changes, out) = roundtrip(&epub);
    assert!(changes.is_empty(), "got {changes:?}");
    assert!(has(&out, "OEBPS/stray.txt"));
}

#[test]
fn fixing_is_idempotent() {
    let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="wrong"/></head>
  <navMap>
    <navPoint id="np1" playOrder="0"><content src="ch1 a.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let ch = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1 id="1a">x</h1></body></html>"#;
    let epub = make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", common::CLEAN_OPF.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1 a.xhtml", ch.as_bytes()),
    ]);

    let (first, out) = roundtrip(&epub);
    assert!(!first.is_empty());

    let repacked = make_epub(
        &out.iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect::<Vec<_>>(),
    );
    let (second, _) = roundtrip(&repacked);
    assert!(
        second.is_empty(),
        "a second pass should find nothing: {second:?}"
    );
}

#[test]
fn non_utf8_text_resources_are_left_alone() {
    // A latin-1 stylesheet: textual by extension, but not decodable.
    let latin1: &[u8] = b"/* caf\xe9 */ body { margin: 0 }";
    let epub = make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", common::CLEAN_OPF.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/style.css", latin1),
    ]);
    let (_, out) = roundtrip(&epub);
    let (_, data) = out.iter().find(|(n, _)| n == "OEBPS/style.css").unwrap();
    assert_eq!(data, latin1);
}

#[test]
fn only_runs_the_selected_fixers() {
    use epubfix::fixers;

    let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="wrong"/></head>
  <navMap>
    <navPoint id="np1" playOrder="0"><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
    let files = with(clean_book(), "OEBPS/toc.ncx", ncx);

    let mut book = Book::load(Cursor::new(make_text_epub(&files))).unwrap();
    let selected: Vec<Box<dyn epubfix::Fixer>> = fixers::all()
        .into_iter()
        .filter(|f| f.name() == "ncx-uid")
        .collect();
    let changes = epubfix::fix_book_with(&mut book, &selected).changes;

    assert_eq!(changes, vec!["synced NCX dtb:uid to OPF identifier"]);
    assert!(
        book.ncx_text().unwrap().contains(r#"playOrder="0""#),
        "the unselected fixer must not have run"
    );
}

#[test]
fn every_fixer_has_a_unique_name_and_a_code() {
    let all = epubfix::fixers::all();
    let mut seen = std::collections::HashSet::new();
    for f in &all {
        assert!(seen.insert(f.name()), "duplicate fixer name {}", f.name());
        assert!(!f.codes().is_empty(), "{} declares no codes", f.name());
        assert!(
            !f.description().is_empty(),
            "{} has no description",
            f.name()
        );
    }
}
