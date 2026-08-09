# epubfix

Repairs common EPUB validation errors in place — the recurring ones that
epubcheck flags across a library of books. Handles both EPUB 2 and EPUB 3, and
knows the difference: several of these fixes are only correct for one of them.

Drop the executable into a folder full of `.epub` files and run it. With no
arguments it scans the folder the executable itself lives in (not the working
directory), so double-clicking it works.

```
$ epubfix
Scanning /home/you/Books ...
Broken Book.epub:
    package version 1.0 -> 2.0
    removed spine/@page-map
    sanitised 2 id(s)
    stripped 697 legacy attribute(s) for EPUB 3 [valignx693, borderx4]
    index_split_212.html: 6 empty anchor(s) removed, 8 anchor id(s) moved to a legal element
    renamed 1 file(s)
    renumbered playOrder (3 target(s))
    synced NCX dtb:uid to OPF identifier
Fine Book.epub: nothing to do

Done: 1 fixed, 1 already clean, 0 failed.
```

## What it fixes

| Fixer | epubcheck | What it does |
| --- | --- | --- |
| `opf-version` | OPF-001 | `<package version="1.0">` (OEBPS 1.0) → `"2.0"` |
| `spine-page-map` | RSC-005 | drops the Adobe `<spine page-map="...">` extension |
| `font-media-type` | CSS-007 | fixes the `application/application/x-font-ttf` typo |
| `xml-ids` | RSC-005 | rewrites `id`/`name` values that are not valid XML Names, and every `href`/`src` fragment pointing at them |
| `legacy-table-attrs` | RSC-005 | strips presentational table attributes (`valign`, `align`, `bgcolor`, `nowrap`, …) the book's ruleset rejects, and clamps `border` |
| `img-alt` | RSC-005 | adds `alt=""` to decorative images in EPUB 2, and reports the rest rather than inventing captions |
| `misplaced-anchors` | RSC-005 | removes or rehomes `<a>` elements stranded between table rows, keeping every link target alive |
| `dangling-resources` | RSC-007 | repoints references whose file moved, and drops dead stylesheet/script includes — never an `<img>` or `<a>` |
| `broken-fragments` | RSC-012 | recovers undefined fragment targets via backlinks or unique relocation, else drops the fragment |
| `ncx-dead-entries` | RSC-007 | removes navigation entries pointing at documents that are not in the book |
| `ncx-duplicate-ids` | RSC-005 | makes duplicated NCX ids unique, leaving any that are referenced alone |
| `ncx-pagelist-attrs` | RSC-005 | gives `<pageList>` both of the attributes its DTD requires, or neither |
| `filenames` | RSC-020, PKG-010 | renames resources whose filenames need URL escaping (spaces, non-ASCII, …) and updates every reference, raw or percent-encoded |
| `ncx-play-order` | RSC-005 | renumbers `toc.ncx` `playOrder` from 1, consecutive, one number per distinct target |
| `ncx-uid` | NCX-001 | syncs `dtb:uid` to the OPF `unique-identifier`, byte for byte |
| `version-mismatch` | RSC-005 | reports markup that does not match the declared version, where the evidence is too weak to retag on — diagnostic only |

`epubfix --list` prints the same table.

### EPUB 2 and EPUB 3 are not the same job

`legacy-table-attrs` reads the OPF `<package version>` and behaves differently,
because the two rulesets genuinely differ:

* **EPUB 3** content documents are validated as HTML5, which removed the whole
  presentational set and restricts `border` to `""` or `"1"`.
* **EPUB 2** content documents are validated as XHTML 1.1, which *keeps*
  `align` and `valign` on rows and cells, and `cellpadding`, `cellspacing`,
  `frame`, `rules`, `width` and any `border` on `<table>`. Only `align`,
  `valign` and `bgcolor` on `<table>`, `bgcolor` everywhere else, cell
  `width`/`height`/`nowrap`, and `hspace`/`vspace` are errors.

Stripping attributes that EPUB 2 permits would change how those books render for
no validation benefit, so it does not. The split was **measured against EPUB
Check 5.2.1**, not read off a specification, and `tests/content.rs` pins it.

The same version split decides how `misplaced-anchors` repairs things. Moving a
stranded `<a>` to just after `</table>` is valid in EPUB 3, but in EPUB 2 an
`<a>` cannot be a child of `<body>` — doing it there would trade one epubcheck
error for another. So the default repair is to migrate the anchor's *id* onto
the nearest legal element (the following row, else the preceding one, else the
table), which is valid under both rulesets and keeps the link landing in the
same place.

## Version retagging

Books are often declared as the wrong EPUB version, and it happens in both
directions. **The declaration is one attribute; the content is thousands of
elements.** So when they disagree, epubfix moves the declaration to match the
content — not the other way round. This is the default, in both directions.

Toward EPUB 2 it is not even a choice: there is no XHTML 1.1 spelling of a nav
document or of inline SVG, so a genuinely EPUB 3 book cannot be rewritten
downward at all.

| The book | What happens |
| --- | --- |
| declares EPUB 2, content needs HTML5 | **upgraded**, and given the nav document, metadata and manifest properties EPUB 3 requires |
| declares EPUB 3, nothing needs HTML5 | **downgraded** — one attribute, plus stripping the EPUB 3-only package constructs |
| declares EPUB 3, content needs HTML5 | declaration kept, book repaired forward to satisfy it |
| evidence mixed or weak | declaration kept, and the book is reported |

Measured against EPUB Check 5.2.1, on a default run with no flags:

```
A  EPUB 2 content, declared 3   v3.0  1 fatal + 11 errors  ->  v2.0  0
B  HTML5 content, declared 2    v2.0        24 errors      ->  v3.0  0
C  HTML5 content, declared 3    v3.0  1 fatal + 13 errors  ->  v3.0  0
```

### The two-sided test

A book can carry evidence of both, so one signal is not enough. Case C above has
every EPUB 2 marker there is — XHTML 1.1 DOCTYPEs, bare `&mdash;`, `opf:role`,
no nav — and is still not downgraded, because its verse only validates as HTML5.
Downgrading would trade a handful of errors for a great many.

**Decisive** (any occurrence means the book is EPUB 3): HTML5-only elements,
`epub:` attributes, `<meta charset>`, inline SVG or MathML, a nav document,
`<meta refines>` metadata.

**Suggestive** (only counts in quantity): inline content inside `<blockquote>`
and other block-only containers. A handful of these means a few paragraphs need
a `<div>`, not that the whole book is the wrong format — retagging on that
evidence would be wildly out of proportion. Past a threshold the balance flips:
repairing would mean hundreds of edits, so the one wrong attribute is
overwhelmingly the likelier error. Below it, the book is reported and left
alone.

**EPUB 2 markers** (suggestive, and repairable either way): XHTML 1.1 DOCTYPEs,
named character entities, `opf:role`/`file-as`/`scheme`, an NCX with no nav.

### What EPUB 3 requires, and where it comes from

| Requirement | Where it comes from |
| --- | --- |
| `<package version="3.0">` | attribute edit |
| `<!DOCTYPE html>` in content documents | replaces the XHTML 1.1 DOCTYPE |
| named entities rewritten as numeric | XHTML 1.1's DTD declared them; HTML5's declares none |
| one `dcterms:modified` | generated, and only when absent |
| one manifest item with `properties="nav"` | a nav document built from the NCX |
| `opf:role` / `opf:file-as` / `opf:scheme` | rewritten as `<meta refines>` |
| `properties="svg\|scripted\|mathml\|remote-resources"` | scanned from each document |

The NCX becomes a real nav document: `navMap` nesting turns into nested `<ol>`,
`pageList` into `<nav epub:type="page-list">`, and every `href` is rebased to
the nav document's directory. The NCX itself stays in the manifest, since EPUB 3
still permits it and EPUB 2 readers fall back to it.

Two of those rows are not in any specification I was given — they came from
running EPUB Check against a migrated book. The entity one is **fatal**:
`<!DOCTYPE html>` declares no named entities, so a single unconverted `&mdash;`
stops the parse dead.

A downgrade is the mirror: the content documents are already right, so only the
package changes — the version, the `<meta property>` and `properties=`
constructs that would now be errors, and `spine/@toc` so EPUB 2 can find the
NCX.

### Why this is safe to automate

Whether the declaration was the mistake or the markup was is not decidable, and
does not need to be. Every retag runs against a **clone** of the book and is
kept only if it preserved everything. Gating on the outcome sidesteps the
unanswerable question.

**The gate is differential, not absolute** — and that distinction is not
pedantry. An earlier version asserted that every internal reference in the
result resolves, which sounds right and is wrong: it refused nine books out of a
real 37-book library, every one of them over a defect already present in the
input — a `<link>` to a `page-template.xpgt` Calibre had dropped, a Kobo
`<script>` with no file behind it, fragments pointing at ids that never existed.
None of it had anything to do with the operation being gated, and refusing on
that basis turns away exactly the books that most need help.

So the question is never "is the result perfect" but "did this make anything
worse": no entry dropped, no id lost or *newly* duplicated, no visible text
changed, and no reference that used to resolve that stopped. A book is not
required to arrive undamaged to be helped.

Retagging runs **before** every other fix, because the version decides what
those fixes should do — `legacy-table-attrs` strips a much larger set under
HTML5 rules, and `img-alt` stops applying at all.

```sh
epubfix --dry-run -r ~/Books        # what would move, and why
epubfix --keep-version book.epub    # repair, but never touch the declaration
epubfix --migrate-epub3 book.epub   # force EPUB 3 even if unnecessary
```

## Usage

```
epubfix [OPTIONS] [FILE_OR_DIR ...]

-n, --dry-run       report what would change; write nothing
    --no-backup     do not keep a .bak copy of the original
-r, --recursive     descend into subdirectories when scanning a folder
    --keep-version  never change a book's declared EPUB version
    --migrate-epub3 force an upgrade to EPUB 3 even if unnecessary
    --only NAMES    run only these fixers (comma-separated, see --list)
-l, --list          list the available fixers and exit
    --pause         wait for Enter before exiting
-h, --help          show help
-V, --version       show the version
--                  treat all remaining arguments as paths
```

| Exit | Meaning |
| --- | --- |
| 0 | all good |
| 1 | at least one book failed to process |
| 2 | bad arguments |
| 3 | everything worked, but some books need manual attention |

### Needs manual attention

A fixer that meets something it recognises as wrong but cannot repair safely
records a **finding** rather than guessing — a stranded anchor with visible text
in an EPUB 2 book, a stray `<p>` inside a `<table>`, a document that is not
well-formed. Findings never modify the book; they print at the end of the run
and set exit status 3.

`epubfix --dry-run -r ~/Books` is therefore a triage pass over a whole library:
it writes nothing and tells you which handful of books have something genuinely
unusual in them.

```sh
epubfix --dry-run ~/Books          # see what it would do
epubfix -r ~/"Calibre Library"     # a whole library
epubfix --only ncx-uid book.epub   # just one fix
```

## Safety

- **Nothing is written unless something actually changed.** A clean book keeps
  its exact bytes, so re-running over a library is free and idempotent.
- **One read, one write, whatever happens.** The book is loaded into memory
  once; migration, conformance repair and every fixer all run there, in order,
  on that single copy. Only the finished result reaches the disk. Converting a
  book to EPUB 3 does not produce an intermediate file that then gets rewritten
  — after any run there are exactly two files: the repaired book, and one
  `.bak` holding the original.
- **The replacement is staged.** Each book is written to a sibling
  `.epubfix.tmp` and *renamed* over the original — a rename, not a second copy —
  only once it is complete, so an interrupted run cannot leave a truncated book
  behind. A failed run removes the temporary file.
- **A `.bak` is kept** next to each modified book. An existing `.bak` is never
  overwritten, so a second run cannot replace the pristine original with an
  already-modified copy. `--no-backup` opts out.
- **Resources that are not valid UTF-8 are never decoded or touched** — images,
  fonts, and legacy-encoded stylesheets pass through byte for byte.
- **Content documents are edited by byte splice, never reserialised.** The
  scanner records the byte range of every tag and attribute and the fixers
  replace only those ranges, so a document keeps its exact quoting, self-closing
  syntax and entity references. Anything no fixer touched is unchanged by
  construction, not by luck.
- **Links are checked before anchors are touched.** An anchor is only deleted
  when nothing in the book — content document, NCX or OPF — links to it, and an
  id is only moved onto an element that does not already have one.
- **`mimetype` is rewritten first and uncompressed**, with no extra field, as
  OCF requires.
- Entry order, per-entry compression method, and timestamps are preserved.

EPUB/OCF permits only the Stored and Deflate compression methods, and that is
all this tool reads. An archive using anything else is reported as failed rather
than silently mangled.

## Building

```sh
cargo build --release      # target/release/epubfix
```

Rust 1.88 or newer. There are no C dependencies, so a Windows binary can be
cross-compiled from Linux with:

```sh
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

## Adding a fix

The recurring errors above are just the ones encountered so far. A new fix is a
unit struct implementing `Fixer`:

```rust
pub trait Fixer {
    fn name(&self) -> &'static str;              // stable id, used by --only
    fn codes(&self) -> &'static [&'static str];  // epubcheck message ids
    fn description(&self) -> &'static str;
    fn apply(&self, book: &mut Book) -> Outcome; // changes + findings
}
```

1. Add the struct to a module under `src/fixers/` — `opf.rs`, `ncx.rs`,
   `ids.rs`, `filenames.rs`, `tables.rs`, `anchors.rs`, or a new file.
2. Register it in `fixers::all()`. Fixers run in list order, and the one
   ordering constraint that matters is that everything touching internal links
   runs *before* `filenames`, which renames the resources those links point at.
3. Add a test covering both the broken input and an already-clean one —
   `tests/fixers.rs` for packaging fixes, `tests/content.rs` for anything
   inside a content document.

`Book` gives you the pieces without any zip handling:

```rust
book.opf_text()        // package document, if present and decodable
book.ncx_text()        // NCX table of contents
book.text(name)        // any decodable entry, by archive name
book.set_text(name, s) // replace one
book.for_each_text(|name, text| { ... })    // every decodable entry, in order
book.for_each_markup(|name, text| { ... })  // .html / .xhtml / .htm only
book.rename(old, new)  // schedule a rename, applied on repack
book.epub_version()    // 2 or 3, from the OPF - check this before assuming a rule
book.reference_index() // what links where, for deciding if an anchor is live
```

For anything that depends on where an element sits in the tree, use the scanner
rather than a regex:

```rust
use epubfix::markup::{scan, Edits};

let nodes = scan(text)?;              // every tag, with parent, spans, attrs
let mut edits = Edits::new();
edits.delete(node.attr("valign").unwrap().span_with_space.clone());
edits.insert(node.name_end, " id=\"x\"");
let fixed = edits.apply(text);        // untouched bytes copied verbatim
```

Three rules make the whole thing safe: a fixer must be a **no-op on input it
does not recognise**; it must **report only changes it actually made**, since a
non-empty change list is what decides whether the file gets rewritten at all;
and when it is not sure, it must **record a finding rather than guess**.

## Tests

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

66 tests. Each fixer is checked against both broken and already-clean input,
alongside repacking, backups, dry runs, rename collisions, non-UTF-8
passthrough, and idempotency.

`tests/common/verify.rs` is a reusable structural harness that every
content-document test runs automatically. It is differential in the same way the
library gate is: after a repair it asserts that no entry vanished, no id became
*newly* duplicated, no visible text changed, and no reference that used to
resolve stopped resolving — the check that catches a fix which silences
epubcheck while quietly breaking navigation. `tests/verify.rs` proves those
checks fire on a real regression and stay quiet on a pre-existing defect.

Two rules learned the hard way and worth stating: **when a schema requires a set
of attributes, supply the whole set or none** — adding only `class` to a
`<pageList>` pushed one book onto the strict validation path and introduced an
error epubcheck had not been reporting. And **verify by re-running epubcheck,
not by reasoning about the fix**: the error count must strictly decrease and no
new error *code* may appear.

Verified end to end against EPUB Check 5.2.1. Fixture books carrying every
defect above validate with zero errors afterwards, as **both** EPUB 2 (9 errors
to 0) and EPUB 3 (20 errors to 0); a Coleridge-shaped EPUB 2 book with verse in
`<blockquote>`, legacy `opf:` metadata, a nested NCX with a `pageList` and
uncaptioned images goes from 24 errors to 0, and a book declaring EPUB 3 while written as EPUB 2
goes from 1 fatal + 11 errors to 0 by being retagged downward.

## Origin

A port of `epubfix.py`, matching its behaviour on the packaging fixes (including
the exact sanitising rules) while adding rename-collision handling, staged
writes, backup preservation, folder scanning, and the fixer registry.

It has since grown past that scope: it handles EPUB 3 as well as EPUB 2, repairs
content documents rather than only packaging metadata, and can migrate a book
between the two.

Two bugs inherited from the original are fixed here. `name` was treated as a
synonym for `id` and sanitised everywhere, which rewrote valid markup such as
`<meta name="calibre:cover">` and would have mangled any form field with a colon
in its name; it is now restricted to `<a>` and `<map>`, where the attribute is
genuinely ID-like. And sanitising could rename `a:b` onto a document's existing
`a_b`, trading one epubcheck error for a duplicate-id error; ids are now scoped
per document and checked for collisions.
