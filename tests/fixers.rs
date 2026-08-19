//! One test per fixer: it repairs the defect, and it leaves a clean book alone.

mod common;

use std::io::Cursor;

use common::{
    clean_book, entry, has, make_epub, make_text_epub, names, plus, roundtrip, roundtrip_full,
    roundtrip_kept, with,
};
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
  <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
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
  <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
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

/// An empty `toc=""` is worse than the missing attribute it resembles.
/// Measured: `value of attribute "toc" is invalid` *and* `OPF-049 Item id ""
/// was not found in the manifest` — two errors where an absent one is a single
/// error, and `attr("toc").is_some()` read it as already repaired.
#[test]
fn an_empty_spine_toc_is_pointed_at_the_ncx_like_a_missing_one() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc=""><itemref idref="ch1"/></spine>
</package>"#;
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>x</p></body></html>"#;
    let (changes, out) = roundtrip(&make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
    ]));
    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(fixed.contains(r#"<spine toc="ncx">"#), "got {fixed}");
    // Overwritten, not appended beside the empty one.
    assert_eq!(fixed.matches("toc=").count(), 1, "got {fixed}");
    assert!(
        changes.iter().any(|c| c.contains("spine/@toc")),
        "got {changes:?}"
    );
}

#[test]
fn an_epub2_spine_with_no_toc_attribute_is_pointed_at_the_ncx() {
    // Dune: a complete NCX in the manifest, and nothing pointing at it.
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    assert_eq!(changes, vec![r#"pointed spine/@toc at the NCX ("ncx")"#]);
    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(fixed.contains(r#"<spine toc="ncx">"#), "{fixed}");
}

#[test]
fn a_spine_toc_repair_that_would_guess_is_reported_instead() {
    // Two NCX entries: which one is the table of contents is a person's call.
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest><item id="n1" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="n2" href="toc2.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let (outcome, out) = roundtrip_full(&make_text_epub(&plus(
        with(clean_book(), "OEBPS/content.opf", opf),
        "OEBPS/toc2.ncx",
        "<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"/>",
    )));

    assert!(!outcome.has_changes(), "got {:?}", outcome.changes);
    assert!(
        outcome.findings.iter().any(|f| f.contains("2 NCX")),
        "got {:?}",
        outcome.findings
    );
    assert!(entry(&out, "OEBPS/content.opf").contains("<spine>"));
}

#[test]
fn an_unused_reserved_prefix_declaration_is_removed() {
    // Quantum Mechanics and Tales from Shakespeare: a rendition declaration
    // EPUB itself reserves, never used anywhere in the book.
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" prefix="rendition: http://www.idpf.org/vocab/rendition# ibooks: http://vocabulary.itunes.apple.com/rdf/ibooks/vocabulary-extensions-1.0/" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let (outcome, out) = roundtrip_kept(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(!fixed.contains("rendition:"), "{fixed}");
    assert!(
        fixed.contains(r#"prefix="ibooks: http://vocabulary.itunes.apple.com/rdf/ibooks/vocabulary-extensions-1.0/""#),
        "the other declaration survives: {fixed}"
    );
    assert!(
        outcome.changes.iter().any(|c| c.contains("rendition")),
        "got {:?}",
        outcome.changes
    );
}

/// Re-declaring the prefix is not the defect; binding it to the *wrong* URI is.
///
/// Measured: `rendition: http://www.idpf.org/vocab/rendition/#` is clean, and
/// the one slash short that the two library books write is `OPF-007`. A
/// Hemingway *In Our Time* binds the reserved URI exactly, epubcheck reports
/// 0/0/0/0 on it, and this pass used to rewrite the package anyway.
#[test]
fn the_reserved_prefix_bound_to_its_own_uri_is_left_alone() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" prefix="schema: http://schema.org/ rendition: http://www.idpf.org/vocab/rendition/# ibooks: http://vocabulary.itunes.apple.com/rdf/ibooks/vocabulary-extensions-1.0/" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let (outcome, out) = roundtrip_kept(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(
        fixed.contains("rendition: http://www.idpf.org/vocab/rendition/#"),
        "the declaration stands: {fixed}"
    );
    assert!(
        !outcome.changes.iter().any(|c| c.contains("rendition")),
        "nothing to repair, so nothing reported: {:?}",
        outcome.changes
    );
}

/// A mapping is `name: url`, and the removal takes both tokens. A book that
/// wrote the pair with no space has put the whole mapping in one token, so
/// taking the next one as well would eat the *following* declaration's name and
/// leave its URL stranded as a nonsense prefix.
#[test]
fn removing_the_reserved_prefix_does_not_eat_its_neighbour() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" prefix="rendition:http://www.idpf.org/vocab/rendition# foaf: http://xmlns.com/foaf/spec/" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let (_, out) = roundtrip_kept(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(
        fixed.contains(r#"prefix="foaf: http://xmlns.com/foaf/spec/""#),
        "the neighbouring declaration must survive whole: {fixed}"
    );
}

#[test]
fn a_used_reserved_prefix_is_reported_not_removed() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" prefix="rendition: http://www.idpf.org/vocab/rendition#" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <meta property="rendition:spread">auto</meta>
  </metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let (outcome, out) = roundtrip_kept(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    // The conformance pass may still repair unrelated EPUB 3 requirements, but
    // the prefix itself must survive untouched.
    assert!(
        !outcome.changes.iter().any(|c| c.contains("rendition")),
        "got {:?}",
        outcome.changes
    );
    assert!(
        outcome
            .findings
            .iter()
            .any(|f| f.contains("the book uses the prefix")),
        "got {:?}",
        outcome.findings
    );
    assert!(entry(&out, "OEBPS/content.opf").contains("rendition:"));
}

#[test]
fn invalid_manifest_ids_are_sanitised_and_idrefs_follow() {
    // Lost Worlds of 2001: manifest ids opening with "(" — invalid XML Names
    // epubcheck reports against the package itself.
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="(e-book)Arthur_Clarke-The_Lost_Worlds_of_20014" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine toc="ncx"><itemref idref="(e-book)Arthur_Clarke-The_Lost_Worlds_of_20014"/></spine>
</package>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(
        !fixed.contains("(e-book)"),
        "the invalid id must move: {fixed}"
    );
    assert!(
        fixed.contains(r#"idref="_e-book_Arthur_Clarke-The_Lost_Worlds_of_20014""#),
        "the idref must follow the id: {fixed}"
    );
    assert!(
        changes.iter().any(|c| c.contains("sanitised")),
        "got {changes:?}"
    );
}

/// Sanitising an id in the package is only half a repair. The OPF names its own
/// ids through attributes that are not hrefs, and leaving those behind trades an
/// RSC-005 for an OPF-030 — a `unique-identifier` resolving to nothing, and a
/// `refines` that has silently stopped refining anything.
#[test]
fn every_opf_pointer_at_a_sanitised_id_follows_it() {
    let opf = r##"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="(BookId)">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="(BookId)">urn:uuid:1234-5678</dc:identifier>
    <dc:title>Test</dc:title><dc:language>en</dc:language>
    <dc:creator id="(cre)">Carlo Collodi</dc:creator>
    <meta refines="#(cre)" property="role">aut</meta>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"##;
    let (changes, out) = roundtrip(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(
        !fixed.contains("(BookId)") && !fixed.contains("(cre)"),
        "the invalid ids must move: {fixed}"
    );
    assert!(
        fixed.contains(r#"unique-identifier="_BookId_""#),
        "the package pointer must follow the identifier: {fixed}"
    );
    assert!(
        fixed.contains(r##"refines="#_cre_""##),
        "the refinement must follow the creator: {fixed}"
    );
    assert!(
        changes.iter().any(|c| c.contains("sanitised")),
        "got {changes:?}"
    );
}

#[test]
fn unnamespaced_opf_vocabulary_attributes_get_the_prefix() {
    // Pinocchio: bare scheme/file-as/role/event on dc: elements in an EPUB 2
    // package that already declares xmlns:opf.
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="id" scheme="URI">urn:x</dc:identifier>
    <dc:creator file-as="Collodi, Carlo" role="aut">Carlo Collodi</dc:creator>
    <dc:date event="conversion">2022-12-08</dc:date>
  </metadata>
  <manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    let (changes, out) = roundtrip(&make_text_epub(&with(
        clean_book(),
        "OEBPS/content.opf",
        opf,
    )));

    let fixed = entry(&out, "OEBPS/content.opf");
    assert!(fixed.contains(r#"opf:scheme="URI""#), "{fixed}");
    assert!(fixed.contains(r#"opf:file-as="Collodi, Carlo""#), "{fixed}");
    assert!(fixed.contains(r#"opf:role="aut""#), "{fixed}");
    assert!(fixed.contains(r#"opf:event="conversion""#), "{fixed}");
    assert!(
        changes.iter().any(|c| c.contains("prefixed 4")),
        "got {changes:?}"
    );
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
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    // The font is really in the archive: this test is about the media type,
    // and an item for a file that is not there is a different defect.
    let (changes, out) = roundtrip(&make_text_epub(&with(
        plus(clean_book(), "OEBPS/f.ttf", "not really a font"),
        "OEBPS/content.opf",
        opf,
    )));

    assert_eq!(
        changes,
        vec!["corrected 1 manifest media-type(s) [application/application/x-font-ttf]"]
    );
    assert!(
        entry(&out, "OEBPS/content.opf").contains(r#"media-type="application/vnd.ms-opentype""#)
    );
}

#[test]
fn invalid_ids_are_sanitised_and_links_follow() {
    // The anchors sit inside <p> so the book stays EPUB 2: an <a> bare in the
    // body is flow content only HTML5 accepts, and the retagger would move the
    // declaration before this fixer ever ran.
    let ch1 = r##"<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <h1 id="1intro">One</h1>
  <p><a name="ch:two">two</a></p>
  <p><a href="#1intro">back to intro</a></p>
  <p><a href="ch1.xhtml#ch:two">to two</a></p>
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
    // The anchor's `name` becomes the `id` it stood for -- XHTML 1.1 has no
    // `name` on an `<a>` -- and is sanitised in the same run.
    assert!(fixed.contains(r#"id="ch_two""#), "{fixed}");
    assert!(!fixed.contains("<a name="), "{fixed}");
    assert!(fixed.contains(r##"href="#id_1intro""##));
    assert!(fixed.contains(r#"href="ch1.xhtml#ch_two""#));
    // A fragment in the NCX points at the same anchor and must be updated too.
    assert!(entry(&out, "OEBPS/toc.ncx").contains(r#"src="ch1.xhtml#id_1intro""#));
}

#[test]
fn valid_ids_are_left_alone() {
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <h1 id="intro">One</h1><p><a id="_x-1.2">ok</a></p><p id="">empty</p>
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

/// References are found by resolving them, not by matching bytes — and this is
/// the book that forced it.
///
/// `B.xhtml` and `b.xhtml` fold onto one name under OCF, so the later one is
/// renamed to `b_2.xhtml`. The old repair searched every text entry for the
/// string `b.xhtml` and swapped it, which is also the tail of `ab.xhtml`:
///
/// ```text
/// before   <a href="ab.xhtml">   <item href="ab.xhtml">   <content src="ab.xhtml"/>
/// after    <a href="ab_2.xhtml"> <item href="ab_2.xhtml"> <content src="ab_2.xhtml"/>
///          ERROR(RSC-001) File "OEBPS/ab_2.xhtml" could not be found.
/// ```
///
/// Three references broken and a file invented, out of one legitimate rename.
#[test]
fn a_rename_does_not_chew_through_a_name_that_merely_contains_it() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="B" href="B.xhtml" media-type="application/xhtml+xml"/>
    <item id="b" href="b.xhtml" media-type="application/xhtml+xml"/>
    <item id="ab" href="ab.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="B"/><itemref idref="b"/><itemref idref="ab"/></spine>
</package>"#;
    let page = |body: &str| {
        format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>{body}</body></html>"#)
    };
    let (changes, out) = roundtrip(&make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        (
            "OEBPS/B.xhtml",
            page(r#"<p><a href="ab.xhtml">go</a></p>"#).as_bytes(),
        ),
        ("OEBPS/b.xhtml", page("<p>b</p>").as_bytes()),
        ("OEBPS/ab.xhtml", page("<p>ab</p>").as_bytes()),
    ]));

    assert!(
        changes.iter().any(|c| c == "renamed 1 file(s)"),
        "got {changes:?}"
    );
    assert!(has(&out, "OEBPS/b_2.xhtml"), "{:?}", names(&out));
    assert!(
        has(&out, "OEBPS/ab.xhtml"),
        "the innocent file keeps its name"
    );
    // The rename it earned, and nothing it did not.
    assert!(entry(&out, "OEBPS/content.opf").contains(r#"href="b_2.xhtml""#));
    assert!(entry(&out, "OEBPS/content.opf").contains(r#"href="ab.xhtml""#));
    assert!(entry(&out, "OEBPS/B.xhtml").contains(r#"href="ab.xhtml""#));
    assert!(
        !entry(&out, "OEBPS/B.xhtml").contains("ab_2"),
        "{}",
        entry(&out, "OEBPS/B.xhtml")
    );
}

/// A renamed package document has exactly one pointer, and it is not an `href`.
/// `container.xml` spells it `full-path`, so a targeted rewrite that only knew
/// about `href`/`src` would leave the book unopenable.
#[test]
fn a_renamed_package_document_is_followed_in_container_xml() {
    let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/my content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>x</p></body></html>"#;
    let (_, out) = roundtrip(&make_epub(&[
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/my content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
    ]));

    assert!(has(&out, "OEBPS/my_content.opf"), "{:?}", names(&out));
    assert!(
        entry(&out, "META-INF/container.xml").contains(r#"full-path="OEBPS/my_content.opf""#),
        "got {}",
        entry(&out, "META-INF/container.xml")
    );
}

/// A rename is pending until the archive is repacked, so `texts` stays keyed by
/// the name the entry arrived under — while `filenames` has already rewritten
/// every reference to the name it is *going* to have. Every pass after it looks
/// the document up by the new name and used to find nothing at all.
///
/// Measured: this book's `ch 1.xhtml` holds an `<svg>`, so `finalise_properties`
/// owes its manifest item `properties="svg"`. With the rename it silently
/// skipped the document and left `OPF-014`; the identical book under a safe
/// filename validated clean.
#[test]
fn a_renamed_document_is_still_readable_by_the_passes_after_the_rename() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>T</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="ch 1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let nav = r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>N</title></head>
<body><nav epub:type="toc"><ol><li><a href="ch 1.xhtml">One</a></li></ol></nav></body></html>"#;
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head>
<body><p>t</p><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg></body></html>"#;
    let (changes, out) = roundtrip(&make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/ch 1.xhtml", ch1.as_bytes()),
    ]));

    assert!(has(&out, "OEBPS/ch_1.xhtml"), "{:?}", names(&out));
    assert!(
        entry(&out, "OEBPS/content.opf").contains(r#"properties="svg""#),
        "the pass after the rename could still read the document: {}",
        entry(&out, "OEBPS/content.opf")
    );
    assert!(
        changes.iter().any(|c| c.contains("manifest propert")),
        "got {changes:?}"
    );
}

/// An unclosed `<navPoint>` is a `Start` node with no close tag, and the sort
/// stitched each child from its open tag to its close tag. It used to panic —
/// `navPoint with a close tag` — which the unwind guard turned into a failed
/// book. The container is left exactly as written instead: leaving the child
/// out would drop its bytes.
#[test]
fn an_unclosed_nav_point_does_not_panic_the_sorter() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>T</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch2"/><itemref idref="ch1"/></spine>
</package>"#;
    let ncx = r#"<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:1234-5678"/></head>
  <docTitle><text>T</text></docTitle>
  <navMap>
    <navPoint id="n2" playOrder="2"><navLabel><text>Two</text></navLabel><content src="ch2.xhtml"/></navPoint>
    <navPoint id="n1" playOrder="1"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/>
  </navMap>
</ncx>"#;
    let nav = r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>N</title></head>
<body><nav epub:type="toc"><ol><li><a href="ch2.xhtml">Two</a></li></ol></nav></body></html>"#;
    let page = r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>P</title></head><body><p>x</p></body></html>"#;
    // The point is that this returns at all.
    let (_, out) = roundtrip(&make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/ch1.xhtml", page.as_bytes()),
        ("OEBPS/ch2.xhtml", page.as_bytes()),
    ]));
    // Nothing was dropped on the way through.
    let got = entry(&out, "OEBPS/toc.ncx");
    assert!(got.contains("Two") && got.contains("One"), "got {got}");
}

/// The same guard on the EPUB 3 side. `ncx-nav-order` sorts `<ol>`/`<li>`
/// subtrees with the identical stitching, so all three collections needed the
/// check and all three should be proved, not just the one the bug arrived on.
#[test]
fn an_unclosed_list_item_does_not_panic_the_nav_sorter() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
    <dc:title>T</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/><itemref idref="ch2"/></spine>
</package>"#;
    // The second <li> is opened and never closed.
    let nav = r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>N</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="ch2.xhtml">Two</a></li>
<li><a href="ch1.xhtml">One</a>
</ol></nav></body></html>"#;
    let page = r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>P</title></head><body><p>x</p></body></html>"#;
    let (_, out) = roundtrip(&make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/ch1.xhtml", page.as_bytes()),
        ("OEBPS/ch2.xhtml", page.as_bytes()),
    ]));
    // It returned, and neither entry was dropped on the way through.
    let got = entry(&out, "OEBPS/nav.xhtml");
    assert!(got.contains("Two") && got.contains("One"), "got {got}");
}

/// A file's name and the manifest's label are both claims; the bytes are the
/// fact. A Gutenberg *Dracula* ships a PNG called `cover.jpg` declared
/// `image/jpeg`, which is two messages at once — and correcting only one of
/// them leaves the other standing:
///
/// ```text
/// PNG bytes, cover.jpg, image/jpeg    OPF-029 + PKG-022
/// PNG bytes, cover.jpg, image/png     PKG-022
/// PNG bytes, cover.png, image/png     clean
/// ```
#[test]
fn an_image_whose_bytes_contradict_its_name_is_renamed_and_relabelled() {
    // A real 1x1 PNG: the signature is what the repair reads.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="cover.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p><img src="cover.jpg" alt="c"/></p></body></html>"#;
    let (changes, out) = roundtrip(&make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/cover.jpg", PNG),
    ]));

    assert!(has(&out, "OEBPS/cover.png"), "{:?}", names(&out));
    assert!(!has(&out, "OEBPS/cover.jpg"));
    let opf_after = entry(&out, "OEBPS/content.opf");
    assert!(opf_after.contains(r#"href="cover.png""#), "{opf_after}");
    assert!(
        opf_after.contains(r#"media-type="image/png""#),
        "{opf_after}"
    );
    // Every reference follows, not just the manifest's.
    assert!(entry(&out, "OEBPS/ch1.xhtml").contains(r#"src="cover.png""#));
    assert!(
        changes.iter().any(|c| c.contains("media-type")),
        "{changes:?}"
    );
    assert!(changes.iter().any(|c| c.contains("renamed")), "{changes:?}");
    // The bytes themselves are untouched.
    assert_eq!(
        out.iter()
            .find(|(n, _)| n == "OEBPS/cover.png")
            .map(|(_, d)| d.as_slice()),
        Some(PNG)
    );
}

/// `.jpeg` and `.jpg` are one format spelled two ways and epubcheck accepts
/// both, so a rename between them would be churn for nothing.
#[test]
fn a_jpeg_spelled_the_long_way_is_left_alone() {
    const JPG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="cover.jpeg" media-type="image/jpeg"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p><img src="cover.jpeg" alt="c"/></p></body></html>"#;
    let (changes, out) = roundtrip(&make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/cover.jpeg", JPG),
    ]));
    assert!(has(&out, "OEBPS/cover.jpeg"), "{:?}", names(&out));
    assert!(
        !changes.iter().any(|c| c.contains("renamed")),
        "{changes:?}"
    );
}

/// An unsafe *directory* component is the same error as an unsafe basename —
/// measured, a Word-saved *Sound and the Fury* keeps its images under
/// `the sound and the fury_files/`, and every reference to them is RSC-020 —
/// so the component is renamed and cross-directory references rewritten.
/// References from *inside* the directory spell no directory at all, and stay
/// valid by construction.
#[test]
fn unsafe_directory_components_are_renamed_and_references_updated() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:1234-5678</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="the sound and the fury_files/image001.gif" media-type="image/gif"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;
    // One reference from the root (the manifest), one relative reference that
    // crosses the directory, and one sibling reference inside it that names
    // no directory at all.
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <p><img src="the sound and the fury_files/image001.gif" alt=""/></p>
</body></html>"#;
    let page = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <p><img src="image001.gif" alt=""/></p>
</body></html>"#;

    let gif: &[u8] = &[0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00];
    let epub = make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
        ("OEBPS/the sound and the fury_files/image001.gif", gif),
        (
            "OEBPS/the sound and the fury_files/page.xhtml",
            page.as_bytes(),
        ),
    ]);
    let (changes, out) = roundtrip(&epub);

    assert!(
        changes.iter().any(|c| c == "renamed 2 file(s)"),
        "got {changes:?}"
    );
    assert!(
        has(&out, "OEBPS/the_sound_and_the_fury_files/image001.gif"),
        "{:?}",
        names(&out)
    );
    assert!(!has(
        &out,
        "OEBPS/the sound and the fury_files/image001.gif"
    ));
    // Both spellings of the directory in the manifest and the cross-directory
    // reference are rewritten; the sibling reference needed nothing.
    assert!(
        entry(&out, "OEBPS/content.opf")
            .contains(r#"href="the_sound_and_the_fury_files/image001.gif""#)
    );
    assert!(
        entry(&out, "OEBPS/ch1.xhtml")
            .contains(r#"src="the_sound_and_the_fury_files/image001.gif""#)
    );
    assert!(
        entry(&out, "OEBPS/the_sound_and_the_fury_files/page.xhtml")
            .contains(r#"src="image001.gif""#)
    );
}

#[test]
fn colliding_renames_do_not_lose_an_entry() {
    // "a b.css" and "a:b.css" both sanitise to "a_b.css", which a third entry
    // already occupies; three distinct originals must still end up as three
    // distinct entries.
    let epub = make_epub(&[
        ("META-INF/container.xml", common::CONTAINER.as_bytes()),
        ("OEBPS/content.opf", common::CLEAN_OPF.as_bytes()),
        ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", common::CLEAN_CH1.as_bytes()),
        ("OEBPS/a b.css", b"a{}"),
        ("OEBPS/a:b.css", b"b{}"),
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

/// The 321-file regression.
///
/// One real book had 321 files with an apostrophe in the name and 42 with an
/// exclamation mark, and the fixer proposed renaming every one of them —
/// rewriting the manifest, the spine and every link in the book — to fix
/// nothing at all. Both characters are RFC 3986 sub-delims, legal unencoded in
/// a path segment, and epubcheck says nothing about either. Nor about the
/// accented and CJK names it also used to mangle.
#[test]
fn filenames_that_epubcheck_accepts_are_left_alone() {
    let legal = [
        "Don't-Panic.css",
        "hello!.css",
        "a&b.css",
        "a(b).css",
        "a+b.css",
        "a,b.css",
        "a;b.css",
        "a=b.css",
        "a@b.css",
        "a~b.css",
        "Ünïcödé.css",
        "中文.css",
    ];

    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (
            "META-INF/container.xml".into(),
            common::CONTAINER.as_bytes().to_vec(),
        ),
        (
            "OEBPS/content.opf".into(),
            common::CLEAN_OPF.as_bytes().to_vec(),
        ),
        (
            "OEBPS/toc.ncx".into(),
            common::CLEAN_NCX.as_bytes().to_vec(),
        ),
        (
            "OEBPS/ch1.xhtml".into(),
            common::CLEAN_CH1.as_bytes().to_vec(),
        ),
    ];
    for base in legal {
        entries.push((format!("OEBPS/{base}"), b"p{}".to_vec()));
    }
    let borrowed: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let (changes, out) = roundtrip(&make_epub(&borrowed));

    assert!(
        !changes.iter().any(|c| c.starts_with("renamed")),
        "no rename is warranted here: {changes:?}"
    );
    for base in legal {
        assert!(
            has(&out, &format!("OEBPS/{base}")),
            "{base} must survive untouched: {:?}",
            names(&out)
        );
    }
}

/// `[` is legal in a file name and only breaks a reference that spells it raw,
/// so the rename waits for that evidence rather than assuming it.
#[test]
fn encode_required_names_are_renamed_only_when_referenced_raw() {
    let with_css = |href: &str| {
        let ch1 = format!(
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><head>
  <link rel="stylesheet" type="text/css" href="{href}"/>
</head><body><p>text</p></body></html>"#
        );
        make_epub(&[
            ("META-INF/container.xml", common::CONTAINER.as_bytes()),
            ("OEBPS/content.opf", common::CLEAN_OPF.as_bytes()),
            ("OEBPS/toc.ncx", common::CLEAN_NCX.as_bytes()),
            ("OEBPS/ch1.xhtml", ch1.as_bytes()),
            ("OEBPS/a[1].css", b"p{}"),
        ])
    };

    let (changes, out) = roundtrip(&with_css("a%5B1%5D.css"));
    assert!(
        !changes.iter().any(|c| c.starts_with("renamed")),
        "the reference is encoded, so the book is already valid: {changes:?}"
    );
    assert!(has(&out, "OEBPS/a[1].css"), "{:?}", names(&out));

    let (changes, out) = roundtrip(&with_css("a[1].css"));
    assert!(
        changes.iter().any(|c| c == "renamed 1 file(s)"),
        "a raw \"[\" in a path segment is RSC-020: {changes:?}"
    );
    assert!(has(&out, "OEBPS/a_1_.css"), "{:?}", names(&out));
    assert!(entry(&out, "OEBPS/ch1.xhtml").contains(r#"href="a_1_.css""#));
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
    let files = plus(
        plus(
            with(clean_book(), "OEBPS/toc.ncx", ncx),
            "OEBPS/ch2.xhtml",
            common::CLEAN_CH1,
        ),
        "OEBPS/ch3.xhtml",
        common::CLEAN_CH1,
    );
    let (changes, out) = roundtrip(&make_text_epub(&files));

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
    let files = plus(
        with(clean_book(), "OEBPS/toc.ncx", ncx),
        "OEBPS/ch2.xhtml",
        common::CLEAN_CH1,
    );
    let (changes, out) = roundtrip(&make_text_epub(&files));

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
    let selected: Vec<Box<dyn epubfix::Fixer>> = fixers::all(&epubfix::Options::default())
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
    let all = epubfix::fixers::all(&epubfix::Options::default());
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
