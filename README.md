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
| `spine-toc` | RSC-005 | points EPUB 2 `spine/@toc` at the NCX the manifest declares, whether the attribute is missing or empty |
| `spine-page-map` | RSC-005 | moves an Adobe page map's print page numbers into the NCX `<pageList>` EPUB 2 has for them, then drops the `<spine page-map>` attribute |
| `media-types` | CSS-007, OPF-035, OPF-037 | replaces a manifest `media-type` that is mistyped or superseded |
| `manifest-items` | OPF-091, OPF-099 | drops a fragment from a manifest `href`, and the entry a manifest makes for itself |
| `package-references` | OPF-031, RSC-001, RSC-007 | repoints manifest and guide hrefs whose file moved, and drops the ones with no file behind them — never a document the spine names |
| `empty-metadata` | OPF-054 | removes `<dc:*>` elements with no content, keeping the three both versions require |
| `identifier-uuid` | OPF-085 | generates a real UUID for an identifier that claims an empty or malformed one |
| `unique-identifier` | OPF-030 | makes `<package unique-identifier>` and the `<dc:identifier>` id agree, by creating the id or by moving the pointer |
| `dc-language` | RSC-005 | adds the required `<dc:language>`, taken from what the documents declare or from the text — never from the machine's locale |
| `fragment-documents` | RSC-005 | gives a bare markup fragment the document *and* the block container XHTML 1.1 needs inside `<body>` |
| `head-content` | RSC-005 | removes empty elements a `<head>` may not hold, and reports any carrying text |
| `content-type-meta` | RSC-005 | corrects the `<meta http-equiv="content-type">` value HTML5 fixes, which XHTML 1.1 never checked, charset included |
| `link-charset` | RSC-005 | removes the `charset` attribute Word puts on stylesheet `<link>` elements — meaningless and an error under both rulesets |
| `css-prohibited` | CSS-001 | remove the bidi properties an EPUB style sheet may not contain |
| `column-groups` | RSC-005 | put a bare `<col>` inside the `<colgroup>` HTML5 requires |
| `document-titles` | RSC-005 | give an empty `<title>` the name the NCX uses for its document |
| `namespace-escapes` | RSC-005 | put a package element that `xmlns=""` pushed out of the OPF namespace back into it |
| `truncated-documents` | RSC-016 | closes the elements a document left open when it stopped mid-air |
| `xhtml-namespace` | RSC-005 | declares the XHTML namespace on a root `<html>` missing it, which otherwise fails the whole document |
| `anchor-names` | RSC-005 | replaces the `name` attribute XHTML 1.1 removed from `<a>` with the `id` it stood for |
| `xml-ids` | RSC-005 | rewrites `id`/`name` values that are not valid XML Names — in content documents and the NCX — and every fragment pointing at them |
| `duplicate-ids` | RSC-005 | makes duplicated ids unique in content documents and the NCX alike, keeping the first — which is the one every link already resolves to |
| `data-attributes` | HTM_061 | removes custom data attributes whose names HTML5 rejects (Kindle's `data-AmznRemoved`) |
| `keyword-case` | RSC-005 | lower-cases an enumerated attribute value (`dir="LTR"`, `valign="TOP"`) that XHTML 1.1 spells in lower case |
| `obsolete-attributes` | RSC-005 | removes the image-map attributes HTML5 dropped from `<a>`, and inside a `<map>` moves them onto the `<area>` HTML5 keeps them on, leaving the link and its text |
| `legacy-table-attrs` | RSC-005 | strips presentational table attributes (`valign`, `align`, `bgcolor`, `nowrap`, …) the book's ruleset rejects, and clamps `border` |
| `image-dimensions` | RSC-005 | move a non-integer `<img>` width or height into CSS, where it is still valid |
| `underline-elements` | RSC-005 | turns the removed `<u>` into a `<span>` that still underlines |
| `img-alt` | RSC-005 | adds `alt=""` to decorative images in EPUB 2, and reports the rest rather than inventing captions |
| `nested-anchors` | RSC-005 | unwraps an `<a>` nested inside another, keeping its text and rehoming any id on a `<span>`, and naming any destination that could not be kept |
| `misplaced-blockquotes` | RSC-005 | splits a paragraph around a `<blockquote>` it swallowed, or demotes the quotation to a `<span>` |
| `inline-in-block` | RSC-005 | wraps a short run of inline content in a `<div>` where XHTML 1.1 wants a block, instead of retagging the book |
| `misplaced-anchors` | RSC-005 | removes or rehomes `<a>` elements stranded between table rows, keeping every link target alive |
| `guide-references` | OPF-032, RSC-005 | repoints an OPF `guide` entry naming an image at the one page that shows it, reports the ones several pages could equally mean, drops the ones with nothing to point at — recording the cover first, if that entry was the only thing declaring it — and drops the whole `<guide>` when that empties it |
| `orphan-links` | RSC-007 | repoints a link whose anchor was discarded at the heading its own text names |
| `dangling-resources` | RSC-007 | repoints references whose file moved; drops dead `<link>`/`<script>` includes, an `<img>` whose file is proven absent, and the `href` of an `<a>` that cannot resolve — the `<a>` itself, its text and its id always stay |
| `css-paths` | RSC-007 | repoints `url()` and `@import` in stylesheets, and drops dead `@font-face` rules, imports and declarations |
| `dead-schemes` | HTM-025 | repoints links using a reading system's private scheme (`kindle:`, `calibre:`, …) at what the package says they are for, or drops the `href` when nothing does |
| `broken-fragments` | RSC-012 | recovers undefined fragment targets via backlinks or unique relocation, else drops the fragment — and unlinks a same-document one with nothing left to point at, including a `../#frag` spelling that names no file |
| `reference-fragments` | RSC-009, RSC-013 | drops a fragment from a reference to a stylesheet or raster image, which cannot have one |
| `ncx-dead-entries` | RSC-007 | repoints a navigation entry whose document moved, and removes only the ones whose document is in the archive under no path, spelling or case |
| `ncx-pagelist-attrs` | RSC-005 | completes the co-required `id`/`class` pair on a `<pageList>` that carries only one |
| `ncx-entry-ids` | RSC-005 | gives every NCX navigation entry the `id` its schema requires |
| `undeclared-resources` | RSC-008 | declares a file the book uses that the manifest never mentions |
| `filenames` | PKG-009, PKG-010, PKG-011, OPF-060, RSC-020 | renames resources whose filenames a URL cannot address — illegal characters, a trailing dot, two entries differing only by case, or an unsafe *directory* component — and repoints every reference that resolves to one, `container.xml`'s `full-path` included |
| `ncx-order` | NAV-011 | sorts an EPUB 3 book's NCX navMap into spine order, so an old reading system falling back to the NCX shows the same order as the nav — EPUB 2 books are left alone |
| `nav-order` | NAV-011 | sorts the table of contents of an EPUB 3 nav that shipped with the book into spine order — whole subtrees at every level, fragments within a document, everything else untouched |
| `ncx-play-order` | RSC-005 | renumbers `toc.ncx` `playOrder` from 1, consecutive, one number per distinct target |
| `ncx-uid` | NCX-001 | syncs `dtb:uid` to the OPF `unique-identifier`, byte for byte |
| `dangling-refines` | RSC-005 | repoints a `<meta refines>` whose target id names nothing at the `dc:` element its property describes, or removes the ones that duplicate a refinement already there — ambiguous cases are reported |
| `version-mismatch` | RSC-005 | reports markup that does not match the declared version when retagging was refused with --keep-version — diagnostic only |

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

So each attribute is asked individually: **rehoused in CSS when nothing was
overriding it, stripped when something was, and left in place when epubfix has
not implemented a faithful conversion.** `valign="top"` becomes
`vertical-align: top`, `align="left"` on an image becomes `float: left` rather
than `text-align`, and `cellpadding` — which describes the cells rather than
the table — becomes a generated stylesheet rule on the affected cells.

Rehousing happens in a generated `<style>` block at the start of the document's
`<head>`, wrapped in a cascade layer:

```css
@layer epubfix-hints {
.epubfix-hint-1 { border: 0; }
}
```

The layer is the whole point. A presentational attribute loses to *every* author
declaration, whatever its specificity, and no ordinary selector reproduces that
— a bare `.epubfix-hint-1 { border: 0 }` is specificity (0,1,0) and beats the
book's own `img { border: 2px solid }` at (0,0,1), which is the opposite of what
the attribute did. Putting the block first in `<head>` does not help, because
specificity is compared before order of appearance. An unlayered author
declaration beats a layered one regardless of specificity, so the layer puts the
rule back exactly where the attribute stood. Measured, on real XHTML:

| | wins |
| --- | --- |
| `<img border="0">` vs `img { border: 2px }` | the stylesheet |
| `.gen { border: 0 }` vs `img { border: 2px }` | the generated rule ✗ |
| `@layer { .gen { border: 0 } }` vs `img { border: 2px }` | the stylesheet ✓ |

One class is minted per distinct declaration rather than per attribute, so a
book with the same `border="0"` on three hundred images gets one rule. The cost
is that a reader with no `@layer` support skips the block, losing the
attribute's effect — the same outcome as `--strip-presentation`, which this tool
already offers on purpose. An attribute with no implemented conversion is
reported and kept, rather than silently deleted.

*The Hero of Ages* is what forced this. Its Ars Arcanum table carries `width` on
75 cells, its stylesheet declares no width at all, and stripping all 75 reflows
a reference table people actually consult. In the same run its 191 `valign`,
`text` and `link` attributes *were* covered by rules and are simply removed:

```
moved 75 legacy attribute(s) into generated CSS and stripped 191 that a
stylesheet already overrode, for EPUB 2 [linkx95, textx95, widthx75, ...]
```

No single policy is right for both, which is why there is no longer a single
policy. The CSS-override check ([`src/css.rs`]) decides between the two, but
only between "delete" and "reproduce" — never between two different renderings.
That distinction is what makes an approximate check safe here. It has real blind
spots: it reads `.css` entries only, so a `<style>` block inside a document is
invisible to it, as is any `#id` rule. While the alternative to deleting was an
*inline* style, being wrong about the override moved the declaration above rules
it had never seen. A layered rule loses to those rules without having to know
they exist.

Both policies are still available for supported conversions.
`--preserve-presentation` converts supported attributes even when a stylesheet
was overriding them, and converts to an inline style rather than a layered rule
— it is the mode that deliberately promotes. `--strip-presentation` removes
supported presentational attributes instead. Neither mode deletes an attribute
for which epubfix has no conversion; those stay in place and are reported.

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

### A landmark pointing at a picture still knows where it wanted to go

A Penguin *Iliad* fails OPF-032 three times: its `guide` holds
`other.ms-coverimage-standard`, `other.ms-thumbimage-standard` and
`other.ms-thumbimage`, and all three point straight at a `.jpg`. A reading
system cannot navigate to a JPEG, and `<guide>` is optional in EPUB 2, so
deleting the entries validates. That was what `guide-references` did, and on
this book it was the wrong answer.

`home_9781101153635_msr_cvi_r1.jpg` is not an orphan. The spine's first
document is a page whose entire body is an `<img>` of exactly that file. The
publisher pointed the guide at the picture instead of at the page holding it,
and the page is sitting right there in the manifest. So the entry is repointed
at it, `type` and `title` intact — a landmark recovered rather than a landmark
thrown away. The rule is the same [unique-match derivation](#repairing-a-dead-link-not-just-silencing-it)
the `epub:type="cover"` case uses: exactly one document shows the file, or the
spine leaves exactly one candidate.

Removal is then reserved for the two cases that lose nothing by it. **Nothing in
the book shows the file** — the other two `.jpg`s in that *Iliad* are shown by no
document at all, and EPUB 2 has nowhere to record "this is the PPC thumbnail",
so those entries go while the files and their manifest items stay. Or **the
retarget would land on a `type`/`href` pair the guide already carries**, which is
measured to be a trade rather than a repair:

| | epubcheck |
| --- | --- |
| two `<reference>`, same `type`, same `href` | RSC-017 `Duplicate "reference" elements with the same "type" and "href" attributes` |
| two `<reference>`, same `type`, different `href` | clean |

So the pair is the right granularity, and creating the duplicate would buy a
silenced OPF-032 with a new RSC-017. The surviving entry carries the same
semantic type to the same destination, so no landmark is lost — only the dropped
entry's `title`, which was a second label for one place.

**Several pages showing the file is neither.** Repointing at whichever was seen
first is a guess, and a landmark going to the wrong page is worse than one that
was never repaired; removing throws away an entry that is plainly recoverable.
So the entry is left exactly as it is and the candidate pages are named, the way
`package-references` and `orphan-links` already handle their own ambiguities. The
book keeps its OPF-032, and someone who knows it settles the question in a
second.

**No pass may take the last statement of what the cover is.** A `type="cover"`
entry pointing at an image is a mangled spelling of a real fact, and EPUB has a
place for that fact: `<meta name="cover">` under EPUB 2,
`properties="cover-image"` under EPUB 3. It is written there before the entry is
removed, so the removal is a tidy-up and not a loss. The *Iliad* already said it
and nothing was added — which is the point, this is a guard rather than a
repair.

Stated generally, because the day it matters is the day it is expensive:
**nothing may remove the last inbound reference to a manifest item without first
checking that what the reference meant is recorded somewhere else.** No pass
sweeps unreferenced manifest items today. The day one does, it and this one will
be individually correct and jointly capable of deleting a book's cover.

Across a 179-book library this recovers a landmark in **7 books** that the older
behaviour would have deleted, and still removes 10 entries that lead nowhere.

### An image map keeps its geometry

`obsolete-attributes` strips `shape`/`coords` from every `<a>` in an EPUB 3
book, on the argument that they never did anything — true *outside* a `<map>`,
which is where all 33 in the library are: ordinary Calibre-written links like
`<a href="cover.xhtml" shape="rect">`. Inside one those numbers are the
clickable region, and HTML5 did not so much drop them as move them to `<area>`.

There is **not one `<map>` element in any of the 179 books**, so this branch is
written against synthetic fixtures — which is sound here in a way it usually is
not, because `<area>` is completely specified and every rule was still measured
against epubcheck rather than read off the standard:

```text
<map><a href shape coords>North</a></map>          2x RSC-005 (the defect)
<area alt> with no href                            missing required "href"
<area href> with no alt                            missing required "alt"
<area href alt="">                                 clean
<area> with neither href nor alt                   clean
<area shape="default" coords>                      "coords" not allowed here
<area coords> with no shape                        clean
shape="circ" / "rectangle" / "polygon"             value ... is invalid
<map id> with no name                              missing required "name"
```

The obvious conversion — retag the `<a>` as an `<area>` — throws away the
anchor's text, and a `<map>`'s content is *rendered*: that text is a visible
link, often the list that makes the image map usable without a pointer. So the
`<area>` is **inserted** carrying the geometry and the `<a>` stays exactly where
it was, minus the two attributes it may not have. Both point at the same place,
which is what the original markup meant in the readers that honoured it.

The rest falls out of the table. `type`-less shapes are mapped to the four
HTML5 accepts and an unknown one is reported rather than guessed at; `default`
sheds its coordinates; the anchor's words become the co-required `alt`, and an
anchor with no `href` gets neither. A `<map>` carrying only an `id` is given the
matching `name`, since `usemap` resolves against either and without one the map
is an error by itself. Measured end to end on a synthetic book: **7 errors to
0**, with every link and every word still on the page.

### An inner anchor's destination is named, not dropped

`nested-anchors` discards the inner anchor's `href`, and "it was unreachable
anyway" is weaker than it sounds: an EPUB content document is parsed as XML, so
unlike HTML's tag-soup rules — which would have split the two into siblings —
the nesting is real, and which link a reading system activates is its own
business. Two links still cannot share one run of text, so the destination
cannot be kept; it is now reported instead of vanishing.

Nothing in this library is affected. There are **three nested anchors in the
whole of it**, all in one Kenny *Ancient Philosophy*, and all three are
`<a id="page_viii"/>`: self-closing page markers with no `href` at all, which
the `<span>` rehoming already handles without loss. The report exists for the
book that is not here, and fires zero times across the 179.

### The table of contents had the same mistake

`ncx-dead-entries` asked whether an archive entry was called *exactly* what
`<content src>` said, and removed the entry when it was not — reading a
misspelled path as a missing chapter. An NCX saying `ch1.xhtml` for a file at
`Text/ch1.xhtml` lost the entry, its label and every entry nested underneath,
and epubcheck reported a clean book afterwards. Nothing else in the pipeline
repoints an NCX destination: `dangling-resources` walks the book's markup and
the NCX is not markup, and `broken-fragments` only reaches an entry carrying a
`#`. It now runs the same [resolver](#finding-what-a-broken-reference-meant) the
package document gets, so only a document that is in the archive under no path,
spelling or case takes its entry with it.

### Print page numbers are not an Adobe extension to be swept away

`<spine page-map="...">` is an Adobe extension and epubcheck rejects it, so
removing the attribute validates. That was all this did — and what the attribute
points at is a file of *print page numbers*, the ones a reader shows in the
margin and a citation refers to. Dropping the only pointer to it retires the
whole apparatus in one attribute.

It is not a rare shape: **10 books in the 179 carry one**, holding between 189
and 2503 `<page>` elements each. EPUB 2 already has a standard spelling for
exactly this — the NCX `<pageList>` — so the page map is converted into one and
only then is the attribute removed.

Almost none of the conversion is guessable, so it was measured a construct at a
time. `type` is the one attribute that must be written. `playOrder` is
**omitted**: the only way to get it wrong is to collide with the navMap, and
there is no way to get it right that omitting does not also get right. `value`
is written only for a genuine page *number*, which is what makes the required
`(type, value)` pair unique — so a name that is all digits is `type="normal"`
carrying that value, one that is all roman numerals is `type="front"` with none,
and anything else is `type="special"`. A blank name is not a page and is
skipped. Paths are re-resolved through the [resolver](#finding-what-a-broken-reference-meant),
which earns its place immediately: a Blake page map lives in `OEBPS/Misc/` and
writes `fm.html`, while the file is at `OEBPS/Text/fm.html`.

Six books had no `<pageList>` and gained one: **3161 page numbers** that a
reading system can now use, all ten books still validating clean. The manifest
item and the page-map file are left exactly where they are, measured clean and
unreferenced — the pagination now has two homes rather than none.

The other four already had a `<pageList>`, and three of those match the page map
entry for entry — 189/189, 2503/2503, 231/231 — so saying anything would be
noise. The fourth is why this compares page *labels* rather than counting them:
a Sanderson *Oathbringer* has 1431 pages in its Adobe map and 1221 in its
pageList, so 211 page numbers sit in a place no reading system looks. Merging
two paginations is a judgement about which is right, so that one is reported and
left alone.

### Two ways the tool damaged a book and said it had fixed it

Both came out of an external audit, and both are the same shape: a repair whose
*mechanism* was wider than its intent.

**A filename is an ordinary run of bytes.** `filenames` used to update
references by searching every text entry for the old spelling of each renamed
path component and swapping it. Short, general, finds a reference written from
any directory — and unable to tell a reference from anything else made of those
bytes. On a book with `B.xhtml`, `b.xhtml` and `ab.xhtml`, the first two fold
onto one name under OCF so the later is renamed to `b_2.xhtml`, and:

```text
before   <a href="ab.xhtml">   <item href="ab.xhtml">   <content src="ab.xhtml"/>
after    <a href="ab_2.xhtml"> <item href="ab_2.xhtml"> <content src="ab_2.xhtml"/>
         ERROR(RSC-001) File "OEBPS/ab_2.xhtml" could not be found.
```

Three references broken and a file invented, out of one legitimate rename. The
same mechanism turns a renamed `img` directory into `<img_2 src=…>`. References
are now found by *resolving* them and rewritten relative to wherever the
referring document itself ends up, so a moved directory takes its documents with
it correctly. Writing that turned up a bug of its own, caught by its own test:
`container.xml` names the package document with `full-path`, which OCF resolves
from the *container root*, and a rename that treated it as an ordinary relative
reference produced `full-path="../OEBPS/…"` — a book nothing can open.

**A prolog is not content.** `fragment-documents` wraps a file with no root
element in `<body><div>…</div></body>`. When the fragment opened with its own
`<?xml version="1.0"?>`, that declaration went inside the `<div>`, where it is
not a declaration at all:

```text
input:  1 error   →   epubfix: "1 fixed"   →   FATAL(RSC-016)
```

One error in, one *fatal* out, reported as a repair. The prolog is now stripped
before wrapping — but the more important half is why nothing noticed. The pass
guard asks [`well_formed`](#no-pass-may-leave-a-document-worse-than-it-found-it)
whether the result still parses, and quick-xml reads a late `<?xml?>` as an
ordinary processing instruction and says yes. It now rejects a declaration after
the start of the document, and any reserved `[xX][mM][lL]` processing-instruction
target, so the whole class is caught rather than this one instance of it.

Neither book in a 179-book library triggers either bug; both are in the fixture
suite now. The audit also found `package-references` dropping the `#fragment`
from a moved `<guide>` landmark, and `spine-toc` reading an empty `toc=""` as an
attribute already present — measured, that spelling is *two* errors where the
missing attribute is one.

### A pending rename made three later passes read an empty book

A rename does not take effect until the archive is repacked: `texts` stays
keyed by the name the entry arrived under, because that is what `save` writes
from. But `filenames` rewrites every *reference* at the same time, so every
pass after it resolves an href to the name the file is going to have — and
asking for that name found nothing at all.

Three passes had the same blind spot: `finalise_properties`, `ncx-order` and
`ncx-nav-order`. Measured on an EPUB 3 book whose `ch 1.xhtml` holds an `<svg>`,
where the manifest item is owed `properties="svg"`:

```text
ch 1.xhtml   renamed to ch_1.xhtml, 4x RSC-020 fixed
             ...and OPF-014 "the property svg should be declared" left behind
safe.xhtml   the identical book, never renamed: clean
```

None of the three could have known, so the lookup is what gave: `Book::text`
follows a pending rename back to the entry it is stored under, and `set_text`
writes back through the same mapping — it used to miss silently, dropping the
edit rather than misplacing it.

### Four more ways a repair was wider than its intent

All four came from a second audit, all four are reproduced in the fixture suite,
and none of them fires on any book in the 179-book library.

**Two anchors cannot both claim one row.** `misplaced-anchors` moves the `id`
off an `<a>` stranded between table rows onto a row that has none — asking the
*unedited* tree whether a candidate is free. Two stranded anchors therefore both
chose the same row and both wrote to it: `<tr id="pos1" id="pos2">`, a duplicate
attribute and not well-formed XML. The pass guard caught the malformed document
and rolled the whole pass back, so a book with two stranded anchors got no
repair at all. Rows are now claimed as they are taken, and the second anchor is
reported rather than silently costing the first its fix.

**A split paragraph cannot wear its `id` twice.** `misplaced-blockquotes`
reopens each fragment with the paragraph's own tag, `id` and all, so
`<p id="intro">before<blockquote/>after</p>` came out as two elements called
`intro` — one RSC-005 traded for two `Duplicate ID`s, with `duplicate-ids`
running earlier in the pipeline and nothing left to clean up after it. The first
fragment keeps the id, since that is what every existing link already resolves
to; the continuations keep every other attribute.

**`<span>` and `<div>` are not void elements.** Retagging a self-closing
`<center/>` rewrote the name and left the `/>`, producing `<div/>`: an empty
element to an XML parser, an *unclosed* start tag to an HTML one, swallowing the
rest of the document. `nesting::rewrap` had already learned this on a real
`<a id="page_viii"/>` — the same trap, one module over — so the `/>` is now what
gets replaced, with `></div>`.

**An unclosed `<navPoint>` panicked the sorter.** The NCX sort stitches each
child from its open tag to its close tag and asserted the close tag existed.
An unclosed one is a `Start` node with none, and the assertion fired —
`navPoint with a close tag` — which the unwind guard turned into a failed book.
The container is now left exactly as written when any child is incomplete;
leaving the child *out* would have dropped its bytes, which is worse than not
sorting.

### One place where the audit was right about the symptom and wrong about the cure

`resolve_href` documented itself as returning `None` for "paths that climb out
of the root", and it does not: `../../outside.xhtml` from `OEBPS/a.xhtml` comes
back as `outside.xhtml`, because the segment stack runs out of things to pop and
stops. Read cold that is an underflow bug, and the obvious repair is to return
`None`.

It would make the tool worse. `None` is [`Resolution::NotOurs`], and every
consumer treats that as *skip this, it is not a container reference* — the arm
that exists for `mailto:` and `https:`. But a reference climbing out of the
container is not somebody else's business; it is broken. Clamping it to a
root-relative path means the resolver either finds the file the author meant or
returns `Missing`, after which the link gets repaired or unlinked and named.
Returning `None` trades both of those for silence.

Nothing escapes either way: the result is only ever looked up among the
container's own entry names, and the repacked archive is written entry by entry
rather than extracted to a path. So the defect here was the sentence, and the
sentence is what changed.

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

### A link that cannot resolve stops claiming it can

An `<a>` used to be the one thing this never touched: it carries navigation, and
`dangling-resources` reported it instead. That was too absolute, for the same
reason it was too absolute for `<img>`.

`Resolution::Missing` is not a suspicion. It comes back only after five candidate
resolutions have failed — the path as written, read from the archive root, read
from the package directory, the unique longest matching suffix, and the same
ignoring case. By then the file is in the archive under no path, spelling or
case, and the link is dead however anyone looks at it. Keeping the `href`
preserves nothing except the appearance of a working link.

So the `href` goes and everything else stays: the element, its text, its class,
and any `id` something else may point at. Measured, `<a>` with no href is clean
under both rulesets. Every target is named in the report, and the reason is
given, because the two are different:

* **The target was never a filename.** Two Eddings volumes carry
  `<a href="XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX">`, a placeholder a converter
  never filled in; *Rise of the Horde* has `%EF%BF%BD%EF%BF%BD`, two U+FFFD
  replacement characters — the name was already mojibake when it was written, so
  there is nothing any search could find. Measured: epubcheck treats both as an
  ordinary missing relative path, not as a fragment.
* **The file is genuinely gone**, after the search above.

Three guards keep this from eating a table of contents. `orphan-links` runs
first and gets the refusal, so a Word bookmark whose text names exactly one
heading in the book is repointed at that heading rather than unlinked — all ten
of *Girl With Curious Hair*'s story links still resolve. A link into a
*document* that exists takes the repointing path long before this one. And the
third is the subject of the next section.

### Unresolvable by code is not unresolvable by a person

`orphan-links` declines when *two* headings carry the words a link's text names,
because a contents page pointing at the wrong chapter is worse than one pointing
nowhere. It reports, and the person now knows exactly which two headings clash —
they can open the book and tell which was meant in seconds.

Unlinking that would be the worst of both. The report still arrives, but the
`href` recording what the link was *for* is gone, and the repair a person could
have made has been made impossible. The blunter the fallback repair, the more
likely it is to silence precisely the thing someone is being asked to look at.

So a fixer that declines can say so, with `Book::reserve`, and every later pass
leaves that reference exactly as it is. The test is not "can this be resolved"
but **"could anyone resolve it"**: where candidate targets exist and only a
person can choose between them, nothing touches it. Where the target was never a
name — a converter's placeholder, a mojibake filename — there is nothing for
anyone to choose from, and it is unlinked.

The reserved link is also reported *once*. The pass that reserved it has already
explained the problem, so the later pass that steps around it says nothing,
rather than printing the same defect twice under a different heading.

### Retagging is the strictest thing this tool does

Moving a book into EPUB 3 puts it in a ruleset that checks more, so every defect
only EPUB 3 objects to arrives at once — which is why the retag runs *before*
the fixers, so that they repair rather than merely expose. Measured on three
real books, before any of the fixers below existed, forcing the upgrade (the
retagger does this on its own today when the content calls for it):

| | as found | repaired | forced EPUB 3 |
| --- | --- | --- | --- |
| *1984* | 42 | 0 | **51** |
| *The Well of Ascension* | 722 | 0 | **83** |

Nothing was being corrupted — every one of those errors was already in the book
and EPUB 2 simply never asked. But an upgrade that takes a book from 0 errors to
83 is not usable, and "they were already there" is no comfort to someone whose
library got worse. Three classes accounted for all of it:

* **`direction: ltr` in a stylesheet** (51 + 7). Prohibited outright in EPUB 3,
  invisible in EPUB 2. Every occurrence in the library sets the CSS *initial*
  value, so removing them is provably a no-op — see `css-prohibited`.
* **`charset=windows-1252`** in the encoding declaration (70). See below.
* **`<col>` directly inside `<table>`** (6), which XHTML 1.1 allows and HTML5
  does not — see `column-groups`.

With those three, both books migrate to 0. Two more classes came out of the
same sweep across the rest of the library:

* **`shape="rect"` on an `<a>`** (33 in one *Skylark*). HTML5 keeps `shape` on
  `<area>`, where it describes a region of an image map, and dropped it from
  `<a>`, where it only ever applied inside a `<map>`. It carries no
  presentation, so `obsolete-attributes` simply removes it rather than leaving
  `legacy-table-attrs` to strip it and report "no single-property CSS
  equivalent" — true, and beside the point.
* **`<img width="100%">`** (10 in one *Hobbit*). HTML5 keeps the attribute but
  narrows it to a whole number of pixels. Stripping is not an option, which is
  why `image-dimensions` is separate from `legacy-table-attrs`: the percentage
  is doing real work, and losing it drops the picture to its natural size. The
  value moves to inline CSS whatever `--strip-presentation` says, because there
  the choice is between two valid renderings and here it is between keeping the
  layout and losing it.

### A prefix is not a namespace

The package converter turned `opf:role`, `opf:file-as` and `opf:scheme` into
`<meta refines>` and left three books' worth of them behind, because it matched
the literal string `opf:`. A prefix is only a local name for a namespace, and
Calibre binds one *per element*:

```xml
<dc:creator xmlns:ns0="http://www.idpf.org/2007/opf" ns0:role="aut">
<dc:contributor xmlns:ns1="http://www.idpf.org/2007/opf" ns1:role="bkp">
<dc:identifier xmlns:ns2="http://www.idpf.org/2007/opf" ns2:scheme="calibre">
```

Three prefixes for one namespace in one file, none of them `opf`. The converter
saw nothing, reported nothing, and left all three for epubcheck. It now resolves
by namespace and takes whatever prefixes a document binds to it.

Alongside that, EPUB 3 has no `opf:event` and permits at most **one**
`<dc:date>`. A book can break both at once — a Penguin *Keats* has two dates,
one of them `opf:event="converted"` — so both rules are applied, and the
survivor is the one that says it is the publication date rather than merely the
first.

### A byte-order mark made whole files invisible

The worst bug in this round, and the one with no symptom. quick-xml reports
buffer positions relative to the text *after* a UTF-8 byte-order mark, so with a
`U+FEFF` still on the front every span the scanner produced was three bytes out
and `dissect` read each element name from the wrong offset. The result is a node
list of the right length in which **every name is the empty string** — so the
file matches nothing, and every fixer silently skips it.

That is how a Kodansha *Wild Sheep Chase* and three Dune books came to report
"the NCX has no navMap": their NCX has 58 navPoints and a perfectly good
`<navMap>`, and not one element of it was visible. The forced migration then
produced a book declaring EPUB 3 with no navigation document at all.

The mark is now stripped when the book is read. Measured: epubcheck scores those
books identically with it and without it, and because it comes off on the way
in, `verify` compares two texts that never had one.

Across the twenty-one books here, the forced upgrade goes from 58 errors to 3 —
all three in one book, whose `<p>`-inside-`<p>` damage is pre-existing and
identical without it.

### The bytes decide the encoding, not the label

`content-type-meta` used to refuse any charset but UTF-8, reasoning that
`charset=` is a claim about the bytes and relabelling one would turn a wrong
declaration into a wrong document. That had the evidence backwards, and it cost
165 unrepaired findings across the library.

[`Book::load`] decodes with `std::str::from_utf8`, which is strict. A file that
is not valid UTF-8 — and not UTF-16, which is transcoded and recorded — never
enters the text map, and no fixer ever sees it. So every document this can be
looking at **is** UTF-8, by construction, and the label is the only thing
disagreeing with the bytes.

The real book settles it past the argument from types. *The Well of Ascension*
declares `charset=utf-8` **and** `charset=windows-1252` in each of its 70
documents, and the bytes they hold are `E2 80 94` — a UTF-8 em dash. Two
declarations and the bytes themselves say UTF-8; one Calibre artefact says
otherwise. The relabelling is named in the report, because rewriting an encoding
claim is worth saying out loud even when it is certainly right.

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
| evidence mixed or weak | declaration kept, book repaired against it; anything unrepaired is reported |
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

**Suggestive** (mostly counts in quantity): inline content inside `<blockquote>`
and other block-only containers. A handful of these means a few paragraphs need
a `<div>`, not that the whole book is the wrong format — retagging on that
evidence would be wildly out of proportion. Past a threshold the balance flips:
repairing would mean hundreds of edits, so the one wrong attribute is
overwhelmingly the likelier error. Below it, the runs are wrapped in place and
the book stays EPUB 2.

One form of this is decisive even in a single occurrence: inline content inside
`<form>` or `<fieldset>`. Measured, wrapping their children changes nothing —
the errors stay — so such a run cannot be repaired as EPUB 2 at all, and the
declaration moves on the first one.

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
still permits it and EPUB 2 readers fall back to it. Before any of that, and
only when a nav is actually generated, the navMap is sorted into spine order —
including fragments within a document — because a nav must follow the reading
order (NAV-011) and the NCX is where it inherits its order from. A book that
keeps its EPUB 2 declaration is left exactly as it was. An EPUB 3 book that
shipped with its own TOC files gets both sorted — the nav by `nav-order` and
the NCX by `ncx-order` — so an old reading system using the NCX and a new one
using the nav show the same order.

An NCX whose `navMap` holds no navigation at all — an empty `<navMap/>`, which
epubcheck reports as incomplete — falls back to the spine: the headings `h1`
through `h3` become the table of contents, nested by level, and a document
with no headings contributes one entry named by its `<title>` or its filename.
The same entries are written back into the NCX navMap, so both navigation
files come out valid. Only when the spine has nothing to build from either is
the book reported.

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
epubfix --preserve-presentation .   # legacy table attributes -> inline CSS
```

## Usage

```
epubfix [OPTIONS] [FILE_OR_DIR ...]

-n, --dry-run       report what would change; write nothing
    --no-backup     do not keep a .bak copy of the original
-r, --recursive     descend into subdirectories when scanning a folder
    --keep-version  never change a book's declared EPUB version
    --preserve-presentation
                    convert supported legacy presentation to CSS, even when
                    an existing stylesheet was overriding it
    --strip-presentation
                    remove supported legacy presentation instead; unsupported
                    attributes are reported and kept
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
*newly* duplicated, no visible text changed, no reference that used to resolve
stopped resolving, and no document the navigation could reach became
unreachable. `tests/verify.rs` proves those checks fire on a real regression and
stay quiet on a pre-existing defect.

That last clause is new, and the sentence it replaced claimed something the
harness could not do. It said these checks catch "a fix that silences epubcheck
while quietly breaking navigation" — and then `ncx-dead-entries` deleted a
chapter's table-of-contents entry over a misspelled path, and every fixture test
in the suite called `assert_sound()` on the result and passed. A deleted entry
keeps the page, its text, its ids and every link, which was the whole of what
the harness modelled, and it takes an epubcheck error away with it, so the book
measures *cleaner*. A gate that cannot see the table of contents cannot notice
one being thrown away.

Getting the invariant right took two attempts, and the first one is the more
instructive. Counting the document each entry *names* immediately failed a
legitimate fixture: a Complete Works of Aristotle navPoint labelled "Page xii"
points at `v1_preface.html#page_xii`, the id is in `v1_ack.html` because a
content split moved it, and `broken-fragments` correctly follows it there — at
which point the preface stops being named. So an entry with an anchor is
counted as leading to whichever document *defines* that anchor, and only an
entry without one is about the file it names. Paths resolve leniently for the
same reason the bug existed: `ch1.xhtml` for a file at `Text/ch1.xhtml` still
leads somewhere, and exact string matching is precisely the mistake.

Reverting the `ncx-dead-entries` repair with the harness in place turns the
suite red at `assert_sound()` rather than at any hand-written assertion, which
is the property worth having: the net catches this class in every fixture test,
not only in the one written for it.

The same blind spot existed for stylesheets, and was found the same way — by
someone reading the harness rather than by a book breaking. `defects()` only
looked at `href` and `src` inside markup, the NCX and the package, so a `url()`
or an `@import` left pointing at a file that had just been renamed was invisible
to it. That is not hypothetical territory: `filenames` and `css-paths` both
rewrite stylesheet references, and the `filenames` rewrite was validated only by
epubcheck runs done by hand. Both harnesses now resolve CSS references too, and
`tests/verify.rs` proves the check fires on a broken one and stays quiet on a
book that arrived that way.

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
| `shape`/`coords` on an `<a>` are dead weight | true *outside* a `<map>`, which is where all 33 in the library are; inside one they are the clickable region, and HTML5 moved them to `<area>` rather than dropping them |

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
in one pass, and again under `--preserve-presentation`.

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
