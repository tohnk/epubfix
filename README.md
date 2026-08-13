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
| `encoding` | CSS-003, CSS-004, RSC-027, RSC-028, HTM-058 | re-encodes a UTF-16 text entry as UTF-8 and corrects what it declares |
| `mimetype` | PKG-005, PKG-006, PKG-007 | puts the OCF `mimetype` entry first, uncompressed, with exactly the required bytes |
| `container-rootfile` | OPF-016, OPF-017 | points `container.xml` at the package document when its `full-path` is missing or empty |
| `opf-version` | OPF-001 | `<package version="1.0">` (OEBPS 1.0) → `"2.0"` |
| `spine-page-map` | RSC-005 | drops the Adobe `<spine page-map="...">` extension |
| `media-types` | CSS-007, OPF-035, OPF-037 | replaces a manifest `media-type` that is mistyped or superseded |
| `manifest-items` | OPF-091, OPF-099 | drops a fragment from a manifest `href`, and the entry a manifest makes for itself |
| `package-references` | OPF-031, RSC-001, RSC-007 | repoints manifest and guide hrefs whose file moved, and drops the ones with no file behind them — never a document the spine names |
| `empty-metadata` | OPF-054 | removes `<dc:*>` elements with no content, keeping the three both versions require |
| `unique-identifier` | OPF-030 | makes `<package unique-identifier>` and the `<dc:identifier>` id agree, by creating the id or by moving the pointer |
| `dc-language` | RSC-005 | adds the required `<dc:language>`, taken from what the documents declare or from the text — never from the machine's locale |
| `fragment-documents` | RSC-005 | gives a bare markup fragment the document *and* the block container XHTML 1.1 needs inside `<body>` |
| `head-content` | RSC-005 | removes empty elements a `<head>` may not hold, and reports any carrying text |
| `content-type-meta` | RSC-005 | corrects the `<meta http-equiv="content-type">` value HTML5 fixes, which XHTML 1.1 never checked |
| `truncated-documents` | RSC-016 | closes the elements a document left open when it stopped mid-air |
| `xhtml-namespace` | RSC-005 | declares the XHTML namespace on a root `<html>` missing it, which otherwise fails the whole document |
| `anchor-names` | RSC-005 | replaces the `name` attribute XHTML 1.1 removed from `<a>` with the `id` it stood for |
| `xml-ids` | RSC-005 | rewrites `id`/`name` values that are not valid XML Names — in content documents and the NCX — and every fragment pointing at them |
| `duplicate-ids` | RSC-005 | makes duplicated ids unique in content documents and the NCX alike, keeping the first — which is the one every link already resolves to |
| `data-attributes` | HTM_061 | removes custom data attributes whose names HTML5 rejects (Kindle's `data-AmznRemoved`) |
| `keyword-case` | RSC-005 | lower-cases an enumerated attribute value (`dir="LTR"`, `valign="TOP"`) that XHTML 1.1 spells in lower case |
| `legacy-table-attrs` | RSC-005 | strips presentational table attributes (`valign`, `align`, `bgcolor`, `nowrap`, …) the book's ruleset rejects, and clamps `border` |
| `underline-elements` | RSC-005 | turns the removed `<u>` into a `<span>` that still underlines |
| `img-alt` | RSC-005 | adds `alt=""` to decorative images in EPUB 2, and reports the rest rather than inventing captions |
| `nested-anchors` | RSC-005 | unwraps an `<a>` nested inside another, keeping its text and rehoming any id on a `<span>` |
| `misplaced-blockquotes` | RSC-005 | splits a paragraph around a `<blockquote>` it swallowed, or demotes the quotation to a `<span>` |
| `inline-in-block` | RSC-005 | wraps a short run of inline content in a `<div>` where XHTML 1.1 wants a block, instead of retagging the book |
| `misplaced-anchors` | RSC-005 | removes or rehomes `<a>` elements stranded between table rows, keeping every link target alive |
| `guide-references` | OPF-032, RSC-005 | drops OPF `guide` entries pointing at something that is not a content document, and the whole `<guide>` when that empties it |
| `orphan-links` | RSC-007 | repoints a link whose anchor was discarded at the heading its own text names |
| `dangling-resources` | RSC-007 | repoints references whose file moved; drops dead `<link>`/`<script>` includes, and an `<img>` whose file is proven absent — never an `<a>` |
| `css-paths` | RSC-007 | repoints `url()` and `@import` in stylesheets, and drops dead `@font-face` rules, imports and declarations |
| `dead-schemes` | HTM-025 | repoints links using a reading system's private scheme (`kindle:`, `calibre:`, …) at what the package says they are for, or drops the `href` when nothing does |
| `broken-fragments` | RSC-012 | recovers undefined fragment targets via backlinks or unique relocation, else drops the fragment — and unlinks a same-document one with nothing left to point at |
| `reference-fragments` | RSC-009, RSC-013 | drops a fragment from a reference to a stylesheet or raster image, which cannot have one |
| `ncx-dead-entries` | RSC-007 | removes navigation entries pointing at documents that are not in the book |
| `ncx-pagelist-attrs` | RSC-005 | completes the co-required `id`/`class` pair on a `<pageList>` that carries only one |
| `ncx-entry-ids` | RSC-005 | gives every NCX navigation entry the `id` its schema requires |
| `undeclared-resources` | RSC-008 | declares a file the book uses that the manifest never mentions |
| `filenames` | PKG-009, PKG-010, PKG-011, OPF-060, RSC-020 | renames resources whose filenames a URL cannot address — illegal characters, a trailing dot, or two entries differing only by case — and updates every reference |
| `ncx-play-order` | RSC-005 | renumbers `toc.ncx` `playOrder` from 1, consecutive, one number per distinct target |
| `ncx-uid` | NCX-001 | syncs `dtb:uid` to the OPF `unique-identifier`, byte for byte |
| `version-mismatch` | RSC-005 | reports markup that does not match the declared version, where the evidence is too weak to retag on — diagnostic only |

`epubfix --list` prints the same table.

### EPUB 2 and EPUB 3 are not the same job

Five fixers read the OPF `<package version>` and behave differently either side
of it. `img-alt` and `version-mismatch` are EPUB 2 only, and `content-type-meta`
is EPUB 3 only — XHTML 1.1 does not check that value at all, so a book only
meets this one on the way across. `data-attributes`
removes malformed names in both, but under XHTML 1.1 *every* `data-*` attribute
is an error — valid name or not — and deleting the well-formed ones would be
data loss to satisfy a declaration that is very likely the thing at fault, so
those are reported instead. And `legacy-table-attrs` differs most of all,
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

Removing one is not automatically invisible either, and this is where the
default stopped being "strip". A presentational attribute contributes at the
presentational-hints origin, *below* author stylesheets. So the book has already
answered the question:

* **a stylesheet sets the property** — the attribute has been inert for as long
  as the book has existed, and removing it cannot change the page;
* **no stylesheet sets it** — the attribute is the only thing holding that
  layout up, and removing it drops to the user-agent default.

So each attribute is asked individually: **rehoused in the element's `style`
when nothing was overriding it, stripped when something was.** `valign="top"`
becomes `vertical-align: top`, `align="left"` on an image becomes `float: left`
rather than `text-align`, and `cellpadding` — which has no single-property
equivalent, since it describes the cells — is removed and reported.

*The Hero of Ages* is what forced this. Its Ars Arcanum table carries `width` on
75 cells, its stylesheet declares no width at all, and stripping all 75 reflows
a reference table people actually consult. In the same run its 191 `valign`,
`text` and `link` attributes *were* covered by rules and are simply removed:

```
moved 75 legacy attribute(s) into inline CSS and stripped 191 that a
stylesheet already overrode, for EPUB 2 [linkx95, textx95, widthx75, ...]
```

No single policy is right for both, which is why there is no longer a single
policy. This is also the one place the CSS-override check ([`src/css.rs`])
changes bytes rather than a line of report, and that is only tolerable because
its bias runs the safe way: it matches element names and classes anywhere in a
selector, so it *over*-reports "declared", which lands on stripping — exactly
what the tool did before.

Both absolutes are still available. `--preserve-presentation` converts
everything, including attributes a stylesheet was overriding; since an inline
style sits *above* author rules where the attribute sat below them, that is the
one setting that can change rendering a book was previously getting right.
`--strip-presentation` removes everything, which is the tidiest markup and the
least faithful page.

The same version split decides how `misplaced-anchors` repairs things. Moving a
stranded `<a>` to just after `</table>` is valid in EPUB 3, but in EPUB 2 an
`<a>` cannot be a child of `<body>` — doing it there would trade one epubcheck
error for another. So the default repair is to migrate the anchor's *id* onto
the nearest legal element (the following row, else the preceding one, else the
table), which is valid under both rulesets and keeps the link landing in the
same place.

### Finding what a broken reference meant

There are several ways a path can be wrong and only one way it can be right, so
rather than special-casing each mistake, `dangling-resources` and `css-paths`
generate the paths a reference could plausibly have meant and let **existence in
the archive** decide. First candidate that exists wins:

| | Candidate | The mistake it catches |
| --- | --- | --- |
| 1 | relative to the referring file | none — this is the correct reading |
| 2 | relative to the archive root | a relative URL written as if root-relative |
| 3 | relative to the package document | the same, anchored on the OPF |
| 4 | the unique entry ending with the longest run of the written path | the file moved |
| 5 | as 4, ignoring case | `styles/` written for `Styles/` |

Candidate 2 is *Butcher's Crossing*: `OEBPS/Styles/nyrb.css` contains
`url(OEBPS/Fonts/AGaramondPro-Regular.otf)`, and CSS resolves against the
stylesheet rather than the document that links it, so that reads as
`OEBPS/Styles/OEBPS/Fonts/…` — the doubled directory epubcheck reports. The root
is taken from the archive rather than assumed: real books use `OPS/`, `ops/`,
`OEBPS/html/` and `CompletePoems/`.

Candidates 4 and 5 require the match to be unique, and they use as much of the
written path as still exists rather than the filename alone — a reference names
a directory too, and that is evidence. With `OEBPS/assets/Images/plate.jpg` and
`OEBPS/Thumbs/plate.jpg` both in the book, `Images/plate.jpg` picks out one of
them; matching on `plate.jpg` would throw the directory away and report a tie it
did not have to. Suffixes are tried longest-first on segment boundaries, and
because a shorter suffix always matches at least as many files as a longer one,
the first length that matches anything is the most specific there is — if that
one is ambiguous, so is every shorter one, and the answer is a report.

When nothing resolves, what gets deleted is whatever unit has become
meaningless — the whole `@font-face` (a face with no source is nothing), the
`@import` statement, or just the one declaration.

An `<a>` is never deleted: it carries navigation, and its absence is a defect to
report — and often a defect that can be repaired instead. *Girl With Curious
Hair* has a contents page of ten links to Word bookmarks (`href="_Toc73360389"`,
no `#`, so epubcheck calls it a missing *file*) which Calibre discarded when it
split the book at those very anchors. Deleting them would validate and would
delete the book's table of contents. But the link text *is* the destination —
each of the ten is the exact title of a story, and each story document opens
with an `<h1>` carrying that title and an id — so `orphan-links` repoints them
and the contents page works again. The match must be **exact** on
whitespace-collapsed, case-folded text, and **unique**: two headings with the
same words means the link is reported, because a contents page pointing at the
wrong chapter is worse than one pointing nowhere. An `<img>` used to be treated the same way, and that was too absolute.
The distinction that matters is not the element type but whether the file might
exist somewhere, and by the time all five candidates have failed it does not —
not under any path, spelling or case. The choice is then between an element that
renders as a broken-image placeholder forever and no element at all, on a page
that is already not showing what it should. So it goes, with three constraints:

* **the wrapper goes too, when the image was its only content.** In *The Hobbit*
  the `<img>` sits alone in `<p class="ct-2">`, and an empty paragraph can still
  take vertical space. Only one level — a `<div>` holding other material stays,
  and so does a wrapper holding a caption.
* **unless that would empty the `<body>`.** Measured: removing the lone wrapper
  produces `element "body" incomplete`, a worse error than the RSC-007 being
  fixed. The empty `<p>` stays and the book still reaches zero.
* **it is reported by name**, with the alt text when there is one, as a change
  rather than a finding. This is the one repair that removes something a reader
  could have seen.

`--keep-missing-images` restores the old behaviour. It is off by default because
the page is already broken and the `.bak` beside the book keeps the record.

### Working out a book's language

`dc:language` is required by both versions, and epubcheck accepts whatever it
finds there — `<dc:language>zz</dc:language>` validates clean. So this is a
field where being wrong is *silent*, and being wrong costs something: reading
systems pick hyphenation dictionaries, speech voices and sometimes fonts off it.
An absent value falls back to something sensible; a wrong one mis-hyphenates
every page.

Which rules out the obvious shortcut. Calibre writes a default from the
**system locale** without reading the book, so a Hungarian novel that passes
through Calibre on an English machine comes out declaring `en`. That is a guess
wearing the costume of metadata.

1. **What the documents already say.** `xml:lang` or `lang` on `<html>` is a
   stated fact in the file — the same class of evidence as the NCX identifier
   that gets synced to the OPF. Used whatever the detection policy says, and
   reduced to its primary subtag, since nothing can tell `en-GB` from `en-US`.
2. **Detection from the text**, only if the first step found nothing.
3. **A report.** Never a locale, never a default.

Every sampling rule comes from a book that broke a simpler version. *The
Complete Works of Aristotle* votes thirteen English to two Latin — the two are
footnote files of nothing but Latin citations, so one unlucky sample declares
the book Latin with confidence. *A Supposedly Fun Thing I'll Never Do Again* has
323 documents with a **median length of 281 characters**, because Calibre split
it into fragments, so a fixed "take fifteen documents" yields two usable
samples. Hence: skip the front matter, glue short documents together rather than
discarding them, require at least three samples, and require more than half.

`--language-detect` decides when a *detected* language may be written.
`en-only` is the default, and not because detection is worse in other languages
— it isn't. It is about the shape of the failure: restricted to English the
worst case is "did nothing, the error remains", unrestricted it is a book
confidently labelled wrong, which nobody ever notices. `any` writes whatever the
text says; `off` uses only what the documents declare.

### Repairing a dead link, not just silencing it

A Kindle-derived book carries `<a href="kindle:embed:0001?mime=image/jpg">`,
which names a position in a different file format — dead for every reader of the
EPUB. Dropping the `href` clears the warning and leaves the document valid, but
when the anchor is a landmark it also leaves the reader's *go to cover* doing
nothing. That is silencing the warning rather than fixing what it is about.

Often the anchor says what it is for. `epub:type="cover"` is not a hint to be
interpreted, it is a declaration, and EPUB 3 says where those things live:

| `epub:type` | Where the package says it is |
| --- | --- |
| `cover` | the manifest names the cover *image* via `properties="cover-image"`; the cover *document* is whichever content document displays it |
| `toc` | the manifest item with `properties="nav"` |

Both are derivations, not preferences — the archive answers them. The cover case
requires the match to be unique, so a book where two documents show the cover
image gets a report instead. Anything else keeps the old behaviour: the `href`
goes, the text and any `id` stay, and if the anchor was inside a `<nav>` the
lost entry is named, because nothing in the package says where it should have
gone.

### The package document is where the archive is described, and it lied

`dangling-resources` has resolved broken references since the beginning, and
until now it could not see the one file whose whole job is to say where things
are: it walks the book's *markup*, and the package document is not markup. So a
manifest or `guide` naming a file that is not where it says was the one class of
broken reference nothing checked. Seventeen books in a 200-book library have a
defect of that shape.

`package-references` runs the same [resolver](#finding-what-a-broken-reference-meant)
over the OPF, and the answer is usually that the file is *there* and the path is
wrong — an abbyy-to-epub *True Hallucinations* keeps its twenty chapters at the
archive root, the manifest says `../chapter0001.html` and is right, the `guide`
says `chapter0001.html` and is missing the `../`. Forty errors from one absent
prefix.

**Removal is the last resort, and never for a document in the spine.** Working
from the epubcheck log alone, twenty chapters reported as both "not declared in
manifest" and "could not be found" read as a phantom apparatus to sweep away —
and sweeping it away *measures as a clean book*, with twenty of its twenty-one
documents unreachable. Only the archive listing shows what is really wrong. A
spine document whose file cannot be found is therefore always reported; a
stylesheet, font or script is not in the reading order and can go.

The mirror case is `undeclared-resources`: the archive holds a file the book
uses and the manifest never mentions it. *Skylark* is the example, and it is one
this tool created — correcting the `..Fonts/` typo in its stylesheet is what let
epubcheck reach the font and say it was undeclared. Trading RSC-007 for RSC-008
is not a repair, so the manifest entry is written too. Files nothing references
are left alone: epubcheck says nothing about them, and declaring them would be
inventing work.

### Reaching the end of a file is not the same as finishing it

`well_formed` asked quick-xml to read a document and reported success if it got
to the end without an error. It does catch a *mismatched* end tag. It does not
catch a document that simply runs out with elements still open — that reads to
EOF and returns `Ok`.

A real book has exactly that: *True Hallucinations* ends its only content
document mid-air, 78 lines and then nothing, no `</div>`, `</body>` or
`</html>`. epubcheck calls it `FATAL(RSC-016)` and stops reading the file, so
every other defect in it was invisible behind that one.

The hole mattered twice, because [the rollback guard](#no-pass-may-leave-a-document-worse-than-it-found-it)
asks the same question to decide whether a pass damaged a file — a pass that
deleted a closing tag would have sailed straight through it. So `well_formed`
now tracks the open stack and fails at EOF with anything left on it, and
`truncated-documents` closes what a stopped document left open. It runs first
among the document passes, because the guard protects a file that parsed
*before* a pass ran: a document that arrives malformed is the one file every
other fixer would otherwise edit unprotected.

### Derived declarations come last

Manifest properties are computed after every fixer has run, and are *synced*
rather than accumulated — a property no longer earned is withdrawn as readily as
a missing one is added. This is not tidiness. Computing them earlier meant
declaring `scripted` for a `<script src="js/kobo.js">` and then watching
`dangling-resources` delete the script that was its only cause; the book went
from two errors to one, and that one was `OPF-015`, *the property "scripted"
should not be declared* — introduced by the tool in the same run that removed
its reason to exist.

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
| no evidence of a problem at all | nothing happens, whichever version it declares |

That last row is a hard precondition, not a fallthrough. A book with no
violations has nothing to fix, so every change is downside.

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

**Decisive** (any occurrence is an *error* under EPUB 2, so the book is EPUB 3
whatever it says): elements XHTML 1.1 has no equivalent for — `section`,
`article`, `nav`, `figure`, `video`, `math` and the rest — plus `epub:`
attributes, `<meta charset>`, a nav document and `<meta refines>` metadata.

Every element in that list was placed in an EPUB 2 book on its own and run
through EPUB Check; only the ones that actually produced an error are in it.
**`svg` is not**, and must never be added: inline SVG is legal in EPUB 2, since
OPS 2.0.1 lists it among the core media types. An earlier version assumed
otherwise and fired on five books in a row, three of which validated with zero
errors — proposing to drag each through thousands of collateral entity and
DOCTYPE rewrites for no benefit. The general rule that prevents a repeat:
**trigger on violations, never on features.** "Does this book contain something
HTML5-ish" needs a complete model of both content models; "does this book contain
something that is an error where it stands" does not.

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
epubfix --preserve-presentation .   # legacy table attributes -> inline CSS
```

## Usage

```
epubfix [OPTIONS] [FILE_OR_DIR ...]

-n, --dry-run       report what would change; write nothing
    --no-backup     do not keep a .bak copy of the original
-r, --recursive     descend into subdirectories when scanning a folder
    --keep-version  never change a book's declared EPUB version
    --migrate-epub3 force an upgrade to EPUB 3 even if unnecessary
    --preserve-presentation
                    convert legacy table attributes to inline CSS
                    instead of removing them
    --language-detect=MODE
                    when a missing <dc:language> may be written from
                    detected text: en-only (default), any, or off
    --keep-missing-images
                    report an <img> whose file is proven absent instead
                    of removing it and its now-empty wrapper
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

One rule, learned the hard way and then learned again: **fire on an observed
error, never on an inferred condition.** Every false positive this tool has had
came from the same move — deciding a book needed repairing because something in
it *looked* like it belonged to the other EPUB version, or was missing something
a schema mentions somewhere.

| what it fired on | what epubcheck actually says |
| --- | --- |
| inline `<svg>` means HTML5 | SVG is an OPS 2.0.1 core media type; silent in EPUB 2 |
| `[^A-Za-z0-9_.-]` in a filename is unsafe | only `" * : < > ? \ \|`, controls (PKG-009) and spaces (PKG-010); `!$&'()*+,;=@~` and all non-ASCII are fine |
| `<pageList>` missing `id` or `class` | the two are *co-required*: neither is clean, both is clean, exactly one is the error |
| any absolute-URL `href` is a remote resource | hyperlinks are not; only embedding contexts (`img@src`, `object@data`, …) count |
| an empty `<dc:*>` is safe to delete | not for the three required ones: absent `<dc:title>` is `metadata incomplete`, a harder error than the empty one |
| an empty wrapper left by a removed `<img>` is untidy | removing it when it is the body's only child is `element "body" incomplete` |
| a `<body>` holding only an `<svg>` has no block content | `ns:svg` is in the permitted set epubcheck prints; an SVG cover page is a document, not a fragment |
| an empty `<body>` has nothing to repair | it is `element "body" incomplete` under EPUB 2 — and clean under EPUB 3, so the container is version-gated |
| a content document is a file ending `.html`/`.xhtml` | it is whatever the manifest declares `application/xhtml+xml`, whatever it is called |
| an element has a start tag and an end tag to rewrite | a self-closing one has neither pair; replacing only the start tag emits an unclosed element |

The first cost five books' worth of pointless entity rewriting, the second 321
renames in a single book, and the fourth actually *introduced* two OPF-018
warnings into a package that had none. The last two are the same shape caught
before shipping: both were specified as unconditional and both would have traded
one error for a worse one, which is why the guards exist and why each has a
test quoting the measurement. Each was caught only by running epubcheck
against the book and finding it had nothing to say — which is the rule's
corollary: **verify by re-running epubcheck, not by reasoning about the fix.**
The error count must strictly decrease and no new error *code* may appear.

### What it does not claim

A run ends by scanning the finished book again and saying what is still wrong:

```
book.epub:
    made 1 duplicated id(s) unique
    ...but 1 problem still remains.
other.epub: nothing I can fix, and 2 problems still remain.

Done: 1 fixed, 0 nothing to do, 0 failed, 2 not fully repaired.
epubfix checks far less than EPUB Check does — run that for the real answer.
```

The point is the wording. "Fixed" used to be the only thing the summary said,
and on a book with defects no fixer covers that reads as *this book is now in
good order* — a claim epubfix is in no position to make. It changed some bytes;
whether the result validates is a different question and belongs to EPUB Check.

The check reuses the differential gate's defect model — duplicate ids, links to
files that are not there, fragments that resolve to nothing — plus one thing
that belongs nowhere else: **whether each document is well-formed XML at all**.

That last one is worth its own note, because it is the case where saying
"nothing to do" is worst and where the tool was silent. Every fixer opens a
document with the lenient scanner and skips what it cannot read, so a chapter
with an unclosed `<b>` slid past all of them — while EPUB Check calls it fatal
and stops reading the file. The scanner's leniency is right: it exists to *edit*
real books, and refusing to open a sloppy file means refusing to help the books
that need it most. But leniency in the editor must not become silence in the
report, so the closing scan asks a second, strict parser instead.

All of that together is still a small fraction of what EPUB Check looks at, so
**silence here is not validity** — which is why the caveat prints alongside the
counts rather than being left for the reader to infer. Exit status 3 covers this
as well as findings, so a library sweep can be scripted on it.

### No pass may leave a document worse than it found it

The tool once wrote a book it had already diagnosed as broken. `nested-anchors`
met a *self-closing* inner anchor — `<a id="page_viii"/>` — replaced its start
tag with `<span id="page_viii">` and its end tag with `</span>`, except there
was no end tag, so the `<span>` was never closed. Two content-model errors
became two **fatal** ones, and the closing scan said so:

```
not well-formed XML at line 41 (expected `</span>`, but `</a>` was found)
```

and the file was saved anyway. That is the worst thing a repair tool can do,
and the bug is the smaller half of it.

So each pass now runs against an undo log. Any XML entry that parsed before the
pass and does not parse after it causes the **whole pass** to be rolled back —
not just the damaged file, because nothing at that point can tell which of its
edits were the bad ones — and the pass reports what it declined instead of
claiming a change. The defect it meant to repair is still there, which is the
right outcome: a reported error beats a book nothing can open. The check is
differential like every other gate here, so a book that arrives with an
unclosed tag is still worked on.

A crash counts as a failed book, not a failed run. A comment inside
`<metadata>` was once enough to abort a sweep partway through, which on a
library is the worst possible failure: the books after it in the list are
silently never looked at. Each book is now repaired inside a panic guard, so a
bug takes down one book and the sweep carries on. Every repair happens in memory
and the archive is written only at the end, through a temp file and an atomic
replace, so a crash leaves the book exactly as it was found. This is why the
release profile does not set `panic = "abort"` — it costs about 100 KB.

A residual whose subject a finding already named is not printed twice:
`dangling-resources` declining to delete an `<img>` and the final scan seeing
the link it left behind are the same defect from two directions.

### Working from epubcheck's catalogue instead of from broken books

Every fixer above up to this point came from a book that failed: find the
error, work backwards to the cause, write the repair. That reaches the *dense*
part of the distribution — across eight real books the errors were almost
entirely RSC-005, RSC-007, NCX-001 and OPF-054 — but it can only ever find
defects that happen to be in the library.

The later ones came from working down epubcheck's own message catalogue
correlated against its call sites. The argument for it is the `mimetype` fixer:
`save()` had *always* written a correct OCF mimetype entry, so any book this
tool rewrote came out right — but nothing noticed the condition, so a book whose
only defect was its mimetype drew `nothing to do`, was never rewritten, and kept
both errors. The repair existed and could not reach disk. No book in the library
has that defect, and one with it would have been quietly returned unrepaired.

The method has one rule, and it is the same rule as everywhere else here. The
catalogue tells you **what to measure, not what is true**. Building a fixture for
each code and running epubcheck against it corrected three guesses out of
fourteen before a line of Rust was written:

| what the call site suggested | what EPUB Check 5.2.1 actually did |
| --- | --- |
| an XHTML 1.0 DOCTYPE in EPUB 3 is HTM-009 | HTM-004, a different code with a different repair — HTM-009 never reproduced, so it was dropped |
| a UTF-16 XHTML document is HTM-058 | nothing at all; the fatal case is UTF-16 bytes under a `utf-8` declaration, `RSC-016` |
| a filename *containing* a dot is PKG-011 | only a filename whose **last** character is a dot |

The same pass found a live bug by accident. A probe for RSC-009 used an SVG
image, and the run turned a clean book into two errors: manifest properties were
derived for *every* item, and an `.svg` file is readable text, so the image was
handed `properties="svg"`. Measured, the rule is per-property, not per-item:

| `properties` on an `image/svg+xml` item | |
| --- | --- |
| none | **OPF-014** — `remote-resources` should be declared |
| `remote-resources` | clean |
| `svg` | **OPF-012** + OPF-015 |

So `svg`, `mathml` and `scripted` describe an XHTML content document and are
undefined anywhere else, while `remote-resources` belongs on anything that can
reference something remote.

Two codes on the shortlist were deliberately **not** implemented. PKG-012
(non-ASCII filenames) is USAGE severity and renaming for it is precisely the
321-rename false positive above. RSC-029 (`data:` URLs) cannot be mechanically
repaired, because the data *is* the content.

### The fixture suite, and one thing it cannot express

`epubfix-fixtures/run_suite.py` diffs EPUBCheck before and after across nine
hand-built books. Current result: **8 of 9**, under a default run and again
under `--keep-version`.

| Fixture | | |
| --- | --- | --- |
| `clean-epub2`, `clean-epub3` | 0 → 0 | byte-identical |
| `epub2-defects` | 32 → 1 | the survivor is the non-decorative `<img>` with no `alt`, report-only by design |
| `epub3-defects` | 17 → 0 | |
| `oebps10-declaration` | 1 → 0 | |
| `missing-language` | 1 → 0 | `en`, 9 of 11 samples, the two Latin footnote files outvoted |
| `missing-language-derivable` | 1 → 0 | from `xml:lang`, detection never reached |
| `epub2-mistagged` | 122 → 0 | retagged; `--keep-version` leaves it byte-identical and reports |
| `fragment-document` | 3 → 0 | **suite says FAIL** |

Every book reaches the error count the suite asks for, `fragment-document`
included. Its FAIL is the suite's `visible text changed` invariant, and it is
the one place that invariant cannot be satisfied: the check strips tags from the
*whole file* rather than the body, so it counts `<title>` as visible text, and
a wrapped fragment must acquire a `<title>` because a `<head>` without one is
itself an RSC-005 — measured:

```
element "head" incomplete; missing required element "title"
```

There is no title that passes, including an empty one, so the fixer is right and
the check is over-broad. `tests/common/verify.rs` runs the same invariant on the
same repair and stays quiet, because it suppresses `<head>` before comparing.

Verified end to end against EPUB Check 5.2.1. Fixture books carrying every
defect above validate with zero errors afterwards, as **both** EPUB 2 (9 errors
to 0) and EPUB 3 (20 errors to 0); a Coleridge-shaped EPUB 2 book with verse in
`<blockquote>`, legacy `opf:` metadata, a nested NCX with a `pageList` and
uncaptioned images goes from 24 errors to 0, and a book declaring EPUB 3 while written as EPUB 2
goes from 1 fatal + 11 errors to 0 by being retagged downward.

*Butcher's Crossing* — the book the path work came from — goes from 3 errors to
0: two dead `@font-face` rules removed with all 21 `font-family` fallbacks
intact, and the NCX identifier synced.

*The Hero of Ages* is the worst book in the library and goes from **1049 errors
and 96 warnings to 0 and 0**. All of it is six habits of one converter, repeated
across 95 documents: `<a name="x" id="x">` on every anchor (303), ids that are
UUIDs or start with a digit (172), spaces in every filename (191), `<body
text="#000000" link="#0000ff" dir="LTR">` (285), `width` on 75 table cells, and
four `<u>` elements. Nothing in it is unusual; there is simply a lot of it.

It also exposed a bug this tool had introduced. `<a name="x" id="x">` is the
legacy anchor pattern — one identity written twice on purpose, and required to
match — but `duplicate-ids` counted the two attributes separately, saw a
duplicate that was not there, and renamed the id to `x_2`. It reported "made 303
duplicated id(s) unique" while desynchronising 303 anchors. Identity is now
counted per element.

*A New History of Western Philosophy* goes from 4 errors to 0, and is the book
that produced the rollback guarantee above.

A Kobo *Essays and Aphorisms* goes from 57 errors to 0, and the last one of
those took two fixes that are worth stating separately. It ships an empty XHTML
document called `page-map.xml`, declared `application/xhtml+xml` and listed in
the spine. Every fixer skipped it on sight of the extension while epubcheck
validated it as a content document, so `markup_names` now follows the manifest
as well as the name. And an empty `<body>` was treated as nothing to repair,
when under EPUB 2 it is `element "body" incomplete`.

*Girl With Curious Hair* goes from 46 errors to 0. Thirty-six of those are one
Calibre bug repeated: three empty `<p> </p>` in the `<head>` of each of its
twelve documents, in a part of the file nothing renders. The other ten are the
contents page described above, recovered rather than deleted.

*The Hobbit* goes from 9 errors to 0 and is where two other classes came
from. Its metadata is padded
with `<dc:date/>`, `<dc:subject/>`, `<dc:description/>` and `<dc:rights/>`, only
the first of which epubcheck reports — an empty string is not a W3C date, and
the other three are legal and equally meaningless. And it references
`images/Art_logo.jpg`, the publisher's logo on the "About the Publisher" page,
which is absent from the archive, absent from the manifest, and shares no
basename with any image that is present. That image was the one defect in a
37-book sweep that resisted repair, under the rule that an `<img>` is never
deleted; it is now removed along with the `<p>` it was alone in.

It also caught a false positive on the way, which is the fifth row of the table
above and the reason the real book matters more than the fixture built from its
error log. Both of its cover documents hold an `<svg>` directly in `<body>`,
`BLOCK` did not list `svg`, and `fragment-documents` wrapped two documents
epubcheck had no complaint about. Only three files are touched now, and they are
the three with defects in them.

Three more real books, each the source of one class above, all reach 0. A study
Bible whose cover landmark pointed at `kindle:embed:0002` goes from 3 to 0, the
landmark repointed at the cover document the package already identifies rather
than having its `href` stripped. A *Slaughterhouse-Five* with no `<dc:language>`
anywhere goes from 2 to 0. And a Penguin *Complete Poems of John Keats* whose
`cover.html` is 45 bytes — one `<img>`, no root element, no namespace, listed in
both the manifest and the spine — goes from 2 to 0.

A book carrying every defect from the second sweep at once — duplicated Kobo
ids, `data-AmznRemoved`, a paragraph that swallowed a `<blockquote>`, a nested
`<a>`, a content document with no XHTML namespace, a `guide` pointing at a JPEG,
and an `@font-face` whose path doubles the directory — goes from 12 errors to 0
in one pass, and to 0 again under both `--preserve-presentation` and
`--migrate-epub3`.

In the other direction, a book carrying every shape that used to trigger a false
positive — six filenames with apostrophes, exclamation marks, parentheses,
accented Latin and CJK; a `<pageList>` with neither attribute; and three
hyperlinks to the open web in every chapter — validates clean before the run,
draws `nothing to do`, and validates clean after.

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
