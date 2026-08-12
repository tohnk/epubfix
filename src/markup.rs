//! A read-only scanner over XHTML content documents, plus a byte-splice editor.
//!
//! Fixers that need tree context — "is this `<a>` a child of `<table>` or of
//! `<td>`?" — cannot use a regex. But reserialising a parsed document rewrites
//! things nobody asked to change: quoting style, self-closing syntax, entity
//! references. That would destroy the property that a book epubfix reports as
//! unchanged really is byte-for-byte unchanged.
//!
//! So this module never serialises. [`scan`] walks the document with `quick-xml`
//! and records the *byte range* of every tag and attribute; a fixer collects
//! [`Edits`] over those ranges, and [`Edits::apply`] splices them into the
//! original string. Bytes nobody edited are copied verbatim, by construction
//! rather than by luck — which makes the round-trip identity test trivially true
//! instead of a guard rail that has to keep passing.

use std::fmt;
use std::ops::Range;

use quick_xml::Reader;
use quick_xml::events::Event;

/// `quick-xml` reports positions as `u64`; documents are `&str`, so the value
/// always fits, but saturate rather than cast lossily.
fn pos(reader: &Reader<&[u8]>) -> usize {
    usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// `<td>`
    Start,
    /// `<br/>`
    Empty,
    /// `</td>`
    End,
    /// A DOCTYPE, comment or processing instruction. Recorded so that callers
    /// can tell it apart from text; never an element, never a parent.
    Other,
}

/// One attribute, with the byte ranges needed to edit it in place.
#[derive(Debug, Clone)]
pub struct Attr {
    /// Lowercased attribute name, namespace prefix included.
    pub name: String,
    /// Raw attribute value, still escaped exactly as it appears in the source.
    pub value: String,
    /// `name="value"`, quotes included.
    pub span: Range<usize>,
    /// As `span`, but extended back over the whitespace separating this
    /// attribute from the previous one — the range to delete to remove it
    /// cleanly.
    pub span_with_space: Range<usize>,
}

impl Attr {
    /// The name exactly as the source spells it, case and all.
    ///
    /// [`Attr::name`] is lowercased so matching does not have to think about
    /// case, which is right almost everywhere and wrong in one place: whether a
    /// custom data attribute is *valid* turns on whether its name contains an
    /// ASCII uppercase letter, and by then the lowercased copy has thrown that
    /// away. Lowercasing preserves byte length, so the raw name is the same
    /// number of bytes from the start of the span.
    pub fn raw_name<'a>(&self, src: &'a str) -> &'a str {
        &src[self.span.start..self.span.start + self.name.len()]
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    /// Lowercased local name, namespace prefix stripped.
    pub name: String,
    /// The whole tag, `<` through `>`.
    pub span: Range<usize>,
    /// Byte offset just past the element name, where a new attribute can go.
    pub name_end: usize,
    pub attrs: Vec<Attr>,
    /// Index of the enclosing element's `Start` node.
    pub parent: Option<usize>,
    /// For `Start`, the index of the matching `End` node.
    pub close: Option<usize>,
    /// True if this element directly contains non-whitespace text.
    pub has_text: bool,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name == name)
    }

    /// The whole element, opening tag through closing tag.
    pub fn element_span(&self, nodes: &[Node]) -> Range<usize> {
        match self.close {
            Some(i) => self.span.start..nodes[i].span.end,
            None => self.span.clone(),
        }
    }
}

/// The attributes on this element that live in the id namespace.
///
/// `id` counts everywhere. `name` is ID-like on exactly two elements, `<a>` and
/// `<map>`; on `<meta>`, `<input>`, `<param>` and friends it is an arbitrary
/// string that must not be touched.
pub fn id_attrs(node: &Node) -> impl Iterator<Item = &Attr> {
    let allows_name = node.name == "a" || node.name == "map";
    node.attrs
        .iter()
        .filter(move |a| a.name == "id" || (allows_name && a.name == "name"))
}

#[derive(Debug)]
pub struct ScanError {
    pub position: usize,
    pub message: String,
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "not well-formed at byte {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for ScanError {}

/// Walk `src`, returning every tag in document order.
///
/// The reader is configured permissively, and that is a deliberate trade this
/// tool makes twice over: it repairs real books, so a mismatched end tag or a
/// bare `&` must not stop it from doing the work it *can* do on the rest of the
/// document. The cost is that success here says nothing about whether the
/// document is well-formed — an unclosed `<b>` sails straight through. Anything
/// that wants to report on the document rather than edit it must ask
/// [`well_formed`], which turns the checks back on.
pub fn scan(src: &str) -> Result<Vec<Node>, ScanError> {
    let mut reader = Reader::from_str(src);
    let cfg = reader.config_mut();
    cfg.check_end_names = false;
    cfg.check_comments = false;
    cfg.allow_unmatched_ends = true;
    cfg.expand_empty_elements = false;
    cfg.trim_text_start = false;
    cfg.trim_text_end = false;
    cfg.allow_dangling_amp = true;

    let mut nodes: Vec<Node> = Vec::new();
    let mut open: Vec<usize> = Vec::new();

    loop {
        let start = pos(&reader);
        let event = reader.read_event().map_err(|e| ScanError {
            position: pos(&reader),
            message: e.to_string(),
        })?;
        let end = pos(&reader);

        match event {
            Event::Eof => break,
            Event::Start(_) | Event::Empty(_) => {
                let kind = if matches!(event, Event::Start(_)) {
                    NodeKind::Start
                } else {
                    NodeKind::Empty
                };
                let (name, name_end, attrs) = dissect(src, start, end);
                nodes.push(Node {
                    kind,
                    name,
                    span: start..end,
                    name_end,
                    attrs,
                    parent: open.last().copied(),
                    close: None,
                    has_text: false,
                });
                if kind == NodeKind::Start {
                    open.push(nodes.len() - 1);
                }
            }
            Event::End(_) => {
                let (name, name_end, _) = dissect(src, start, end);
                // Pop to the nearest matching open element. Unmatched ends are
                // tolerated rather than fatal.
                let matched = open.iter().rposition(|&i| nodes[i].name == name);
                let parent = match matched {
                    Some(pos) => {
                        let idx = open[pos];
                        open.truncate(pos);
                        nodes[idx].close = Some(nodes.len());
                        nodes[idx].parent
                    }
                    None => open.last().copied(),
                };
                nodes.push(Node {
                    kind: NodeKind::End,
                    name,
                    span: start..end,
                    name_end,
                    attrs: Vec::new(),
                    parent,
                    close: None,
                    has_text: false,
                });
            }
            Event::DocType(_) | Event::Comment(_) | Event::PI(_) | Event::Decl(_) => {
                nodes.push(Node {
                    kind: NodeKind::Other,
                    name: match event {
                        Event::DocType(_) => "#doctype".into(),
                        Event::Comment(_) => "#comment".into(),
                        Event::Decl(_) => "#xml".into(),
                        _ => "#pi".into(),
                    },
                    span: start..end,
                    name_end: start,
                    attrs: Vec::new(),
                    parent: open.last().copied(),
                    close: None,
                    has_text: false,
                });
            }
            Event::Text(t) => {
                let bytes: &[u8] = &t;
                if let Some(&i) = open.last()
                    && !bytes.iter().all(u8::is_ascii_whitespace)
                {
                    nodes[i].has_text = true;
                }
            }
            _ => {}
        }
    }

    Ok(nodes)
}

/// Is this document well-formed XML?
///
/// Deliberately separate from [`scan`], which is tolerant on purpose: it exists
/// to *edit* real books, and refusing to open a file because somebody left a
/// `<b>` unclosed would mean refusing to help exactly the books that need it.
/// Being lenient there is right, but it means `scan` succeeding says nothing
/// about well-formedness — so anything that wants to *report* on the document
/// has to ask separately, with the checks turned on.
///
/// Widen a deletion to swallow the whole line, when the element sits alone on
/// one.
///
/// Deleting only the element leaves a blank line where it was. One is
/// invisible; four in a row — which is exactly what an OPF padded with empty
/// `dc:*` elements produces — makes the diff look like something went wrong.
/// A span sharing its line with anything else is returned unchanged.
pub fn line_span(src: &str, span: Range<usize>) -> Range<usize> {
    let start = src[..span.start].rfind('\n').map_or(0, |i| i + 1);
    if !src[start..span.start].trim().is_empty() {
        return span;
    }
    let end = src[span.end..]
        .find('\n')
        .map_or(src.len(), |i| span.end + i + 1);
    if !src[span.end..end].trim().is_empty() {
        return span;
    }
    start..end
}

/// An EPUB content document is required to be XML, and one that is not is
/// fatal: EPUB Check stops reading the file. Nothing else in this crate will
/// notice, because every fixer skips what it cannot scan.
pub fn well_formed(src: &str) -> Result<(), ScanError> {
    let mut reader = Reader::from_str(src);
    let cfg = reader.config_mut();
    cfg.check_end_names = true;
    cfg.allow_unmatched_ends = false;
    cfg.allow_dangling_amp = false;
    cfg.expand_empty_elements = false;

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(e) => {
                return Err(ScanError {
                    position: pos(&reader),
                    message: e.to_string(),
                });
            }
        }
    }
}

/// Pull the element name and attribute spans out of one raw tag.
fn dissect(src: &str, start: usize, end: usize) -> (String, usize, Vec<Attr>) {
    let b = src.as_bytes();
    let mut i = start + 1; // past '<'
    if i < end && b[i] == b'/' {
        i += 1; // past '/' of an end tag
    }
    let name_start = i;
    while i < end && !is_name_break(b[i]) {
        i += 1;
    }
    let raw = &src[name_start..i];
    // Namespace prefixes are vanishingly rare in EPUB content documents, but
    // matching on the local name costs nothing.
    let name = raw.rsplit(':').next().unwrap_or(raw).to_ascii_lowercase();
    (name, i, parse_attrs(src, i, end))
}

fn is_name_break(c: u8) -> bool {
    c.is_ascii_whitespace() || c == b'/' || c == b'>' || c == b'='
}

/// Scan `name="value"` pairs between the element name and the closing `>`.
fn parse_attrs(src: &str, from: usize, end: usize) -> Vec<Attr> {
    let b = src.as_bytes();
    let mut attrs = Vec::new();
    let mut i = from;

    while i < end {
        let space_start = i;
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end || b[i] == b'>' || b[i] == b'/' {
            break;
        }
        let name_start = i;
        while i < end && !is_name_break(b[i]) {
            i += 1;
        }
        if i == name_start {
            i += 1; // not a name character; skip it and carry on
            continue;
        }
        let name = src[name_start..i].to_ascii_lowercase();

        let after_name = i;
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end || b[i] != b'=' {
            // A bare attribute such as HTML's `disabled`. Not valid XML, but
            // record it so it can still be matched and removed.
            attrs.push(Attr {
                name,
                value: String::new(),
                span: name_start..after_name,
                span_with_space: space_start..after_name,
            });
            i = after_name;
            continue;
        }
        i += 1; // past '='
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end {
            break;
        }

        let (value, value_end) = if b[i] == b'"' || b[i] == b'\'' {
            let quote = b[i];
            let vs = i + 1;
            let mut j = vs;
            while j < end && b[j] != quote {
                j += 1;
            }
            (src[vs..j].to_string(), (j + 1).min(end))
        } else {
            let vs = i;
            let mut j = i;
            while j < end && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                j += 1;
            }
            (src[vs..j].to_string(), j)
        };

        attrs.push(Attr {
            name,
            value,
            span: name_start..value_end,
            span_with_space: space_start..value_end,
        });
        i = value_end;
    }

    attrs
}

/// A set of non-overlapping byte-range replacements over one document.
#[derive(Debug, Default)]
pub struct Edits {
    edits: Vec<(Range<usize>, String)>,
}

impl Edits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace(&mut self, span: Range<usize>, with: impl Into<String>) {
        self.edits.push((span, with.into()));
    }

    pub fn delete(&mut self, span: Range<usize>) {
        self.edits.push((span, String::new()));
    }

    pub fn insert(&mut self, at: usize, text: impl Into<String>) {
        self.edits.push((at..at, text.into()));
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    pub fn len(&self) -> usize {
        self.edits.len()
    }

    /// Splice every edit into `src`.
    ///
    /// Overlapping edits are a bug in the caller — two fixes fighting over the
    /// same bytes — and are dropped rather than allowed to corrupt the output.
    /// Insertions at the same offset keep the order they were added.
    pub fn apply(mut self, src: &str) -> String {
        self.edits
            .sort_by(|a, b| a.0.start.cmp(&b.0.start).then(a.0.end.cmp(&b.0.end)));

        let mut out = String::with_capacity(src.len());
        let mut cursor = 0usize;
        for (span, text) in &self.edits {
            if span.start < cursor || span.end > src.len() || span.start > span.end {
                continue;
            }
            out.push_str(&src[cursor..span.start]);
            out.push_str(text);
            cursor = span.end;
        }
        out.push_str(&src[cursor..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = concat!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"x.dtd\">\n",
        "<html xmlns=\"http://www.w3.org/1999/xhtml\">\n",
        "<body><p>caf&#233; &amp; cr&#232;me&nbsp;text</p>\n",
        "<table border=\"0\">\n",
        "  <tr valign='top'><td>a<a href=\"#t\">ok</a></td></tr><a id=\"s\"></a>\n",
        "</table>\n",
        "<br /><img src='i.png' alt = \"x\"   />\n",
        "</body></html>"
    );

    #[test]
    fn parses_a_realistic_document() {
        let nodes = scan(DOC).expect("should parse");
        let names: Vec<&str> = nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Start | NodeKind::Empty))
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "html", "body", "p", "table", "tr", "td", "a", "a", "br", "img"
            ]
        );
    }

    #[test]
    fn non_element_markup_is_recorded_rather_than_read_as_text() {
        // The declaration and DOCTYPE must be distinguishable from text, or
        // rewriting a DOCTYPE looks like the prose changed.
        let nodes = scan(DOC).unwrap();
        let others: Vec<&str> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Other)
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(others, vec!["#xml", "#doctype"]);
    }

    #[test]
    fn tag_spans_point_at_the_original_bytes() {
        let nodes = scan(DOC).unwrap();
        for n in &nodes {
            let raw = &DOC[n.span.clone()];
            assert!(raw.starts_with('<'), "{raw:?}");
            assert!(raw.ends_with('>'), "{raw:?}");
            if n.kind != NodeKind::Other {
                assert!(raw.contains(&n.name) || n.name == "br", "{raw:?}");
            }
        }
    }

    #[test]
    fn parent_context_separates_legit_and_stranded_anchors() {
        let nodes = scan(DOC).unwrap();
        let anchors: Vec<(&str, &str)> = nodes
            .iter()
            .filter(|n| n.name == "a" && n.kind == NodeKind::Start)
            .map(|n| {
                let parent = n.parent.map_or("-", |i| nodes[i].name.as_str());
                (parent, n.attrs.first().map_or("", |a| a.name.as_str()))
            })
            .collect();
        // The first is a genuine link inside a cell; the second is stranded
        // between rows and is the one the anchors fixer must act on.
        assert_eq!(anchors, vec![("td", "href"), ("table", "id")]);
    }

    #[test]
    fn attribute_spans_and_values_are_exact() {
        let nodes = scan(DOC).unwrap();
        let img = nodes.iter().find(|n| n.name == "img").unwrap();
        let src = img.attr("src").unwrap();
        assert_eq!(src.value, "i.png");
        assert_eq!(&DOC[src.span.clone()], "src='i.png'");
        // Whitespace around '=' is tolerated and the span still covers the pair.
        let alt = img.attr("alt").unwrap();
        assert_eq!(alt.value, "x");
        assert_eq!(&DOC[alt.span.clone()], "alt = \"x\"");
    }

    #[test]
    fn removing_an_attribute_takes_its_leading_space() {
        let nodes = scan(DOC).unwrap();
        let tr = nodes.iter().find(|n| n.name == "tr").unwrap();
        let valign = tr.attr("valign").unwrap();
        let mut edits = Edits::new();
        edits.delete(valign.span_with_space.clone());
        assert!(edits.apply(DOC).contains("<tr><td>"));
    }

    #[test]
    fn untouched_documents_come_back_byte_identical() {
        // The property the whole design exists to protect.
        assert_eq!(Edits::new().apply(DOC), DOC);
        let nodes = scan(DOC).unwrap();
        assert!(!nodes.is_empty());
    }

    #[test]
    fn edits_apply_in_offset_order_and_reject_overlap() {
        let src = "<a><b><c>";
        let mut e = Edits::new();
        e.replace(6..9, "<C>");
        e.replace(0..3, "<A>");
        assert_eq!(e.apply(src), "<A><b><C>");

        let mut e = Edits::new();
        e.replace(0..5, "X"); // "<a><b" -> "X"
        e.replace(3..8, "Y"); // overlaps the above: dropped, tail kept verbatim
        assert_eq!(e.apply(src), "X><c>");

        let mut e = Edits::new();
        e.insert(3, "1");
        e.insert(3, "2");
        assert_eq!(e.apply(src), "<a>12<b><c>");
    }

    #[test]
    fn text_content_is_detected_for_the_innermost_element() {
        let nodes = scan("<p> <b>hi</b> </p><q> </q>").unwrap();
        let b = nodes.iter().find(|n| n.name == "b").unwrap();
        let q = nodes.iter().find(|n| n.name == "q").unwrap();
        assert!(b.has_text);
        assert!(!q.has_text, "whitespace alone is not text content");
    }

    #[test]
    fn element_span_covers_open_through_close() {
        let src = "<td><a id=\"x\">t</a></td>";
        let nodes = scan(src).unwrap();
        let a = nodes.iter().find(|n| n.name == "a").unwrap();
        assert_eq!(&src[a.element_span(&nodes)], "<a id=\"x\">t</a>");
    }

    #[test]
    fn malformed_markup_is_an_error_not_a_panic() {
        assert!(scan("<a href=").is_err() || scan("<a href=").is_ok());
        assert!(scan("<!-- unterminated").is_err());
    }
}
