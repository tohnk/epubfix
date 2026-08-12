//! epubfix — repair common EPUB 2 validation errors in place.
//!
//! Drop the executable into a folder full of `.epub` files and run it. With no
//! arguments it scans the folder the executable itself lives in, fixes what it
//! can, and leaves everything else alone.

use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use epubfix::{Options, collect_epubs, fix_file, fixers};

const USAGE: &str = "\
epubfix — repair common EPUB 2 validation errors in place.

USAGE:
    epubfix [OPTIONS] [FILE_OR_DIR ...]

With no FILE_OR_DIR, every .epub in the folder containing the executable is
processed, so the tool can simply be dropped into a folder and run.

OPTIONS:
    -n, --dry-run       report what would change; write nothing
        --no-backup     do not keep a .bak copy of the original
    -r, --recursive     descend into subdirectories when scanning a folder
        --migrate-epub3 force an upgrade to EPUB 3 even if unnecessary
        --keep-version  never change a book's declared EPUB version
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
    -h, --help          show this help and exit
    -V, --version       show the version and exit
    --                  treat all remaining arguments as paths

A .bak copy is written beside each book that is modified, unless one already
exists — a second run never overwrites the pristine original.

Anything a fixer recognises as wrong but will not repair on its own is listed
under \"needs manual attention\" instead of being guessed at. Use --dry-run to
triage a whole library without writing to it.

VERSION RETAGGING:
    Books are often declared as the wrong EPUB version, in both directions. The
    declaration is one attribute; the content is thousands of elements. So when
    they disagree, epubfix moves the declaration to match the content rather
    than rewriting the content to match the declaration.

    Content that needs EPUB 3 (verse in <blockquote>, HTML5 elements, epub:
    attributes, a nav document) in a book declaring EPUB 2 -> upgraded, and the
    nav document, metadata and manifest properties EPUB 3 requires are
    generated. A book declaring EPUB 3 with none of that, written throughout as
    EPUB 2 -> downgraded, which is a single attribute plus a little package
    cleanup.

    Only decisive evidence retags. Mixed evidence leaves the declaration alone
    and repairs the book against whatever it currently claims. A retag that
    would lose a link, an id or any visible text is abandoned and the book left
    untouched. This all runs before every other fix, since the version decides
    what those fixes should do.

    --keep-version turns retagging off; --migrate-epub3 forces an upgrade even
    when the content does not require one.

EXIT STATUS:
    0  all good
    1  at least one book failed to process
    2  bad arguments
    3  everything worked, but some books need manual attention";

struct Args {
    opts: Options,
    paths: Vec<String>,
    recursive: bool,
    pause: Option<bool>,
}

fn parse_args(argv: Vec<String>) -> std::result::Result<Option<Args>, String> {
    let mut parsed = Args {
        opts: Options::default(),
        paths: Vec::new(),
        recursive: false,
        pause: None,
    };
    let mut it = argv.into_iter();
    let mut literal = false;

    while let Some(a) = it.next() {
        if literal || !a.starts_with('-') || a == "-" {
            parsed.paths.push(a);
            continue;
        }
        match a.as_str() {
            "--" => literal = true,
            "-n" | "--dry-run" => parsed.opts.dry_run = true,
            "--no-backup" => parsed.opts.backup = false,
            "-r" | "--recursive" => parsed.recursive = true,
            "--migrate-epub3" => parsed.opts.migrate_epub3 = true,
            "--keep-version" => parsed.opts.keep_version = true,
            "--keep-missing-images" => parsed.opts.keep_missing_images = true,
            "--preserve-presentation" => {
                parsed.opts.presentation = epubfix::Presentation::Preserve;
            }
            a if a.starts_with("--language-detect") => {
                let mode = a.strip_prefix("--language-detect=").unwrap_or("");
                parsed.opts.language = match mode {
                    "en-only" => epubfix::LanguagePolicy::EnglishOnly,
                    "any" => epubfix::LanguagePolicy::Any,
                    "off" => epubfix::LanguagePolicy::Off,
                    other => {
                        return Err(format!(
                            "--language-detect must be en-only, any or off (got \"{other}\")"
                        ));
                    }
                };
            }
            "--pause" => parsed.pause = Some(true),
            "--no-pause" => parsed.pause = Some(false),
            "-l" | "--list" => {
                list_fixers();
                return Ok(None);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("epubfix {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--only" => {
                let v = it.next().ok_or("--only needs a comma-separated list")?;
                parsed.opts.only.extend(split_names(&v));
            }
            _ => match a.strip_prefix("--only=") {
                Some(v) => parsed.opts.only.extend(split_names(v)),
                None => return Err(format!("unknown option `{a}` (try --help)")),
            },
        }
    }

    let known: Vec<&str> = fixers::all(&Options::default())
        .iter()
        .map(|f| f.name())
        .collect();
    if let Some(bad) = parsed
        .opts
        .only
        .iter()
        .find(|n| !known.contains(&n.as_str()))
    {
        return Err(format!("unknown fixer `{bad}` (try --list)"));
    }

    Ok(Some(parsed))
}

fn split_names(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn list_fixers() {
    let all = fixers::all(&Options::default());
    let width = all.iter().map(|f| f.name().len()).max().unwrap_or(0);
    println!("Available fixers:\n");
    for f in &all {
        println!(
            "  {:<width$}  {:<18}  {}",
            f.name(),
            f.codes().join(", "),
            f.description()
        );
    }
}

/// Where to look when the user gave no paths: the folder holding the executable,
/// falling back to the working directory if that cannot be determined.
/// Report one book, and say whether it was changed.
///
/// What is still wrong is said right where the work is reported, so a long
/// sweep cannot leave the impression that a book came out in good order when
/// it did not.
fn report_book(name: &str, outcome: &epubfix::Outcome, dry_run: bool) -> bool {
    let left = match outcome.remaining.len() {
        0 => String::new(),
        1 => "1 problem still remains".to_string(),
        n => format!("{n} problems still remain"),
    };
    if outcome.has_changes() {
        let label = if dry_run { " (dry run)" } else { "" };
        println!("{name}:{label}\n    {}", outcome.changes.join("\n    "));
        if !left.is_empty() {
            println!("    ...but {left}.");
        }
        return true;
    }
    if left.is_empty() {
        println!("{name}: nothing to do");
    } else {
        println!("{name}: nothing I can fix, and {left}.");
    }
    false
}

/// Print a per-book list, skipping books with nothing in it.
fn report_section(heading: &str, entries: &[(String, Vec<String>)]) {
    if entries.iter().all(|(_, lines)| lines.is_empty()) {
        return;
    }
    println!("\n{heading}");
    for (name, lines) in entries.iter().filter(|(_, l)| !l.is_empty()) {
        println!("  {name}");
        for line in lines {
            println!("      {line}");
        }
    }
}

/// True if one of `findings` is already about the same file as `residual`.
///
/// The final scan and the fixers overlap by design — `dangling-resources`
/// declines to delete an `<img>` and reports it, and the scan then sees the
/// link it left behind. Both are right; printing both is the same defect twice.
/// Matching on the last path segment is enough, since the two describe it
/// differently ("images/logo.jpg" against "OEBPS/images/logo.jpg").
fn already_explained(residual: &str, findings: &[String]) -> bool {
    residual
        .split('"')
        .nth(1)
        .map(|target| target.rsplit('/').next().unwrap_or(target))
        .is_some_and(|leaf| !leaf.is_empty() && findings.iter().any(|f| f.contains(leaf)))
}

fn default_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Repair one book, turning a crash into an ordinary failure.
///
/// A bug that panics on one book must not take the rest of a library sweep with
/// it, which is what happened on a real one: a comment inside `<metadata>` was
/// enough to abort a whole run. Every repair happens in memory and the archive
/// is written only at the end, through a temp file and an atomic replace, so a
/// panic leaves the book exactly as it was found — all this has to clean up is
/// a temp file that never got moved into place.
///
/// This is why the release profile does not set `panic = "abort"`.
fn fix_file_guarded(path: &Path, opts: &Options) -> Result<epubfix::Outcome, String> {
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fix_file(path, opts)));
    match attempt {
        Ok(Ok(outcome)) => Ok(outcome),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            let mut tmp = path.as_os_str().to_owned();
            tmp.push(".epubfix.tmp");
            let _ = std::fs::remove_file(PathBuf::from(tmp));
            Err("internal error — see the panic above; the book was not modified, and this is \
                 a bug worth reporting"
                .to_string())
        }
    }
}

fn main() -> ExitCode {
    let args = match parse_args(env::args().skip(1).collect()) {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("epubfix: {e}");
            return ExitCode::from(2);
        }
    };

    let scanned = args.paths.is_empty();
    let mut targets: Vec<PathBuf> = Vec::new();
    if scanned {
        let dir = default_dir();
        println!("Scanning {} ...", dir.display());
        targets = collect_epubs(&dir, args.recursive);
    } else {
        for a in &args.paths {
            let p = PathBuf::from(a);
            if p.is_dir() {
                targets.extend(collect_epubs(&p, args.recursive));
            } else {
                targets.push(p);
            }
        }
    }

    if targets.is_empty() {
        println!("No .epub files found.");
    }

    let (mut fixed, mut clean, mut failed) = (0u32, 0u32, 0u32);
    let mut attention: Vec<(String, Vec<String>)> = Vec::new();
    let mut unresolved: Vec<(String, Vec<String>)> = Vec::new();
    for p in &targets {
        let name = p.file_name().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        match fix_file_guarded(p, &args.opts) {
            Ok(outcome) => {
                match report_book(&name, &outcome, args.opts.dry_run) {
                    true => fixed += 1,
                    false if outcome.remaining.is_empty() => clean += 1,
                    false => {}
                }
                // A residual whose subject a finding already named is the same
                // defect said twice, so only the unexplained ones are listed.
                let unexplained: Vec<String> = outcome
                    .remaining
                    .iter()
                    .filter(|r| !already_explained(r, &outcome.findings))
                    .cloned()
                    .collect();
                if !outcome.findings.is_empty() {
                    attention.push((name.clone(), outcome.findings));
                }
                if !outcome.remaining.is_empty() {
                    unresolved.push((name, unexplained));
                }
            }
            Err(e) => {
                eprintln!("{name}: FAILED ({e})");
                failed += 1;
            }
        }
    }

    report_section("Needs manual attention:", &attention);
    report_section(
        "Still wrong afterwards, and nothing above explains it:",
        &unresolved,
    );

    let verb = if args.opts.dry_run {
        "would fix"
    } else {
        "fixed"
    };
    print!("\nDone: {fixed} {verb}, {clean} nothing to do, {failed} failed");
    if !attention.is_empty() {
        print!(", {} needing a look", attention.len());
    }
    if unresolved.is_empty() {
        println!(".");
    } else {
        println!(", {} not fully repaired.", unresolved.len());
        // Said once, at the end, because the counts above invite exactly the
        // wrong inference: epubfix looks for a handful of things, and a book it
        // has nothing to say about is not thereby a valid EPUB.
        println!("epubfix checks far less than EPUB Check does — run that for the real answer.");
    }

    // Launched by double-click, the console closes the moment we return.
    let pause = args
        .pause
        .unwrap_or(scanned && cfg!(windows) && io::stdin().is_terminal());
    if pause {
        print!("Press Enter to exit...");
        io::stdout().flush().ok();
        io::stdin().read_line(&mut String::new()).ok();
    }

    // Distinct codes so a library sweep can be scripted: 1 means something
    // broke, 3 means everything worked but some books want a human.
    if failed > 0 {
        ExitCode::FAILURE
    } else if attention.is_empty() && unresolved.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}
