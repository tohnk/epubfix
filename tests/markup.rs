
/// A comment, DOCTYPE or processing instruction has no name, and the scanner
/// records that as `name_end == span.start`. Any call site doing the byte
/// arithmetic itself gets an inverted range and panics, which is exactly what
/// happened on a real book, so the accessor answers for them instead.
#[test]
fn nameless_nodes_have_an_empty_raw_name() {
    let src = "<!DOCTYPE html>\n<html><!-- note --><?pi x?><dc:title>T</dc:title></html>";
    let nodes = epubfix::markup::scan(src).unwrap();

    for node in &nodes {
        // The point is that none of these panic.
        let raw = node.raw_name(src);
        match node.name.as_str() {
            "#doctype" | "#comment" | "#pi" | "#xml" => assert_eq!(raw, ""),
            _ => assert!(!raw.is_empty(), "{:?} gave an empty name", node.name),
        }
    }
}

/// The raw name keeps the prefix and the case that [`Node::name`] throws away.
#[test]
fn a_raw_name_keeps_its_namespace_prefix() {
    let src = "<dc:title>T</dc:title>";
    let nodes = epubfix::markup::scan(src).unwrap();
    assert_eq!(nodes[0].raw_name(src), "dc:title");
    assert_eq!(nodes[0].name, "title");
    assert_eq!(nodes[1].raw_name(src), "dc:title", "and on the end tag");
}
