# epubfix

Repairs common EPUB 2 validation errors in place — the recurring ones that
epubcheck flags across a library of books.

Drop the executable into a folder full of `.epub` files and run it. With no
arguments it scans the folder the executable itself lives in (not the working
directory), so double-clicking it works.

```
$ epubfix
Scanning /home/you/Books ...
Broken Book.epub:
    package version 1.0 -> 2.0
    removed spine/@page-map
    corrected font media-type
    sanitised 2 id(s)
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
| `filenames` | RSC-020, PKG-010 | renames resources whose filenames need URL escaping (spaces, non-ASCII, …) and updates every reference, raw or percent-encoded |
| `ncx-play-order` | RSC-005 | renumbers `toc.ncx` `playOrder` from 1, consecutive, one number per distinct target |
| `ncx-uid` | NCX-001 | syncs `dtb:uid` to the OPF `unique-identifier`, byte for byte |

`epubfix --list` prints the same table.

## Usage

```
epubfix [OPTIONS] [FILE_OR_DIR ...]

-n, --dry-run       report what would change; write nothing
    --no-backup     do not keep a .bak copy of the original
-r, --recursive     descend into subdirectories when scanning a folder
    --only NAMES    run only these fixers (comma-separated, see --list)
-l, --list          list the available fixers and exit
    --pause         wait for Enter before exiting
-h, --help          show help
-V, --version       show the version
--                  treat all remaining arguments as paths
```

Exit status is `0` on success, `1` if any book failed, `2` for a bad argument.

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
    fn name(&self) -> &'static str;          // stable id, used by --only
    fn codes(&self) -> &'static [&'static str];  // epubcheck message ids
    fn description(&self) -> &'static str;
    fn apply(&self, book: &mut Book) -> Vec<String>;  // one line per change
}
```

1. Add the struct to a module under `src/fixers/` — `opf.rs`, `ncx.rs`,
   `ids.rs`, `filenames.rs`, or a new file for a new area.
2. Register it in `fixers::all()`. Fixers run in list order, so anything that
   rewrites filenames or ids belongs before passes that read them.
3. Add a test to `tests/fixers.rs` covering both the broken input and an
   already-clean one.

`Book` gives you the pieces without any zip handling:

```rust
book.opf_text()        // package document, if present and decodable
book.ncx_text()        // NCX table of contents
book.text(name)        // any decodable entry, by archive name
book.set_text(name, s) // replace one
book.for_each_text(|name, text| { ... })    // every decodable entry, in order
book.for_each_markup(|name, text| { ... })  // .html / .xhtml / .htm only
book.rename(old, new)  // schedule a rename, applied on repack
```

Two rules make the whole thing safe: a fixer must be a **no-op on input it does
not recognise**, and it must **report only changes it actually made** — the
caller uses a non-empty result to decide whether to rewrite the file at all.

## Tests

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

31 tests cover each fixer against broken and clean input, plus repacking,
backups, dry runs, rename collisions, non-UTF-8 passthrough, and idempotency.
The output has also been checked end to end against EPUBCheck 5.2.1: a book
carrying all seven defects validates clean afterwards.

## Origin

A port of `epubfix.py`, matching its behaviour (including the exact sanitising
rules) while adding rename-collision handling, staged writes, backup
preservation, folder scanning, and the fixer registry.
