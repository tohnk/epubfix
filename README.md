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
| `filenames` | RSC-020, PKG-010 | renames resources whose filenames need URL escaping (spaces, non-ASCII, …) and updates every reference, raw or percent-encoded |
| `ncx-play-order` | RSC-005 | renumbers `toc.ncx` `playOrder` from 1, consecutive, one number per distinct target |
| `ncx-uid` | NCX-001 | syncs `dtb:uid` to the OPF `unique-identifier`, byte for byte |
| `version-mismatch` | RSC-005 | reports an EPUB 2 book whose markup only validates as EPUB 3 — diagnostic only, never rewrites |

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

## Migrating EPUB 2 to EPUB 3

Some books declare EPUB 2 but carry markup that only validates as HTML5 — most
often verse set in `<blockquote>`, which XHTML 1.1 requires to hold block-level
children while HTML5 accepts flow content. One real book reported **28,874
errors as EPUB 2 and 728 as EPUB 3**, from byte-identical markup. Repairing it
to satisfy XHTML 1.1 would mean wrapping ~13,500 inline runs in `<div>`s across
394 files, to work around one wrong attribute in the package document.

`version-mismatch` detects this and says so. `--migrate-epub3` is how you act on
it. Everything EPUB 3 additionally requires is derived from the file itself:

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

### Why this is safe to automate

Whether the EPUB 2 declaration was the mistake or the markup was is not
decidable, and does not need to be. Migration runs against a **clone** of the
book and is kept only if it preserved everything: no entry dropped, no id lost
or duplicated, no visible text changed, and every internal link still resolving.
If any of that fails the migration is abandoned and the book is left untouched.
Gating on the outcome sidesteps the unanswerable question.

It runs **before** every other fix, because the version decides what those fixes
should do — `legacy-table-attrs` strips a much larger set under HTML5 rules, and
`img-alt` stops applying at all.

It is **off by default**. Migration changes the file's format identity, and some
older reading systems are EPUB 2 only. That is a policy choice, not something
the tool can compute.

```sh
epubfix --dry-run -r ~/Books        # which books would migration help?
epubfix --migrate-epub3 book.epub   # then act on it
```

## Usage

```
epubfix [OPTIONS] [FILE_OR_DIR ...]

-n, --dry-run       report what would change; write nothing
    --no-backup     do not keep a .bak copy of the original
-r, --recursive     descend into subdirectories when scanning a folder
    --migrate-epub3 convert EPUB 2 books to EPUB 3 first
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
- **The replacement is staged.** Each book is rebuilt into a sibling
  `.epubfix.tmp` and moved into place only once it is complete, so an
  interrupted run cannot leave a truncated book behind. A failed run removes the
  temporary file.
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
content-document test runs automatically. After a repair it asserts that no
entry vanished, no id became a duplicate, and **every internal link still
resolves to a file that exists and an id that exists in it** — the check that
catches a fix which silences epubcheck while quietly breaking navigation.
`tests/verify.rs` proves those checks actually fire rather than passing
vacuously.

Verified end to end against EPUB Check 5.2.1. Fixture books carrying every
defect above validate with zero errors afterwards, as **both** EPUB 2 (9 errors
to 0) and EPUB 3 (20 errors to 0); a Coleridge-shaped EPUB 2 book with verse in
`<blockquote>`, legacy `opf:` metadata, a nested NCX with a `pageList` and
uncaptioned images goes from 24 errors to 0 under `--migrate-epub3`.

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
